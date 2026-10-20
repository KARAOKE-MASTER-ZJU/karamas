use super::*;
use std::cell::RefCell;
use std::sync::Once;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

thread_local! {
    static MESSAGES: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

struct DiscoveryLogger;

impl log::Log for DiscoveryLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.target() == "DLNA_DISCOVERY" && metadata.level() <= log::Level::Info
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            MESSAGES.with(|messages| messages.borrow_mut().push(record.args().to_string()));
        }
    }

    fn flush(&self) {}
}

fn begin_capture() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        log::set_logger(&DiscoveryLogger).unwrap();
        log::set_max_level(log::LevelFilter::Info);
    });
    MESSAGES.with(|messages| messages.borrow_mut().clear());
}

fn captured() -> String {
    MESSAGES.with(|messages| messages.borrow().join("\n"))
}

async fn serve_description(body: String, status: u16) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/description.xml?token=secret", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0; 4096];
        stream.read(&mut request).await.unwrap();
        let response = format!("HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        stream.write_all(response.as_bytes()).await.unwrap();
    });
    (url, server)
}

async fn device(kind: &str) -> Device {
    let xml = format!(r#"<root><device>
<deviceType>urn:schemas-upnp-org:device:{kind}:1</deviceType>
<friendlyName>Test renderer</friendlyName><serviceList><service>
<serviceType>urn:schemas-upnp-org:service:AVTransport:1</serviceType>
<serviceId>urn:upnp-org:serviceId:AVTransport</serviceId>
<SCPDURL>/service.xml</SCPDURL><controlURL>/control</controlURL>
<eventSubURL>/events</eventSubURL></service></serviceList></device></root>"#);
    let (url, server) = serve_description(xml, 200).await;
    let device = Device::from_url(url.parse().unwrap()).await.unwrap();
    server.await.unwrap();
    device
}

#[tokio::test]
async fn empty_discovery_is_visible_at_info_level_without_claiming_offline() {
    begin_capture();
    let devices = collect_discovery_results(futures::stream::empty(), Instant::now()).await;
    assert!(devices.is_empty());
    let logs = captured();
    assert!(logs.contains("结果项=0，可用设备=0，重复=0，过滤=0，错误=0"), "{logs}");
    assert!(logs.contains("不能单独证明设备离线"), "{logs}");
}

#[tokio::test]
async fn discovery_reports_duplicates_filters_and_errors_while_preserving_valid_devices() {
    begin_capture();
    let renderer = device("MediaRenderer").await;
    let server = device("MediaServer").await;
    let results = vec![
        Ok(renderer.clone()), Ok(renderer), Ok(server),
        Err(rupnp::Error::ParseError("fixture malformed description")),
    ];
    let devices = collect_discovery_results(futures::stream::iter(results), Instant::now()).await;
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].friendly_name, "Test renderer");
    let logs = captured();
    assert!(logs.contains("结果项=4，可用设备=1，重复=1，过滤=1，错误=1"), "{logs}");
    assert!(logs.contains("非MediaRenderer"), "{logs}");
    assert!(logs.contains("fixture malformed description"), "{logs}");
    assert!(!logs.contains("token=secret"), "{logs}");
}

#[tokio::test]
async fn direct_connection_logs_invalid_address_and_description_failure() {
    begin_capture();
    let controller = DlnaController::new();
    assert!(controller.get_device_from_url("http://[::").await.is_err());
    assert!(captured().contains("步骤=解析描述地址"));
    for (status, body) in [(200, "not XML"), (500, "unavailable")] {
        begin_capture();
        let (url, server) = serve_description(body.to_owned(), status).await;
        assert!(controller.get_device_from_url(&url).await.is_err());
        server.await.unwrap();
        let logs = captured();
        assert!(logs.contains("步骤=获取/解析设备描述"), "{logs}");
        assert!(logs.contains("耗时="), "{logs}");
        assert!(!logs.contains("token=secret"), "{logs}");
    }
}

#[tokio::test]
async fn direct_connection_timeout_has_a_terminal_diagnostic() {
    begin_capture();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/description.xml", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
        tokio::time::sleep(Duration::from_secs(10)).await;
    });
    let error = DlnaController::new().get_device_from_url(&url).await.unwrap_err();
    server.abort();
    assert!(error.to_string().contains("超时"));
    let logs = captured();
    assert!(logs.contains("步骤=获取设备描述"), "{logs}");
    assert!(logs.contains("错误=超时"), "{logs}");
}

#[test]
fn description_endpoint_keeps_host_port_path_but_removes_credentials() {
    assert_eq!(discovery_endpoint("http://user:password@192.168.1.2:9958/description.xml?token=secret#fragment"), "http://192.168.1.2:9958/description.xml");
}
