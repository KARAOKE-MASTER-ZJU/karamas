//! DLNA 设备类型和播放、进度、音量控制接口。
use rupnp::{Device, http::Uri, ssdp::URN};
use std::{collections::HashMap, net::IpAddr};

mod discovery;
mod soap;
mod ssdp_discovery;

use soap::{avtransport_action_compat, log_upnp_action, timeout_soap};

/// 同一次 GetPositionInfo 响应中的媒体地址和进度。
#[derive(Debug)]
pub(crate) struct PositionInfo {
    pub track_uri: Option<String>,
    pub current_secs: Option<u32>,
    pub total_secs: u32,
}

fn xml_escape(s: &str) -> String {
    // Minimal XML escaping for element text nodes.
    // (Enough to keep SOAP XML well-formed when URLs contain & and friends.)
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// 将设备返回的时长换算为整秒，小数部分直接舍去。
pub(crate) fn parse_time_secs(value: &str) -> Option<u32> {
    let parts: Vec<_> = value.trim().split(':').collect();
    let (hours, minutes, seconds) = match parts.as_slice() {
        [hours, minutes, seconds] => (*hours, *minutes, *seconds),
        [minutes, seconds] => ("0", *minutes, *seconds),
        [seconds] => ("0", "0", *seconds),
        _ => return None,
    };
    let parse_integer = |part: &str| {
        if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        part.parse::<u32>().ok()
    };
    let seconds = if let Some((whole, fraction)) = seconds.split_once('.') {
        if fraction.is_empty() || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        whole
    } else {
        seconds
    };
    let hours = parse_integer(hours)?;
    let minutes = parse_integer(minutes)?;
    let seconds = parse_integer(seconds)?;
    if minutes >= 60 || seconds >= 60 {
        return None;
    }
    hours
        .checked_mul(3600)?
        .checked_add(minutes * 60)?
        .checked_add(seconds)
}

fn build_didl_lite_metadata(title: &str, media_url: &str, protocol_info: Option<&str>) -> String {
    xml_escape(&didl_lite_metadata(title, media_url, protocol_info))
}

pub(crate) fn didl_lite_metadata(
    title: &str,
    media_url: &str,
    protocol_info: Option<&str>,
) -> String {
    // Build a minimal DIDL-Lite and then XML-escape it for embedding into <CurrentURIMetaData>.
    // Many renderers require at least: upnp:class + res@protocolInfo.
    // NOTE: avoid strict DLNA.ORG_PN profile binding; some renderers reject when profile ≠ actual.
    // Start permissive, then tighten if needed.
    let protocol = protocol_info.unwrap_or("http-get:*:video/mp4:*");

    // Important: the <res> inner URL should be XML-escaped *once* (so & -> &amp;).
    let res_url = xml_escape(media_url);

    format!(
        r#"<DIDL-Lite xmlns="urn:schemas-upnp-org:metadata-1-0/DIDL-Lite/" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:upnp="urn:schemas-upnp-org:metadata-1-0/upnp/">
        <item id="0" parentID="-1" restricted="1">
        <dc:title>{}</dc:title>
        <upnp:storageMedium>UNKNOWN</upnp:storageMedium>
        <upnp:writeStatus>UNKNOWN</upnp:writeStatus>
        <res protocolInfo="{}">{}</res>
        <upnp:class>object.item.videoItem</upnp:class>
        </item>
        </DIDL-Lite>"#,
        xml_escape(title),
        protocol,
        res_url
    )
}

fn device_location_uri(device: &DlnaDevice) -> Result<Uri, rupnp::Error> {
    device
        .location
        .parse::<Uri>()
        .map_err(|_| rupnp::Error::ParseError("无法解析设备location为Uri"))
}

// AVTransport服务URN
const AV_TRANSPORT: URN = URN::service("schemas-upnp-org", "AVTransport", 1);

// DLNA设备信息
#[derive(Debug, Clone)]
pub struct DlnaDevice {
    pub device: Device,
    pub friendly_name: String,
    pub location: String,
    pub services: Vec<URN>,
}

#[derive(Clone)]
pub struct DlnaController;

impl DlnaController {
    pub fn new() -> Self {
        Self
    }

    fn get_avtransport_service<'a>(&'a self, device: &'a DlnaDevice) -> Option<&'a rupnp::Service> {
        device
            .device
            .services()
            .iter()
            .find(|s| *s.service_type() == AV_TRANSPORT)
    }

    // 设置AVTransport URI（发送媒体URL给设备）
    pub async fn set_avtransport_uri(
        &self,
        device: &DlnaDevice,
        current_uri: &str,
        current_uri_metadata: &str,
        server_ip: IpAddr,
        server_port: u16,
    ) -> Result<(), rupnp::Error> {
        let media_url = format!("http://{}:{}/{}", server_ip, server_port, current_uri);
        self.set_avtransport_url(device, &media_url, current_uri_metadata)
            .await
    }

    /// Send an absolute media URL unchanged, without wrapping it in the proxy.
    pub async fn set_avtransport_url(
        &self,
        device: &DlnaDevice,
        current_uri: &str,
        current_uri_metadata: &str,
    ) -> Result<(), rupnp::Error> {
        let avtransport = self
            .get_avtransport_service(device)
            .ok_or(rupnp::Error::ParseError("设备不支持AVTransport服务"))?;

        let media_url = current_uri.to_owned();

        log::info!("设置媒体URI: {}", media_url);
        log::debug!("元数据(传入): {}", current_uri_metadata);

        // If caller didn't provide metadata, generate a minimal DIDL-Lite for compatibility.
        let metadata = if current_uri_metadata.trim().is_empty() {
            // Title can be anything; devices often only care about protocolInfo.
            let protocol_info = if current_uri.contains(".m3u8") {
                log::info!("检测到HLS流，使用宽松的协议描述以提高兼容性");
                Some("http-get:*:application/vnd.apple.mpegurl:*")
            } else {
                None
            };
            log::info!("没有提供元数据，正在生成默认的DIDL-Lite元数据...");
            build_didl_lite_metadata(current_uri, &media_url, protocol_info)
        } else {
            current_uri_metadata.to_string()
        };

        // 准备SOAP请求参数 - 只使用标准参数以提高兼容性
        let action = "SetAVTransportURI";
        let args_str = format!(
            "<InstanceID>0</InstanceID><CurrentURI>{}</CurrentURI><CurrentURIMetaData>{}</CurrentURIMetaData>",
            xml_escape(&media_url),
            metadata
        );

        // 发送SOAP请求 - 统一使用设备描述文档URL(location)作为base url
        let base_url = device_location_uri(device)?;
        log_upnp_action(avtransport, &base_url, action, &args_str);
        let response =
            avtransport_action_compat(avtransport, &base_url, action, &args_str, true).await?;

        log::debug!("SetAVTransportURI响应: {:?}", response);

        Ok(())
    }

    // 设置下一个AVTransport URI（用于播放列表）
    pub async fn set_next_avtransport_uri(
        &self,
        device: &DlnaDevice,
        next_uri: &str,
        next_uri_metadata: &str,
        server_ip: IpAddr,
        server_port: u16,
    ) -> Result<(), rupnp::Error> {
        let avtransport = self
            .get_avtransport_service(device)
            .ok_or(rupnp::Error::ParseError("设备不支持AVTransport服务"))?;

        let action = "SetNextAVTransportURI";
        let media_url = format!("http://{}:{}/{}", server_ip, server_port, next_uri);
        let metadata = if next_uri_metadata.trim().is_empty() {
            build_didl_lite_metadata(next_uri, &media_url, None)
        } else {
            next_uri_metadata.to_string()
        };

        let args_str = format!(
            "<InstanceID>0</InstanceID><NextURI>{}</NextURI><NextURIMetaData>{}</NextURIMetaData>",
            xml_escape(&media_url),
            metadata
        );

        let base_url = device_location_uri(device)?;
        log_upnp_action(avtransport, &base_url, action, &args_str);
        let response =
            avtransport_action_compat(avtransport, &base_url, action, &args_str, false).await?;

        log::debug!("SetNextAVTransportURI响应: {:?}", response);

        Ok(())
    }

    // 播放媒体
    pub async fn play(&self, device: &DlnaDevice) -> Result<(), rupnp::Error> {
        self.play_impl(device, false).await
    }

    pub(crate) async fn play_for_cast(&self, device: &DlnaDevice) -> Result<(), rupnp::Error> {
        self.play_impl(device, true).await
    }

    async fn play_impl(
        &self,
        device: &DlnaDevice,
        retry_timeout: bool,
    ) -> Result<(), rupnp::Error> {
        let avtransport = self
            .get_avtransport_service(device)
            .ok_or(rupnp::Error::ParseError("设备不支持AVTransport服务"))?;

        log::info!("正在发送Play指令...");
        let action = "Play";
        let args_str = "<InstanceID>0</InstanceID><Speed>1</Speed>";

        let base_url = device_location_uri(device)?;
        log_upnp_action(avtransport, &base_url, action, args_str);
        let response =
            avtransport_action_compat(avtransport, &base_url, action, args_str, retry_timeout)
                .await?;
        log::debug!("Play响应: {:?}", response);

        Ok(())
    }

    // 暂停播放
    pub async fn pause(&self, device: &DlnaDevice) -> Result<(), rupnp::Error> {
        let avtransport = self
            .get_avtransport_service(device)
            .ok_or(rupnp::Error::ParseError("设备不支持AVTransport服务"))?;

        log::info!("正在发送Pause指令...");
        let action = "Pause";
        let args_str = "<InstanceID>0</InstanceID>";

        let base_url = device_location_uri(device)?;
        log_upnp_action(avtransport, &base_url, action, args_str);
        let response =
            avtransport_action_compat(avtransport, &base_url, action, args_str, false).await?;
        log::debug!("Pause响应: {:?}", response);

        Ok(())
    }

    // 停止播放
    pub async fn stop(&self, device: &DlnaDevice) -> Result<(), rupnp::Error> {
        let avtransport = self
            .get_avtransport_service(device)
            .ok_or(rupnp::Error::ParseError("设备不支持AVTransport服务"))?;

        log::info!("正在发送Stop指令...");
        let action = "Stop";
        let args_str = "<InstanceID>0</InstanceID>";

        let base_url = device_location_uri(device)?;
        log_upnp_action(avtransport, &base_url, action, args_str);
        let response =
            avtransport_action_compat(avtransport, &base_url, action, args_str, false).await?;
        log::debug!("Stop响应: {:?}", response);

        Ok(())
    }

    // 下一首
    pub async fn next(&self, device: &DlnaDevice) -> Result<(), rupnp::Error> {
        let avtransport = self
            .get_avtransport_service(device)
            .ok_or(rupnp::Error::ParseError("设备不支持AVTransport服务"))?;

        let action = "Next";
        let args_str = "<InstanceID>0</InstanceID>";

        let base_url = device_location_uri(device)?;
        log_upnp_action(avtransport, &base_url, action, args_str);
        let response =
            avtransport_action_compat(avtransport, &base_url, action, args_str, false).await?;
        log::debug!("Next响应: {:?}", response);

        Ok(())
    }

    // 获取传输信息
    pub async fn get_transport_info(&self, device: &DlnaDevice) -> Result<(), rupnp::Error> {
        let avtransport = self
            .get_avtransport_service(device)
            .ok_or(rupnp::Error::ParseError("设备不支持AVTransport服务"))?;

        let action = "GetTransportInfo";
        let args_str = "<InstanceID>0</InstanceID>";

        let base_url = device_location_uri(device)?;
        log_upnp_action(avtransport, &base_url, action, args_str);
        let response =
            avtransport_action_compat(avtransport, &base_url, action, args_str, false).await?;
        log::trace!("传输信息: {:?}", response);

        Ok(())
    }

    // 获取位置信息
    pub async fn get_position_info(
        &self,
        device: &DlnaDevice,
    ) -> Result<HashMap<String, String>, rupnp::Error> {
        let avtransport = self
            .get_avtransport_service(device)
            .ok_or(rupnp::Error::ParseError("设备不支持AVTransport服务"))?;

        let action = "GetPositionInfo";
        let args_str = "<InstanceID>0</InstanceID>";

        let base_url = device_location_uri(device)?;
        log_upnp_action(avtransport, &base_url, action, args_str);

        // 获取响应
        let response =
            avtransport_action_compat(avtransport, &base_url, action, args_str, false).await?;

        Ok(response)
    }

    /// 获取当前播放进度，返回 (当前时间秒, 总时长秒)；总时长为 0 表示未知。
    pub async fn get_secs(&self, device: &DlnaDevice) -> Result<(u32, u32), rupnp::Error> {
        let position = self.get_progress_update(device).await?;
        let current = position.current_secs.ok_or(rupnp::Error::ParseError(
            "设备未返回有效的 RelTime 播放进度",
        ))?;
        Ok((current, position.total_secs))
    }

    /// 分别解析位置和时长，缺少当前位置时仍保留可用的总时长。
    pub(crate) async fn get_progress_update(
        &self,
        device: &DlnaDevice,
    ) -> Result<PositionInfo, rupnp::Error> {
        let mut position_info = self.get_position_info(device).await?;
        let track_uri = position_info.remove("TrackURI");
        let current_secs = position_info
            .get("RelTime")
            .and_then(|value| parse_time_secs(value));
        let total_secs = position_info
            .get("TrackDuration")
            .and_then(|value| parse_time_secs(value))
            .unwrap_or(0);
        log::trace!(
            "get_progress_update() : track_uri={:?}, current_secs={:?}, total_secs={}",
            track_uri,
            current_secs,
            total_secs
        );
        Ok(PositionInfo {
            track_uri,
            current_secs,
            total_secs,
        })
    }

    // 设置播放进度（跳转到指定秒数）
    pub async fn seek(&self, device: &DlnaDevice, seconds: u32) -> Result<(), rupnp::Error> {
        let avtransport = self
            .get_avtransport_service(device)
            .ok_or(rupnp::Error::ParseError("设备不支持AVTransport服务"))?;

        // 将秒数转换为 HH:MM:SS 格式
        let hours = seconds / 3600;
        let minutes = (seconds % 3600) / 60;
        let secs = seconds % 60;
        let target_time = format!("{:02}:{:02}:{:02}", hours, minutes, secs);

        log::info!("正在发送Seek指令，跳转至: {}", target_time);

        // Unit 常用选项:
        // REL_TIME: 按照时间进度跳转 (最常用)
        // TRACK_NR: 按照轨道编号跳转
        let action = "Seek";
        let args_str = format!(
            "<InstanceID>0</InstanceID><Unit>REL_TIME</Unit><Target>{}</Target>",
            target_time
        );

        let base_url = device_location_uri(device)?;
        log_upnp_action(avtransport, &base_url, action, &args_str);

        // 发送请求
        let response =
            avtransport_action_compat(avtransport, &base_url, action, &args_str, false).await?;
        log::info!("Seek响应: {:?}", response);

        Ok(())
    }

    // 设置渲染器音量
    pub async fn set_volume(&self, device: &DlnaDevice, volume: u32) -> Result<(), rupnp::Error> {
        let rendering_control = device
            .device
            .services()
            .iter()
            .find(|s| *s.service_type() == URN::service("schemas-upnp-org", "RenderingControl", 1))
            .ok_or(rupnp::Error::ParseError("设备不支持RenderingControl服务"))?;

        let action = "SetVolume";
        let args_str = format!(
            r#"
            <InstanceID>0</InstanceID>
            <Channel>Master</Channel>
            <DesiredVolume>{}</DesiredVolume>
            "#,
            volume
        );

        let base_url = device_location_uri(device)?;
        // RenderingControl uses a different service; still log with a reasonable SOAPAction.
        log::debug!(
            "UPnP Action -> base_url={} service_id={} service_type={} SOAPAction=\"urn:schemas-upnp-org:service:RenderingControl:1#{}\"",
            base_url,
            rendering_control.service_id(),
            rendering_control.service_type(),
            action
        );
        log::debug!(
            "UPnP Action body (approx) => {}",
            format_args!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
                <s:Envelope s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/" xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">
                <s:Body>
                    <u:{action} xmlns:u="urn:schemas-upnp-org:service:RenderingControl:1">{args}</u:{action}>
                </s:Body>
                </s:Envelope>"#,
                action = action,
                args = args_str
            )
        );

        let response = timeout_soap(rendering_control.action(&base_url, action, &args_str)).await?;
        log::debug!("SetVolume响应: {:?}", response);

        Ok(())
    }

    /// 获取渲染器音量；字段缺失或无法解析时返回 None。
    pub async fn get_volume(&self, device: &DlnaDevice) -> Result<Option<u32>, rupnp::Error> {
        let rendering_control = device
            .device
            .services()
            .iter()
            .find(|s| *s.service_type() == URN::service("schemas-upnp-org", "RenderingControl", 1))
            .ok_or(rupnp::Error::ParseError("设备不支持RenderingControl服务"))?;

        let action = "GetVolume";
        let args_str = r#"
            <InstanceID>0</InstanceID>
            <Channel>Master</Channel>
            "#;

        let base_url = device_location_uri(device)?;
        log::trace!(
            "UPnP Action -> base_url={} service_id={} service_type={} SOAPAction=\"urn:schemas-upnp-org:service:RenderingControl:1#{}\"",
            base_url,
            rendering_control.service_id(),
            rendering_control.service_type(),
            action
        );
        log::trace!(
            "UPnP Action body (approx) => {}",
            format_args!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
                <s:Envelope s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/" xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">
                <s:Body>
                    <u:{action} xmlns:u="urn:schemas-upnp-org:service:RenderingControl:1">{args}</u:{action}>
                </s:Body>
                </s:Envelope>"#,
                action = action,
                args = args_str
            )
        );

        let response = timeout_soap(rendering_control.action(&base_url, action, args_str)).await?;

        Ok(response
            .get("CurrentVolume")
            .and_then(|value| value.parse().ok()))
    }
}
