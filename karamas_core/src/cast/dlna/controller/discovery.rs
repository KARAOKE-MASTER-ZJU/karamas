//! SSDP 地址解析、设备描述获取和渲染器筛选，发现后立即通知调用方。
use super::{DlnaController, DlnaDevice, ssdp_discovery};
use futures::{future::join_all, stream::StreamExt};
use rupnp::{Device, http::Uri, ssdp::URN};
use std::{
    collections::HashSet,
    time::{Duration, Instant},
};
use tokio::time::timeout;

/// 设备发现和描述获取的单次超时。
const DEVICE_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(3);

// Keep the endpoint useful for LAN diagnosis without exporting URL credentials.
pub(super) fn discovery_endpoint(raw: &str) -> String {
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

// 各网卡的 SSDP 响应共用去重集合，描述请求不等待探测窗口结束。
fn resolve_discovery_locations(
    locations: impl futures::Stream<Item = String>,
) -> impl futures::Stream<Item = Result<Device, rupnp::Error>> {
    let mut seen = HashSet::new();
    locations
        .filter(move |location| futures::future::ready(seen.insert(location.clone())))
        .map(|location| async move {
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
        }).buffer_unordered(8)
}

async fn collect_discovery_results(
    stream: impl futures::Stream<Item = Result<Device, rupnp::Error>>,
    started: Instant,
    mut on_device: impl FnMut(&DlnaDevice),
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
                let services: Vec<URN> = device
                    .services()
                    .iter()
                    .map(|s| s.service_type().clone())
                    .collect();
                log::info!(target: "DLNA_DISCOVERY", "发现设备：名称={}，描述地址={}，类型={}，服务={:?}", friendly_name, discovery_endpoint(&location), device_type, services);
                let device = DlnaDevice {
                    device,
                    friendly_name,
                    location,
                    services,
                };
                on_device(&device);
                devices.push(device);
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

impl DlnaController {
    // 发现网络中的DLNA渲染器设备
    pub async fn discover_devices(&self) -> Result<Vec<DlnaDevice>, rupnp::Error> {
        self.discover_devices_with_updates(|_| {}).await
    }

    /// 每确认一个渲染器就通知调用方，搜索结束后返回完整列表。
    pub async fn discover_devices_with_updates(
        &self,
        on_device: impl FnMut(&DlnaDevice),
    ) -> Result<Vec<DlnaDevice>, rupnp::Error> {
        let started = Instant::now();
        log::info!(target: "DLNA_DISCOVERY", "自动搜索开始：各可用IPv4网卡并行探测AVTransport:1、MediaRenderer:1、ssdp:all，各5s；收到地址即解析并通知，描述请求沿系统路由、单次超时3s、并发8");
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let discovery = ssdp_discovery::discover_locations(move |location| {
            let _ = sender.send(location.to_owned());
        });
        let locations = tokio_stream::wrappers::UnboundedReceiverStream::new(receiver);
        let collection =
            collect_discovery_results(resolve_discovery_locations(locations), started, on_device);
        // 同时驱动探测和描述解析；探测结束会关闭通道，随后等剩余描述请求完成。
        let (result, devices) = tokio::join!(discovery, collection);
        result?;
        Ok(devices)
    }

    pub async fn get_devices_from_urls(
        &self,
        urls: &Vec<&'static str>,
    ) -> Result<Vec<DlnaDevice>, rupnp::Error> {
        let tasks = urls.iter().map(|url| {
            let uri = Uri::from_static(url);
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
}

#[cfg(test)]
#[path = "tests/discovery_diagnostics.rs"]
mod discovery_diagnostics;
