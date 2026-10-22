use chrono::{NaiveTime, Timelike};
use futures::future::join_all;
use futures::stream::StreamExt;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use rupnp::Device;
use rupnp::http::Uri;
use rupnp::ssdp::URN;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::net::IpAddr;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tokio::time::timeout;

mod ssdp_discovery;

/// 兼容模式的单次 SOAP 请求超时。
const SOAP_TIMEOUT_COMPAT: Duration = Duration::from_millis(1500);

/// 设备发现/连接超时。SSDP/HTTP 描述文件获取不应阻塞过久。
const DEVICE_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(3);

/// 全局复用的 SOAP HTTP 客户端：连接池复用 + 统一超时。
/// 修复前：每次 action 都新建 reqwest Client（新 TCP 连接），配合 1s 一次的
/// GetPositionInfo 轮询，等于每秒向设备开一条新连接，长时间运行会打崩设备
/// 的 UPnP 服务（导致其停止响应）。
fn soap_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .no_proxy()
            .timeout(crate::config::SOAP_TIMEOUT)
            .pool_idle_timeout(Duration::from_secs(30))
            .build()
            .expect("创建 SOAP HTTP 客户端失败")
    })
}

/// 给 rupnp action future 加超时。设备不响应时返回超时错误，而不是永久等待。
async fn timeout_soap<T>(fut: impl Future<Output = Result<T, rupnp::Error>>) -> Result<T, rupnp::Error> {
    match timeout(crate::config::SOAP_TIMEOUT, fut).await {
        Ok(r) => r,
        Err(_) => Err(rupnp::Error::ParseError("SOAP 调用超时")),
    }
}

fn extract_xml_tag_value(xml: &str, tag: &str) -> Option<String> {
    // 解析XML标签值，支持带命名空间属性的标签
    let start_pattern = format!("<{}", tag);
    let end_pattern = format!("</{}>", tag);

    // 找到开始标签
    if let Some(start_idx) = xml.find(&start_pattern) {
        // 找到开始标签的结束位置 >
        if let Some(tag_end_idx) = xml[start_idx..].find('>') {
            let value_start = start_idx + tag_end_idx + 1;

            // 找到对应的结束标签
            if let Some(value_end) = xml[value_start..].find(&end_pattern) {
                let value = &xml[value_start..value_start + value_end];
                return Some(value.trim().to_string());
            }
        }
    }

    None
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

fn build_didl_lite_metadata(title: &str, media_url: &str, protocol_info: Option<&str>) -> String {
    // Build a minimal DIDL-Lite and then XML-escape it for embedding into <CurrentURIMetaData>.
    // Many renderers require at least: upnp:class + res@protocolInfo.
    // NOTE: avoid strict DLNA.ORG_PN profile binding; some renderers reject when profile ≠ actual.
    // Start permissive, then tighten if needed.
    let protocol = protocol_info.unwrap_or("http-get:*:video/mp4:*");

    // Important: the <res> inner URL should be XML-escaped *once* (so & -> &amp;).
    let res_url = xml_escape(media_url);

    let didl = format!(
        r#"<DIDL-Lite xmlns=\"urn:schemas-upnp-org:metadata-1-0/DIDL-Lite/\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:upnp=\"urn:schemas-upnp-org:metadata-1-0/upnp/\">
        <item id=\"0\" parentID=\"-1\" restricted=\"1\">
        <dc:title>{}</dc:title>
        <upnp:storageMedium>UNKNOWN</upnp:storageMedium>
        <upnp:writeStatus>UNKNOWN</upnp:writeStatus>
        <res protocolInfo=\"{}\">{}</res>
        <upnp:class>object.item.videoItem</upnp:class>
        </item>
        </DIDL-Lite>"#,
        xml_escape(title),
        protocol,
        res_url
    );

    // Embed metadata as escaped XML text nodes: <CurrentURIMetaData>&lt;DIDL-Lite ...&gt;...
    xml_escape(&didl)
}

fn build_soap_envelope(action: &str, args_xml: &str) -> String {
    // Keep the shape consistent with what most renderers accept (and close to your B站抓包).
    // Note: `rupnp` will build its own envelope too, but we log a best-effort equivalent
    // so you can diff with a packet capture.
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<s:Envelope s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/" xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">
  <s:Body>
        <u:{action} xmlns:u="urn:schemas-upnp-org:service:AVTransport:1">{args}</u:{action}>
  </s:Body>
</s:Envelope>"#,
        action = action,
        args = args_xml
    )
}

fn device_location_uri(device: &DlnaDevice) -> Result<Uri, rupnp::Error> {
    device
        .location
        .parse::<Uri>()
        .map_err(|_| rupnp::Error::ParseError("无法解析设备location为Uri"))
}

fn log_upnp_action(service: &rupnp::Service, base_url: &Uri, action: &str, args_xml: &str) {
    // `service.action()` internally ends up using control_url(base_url).
    // We can't call the private control_url() here, so we log the base_url and also log
    // the serviceId/type to help confirm we matched the expected service.
    //
    // If you still see 204, compare this log with your抓包，重点确认：Host/port/path。
    let soap_action_header = format!("\"urn:schemas-upnp-org:service:AVTransport:1#{}\"", action);

    // Logged body is a best-effort “wire-like” payload for diffing.
    let envelope = build_soap_envelope(action, args_xml);

    log::debug!(
        "UPnP Action -> base_url={} service_id={} service_type={} SOAPAction={}",
        base_url,
        service.service_id(),
        service.service_type(),
        soap_action_header
    );
    log::debug!("UPnP Action body (approx) => {}", envelope);
}

/// 从 SOAP 响应 XML 中解析本项目关心的字段。
fn parse_soap_response_fields(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for k in [
        "Track",
        "TrackDuration",
        "TrackMetaData",
        "TrackURI",
        "RelTime",
        "AbsTime",
        "RelCount",
        "AbsCount",
    ] {
        if let Some(v) = extract_xml_tag_value(text, k) {
            log::debug!("提取到字段 '{}' 的值: '{}'", k, v);
            out.insert(k.to_string(), v);
        }
    }
    log::debug!("解析后的响应字段: {:?}", out);
    out
}

fn soap_http_failure(status: reqwest::StatusCode, text: &str) -> String {
    let mut summary = format!("HTTP={}", status.as_u16());
    if let Ok(document) = roxmltree::Document::parse(text) {
        if let Some(fault) = document.descendants().find(|node| node.has_tag_name("Fault")) {
            let field = |name| fault.descendants()
                .find(|node| node.has_tag_name(name))
                .and_then(|node| node.text())
                .map(str::trim);
            let code = field("errorCode").unwrap_or("未知");
            let description = field("errorDescription")
                .or_else(|| field("faultstring"))
                .unwrap_or("未提供说明");
            summary.push_str(&format!("，SOAP错误={} {}", code, description));
        }
    }
    if text.trim().is_empty() {
        summary.push_str("，响应体为空");
    }
    summary
}

fn soap_request_failure(action: &str, url: &str, stage: &str, detail: impl std::fmt::Display) -> rupnp::Error {
    let message = format!("[DLNA] {}失败：步骤={}，端点={}，{}", action, stage, url, detail);
    log::warn!("{}", message);
    rupnp::Error::invalid_response(std::io::Error::other(message))
}

/// 用全局共享 client（连接池复用 + 统一超时）发送一次 SOAP 请求到指定 URL。
/// 这是主要 SOAP 通道：避免 rupnp 原生 action 每次新建 TCP 连接、每秒打一次设备，
/// 长时间运行把设备 UPnP 服务打崩。
async fn send_soap_shared(
    final_url: &str,
    action: &str,
    args_xml: &str,
    timeout: Duration,
    retry_timeout: bool,
) -> Result<HashMap<String, String>, rupnp::Error> {
    let soap_action_header =
        format!("\"urn:schemas-upnp-org:service:AVTransport:1#{}\"", action);
    let body = build_soap_envelope(action, args_xml);

    log::debug!("UPnP Action (shared) -> url={} SOAPAction={}", final_url, soap_action_header);

    let mut headers = HeaderMap::new();
    headers.insert(
        "SOAPAction",
        HeaderValue::from_str(&soap_action_header)
            .map_err(|_| rupnp::Error::ParseError("SOAPAction header非法"))?,
    );
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/xml; charset=\"utf-8\""),
    );

    let send = || async {
        soap_client()
            .post(final_url)
            .timeout(timeout)
            .headers(headers.clone())
            .body(body.clone())
            .send()
            .await
    };
    let resp = if retry_timeout {
        let context = format!("[DLNA] 发送 {}：端点={}", action, final_url);
        crate::cast::request::retry_cast_request(&context, send).await
    } else {
        send().await
    }
    .map_err(|error| {
        let detail = if error.is_timeout() {
            "等待响应头超时".to_owned()
        } else {
            error.to_string()
        };
        soap_request_failure(action, final_url, "发送SOAP请求", detail)
    })?;
    let status = resp.status();
    let text = resp.text().await.map_err(|error| {
        soap_request_failure(action, final_url, "读取SOAP响应", format!("HTTP={}，错误={}", status.as_u16(), error))
    })?;

    if status.as_u16() == 200 {
        // GetPositionInfo is polled roughly once per second by the app. A successful
        // SOAP round-trip is useful only at DEBUG level; INFO must stay available for
        // playback, muxing and failure diagnostics.
        log::debug!(
            "UPnP Action (shared) succeeded: action={}, url={}",
            action,
            final_url
        );
        log::debug!("UPnP Action (shared) status=200 body={}", text);
        Ok(parse_soap_response_fields(&text))
    } else {
        let message = format!("[DLNA] {}失败：步骤=设备响应，端点={}，{}", action, final_url, soap_http_failure(status, &text));
        log::warn!("{}，响应体={}", message, text);
        Err(rupnp::Error::invalid_response(std::io::Error::other(message)))
    }
}

/// Some renderers publish a `controlURL` like `_urn:schemas-upnp-org:service:AVTransport_control`
/// (missing the leading `/`). In practice the working endpoint is often `/_urn:...`.
///
/// `rupnp`'s internal URL replacement may produce the wrong path for such devices.
/// To make behavior explicit (and loggable), we send the SOAP request ourselves to:
/// `{scheme}://{host}:{port}/{control_path}`。
///
/// 发送顺序（都带超时，设备不响应时优雅降级而不是无限等待）：
///   1. 首选：从 Service Debug 提取 control endpoint，用共享 client 发送（连接池复用）
///   2. 兜底：rupnp 原生 action（Debug 解析失败的设备；每次新建连接，尽量少走）
///   3. 最后：常见 control 路径遍历（共享 client，短超时）
async fn avtransport_action_compat(
    service: &rupnp::Service,
    base_url: &Uri,
    action: &str,
    args_xml: &str,
    retry_timeout: bool,
) -> Result<HashMap<String, String>, rupnp::Error> {
    let host = base_url
        .host()
        .ok_or(rupnp::Error::ParseError("base_url缺少host"))?
        .to_string();
    let scheme = base_url
        .scheme_str()
        .ok_or(rupnp::Error::ParseError("base_url缺少scheme"))?;
    let port = base_url
        .port_u16()
        .unwrap_or(if scheme == "https" { 443 } else { 80 });

    let service_debug = format!("{:?}", service);
    let mut first_failure = None;
    let mut primary_endpoint = None;

    // 1) 首选：Debug 提取 control endpoint，走共享 client（连接复用，避免每秒新建连接打崩设备）
    if let Some(path) = extract_control_endpoint_from_debug(&service_debug) {
        let p = normalize_control_path(&path);
        let final_url = if p.starts_with("http://") || p.starts_with("https://") {
            p
        } else {
            format!("{}://{}:{}{}", scheme, host, port, p)
        };
        primary_endpoint = Some(final_url.clone());
        match send_soap_shared(&final_url, action, args_xml, crate::config::SOAP_TIMEOUT, retry_timeout).await {
            Ok(response) => return Ok(response),
            Err(error) => {
                first_failure = Some(error.to_string());
                log::debug!("[DLNA] {}首选端点失败，回退至原生接口：端点={}", action, final_url);
            }
        }
    }

    // 2) 兜底：rupnp 原生 action（每次新建连接，仅用于 Debug 解析失败/路径不匹配的设备）
    match timeout_soap(service.action(base_url, action, args_xml)).await {
        Ok(response) => {
            log::debug!("UPnP Action (native) succeeded");
            log::debug!("UPnP Action (native) response: {:?}", response);
            return Ok(response);
        }
        Err(e) => {
            let endpoint = primary_endpoint.unwrap_or_else(|| base_url.to_string());
            let message = format!("[DLNA] {}原生接口失败：端点={}，错误={}，继续尝试候选路径", action, endpoint, e);
            log::warn!("{}", message);
            first_failure.get_or_insert(message);
        }
    }

    // 3) 最后兜底：常见 control 路径遍历（共享 client，短超时）
    log::debug!("Service Debug info: {}", service_debug);
    let mut possible_paths: Vec<String> = Vec::new();

    // 尝试从 debug 中解析出真实的控制路径（常见于 Windows UPnP Host）
    if let Some(start) = service_debug.find("/upnphost/udhisapi.dll?control=")
        && let Some(end) = service_debug[start..].find(", event_sub_endpoint")
    {
        let real_path = &service_debug[start..start + end];
        possible_paths.push(normalize_control_path(real_path));
    }

    // 通用回退路径
    possible_paths.extend(
        [
            "_urn:schemas-upnp-org:service:AVTransport_control",
            "AVTransport/control",
            "upnp/control/AVTransport",
            "control/AVTransport",
        ]
        .into_iter()
        .map(normalize_control_path), // 规范化路径,增加/
    );

    for path in possible_paths {
        let final_url = if path.starts_with("http://") || path.starts_with("https://") {
            path
        } else {
            format!("{}://{}:{}{}", scheme, host, port, path)
        };
        match send_soap_shared(&final_url, action, args_xml, SOAP_TIMEOUT_COMPAT, retry_timeout).await {
            Ok(response) => return Ok(response),
            Err(e) => {
                first_failure.get_or_insert_with(|| e.to_string());
                log::debug!("[DLNA] {}回退候选失败：端点={}，错误={}", action, final_url, e);
            }
        }
    }

    // 所有尝试都失败
    let message = format!("[DLNA] {}失败：所有控制端点均失败；首个失败原因={}",
        action, first_failure.as_deref().unwrap_or("没有可用控制端点"));
    log::warn!("{}", message);
    Err(rupnp::Error::invalid_response(std::io::Error::other(message)))
}

fn normalize_control_path(path: &str) -> String {
    let p = path.trim();
    if p.starts_with("http://") || p.starts_with("https://") {
        return p.to_string();
    }
    if p.starts_with('/') {
        p.to_string()
    } else {
        format!("/{}", p)
    }
}

fn extract_control_endpoint_from_debug(service_debug: &str) -> Option<String> {
    if let Some(start) = service_debug.find("control_endpoint: ") {
        let start = start + "control_endpoint: ".len();
        if let Some(end) = service_debug[start..].find(", event_sub_endpoint") {
            let path = service_debug[start..start + end].trim();
            Some(path.to_string())
        } else {
            None
        }
    } else {
        None
    }
}

// AVTransport服务URN
const AV_TRANSPORT: URN = URN::service("schemas-upnp-org", "AVTransport", 1);

// Keep the endpoint useful for LAN diagnosis without exporting URL credentials.
fn discovery_endpoint(raw: &str) -> String {
    match url::Url::parse(raw) {
        Ok(mut url) => {
            let _ = url.set_username("");
            let _ = url.set_password(None);
            url.set_query(None);
            url.set_fragment(None);
            url.to_string()
        }
        Err(_) => "<无效描述地址>".to_owned(),
    }
}

// DLNA设备信息
#[derive(Debug, Clone)]
pub struct DlnaDevice {
    pub device: Device,
    pub friendly_name: String,
    pub location: String,
    pub services: Vec<URN>,
}

async fn collect_discovery_results(
    stream: impl futures::Stream<Item = Result<Device, rupnp::Error>>,
    started: Instant,
) -> Vec<DlnaDevice> {
    futures::pin_mut!(stream);
    let mut devices = Vec::new();
    let mut seen_locations = HashSet::new();
    let (mut events, mut duplicates, mut filtered, mut errors) = (0, 0, 0, 0);
    while let Some(result) = stream.next().await {
        events += 1;
        match result {
            Ok(device) => {
                let location = device.url().to_string();
                if !seen_locations.insert(location.clone()) {
                    duplicates += 1;
                    continue;
                }
                let device_type = device.device_type().to_string();
                if !device_type.contains("MediaRenderer") {
                    filtered += 1;
                    log::info!(target: "DLNA_DISCOVERY", "忽略设备：原因=非MediaRenderer，类型={}，描述地址={}", device_type, discovery_endpoint(&location));
                    continue;
                }
                let friendly_name = device.friendly_name().to_string();
                let services: Vec<URN> = device.services().iter().map(|s| s.service_type().clone()).collect();
                log::info!(target: "DLNA_DISCOVERY", "发现设备：名称={}，描述地址={}，类型={}，服务={:?}", friendly_name, discovery_endpoint(&location), device_type, services);
                devices.push(DlnaDevice { device, friendly_name, location, services });
            }
            Err(error) => {
                errors += 1;
                log::error!(target: "DLNA_DISCOVERY", "发现过程错误：步骤=获取/解析设备描述，错误={}", error);
            }
        }
    }
    log::info!(target: "DLNA_DISCOVERY", "自动搜索结束：结果项={}，可用设备={}，重复={}，过滤={}，错误={}，耗时={}ms", events, devices.len(), duplicates, filtered, errors, started.elapsed().as_millis());
    if events == 0 {
        log::warn!(target: "DLNA_DISCOVERY", "没有设备描述结果：请结合DLNA_SSDP的实际UDP收包与响应拒绝计数；此结果不能单独证明设备离线");
    } else if devices.is_empty() {
        log::warn!(target: "DLNA_DISCOVERY", "没有可用设备：请查看上方过滤原因和发现过程错误");
    }
    devices
}

#[derive(Clone)]
pub struct DlnaController;

impl DlnaController {
    pub fn new() -> Self {
        Self
    }

    // 发现网络中的DLNA渲染器设备
    pub async fn discover_devices(&self) -> Result<Vec<DlnaDevice>, rupnp::Error> {
        let started = Instant::now();
        log::info!(target: "DLNA_DISCOVERY", "自动搜索开始：并行对照AVTransport:1、MediaRenderer:1、ssdp:all，各5s；描述请求单次超时3s、并发8");
        let locations = ssdp_discovery::discover_locations().await?;
        let devices_stream = futures::stream::iter(locations).map(|location| async move {
            let endpoint = discovery_endpoint(&location);
            let started = Instant::now();
            log::info!(target: "DLNA_DISCOVERY", "获取发现的设备描述：地址={}", endpoint);
            let result = async {
                let uri = location.parse::<Uri>()?;
                timeout(DEVICE_DISCOVERY_TIMEOUT, Device::from_url(uri)).await
                    .map_err(|_| rupnp::Error::ParseError("获取设备描述超时"))?
            }.await;
            match &result {
                Ok(_) => log::info!(target: "DLNA_DISCOVERY", "设备描述解析成功：地址={}，耗时={}ms", endpoint, started.elapsed().as_millis()),
                Err(error) => log::error!(target: "DLNA_DISCOVERY", "设备描述获取失败：地址={}，耗时={}ms，错误={}", endpoint, started.elapsed().as_millis(), error),
            }
            result
        }).buffer_unordered(8);
        Ok(collect_discovery_results(devices_stream, started).await)
    }

    pub async fn get_devices_from_urls(
        &self,
        urls: &Vec<&'static str>,
    ) -> Result<Vec<DlnaDevice>, rupnp::Error> {
        let tasks = urls.iter().map(|url| {
            let uri =Uri::from_static(url);
            async move {
                match timeout(DEVICE_DISCOVERY_TIMEOUT, Device::from_url(uri)).await {
                    Ok(Ok(device)) => Some(device), // 成功获取
                    Ok(Err(e)) => {
                        log::error!("解析 URL 失败 [{}]: {}", url, e);
                        None
                    }
                    Err(_) => {
                        log::warn!("请求 URL 超时 [{}], 判定设备不在线", url);
                        None
                    }
                }
            }
        });

        let results = join_all(tasks).await;
        let dlna_devices: Vec<DlnaDevice> = results
            .into_iter()
            .flatten()
            .map(|device| DlnaDevice {
                device: device.clone(),
                friendly_name: device.friendly_name().to_string(),
                location: device.url().to_string(),
                services: device
                    .services()
                    .iter()
                    .map(|s| s.service_type().clone())
                    .collect(),
            })
            .collect();
        Ok(dlna_devices)
    }

    /// 通过设备描述XML的URL直接获取设备（无需多播，适用于不支持SSDP的WiFi环境）
    pub async fn get_device_from_url(&self, url: &str) -> Result<DlnaDevice, rupnp::Error> {
        let started = Instant::now();
        let endpoint = discovery_endpoint(url);
        log::info!(target: "DLNA_DISCOVERY", "手动连接开始：描述地址={}，超时={}ms", endpoint, DEVICE_DISCOVERY_TIMEOUT.as_millis());
        let uri: Uri = url
            .parse()
            .map_err(|_| {
                log::error!(target: "DLNA_DISCOVERY", "手动连接失败：步骤=解析描述地址，地址={}", endpoint);
                rupnp::Error::ParseError("无法解析设备XML URL")
            })?;

        let device = match timeout(DEVICE_DISCOVERY_TIMEOUT, Device::from_url(uri)).await {
            Ok(Ok(device)) => device,
            Ok(Err(error)) => {
                log::error!(target: "DLNA_DISCOVERY", "手动连接失败：步骤=获取/解析设备描述，描述地址={}，耗时={}ms，错误={}", endpoint, started.elapsed().as_millis(), error);
                return Err(error);
            }
            Err(_) => {
                log::error!(target: "DLNA_DISCOVERY", "手动连接失败：步骤=获取设备描述，描述地址={}，耗时={}ms，错误=超时", endpoint, started.elapsed().as_millis());
                return Err(rupnp::Error::ParseError("连接设备或获取描述文件超时"));
            }
        };

        let friendly_name = device.friendly_name().to_string();
        let location = device.url().to_string();
        let services: Vec<URN> = device
            .services()
            .iter()
            .map(|s| s.service_type().clone())
            .collect();
        log::info!(target: "DLNA_DISCOVERY", "手动连接成功：名称={}，描述地址={}，类型={}，服务={:?}，耗时={}ms", friendly_name, endpoint, device.device_type(), services, started.elapsed().as_millis());
        Ok(DlnaDevice {
            device,
            friendly_name,
            location,
            services,
        })
    }

    // 获取设备的AVTransport服务
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
        self.set_avtransport_url(device, &media_url, current_uri_metadata).await
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
            }else{
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
        let response = avtransport_action_compat(avtransport, &base_url, action, &args_str, true).await?;

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
        let response = avtransport_action_compat(avtransport, &base_url, action, &args_str, false).await?;

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

    async fn play_impl(&self, device: &DlnaDevice, retry_timeout: bool) -> Result<(), rupnp::Error> {
        let avtransport = self
            .get_avtransport_service(device)
            .ok_or(rupnp::Error::ParseError("设备不支持AVTransport服务"))?;

        log::info!("正在发送Play指令...");
        let action = "Play";
        let args_str = "<InstanceID>0</InstanceID><Speed>1</Speed>";

        let base_url = device_location_uri(device)?;
        log_upnp_action(avtransport, &base_url, action, args_str);
        let response = avtransport_action_compat(avtransport, &base_url, action, args_str, retry_timeout).await?;
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
        let response = avtransport_action_compat(avtransport, &base_url, action, args_str, false).await?;
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
        let response = avtransport_action_compat(avtransport, &base_url, action, args_str, false).await?;
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
        let response = avtransport_action_compat(avtransport, &base_url, action, args_str, false).await?;
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
        let response = avtransport_action_compat(avtransport, &base_url, action, args_str, false).await?;
        log::debug!("传输信息: {:?}", response);

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
        let response = avtransport_action_compat(avtransport, &base_url, action, args_str, false).await?;

        log::debug!("GetPositionInfo响应: {:?}", response);

        Ok(response)
    }

    // 获取当前播放进度，返回 (当前时间秒, 总时长秒)
    pub async fn get_secs(&self, device: &DlnaDevice) -> Result<(u32, u32), rupnp::Error> {
        let position_info = self.get_position_info(device).await?;

        // 获取相对时间
        let default_time = "00:00:00".to_string();
        let rel_time = position_info.get("RelTime").unwrap_or(&default_time);
        let duration = position_info
            .get("TrackDuration")
            .or_else(|| position_info.get("AbsTime"))
            .unwrap_or(&default_time);
        log::debug!(
            "get_secs() : RelTime: {}, TrackDuration: {}",
            rel_time,
            duration
        );

        // 解析时间字符串，支持格式如 "0:00:01" 和 "00:00:01"
        fn parse_time_str(time_str: &str) -> Result<NaiveTime, rupnp::Error> {
            let trimmed = time_str.trim();

            // 尝试多种时间格式
            let formats = ["%H:%M:%S", "%M:%S", "%S"];

            for fmt in &formats {
                if let Ok(time) = NaiveTime::parse_from_str(trimmed, fmt) {
                    log::debug!("成功解析时间 '{}' 格式: {}", trimmed, fmt);
                    return Ok(time);
                }
            }

            // 处理格式如 "0:00:01"（单个小时位）的情况
            if trimmed.contains(':') {
                let parts: Vec<&str> = trimmed.split(':').collect();
                if parts.len() == 3 {
                    // 确保每个部分都有正确的位数
                    let formatted = format!(
                        "{:02}:{:02}:{:02}",
                        parts[0].parse::<u32>().unwrap_or(0),
                        parts[1].parse::<u32>().unwrap_or(0),
                        parts[2].parse::<u32>().unwrap_or(0)
                    );
                    log::debug!("格式化时间 '{}' 为 '{}'", trimmed, formatted);
                    if let Ok(time) = NaiveTime::parse_from_str(&formatted, "%H:%M:%S") {
                        return Ok(time);
                    }
                }
            }

            Err(rupnp::Error::ParseError("无法解析时间字符串"))
        }

        let current_time =
            parse_time_str(rel_time).unwrap_or_else(|_| NaiveTime::from_hms_opt(0, 0, 0).unwrap());
        let total_time =
            parse_time_str(duration).unwrap_or_else(|_| NaiveTime::from_hms_opt(0, 0, 0).unwrap());

        let current_secs = current_time.num_seconds_from_midnight();
        let total_secs = total_time.num_seconds_from_midnight();

        Ok((current_secs, total_secs))
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
        let response = avtransport_action_compat(avtransport, &base_url, action, &args_str, false).await?;
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

    // 获取渲染器音量
    pub async fn get_volume(&self, device: &DlnaDevice) -> Result<u32, rupnp::Error> {
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

        let response = timeout_soap(rendering_control.action(&base_url, action, args_str)).await?;

        // 解析音量值
        let default_volume = "0".to_string();
        let volume_str = response.get("CurrentVolume").unwrap_or(&default_volume);
        let volume: u32 = volume_str.parse().unwrap_or(0);

        Ok(volume)
    }
}
#[cfg(test)]
#[path = "controller/tests/diagnostics.rs"]
mod diagnostics;

#[cfg(test)]
#[path = "controller/tests/discovery_diagnostics.rs"]
mod discovery_diagnostics;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires a real DLNA renderer and changes its next URI"]
    async fn test_set_next_avtransport_uri() {
        let controller = DlnaController::new();

        // 发现DLNA设备
        let devices = controller.discover_devices().await;
        match devices {
            Ok(devices) => {
                if devices.is_empty() {
                    println!("未发现DLNA设备，跳过测试");
                    return;
                }

                // 使用第一个设备
                let device = &devices[0];
                println!("使用设备: {}", device.friendly_name);

                // 测试设置下一首媒体URI
                let result = controller
                    .set_next_avtransport_uri(
                        device,
                        "/media/test_next.mp4",
                        "",
                        "127.0.0.1".parse().unwrap(),
                        8080,
                    )
                    .await;

                match result {
                    Ok(_) => println!("设置下一首媒体URI成功"),
                    Err(e) => println!("设置下一首媒体URI失败: {}", e),
                }
            }
            Err(e) => {
                println!("设备发现失败: {}", e);
            }
        }
    }
}
