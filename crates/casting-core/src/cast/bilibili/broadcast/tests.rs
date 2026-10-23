use super::*;
use crate::cast::{
    Caster, SongRef,
    bilibili::{BilibiliCaster, BilibiliSession},
};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

fn reply(method: &str, sequence: i64) -> BroadcastFrame {
    BroadcastFrame {
        target_path: format!("{BROADCAST}{method}"),
        options: Some(FrameOption {
            sequence,
            ..Default::default()
        }),
        body: None,
    }
}

fn progress(session: &PlaySession) -> prost_types::Any {
    prost_types::Any {
        type_url: "type.googleapis.com/bilibili.broadcast.message.tv.ProjReply".into(),
        value: ProjReply {
            cmd_type: 4,
            aid: session.aid as i64,
            cid: session.cid as i64,
            seek_ts: 19,
            extra: serde_json::json!({"bizSessionId": session.id}).to_string(),
            ..Default::default()
        }
        .encode_to_vec(),
    }
}

#[tokio::test(start_paused = true)]
async fn receive_keeps_processing_after_bad_and_stale_messages_and_keeps_sending_acks() {
    let receiver = BroadcastReceiver::new("sender".into());
    let old = receiver.begin_play(1, 2);
    let current = receiver.begin_play(3, 4);
    receiver.clear_play(&old.id); // 旧播放请求晚到的失败不能清除新曲目的回传关联。
    let (outgoing, mut sent) = mpsc::channel(32);
    let (incoming, received) = mpsc::channel(32);
    let (ready, mut subscribed) = oneshot::channel();
    let (events, mut observed) = mpsc::unbounded_channel();
    let task = tokio::spawn(receive(
        ReceiverStream::new(received),
        FrameSender {
            tx: outgoing,
            sequence: 1,
        },
        receiver.active.clone(),
        ready,
        move |event| {
            events.send(event).unwrap();
        },
    ));
    incoming.send(Ok(reply("Auth", 0))).await.unwrap();
    for topic in [PROJ, PROJ_CORE] {
        let frame = sent.recv().await.unwrap();
        let paths = TargetPath::decode(frame.body.unwrap().value.as_slice()).unwrap();
        assert_eq!(paths.target_paths, [topic]);
    }
    incoming.send(Ok(reply("Subscribe", 999))).await.unwrap();
    incoming.send(Ok(reply("Subscribe", 2))).await.unwrap();
    tokio::task::yield_now().await;
    assert_eq!(
        subscribed.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    );
    incoming.send(Ok(reply("Subscribe", 1))).await.unwrap();
    subscribed.await.unwrap();

    let mut corrupt = progress(&current);
    corrupt.value = vec![0xff];
    let mut other_video = current.clone();
    other_video.cid += 1;
    // 最后一条正常消息充当处理屏障，确保前面的坏消息和过期消息都走过接收循环。
    for (index, body) in [
        None,
        Some(corrupt),
        Some(progress(&old)),
        Some(progress(&other_video)),
        Some(progress(&current)),
    ]
    .into_iter()
    .enumerate()
    {
        incoming
            .send(Ok(BroadcastFrame {
                target_path: PROJ.into(),
                body,
                options: Some(FrameOption {
                    message_id: index as i64 + 100,
                    is_ack: true,
                    ack_origin: "origin".into(),
                    msg_type: 3,
                    ..Default::default()
                }),
            }))
            .await
            .unwrap();
        let frame = sent
            .recv()
            .await
            .expect("坏消息和旧会话仍需 ACK，接收循环不能退出");
        assert_eq!(frame.options.unwrap().sequence, index as i64 + 3);
        let ack = MessageAckReq::decode(frame.body.unwrap().value.as_slice()).unwrap();
        assert_eq!(
            (
                ack.ack_id,
                ack.target_path.as_str(),
                ack.ack_origin.as_str(),
                ack.msg_type
            ),
            (index as i64 + 100, PROJ, "origin", 3)
        );
    }
    let event = tokio::time::timeout(Duration::from_secs(1), observed.recv())
        .await
        .expect("坏消息之后必须继续处理正常回传")
        .unwrap();
    assert_eq!(event.session, current.id);
    assert_eq!(event.position_secs, Some(19));
    assert!(observed.try_recv().is_err());
    tokio::time::advance(HEARTBEAT_INTERVAL).await;
    let heartbeat = sent.recv().await.unwrap();
    assert_eq!(heartbeat.target_path, format!("{BROADCAST}Heartbeat"));
    assert_eq!(heartbeat.options.unwrap().sequence, 8);
    drop(incoming);
    assert!(task.await.unwrap().is_err());
}

#[tokio::test]
async fn rejected_auth_does_not_subscribe() {
    let (outgoing, mut sent) = mpsc::channel(32);
    let (ready, subscribed) = oneshot::channel();
    let mut frame = reply("Auth", 0);
    frame.options.as_mut().unwrap().status = Some(RpcStatus { code: -101 });
    let result = receive(
        futures_util::stream::iter([Ok(frame)]),
        FrameSender {
            tx: outgoing,
            sequence: 1,
        },
        Arc::new(RwLock::new(None)),
        ready,
        |_| panic!("鉴权失败不能产生设备回传"),
    )
    .await;
    assert!(result.is_err());
    assert!(subscribed.await.is_err());
    assert!(sent.recv().await.is_none());
}

// 只替换外部曲目信息和服务地址；play_song、HTTP 发送、响应处理和本地计时均走实际实现。
async fn play_with_unavailable_subscription(connection_fails: bool, http_code: i32) {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let grpc = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut receiver = BroadcastReceiver::new("sender".into());
    receiver.test_host = Some(format!("https://{}", grpc.local_addr().unwrap()));
    let active = receiver.active.clone();
    let (accepted, mut connections) = mpsc::unbounded_channel();
    let (closed, mut disconnects) = mpsc::unbounded_channel();
    let _grpc_server = Worker(tokio::spawn(async move {
        loop {
            let (mut socket, _) = grpc.accept().await.unwrap();
            accepted.send(()).unwrap();
            if !connection_fails {
                // TCP 可连但 TLS/订阅一直不响应；客户端关闭时读到 EOF。
                let _ = socket.read_to_end(&mut Vec::new()).await;
                closed.send(()).unwrap();
            }
        }
    }));
    receiver.start("test-token", 1);
    connections.recv().await.unwrap();
    if connection_fails {
        // 确认连接失败已被接收任务处理，再进入播放路径。
        tokio::time::timeout(Duration::from_secs(2), async {
            while !receiver
                .worker
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .0
                .is_finished()
            {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
    }

    let http = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let caster = Arc::new(
        BilibiliCaster::new(
            BilibiliSession {
                access_token: "test-token".into(),
                mid: 1,
                expires_at: None,
            },
            "tv".into(),
            "sender".into(),
        )
        .with_test_transport(
            format!("http://{}/x/tv/stream/cmd", http.local_addr().unwrap()),
            (137649199, 180),
            receiver,
        ),
    );
    let (request, received) = oneshot::channel();
    let (respond, response) = oneshot::channel();
    let _http_server = Worker(tokio::spawn(async move {
        let (socket, _) = http.accept().await.unwrap();
        let mut socket = BufReader::new(socket);
        let mut length = 0;
        loop {
            let mut line = String::new();
            assert!(socket.read_line(&mut line).await.unwrap() > 0);
            if line == "\r\n" {
                break;
            }
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse().unwrap();
            }
        }
        let mut body = vec![0; length];
        socket.read_exact(&mut body).await.unwrap();
        request.send(body).unwrap();
        response.await.unwrap();
        let body = format!(r#"{{"code":{http_code},"message":"test response"}}"#);
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
    }));
    let playing = caster.clone();
    let task = tokio::spawn(async move {
        playing
            .play_song(&SongRef("BV1GJ411x7h7-page0".into()))
            .await
    });
    // 远短于 15 秒订阅超时：重新引入等待会在这里失败。
    let request = tokio::time::timeout(Duration::from_secs(2), received)
        .await
        .expect("订阅未就绪或失败时，仍必须立即发送 HTTP 播放命令")
        .unwrap();
    let form: std::collections::HashMap<_, _> =
        url::form_urlencoded::parse(&request).into_owned().collect();
    assert_eq!(form["command"], "1");
    let extra: serde_json::Value = serde_json::from_str(&form["extra"]).unwrap();
    assert_eq!(
        extra["bizSessionId"],
        active.read().unwrap().as_ref().unwrap().id
    );
    assert_eq!(
        caster.get_progress().await.unwrap().total_secs,
        0,
        "HTTP 尚未成功不能启动本地计时"
    );
    respond.send(()).unwrap();
    let result = tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
    if http_code == 0 {
        result.unwrap();
        assert_eq!(caster.get_progress().await.unwrap().total_secs, 180);
        assert!(active.read().unwrap().is_some());
    } else {
        assert!(result.unwrap_err().to_string().contains("业务错误=-101"));
        assert_eq!(caster.get_progress().await.unwrap().total_secs, 0);
        assert!(active.read().unwrap().is_none());
    }
    caster.stop_status_subscription();
    if !connection_fails {
        tokio::time::timeout(Duration::from_secs(2), disconnects.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            connections.try_recv().is_err(),
            "播放入口应复用已启动的广播连接"
        );
    }
}

#[tokio::test]
async fn pending_subscription_does_not_delay_http_play_or_progress_start() {
    play_with_unavailable_subscription(false, 0).await;
}

#[tokio::test]
async fn failed_subscription_does_not_fail_http_play() {
    play_with_unavailable_subscription(true, 0).await;
}

#[tokio::test]
async fn http_play_failure_still_clears_session_and_does_not_start_progress() {
    play_with_unavailable_subscription(false, -101).await;
}
