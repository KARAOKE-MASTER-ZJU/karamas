//! Observable IPv4 M-SEARCH. Separate sockets associate replies with each probe.
use super::discovery_endpoint;
use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::time::{Instant, sleep_until};

const TARGETS: [&str; 3] = [
    "urn:schemas-upnp-org:service:AVTransport:1",
    "urn:schemas-upnp-org:device:MediaRenderer:1",
    "ssdp:all",
];
const DESTINATION: &str = "239.255.255.250:1900";
const DETAIL_LIMIT: usize = 50;

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

fn log_field(value: &str) -> String {
    value.chars().filter(|c| !c.is_control()).take(200).collect()
}

async fn probe_target(
    target: &str,
    local_ip: IpAddr,
    destination: SocketAddr,
    window: Duration,
) -> std::io::Result<ProbeReport> {
    let socket = UdpSocket::bind(SocketAddr::new(local_ip, 0)).await?;
    socket.set_multicast_ttl_v4(2)?;
    let local = socket.local_addr()?;
    let request = format!("M-SEARCH * HTTP/1.1\r\nHOST: {destination}\r\nMAN: \"ssdp:discover\"\r\nMX: 3\r\nST: {target}\r\n\r\n");
    let started = Instant::now();
    let deadline = started + window;
    let retry_at = started + Duration::from_secs(1);
    let mut retried = false;
    let mut report = ProbeReport::default();
    let mut buffer = vec![0u8; 65535];
    log::info!(target: "DLNA_SSDP", "探测开始：目标={}，本地={}，目的={}，MX=3，TTL=2，窗口={}ms", target, local, destination, window.as_millis());
    match socket.send_to(request.as_bytes(), destination).await {
        Ok(bytes) => {
            report.sent += 1;
            log::info!(target: "DLNA_SSDP", "UDP发送成功：目标={}，本地={}，目的={}，字节={}，序号=1；仅表示系统接受发送", target, local, destination, bytes);
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
                        log::info!(target: "DLNA_SSDP", "UDP发送成功：目标={}，本地={}，目的={}，字节={}，序号=2", target, local, destination, bytes);
                    }
                    Err(error) => {
                        report.errors += 1;
                        log::error!(target: "DLNA_SSDP", "UDP重发失败：目标={}，错误={}", target, error);
                    }
                }
            }
            received = socket.recv_from(&mut buffer) => {
                let (size, source) = match received {
                    Ok(value) => value,
                    Err(error) => {
                        report.errors += 1;
                        log::error!(target: "DLNA_SSDP", "UDP接收失败：目标={}，错误={}", target, error);
                        break;
                    }
                };
                report.received += 1;
                let detailed = report.received <= DETAIL_LIMIT;
                if report.received == DETAIL_LIMIT + 1 {
                    log::info!(target: "DLNA_SSDP", "目标={}：后续数据包省略逐条日志，继续统计和处理", target);
                }
                match parse_reply(&buffer[..size]) {
                    Ok(reply) => {
                        if detailed {
                            log::info!(target: "DLNA_SSDP", "UDP收到响应：请求目标={}，来源={}，字节={}，ST={}，USN={}，LOCATION={}", target, source, size, log_field(reply.st), log_field(reply.usn), discovery_endpoint(reply.location));
                        }
                        if !report.locations.insert(reply.location.to_owned()) { report.duplicates += 1; }
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
    log::info!(target: "DLNA_SSDP", "目标对照结果：目标={}，本地={}，发送={}，UDP收到={}，响应拒绝={}，重复={}，描述地址={}，收发错误={}，耗时={}ms", target, local, report.sent, report.received, report.rejected, report.duplicates, report.locations.len(), report.errors, started.elapsed().as_millis());
    Ok(report)
}

pub(super) async fn discover_locations() -> Result<Vec<String>, rupnp::Error> {
    let destination: SocketAddr = DESTINATION.parse().expect("constant IPv4 endpoint");
    // UDP connect selects a route without sending a packet. Do not connect the
    // receiving sockets: replies come from unicast device addresses, not the group.
    let route = async {
        let probe = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await?;
        probe.connect(destination).await?;
        Ok::<_, std::io::Error>(probe.local_addr()?.ip())
    }.await.map_err(|error| {
        log::error!(target: "DLNA_SSDP", "选择IPv4路由失败：目的={}，错误={}", destination, error);
        rupnp::Error::IO(error)
    })?;
    log::info!(target: "DLNA_SSDP", "系统路由选择本地IPv4={}；三个探测套接字显式绑定此地址，各自等待单播响应", route);
    compare_targets(route, destination, Duration::from_secs(5)).await
}

async fn compare_targets(route: IpAddr, destination: SocketAddr, window: Duration) -> Result<Vec<String>, rupnp::Error> {
    let results = futures::future::join_all(TARGETS.iter().map(|target| {
        probe_target(target, route, destination, window)
    })).await;
    let mut locations = BTreeSet::new();
    let mut failed = 0;
    for (target, result) in TARGETS.iter().zip(results) {
        match result {
            Ok(report) => locations.extend(report.locations),
            Err(error) => {
                failed += 1;
                log::error!(target: "DLNA_SSDP", "目标探测失败：目标={}，错误={}", target, error);
            }
        }
    }
    log::info!(target: "DLNA_SSDP", "三目标对照结束：失败目标={}，合并后描述地址={}", failed, locations.len());
    if failed == TARGETS.len() { return Err(rupnp::Error::ParseError("所有SSDP目标探测失败，请查看DLNA_SSDP日志")); }
    Ok(locations.into_iter().collect())
}

#[cfg(test)]
#[path = "../test/ssdp_discovery.rs"]
mod tests;
