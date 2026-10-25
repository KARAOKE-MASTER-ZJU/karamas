//! 在各个可用 IPv4 网卡上并行发送 M-SEARCH，逐个通知新发现的地址。
use super::discovery_endpoint;
use getifaddrs::{Interface, InterfaceFlags};
use socket2::SockRef;
use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Mutex;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::time::{Instant, sleep_until};

/// SSDP 目标服务类型
const TARGETS: [&str; 3] = [
    "urn:schemas-upnp-org:service:AVTransport:1",
    "urn:schemas-upnp-org:device:MediaRenderer:1",
    "ssdp:all",
];
/// SSDP 目标地址
const DESTINATION: &str = "239.255.255.250:1900";
const DETAIL_LIMIT: usize = 50;

#[derive(Debug, PartialEq, Eq)]
struct DiscoveryInterface {
    name: String,
    address: Ipv4Addr,
}

fn select_interfaces(interfaces: impl IntoIterator<Item = Interface>) -> Vec<DiscoveryInterface> {
    let mut seen = BTreeSet::new();
    interfaces.into_iter().filter_map(|interface| {
        let Some(IpAddr::V4(address)) = interface.address.ip_addr() else {
            return None;
        };
        if !interface.flags.contains(InterfaceFlags::UP | InterfaceFlags::RUNNING | InterfaceFlags::MULTICAST)
            || interface.flags.intersects(InterfaceFlags::LOOPBACK | InterfaceFlags::POINTTOPOINT)
            || address.is_loopback()
            || address.is_unspecified()
            || address.is_multicast()
            || address.is_broadcast()
        {
            log::debug!(target: "DLNA_SSDP", "跳过网卡：接口={}，IPv4={}，标志={:?}", interface.name, address, interface.flags);
            return None;
        }
        if !seen.insert((interface.name.clone(), address)) {
            return None;
        }
        Some(DiscoveryInterface { name: interface.name, address })
    }).collect()
}

async fn create_probe_socket(local_ip: Ipv4Addr) -> std::io::Result<UdpSocket> {
    let socket = UdpSocket::bind((local_ip, 0)).await?;
    // 绑定源地址之外，还需明确指定组播出口，避免组播沿默认路由发出。
    SockRef::from(&socket).set_multicast_if_v4(&local_ip)?;
    socket.set_multicast_ttl_v4(2)?;
    Ok(socket)
}

#[derive(Default, Debug)]
struct ProbeReport {
    sent: usize,
    received: usize,
    rejected: usize,
    duplicates: usize,
    errors: usize,
    locations: BTreeSet<String>,
}

struct Reply<'a> {
    location: &'a str,
    st: &'a str,
    usn: &'a str,
}

/// 解析 SSDP 响应，返回 Reply 结构体或错误信息
fn parse_reply(bytes: &[u8]) -> Result<Reply<'_>, &'static str> {
    let text = std::str::from_utf8(bytes).map_err(|_| "响应不是UTF-8")?;
    let mut lines = text.lines();
    let mut status = lines.next().unwrap_or("").split_whitespace();
    if !matches!(status.next(), Some("HTTP/1.1" | "HTTP/1.0")) || status.next() != Some("200") {
        return Err("不是HTTP 200 SSDP响应");
    }
    let (mut location, mut st, mut usn) = (None, "<未提供>", "<未提供>");
    for line in lines {
        if line.trim().is_empty() { break; }
        let (key, value) = line.split_once(':').ok_or("响应头缺少冒号")?;
        let value = value.trim();
        if key.trim().eq_ignore_ascii_case("location") {
            if location.is_some() { return Err("重复LOCATION响应头"); }
            location = Some(value);
        } else if key.trim().eq_ignore_ascii_case("st") {
            st = value;
        } else if key.trim().eq_ignore_ascii_case("usn") {
            usn = value;
        }
    }
    let location = location.filter(|v| !v.is_empty()).ok_or("缺少LOCATION响应头")?;
    let url = url::Url::parse(location).map_err(|_| "LOCATION不是有效URL")?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("LOCATION不是HTTP(S)设备描述地址");
    }
    Ok(Reply { location, st, usn })
}

/// 将日志字段中的控制字符过滤掉，并限制长度为200个字符
fn log_field(value: &str) -> String {
    value.chars().filter(|c| !c.is_control()).take(200).collect()
}

/// 探测指定目标的设备，返回探测报告
async fn probe_target(
    target: &str,
    local_ip: Ipv4Addr,
    destination: SocketAddr,
    window: Duration,
    on_location: &(impl Fn(&str) + Sync),
) -> std::io::Result<ProbeReport> {
    let socket = create_probe_socket(local_ip).await?;
    let local = socket.local_addr()?;
    let request = format!("M-SEARCH * HTTP/1.1\r\nHOST: {destination}\r\nMAN: \"ssdp:discover\"\r\nMX: 3\r\nST: {target}\r\n\r\n");
    let started = Instant::now();
    let deadline = started + window;
    let retry_at = started + Duration::from_secs(1);
    let mut retried = false;
    let mut report = ProbeReport::default();
    let mut last_error = None;
    let mut buffer = vec![0u8; 65535];
    log::info!(target: "DLNA_SSDP", "探测开始：目标={}，本地={}，目的={}，MX=3，TTL=2，窗口={}ms", target, local, destination, window.as_millis());
    match socket.send_to(request.as_bytes(), destination).await {
        Ok(bytes) => {
            report.sent += 1;
            log::debug!(target: "DLNA_SSDP", "UDP发送成功：目标={}，本地={}，目的={}，字节={}，序号=1；仅表示系统接受发送", target, local, destination, bytes);
        }
        Err(error) => {
            log::error!(target: "DLNA_SSDP", "UDP发送失败：目标={}，本地={}，错误={}", target, local, error);
            return Err(error);
        }
    }
    loop {
        tokio::select! {
            biased;
            _ = sleep_until(deadline) => break,
            _ = sleep_until(retry_at), if !retried => {
                retried = true;
                match socket.send_to(request.as_bytes(), destination).await {
                    Ok(bytes) => {
                        report.sent += 1;
                        log::debug!(target: "DLNA_SSDP", "UDP发送成功：目标={}，本地={}，目的={}，字节={}，序号=2", target, local, destination, bytes);
                    }
                    Err(error) => {
                        report.errors += 1;
                        log::error!(target: "DLNA_SSDP", "UDP重发失败：目标={}，错误={}", target, error);
                        last_error = Some(error);
                    }
                }
            }
            received = socket.recv_from(&mut buffer) => {
                let (size, source) = match received {
                    Ok(value) => value,
                    Err(error) => {
                        report.errors += 1;
                        log::error!(target: "DLNA_SSDP", "UDP接收失败：目标={}，错误={}", target, error);
                        last_error = Some(error);
                        break;
                    }
                };
                report.received += 1;
                let detailed = report.received <= DETAIL_LIMIT;
                if report.received == DETAIL_LIMIT + 1 {
                    log::debug!(target: "DLNA_SSDP", "目标={}：后续数据包省略逐条日志，继续统计和处理", target);
                }
                match parse_reply(&buffer[..size]) {
                    Ok(reply) => {
                        if detailed {
                            log::debug!(target: "DLNA_SSDP", "UDP收到响应：请求目标={}，来源={}，字节={}，ST={}，USN={}，LOCATION={}", target, source, size, log_field(reply.st), log_field(reply.usn), discovery_endpoint(reply.location));
                        }
                        if report.locations.insert(reply.location.to_owned()) {
                            on_location(reply.location);
                        } else {
                            report.duplicates += 1;
                        }
                    }
                    Err(reason) => {
                        report.rejected += 1;
                        if detailed {
                            log::warn!(target: "DLNA_SSDP", "UDP收到但无法使用：请求目标={}，来源={}，字节={}，原因={}", target, source, size, reason);
                        }
                    }
                }
            }
        }
    }
    log::info!(target: "DLNA_SSDP", "目标探测结果：目标={}，本地={}，发送={}，UDP收到={}，响应拒绝={}，重复={}，描述地址={}，收发错误={}，耗时={}ms", target, local, report.sent, report.received, report.rejected, report.duplicates, report.locations.len(), report.errors, started.elapsed().as_millis());
    // 没有可用结果时保留收发错误，供宿主区分网络失败与空列表。
    if report.locations.is_empty() {
        if let Some(error) = last_error { return Err(error); }
    }
    Ok(report)
}

/// SSDP 发现设备，收到新的描述地址就通知调用方。
pub(super) async fn discover_locations(
    on_location: impl Fn(&str) + Sync,
) -> Result<(), rupnp::Error> {
    let destination: SocketAddr = DESTINATION.parse().expect("constant IPv4 endpoint");
    let interfaces = getifaddrs::getifaddrs().map_err(|error| {
        log::error!(target: "DLNA_SSDP", "枚举网卡失败：错误={}", error);
        rupnp::Error::IO(error)
    })?;
    let interfaces = select_interfaces(interfaces);
    search_interfaces(
        &interfaces,
        destination,
        Duration::from_secs(5),
        &on_location,
    )
    .await
    .map(|_| ())
}

/// 各网卡同时搜索所有目标；跨网卡、跨目标的重复地址只通知一次。
async fn search_interfaces(
    interfaces: &[DiscoveryInterface],
    destination: SocketAddr,
    window: Duration,
    on_location: &(impl Fn(&str) + Sync),
) -> Result<Vec<String>, rupnp::Error> {
    if interfaces.is_empty() {
        log::warn!(target: "DLNA_SSDP", "没有可用于SSDP搜索的IPv4网卡");
        return Err(rupnp::Error::IO(std::io::Error::new(
            std::io::ErrorKind::AddrNotAvailable,
            "没有已启用且支持组播的 IPv4 网卡",
        )));
    }
    let locations = Mutex::new(BTreeSet::new());
    let notify_location = |location: &str| {
        let is_new = locations.lock().unwrap().insert(location.to_owned());
        if is_new {
            on_location(location);
        }
    };
    let mut probes = Vec::new();
    for interface in interfaces {
        log::info!(target: "DLNA_SSDP", "搜索网卡：接口={}，IPv4={}，显式指定组播出口，目标数={}", interface.name, interface.address, TARGETS.len());
        for target in TARGETS {
            let notify_location = &notify_location;
            probes.push(async move {
                (
                    interface,
                    target,
                    probe_target(
                        target,
                        interface.address,
                        destination,
                        window,
                        notify_location,
                    )
                    .await,
                )
            });
        }
    }
    let results = futures::future::join_all(probes).await;
    let mut failed = 0;
    let mut first_error = None;
    for (interface, target, result) in results {
        if let Err(error) = result {
            failed += 1;
            log::error!(target: "DLNA_SSDP", "目标探测失败：接口={}，IPv4={}，目标={}，错误={}", interface.name, interface.address, target, error);
            first_error.get_or_insert(rupnp::Error::IO(error));
        }
    }
    let locations = locations.into_inner().unwrap();
    log::info!(target: "DLNA_SSDP", "SSDP探测结束：网卡地址数={}，失败探测={}，合并后描述地址={}", interfaces.len(), failed, locations.len());
    if locations.is_empty() {
        if let Some(error) = first_error {
            return Err(error);
        }
    }
    Ok(locations.into_iter().collect())
}

#[cfg(test)]
#[path = "tests/ssdp_discovery.rs"]
mod tests;
