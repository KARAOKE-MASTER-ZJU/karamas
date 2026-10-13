use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn transport_uri_distinguishes_direct_and_proxy_urls() {
    let original = "https://cdn.example/video.mp4?token=a%2Bb&expires=42";
    for direct in [true, false] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let mut soap = String::new();
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                loop {
                    let mut chunk = [0; 4096];
                    let n = stream.read(&mut chunk).await.unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&chunk[..n]);
                    if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&request[..end]);
                        let length = headers.lines().find_map(|line| {
                            line.to_ascii_lowercase().strip_prefix("content-length:")
                                .and_then(|s| s.trim().parse::<usize>().ok())
                        }).unwrap_or(0);
                        if request.len() >= end + 4 + length { break; }
                    }
                }
                let request = String::from_utf8(request).unwrap();
                let body = if request.starts_with("GET ") {
                    r#"<root><device><deviceType>urn:schemas-upnp-org:device:MediaRenderer:1</deviceType><friendlyName>Test renderer</friendlyName><serviceList><service><serviceType>urn:schemas-upnp-org:service:AVTransport:1</serviceType><serviceId>urn:upnp-org:serviceId:AVTransport</serviceId><SCPDURL>/service.xml</SCPDURL><controlURL>/control</controlURL><eventSubURL>/events</eventSubURL></service></serviceList></device></root>"#
                } else {
                    soap = request;
                    r#"<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"><s:Body><u:SetAVTransportURIResponse xmlns:u="urn:schemas-upnp-org:service:AVTransport:1"/></s:Body></s:Envelope>"#
                };
                stream.write_all(http_response(200, body).as_bytes()).await.unwrap();
            }
            soap
        });
        let controller = DlnaController::new();
        let device = controller.get_device_from_url(&base).await.unwrap();
        if direct {
            controller.set_avtransport_url(&device, original, "").await.unwrap();
        } else {
            controller.set_avtransport_uri(&device, original, "", "127.0.0.1".parse().unwrap(), 8080).await.unwrap();
        }
        let soap = tokio::time::timeout(Duration::from_secs(5), server).await.unwrap().unwrap();
        let expected = if direct { original.to_owned() } else { format!("http://127.0.0.1:8080/{original}") };
        assert!(soap.contains(&format!("<CurrentURI>{}</CurrentURI>", xml_escape(&expected))), "{soap}");
    }
}

const FAULT: &str = r#"<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">
<s:Body><s:Fault><faultcode>s:Client</faultcode><faultstring>UPnPError</faultstring>
<detail><UPnPError xmlns="urn:schemas-upnp-org:control-1-0">
<errorCode>718</errorCode><errorDescription>Not valid InstanceID</errorDescription>
</UPnPError></detail></s:Fault></s:Body></s:Envelope>"#;

async fn serve_once(response: String) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/control", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0; 4096];
        stream.read(&mut request).await.unwrap();
        stream.write_all(response.as_bytes()).await.unwrap();
    });
    (url, server)
}

fn http_response(status: u16, body: &str) -> String {
    format!(
        "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

#[tokio::test]
async fn soap_http_errors_preserve_fault_details_and_endpoint() {
    for (body, expected) in [
        (FAULT.to_owned(), "SOAP错误=718 Not valid InstanceID"),
        (
            FAULT.replace("Not valid InstanceID", "Invalid &amp; unsupported"),
            "SOAP错误=718 Invalid & unsupported",
        ),
        (String::new(), "响应体为空"),
    ] {
        let (url, server) = serve_once(http_response(500, &body)).await;
        let error = send_soap_shared(
            &url,
            "GetPositionInfo",
            "<InstanceID>999</InstanceID>",
            Duration::from_secs(1),
            true,
        )
        .await
        .unwrap_err()
        .to_string();
        server.await.unwrap();
        assert!(
            error.contains("GetPositionInfo失败：步骤=设备响应"),
            "{error}"
        );
        assert!(error.contains(&url), "{error}");
        assert!(error.contains("HTTP=500"), "{error}");
        assert!(error.contains(expected), "{error}");
    }
}

#[tokio::test]
async fn soap_errors_distinguish_sending_from_reading_response() {
    for (response, stage) in [
        (String::new(), "发送SOAP请求"),
        (
            "HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n<".to_owned(),
            "读取SOAP响应",
        ),
    ] {
        let (url, server) = serve_once(response).await;
        let error = send_soap_shared(&url, "Play", "", Duration::from_secs(1), true)
            .await
            .unwrap_err()
            .to_string();
        server.await.unwrap();
        assert!(error.contains("Play失败"), "{error}");
        assert!(error.contains(&format!("步骤={stage}")), "{error}");
        assert!(error.contains(&url), "{error}");
        assert_eq!(
            error.contains("HTTP=200"),
            stage == "读取SOAP响应",
            "{error}"
        );
    }
}

#[tokio::test]
async fn fallback_preserves_primary_fault_in_final_error() {
    let descriptor = r#"<root><device>
<deviceType>urn:schemas-upnp-org:device:MediaRenderer:1</deviceType>
<friendlyName>Diagnostics renderer</friendlyName><serviceList><service>
<serviceType>urn:schemas-upnp-org:service:AVTransport:1</serviceType>
<serviceId>urn:upnp-org:serviceId:AVTransport</serviceId>
<SCPDURL>/service.xml</SCPDURL><controlURL>/actual/control</controlURL>
<eventSubURL>/events</eventSubURL></service></serviceList></device></root>"#;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        for _ in 0..7 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = [0; 8192];
            let size = stream.read(&mut bytes).await.unwrap();
            let line = String::from_utf8_lossy(&bytes[..size])
                .lines()
                .next()
                .unwrap()
                .to_owned();
            let response = if line.starts_with("GET ") {
                http_response(200, descriptor)
            } else if line.starts_with("POST /actual/control ") {
                http_response(500, FAULT)
            } else {
                http_response(500, "")
            };
            requests.push(line);
            stream.write_all(response.as_bytes()).await.unwrap();
        }
        requests
    });
    let uri: Uri = base_url.parse().unwrap();
    let device = Device::from_url(uri.clone()).await.unwrap();
    let service = device
        .services()
        .iter()
        .find(|s| *s.service_type() == AV_TRANSPORT)
        .unwrap();
    let error = avtransport_action_compat(
        service,
        &uri,
        "GetPositionInfo",
        "<InstanceID>999</InstanceID>",
        true,
    )
    .await
    .unwrap_err()
    .to_string();
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 7); // Description + primary + native + four candidates; HTTP 500 is not resent.
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.starts_with("POST /actual/control "))
            .count(),
        2
    );
    assert!(error.contains("所有控制端点均失败"), "{error}");
    assert!(
        error.contains(&format!("端点={}actual/control", base_url)),
        "{error}"
    );
    assert!(
        error.contains("HTTP=500，SOAP错误=718 Not valid InstanceID"),
        "{error}"
    );
    assert!(!error.contains("响应体为空"), "{error}");
}
