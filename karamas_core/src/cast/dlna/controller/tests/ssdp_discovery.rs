use super::*;

fn interface(name: &str, address: Ipv4Addr, flags: InterfaceFlags) -> Interface {
    Interface {
        name: name.into(),
        #[cfg(windows)]
        description: name.into(),
        address: getifaddrs::Address::V4(getifaddrs::NetworkAddress {
            address,
            netmask: None,
            associated_address: None,
        }),
        flags,
        index: None,
    }
}

#[test]
fn selects_active_multicast_ipv4_addresses_without_loopback_or_point_to_point() {
    let flags = InterfaceFlags::UP | InterfaceFlags::RUNNING | InterfaceFlags::MULTICAST;
    let first = interface("wifi", Ipv4Addr::new(192, 168, 1, 2), flags);
    let second = interface("ethernet", Ipv4Addr::new(10, 0, 0, 2), flags);
    let mut candidates = vec![first.clone(), first, second];
    for excluded in [
        flags & !InterfaceFlags::UP,
        flags & !InterfaceFlags::RUNNING,
        flags & !InterfaceFlags::MULTICAST,
        flags | InterfaceFlags::LOOPBACK,
        flags | InterfaceFlags::POINTTOPOINT,
    ] {
        candidates.push(interface("excluded", Ipv4Addr::new(10, 1, 0, 2), excluded));
    }
    for address in [
        Ipv4Addr::LOCALHOST,
        Ipv4Addr::UNSPECIFIED,
        Ipv4Addr::BROADCAST,
        Ipv4Addr::new(239, 1, 2, 3),
    ] {
        candidates.push(interface("excluded", address, flags));
    }
    let mut ipv6 = interface("ipv6", Ipv4Addr::LOCALHOST, flags);
    ipv6.address = getifaddrs::Address::V6(getifaddrs::NetworkAddress {
        address: std::net::Ipv6Addr::LOCALHOST,
        netmask: None,
        associated_address: None,
    });
    candidates.push(ipv6);
    let selected = select_interfaces(candidates);
    assert_eq!(
        selected,
        vec![
            DiscoveryInterface {
                name: "wifi".into(),
                address: Ipv4Addr::new(192, 168, 1, 2)
            },
            DiscoveryInterface {
                name: "ethernet".into(),
                address: Ipv4Addr::new(10, 0, 0, 2)
            },
        ]
    );
}

#[tokio::test]
async fn probe_socket_sets_multicast_egress_and_keeps_unicast_replies_unconnected() {
    let socket = create_probe_socket(Ipv4Addr::LOCALHOST).await.unwrap();
    assert_eq!(
        SockRef::from(&socket).multicast_if_v4().unwrap(),
        Ipv4Addr::LOCALHOST
    );
    assert_eq!(
        socket.local_addr().unwrap().ip(),
        IpAddr::V4(Ipv4Addr::LOCALHOST)
    );
    assert!(socket.peer_addr().is_err());
}

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
    let report = probe_target("ssdp:all", Ipv4Addr::LOCALHOST, destination, Duration::from_millis(200), &|_| {}).await.unwrap();
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
    let interfaces = [DiscoveryInterface { name: "fixture".into(), address: Ipv4Addr::LOCALHOST }];
    let discovery = search_interfaces(&interfaces, destination, Duration::from_millis(1300), &on_location);
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
    let report = probe_target(TARGETS[0], Ipv4Addr::LOCALHOST, server.local_addr().unwrap(), Duration::from_millis(50), &|_| {}).await.unwrap();
    assert_eq!(report.received, 0);
    assert_eq!(report.rejected, 0);
    assert!(report.locations.is_empty());
}

#[tokio::test]
async fn parallel_interfaces_notify_once_even_when_another_interface_fails() {
    let server = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let destination = server.local_addr().unwrap();
    // 两个接口候选借用回环地址，只验证调度和合并，不要求测试机有两张物理网卡。
    let interfaces = [
        DiscoveryInterface {
            name: "unavailable".into(),
            address: Ipv4Addr::new(192, 0, 2, 1),
        },
        DiscoveryInterface {
            name: "first".into(),
            address: Ipv4Addr::LOCALHOST,
        },
        DiscoveryInterface {
            name: "second".into(),
            address: Ipv4Addr::LOCALHOST,
        },
    ];
    let responder = tokio::spawn(async move {
        let mut sources = BTreeSet::new();
        let mut buffer = [0; 2048];
        // 必须在任何一轮探测结束前收到两个接口的请求，串行搜索会超时。
        tokio::time::timeout(Duration::from_secs(1), async {
            while sources.len() < TARGETS.len() * 2 {
                let (_, source) = server.recv_from(&mut buffer).await.unwrap();
                sources.insert(source);
            }
        })
        .await
        .expect("各接口应同时开始搜索");
        let reply = response("http://127.0.0.1/renderer.xml");
        for source in sources {
            server.send_to(reply.as_bytes(), source).await.unwrap();
        }
    });
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let on_location = |location: &str| {
        sender.send(location.to_owned()).unwrap();
    };
    let discovery = search_interfaces(
        &interfaces,
        destination,
        Duration::from_secs(2),
        &on_location,
    );
    tokio::pin!(discovery);
    tokio::select! {
        biased;
        _ = &mut discovery => panic!("应在探测结束前通知设备地址"),
        location = receiver.recv() => assert_eq!(location.as_deref(), Some("http://127.0.0.1/renderer.xml")),
    }
    let locations = discovery.await.unwrap();
    responder.await.unwrap();
    assert_eq!(locations, vec!["http://127.0.0.1/renderer.xml"]);
    assert!(
        receiver.try_recv().is_err(),
        "跨接口和跨目标的重复地址只能通知一次"
    );
}
