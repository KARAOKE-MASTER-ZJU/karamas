use super::*;

fn response(location: &str) -> String {
    format!("HTTP/1.1 200 OK\r\nlocation: {location}\r\nst: ssdp:all\r\nUSN: uuid:fixture\r\n\r\n")
}

#[test]
fn parses_case_insensitive_headers_and_retains_query_for_request_only() {
    let raw = response("http://127.0.0.1:80/description.xml?token=secret");
    let reply = parse_reply(raw.as_bytes()).unwrap();
    assert_eq!(reply.st, "ssdp:all");
    assert_eq!(reply.usn, "uuid:fixture");
    assert!(reply.location.contains("token=secret"));
    assert!(!discovery_endpoint(reply.location).contains("secret"));
}

#[test]
fn rejects_malformed_or_non_description_responses_with_specific_reasons() {
    assert!(matches!(parse_reply(b"\xff"), Err("响应不是UTF-8")));
    assert!(matches!(parse_reply(b"NOTIFY * HTTP/1.1\r\n\r\n"), Err("不是HTTP 200 SSDP响应")));
    assert!(matches!(parse_reply(b"HTTP/1.1 200 OK\r\nST: ssdp:all\r\n\r\n"), Err("缺少LOCATION响应头")));
    assert!(parse_reply(response("file:///secret").as_bytes()).is_err());
    assert!(parse_reply(b"HTTP/1.1 200 OK\r\nLOCATION: http://host/a\r\nlocation: http://host/b\r\n\r\n").is_err());
}

#[tokio::test]
async fn counts_received_datagrams_duplicates_and_rejected_responses() {
    let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let destination = server.local_addr().unwrap();
    let responder = tokio::spawn(async move {
        let mut buffer = [0; 2048];
        let (size, source) = server.recv_from(&mut buffer).await.unwrap();
        let request = std::str::from_utf8(&buffer[..size]).unwrap();
        assert!(request.starts_with("M-SEARCH * HTTP/1.1\r\n"));
        assert!(request.contains("MAN: \"ssdp:discover\"\r\nMX: 3\r\nST: ssdp:all\r\n\r\n"));
        assert!(request.contains(&format!("HOST: {destination}\r\n")));
        let packet = response("http://127.0.0.1/description.xml");
        for bytes in [packet.as_bytes(), b"invalid", packet.as_bytes()] {
            server.send_to(bytes, source).await.unwrap();
        }
    });
    let report = probe_target("ssdp:all", Ipv4Addr::LOCALHOST.into(), destination, Duration::from_millis(200), &|_| {}).await.unwrap();
    responder.await.unwrap();
    assert_eq!(report.sent, 1);
    assert_eq!(report.received, 3);
    assert_eq!(report.rejected, 1);
    assert_eq!(report.duplicates, 1);
    assert_eq!(report.locations.len(), 1);
    assert_eq!(report.errors, 0);
}

#[tokio::test]
async fn target_comparison_finds_device_that_only_answers_ssdp_all_and_deduplicates_retry() {
    let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let destination = server.local_addr().unwrap();
    let responder = tokio::spawn(async move {
        let mut targets = Vec::new();
        let mut buffer = [0; 2048];
        for _ in 0..6 {
            let (size, source) = tokio::time::timeout(Duration::from_secs(3), server.recv_from(&mut buffer)).await.unwrap().unwrap();
            let request = std::str::from_utf8(&buffer[..size]).unwrap();
            let target = request.lines().find_map(|line| line.strip_prefix("ST: ")).unwrap().to_owned();
            if target == "ssdp:all" {
                server.send_to(response("http://127.0.0.1/description.xml").as_bytes(), source).await.unwrap();
            }
            targets.push(target);
        }
        targets
    });
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let on_location = |location: &str| { sender.send(location.to_owned()).unwrap(); };
    let discovery = compare_targets(Ipv4Addr::LOCALHOST.into(), destination, Duration::from_millis(1300), &on_location);
    tokio::pin!(discovery);
    // 通知必须先于探测完成；把通知挪到收集结束后会让这里失败。
    tokio::select! {
        biased;
        _ = &mut discovery => panic!("探测结束前没有通知设备地址"),
        location = receiver.recv() => assert_eq!(location.as_deref(), Some("http://127.0.0.1/description.xml")),
    }
    let locations = discovery.await.unwrap();
    assert!(receiver.try_recv().is_err(), "重复响应不能重复通知");
    assert_eq!(locations, vec!["http://127.0.0.1/description.xml"]);
    let targets = responder.await.unwrap();
    for target in TARGETS { assert_eq!(targets.iter().filter(|value| *value == target).count(), 2); }
}

#[tokio::test]
async fn silent_endpoint_completes_with_zero_received_datagrams() {
    let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let report = probe_target(TARGETS[0], Ipv4Addr::LOCALHOST.into(), server.local_addr().unwrap(), Duration::from_millis(50), &|_| {}).await.unwrap();
    assert_eq!(report.received, 0);
    assert_eq!(report.rejected, 0);
    assert!(report.locations.is_empty());
}
