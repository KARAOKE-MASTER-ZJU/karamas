use super::*;
use crate::playlist::auto_next::AutoNextSong;
use log::error;
use serde_json::json;
use std::collections::VecDeque;
use std::time::Duration;
use std::{future::Future, pin::Pin};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::time::sleep;

// Local-only server. None drops the response to simulate a lost reply after
// the server may already have advanced the playlist.
async fn server(
    replies: Vec<Option<(u16, serde_json::Value)>>,
) -> (
    PlaylistManager,
    mpsc::UnboundedReceiver<serde_json::Value>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let manager = PlaylistManager::new(
        &format!("http://{}", listener.local_addr().unwrap()),
        "test".into(),
    );
    *manager.hash.lock().await = Some("H0".into());
    let (tx, rx) = mpsc::unbounded_channel();
    let task = tokio::spawn(async move {
        let mut replies: VecDeque<_> = replies.into();
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let body_start = loop {
                let mut buffer = [0; 1024];
                let count = stream.read(&mut buffer).await.unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&buffer[..count]);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let header = String::from_utf8_lossy(&bytes[..body_start]);
            assert!(
                header.starts_with("POST /api/nextSong?roomId=test ")
                    || header.starts_with("GET /api/songListInfo?roomId=test&")
                    || header.starts_with("GET /api/songListInfo?roomId=test ")
                    || header.starts_with("POST /api/editCurrentSong?roomId=test ")
            );
            let length: usize = header
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse().unwrap())
                })
                .unwrap_or(0);
            while bytes.len() < body_start + length {
                let mut buffer = [0; 1024];
                let count = stream.read(&mut buffer).await.unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&buffer[..count]);
            }
            let body = if length == 0 {
                serde_json::Value::Null
            } else {
                serde_json::from_slice(&bytes[body_start..body_start + length]).unwrap()
            };
            tx.send(body).unwrap();
            if let Some((status, body)) = replies
                .pop_front()
                .unwrap_or(Some((200, json!({"success":true}))))
            {
                let body = body.to_string();
                stream.write_all(format!(
                    "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()
                ).as_bytes()).await.unwrap();
            }
        }
    });
    (manager, rx, task)
}

async fn request(rx: &mut mpsc::UnboundedReceiver<serde_json::Value>) -> serde_json::Value {
    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap()
}

async fn no_more_requests(rx: &mut mpsc::UnboundedReceiver<serde_json::Value>) {
    assert!(
        tokio::time::timeout(Duration::from_millis(1200), rx.recv())
            .await
            .is_err()
    );
}

async fn poll(trigger: &mut AutoNextSong, manager: &PlaylistManager, current: i32, total: i32) {
    let hash = manager.current_hash().await;
    if trigger.observe(current, total) {
        trigger.start_request(manager.clone(), hash, &tokio::runtime::Handle::current());
    }
}

#[tokio::test]
async fn new_hash_and_old_progress_do_not_post_again_until_play_success() {
    let (manager, mut requests, server) = server(vec![]).await;
    let mut trigger = AutoNextSong::default();
    poll(&mut trigger, &manager, 298, 300).await;
    assert_eq!(request(&mut requests).await["idArrayHash"], "H0");
    *manager.hash.lock().await = Some("H1".into());
    for _ in 0..5 {
        poll(&mut trigger, &manager, 300, 180).await;
    }
    no_more_requests(&mut requests).await;
    trigger.reset_for_playback();
    poll(&mut trigger, &manager, 1, 180).await;
    poll(&mut trigger, &manager, 178, 180).await;
    assert_eq!(request(&mut requests).await["idArrayHash"], "H1");
    server.abort();
}

#[tokio::test]
async fn retry_keeps_original_hash_without_more_progress_polls() {
    let (mut manager, mut requests, server) = server(vec![
        Some((503, json!({"error":"temporarily unavailable"}))),
        Some((200, json!({"success":true}))),
    ])
    .await;
    let mut trigger = AutoNextSong::default();
    poll(&mut trigger, &manager, 298, 300).await;
    assert_eq!(request(&mut requests).await["idArrayHash"], "H0");
    *manager.hash.lock().await = Some("H1".into());
    assert_eq!(request(&mut requests).await["idArrayHash"], "H0");
    no_more_requests(&mut requests).await;
    // Manual next is still independent and uses the latest playlist hash.
    manager.next_song().await.unwrap();
    assert_eq!(request(&mut requests).await["idArrayHash"], "H1");
    server.abort();
}

#[tokio::test]
async fn lost_response_then_reject_stops_without_switching_to_new_hash() {
    let (manager, mut requests, server) = server(vec![
        None,
        Some((200, json!({"success":false,"code":"REJECT"}))),
    ])
    .await;
    let mut trigger = AutoNextSong::default();
    poll(&mut trigger, &manager, 298, 300).await;
    assert_eq!(request(&mut requests).await["idArrayHash"], "H0");
    *manager.hash.lock().await = Some("H1".into());
    assert_eq!(request(&mut requests).await["idArrayHash"], "H0");
    no_more_requests(&mut requests).await;
    poll(&mut trigger, &manager, 300, 300).await;
    no_more_requests(&mut requests).await;
    server.abort();
}

#[tokio::test]
async fn dropping_trigger_cancels_pending_retries() {
    let (manager, mut requests, server) = server(vec![Some((503, json!({})))]).await;
    let mut trigger = AutoNextSong::default();
    poll(&mut trigger, &manager, 298, 300).await;
    request(&mut requests).await;
    drop(trigger);
    no_more_requests(&mut requests).await;
    server.abort();
}

fn playlist(hash: &str, id: &str, url: &str) -> serde_json::Value {
    json!({"changed":true,"hash":hash,"list":{
        "singing":{"id":id,"url":url,"title":id},"queued":[],"sung":[]
    }})
}

async fn progress(manager: &PlaylistManager, current: i32, total: i32) -> (i32, i32) {
    let mut gate = manager.progress_gate.lock().await;
    let revision = gate.revision();
    gate.filter(revision, current, total)
}

async fn confirm_play(manager: &PlaylistManager, hardware: bool) {
    let mut gate = manager.progress_gate.lock().await;
    let token = gate.revision();
    assert!(gate.play_succeeded(token, hardware, std::time::Instant::now()));
}

#[tokio::test]
async fn playlist_fetch_masks_external_change_but_not_queue_only_hash_change() {
    let (manager, mut requests, server) = server(vec![
        Some((200, playlist("H0", "A", "BV-A"))),
        Some((200, playlist("H1", "A", "BV-A"))),
        Some((200, playlist("H2", "B", "BV-B"))),
    ])
    .await;
    manager.fetch_playlist().await.unwrap();
    request(&mut requests).await;
    confirm_play(&manager, false).await;
    assert_eq!(progress(&manager, 240, 300).await, (240, 300));
    manager.fetch_playlist().await.unwrap();
    request(&mut requests).await;
    assert_eq!(progress(&manager, 241, 300).await, (241, 300));
    manager.fetch_playlist().await.unwrap();
    request(&mut requests).await;
    assert_eq!(manager.current_hash().await, "H2");
    assert_eq!(progress(&manager, 241, 180).await, (-1, -1));
    assert_eq!(progress(&manager, 299, 180).await, (-1, -1));
    confirm_play(&manager, true).await;
    assert_eq!(progress(&manager, 299, 180).await, (0, 180));
    assert_eq!(progress(&manager, 1, 180).await, (1, 180));
    server.abort();
}

#[tokio::test]
async fn next_song_response_does_not_confirm_device_playback() {
    let (mut manager, mut requests, server) = server(vec![]).await;
    manager
        .progress_gate
        .lock()
        .await
        .update_song(Some("A".into()), Some("BV-A".into()));
    confirm_play(&manager, false).await;
    assert_eq!(progress(&manager, 240, 300).await, (240, 300));
    manager.next_song().await.unwrap();
    assert_eq!(request(&mut requests).await["idArrayHash"], "H0");
    assert_eq!(progress(&manager, 241, 300).await, (241, 300));
    manager
        .progress_gate
        .lock()
        .await
        .update_song(Some("B".into()), Some("BV-B".into()));
    assert_eq!(progress(&manager, 240, 180).await, (-1, -1));
    assert_eq!(progress(&manager, 1, 180).await, (-1, -1));
    confirm_play(&manager, true).await;
    assert_eq!(progress(&manager, 1, 180).await, (1, 180));
    server.abort();
}

#[tokio::test]
async fn failed_manual_switch_leaves_progress_untouched() {
    let (mut manager, mut requests, server) = server(vec![Some((503, json!({})))]).await;
    manager
        .progress_gate
        .lock()
        .await
        .update_song(Some("A".into()), Some("BV-A".into()));
    confirm_play(&manager, false).await;
    assert_eq!(progress(&manager, 240, 300).await, (240, 300));
    assert!(manager.next_song().await.is_err());
    request(&mut requests).await;
    assert_eq!(progress(&manager, 241, 300).await, (241, 300));
    no_more_requests(&mut requests).await;
    server.abort();
}

#[tokio::test]
async fn failed_manual_switch_does_not_invalidate_pending_device_play() {
    let (mut manager, mut requests, server) = server(vec![Some((503, json!({})))]).await;
    let token = {
        let mut gate = manager.progress_gate.lock().await;
        gate.update_song(Some("B".into()), Some("BV-B".into()));
        gate.play_token("BV-B").unwrap()
    };
    assert!(manager.next_song().await.is_err());
    request(&mut requests).await;
    let mut gate = manager.progress_gate.lock().await;
    assert!(gate.play_succeeded(token, false, std::time::Instant::now()));
    let revision = gate.revision();
    assert_eq!(gate.filter(revision, 1, 180), (1, 180));
    server.abort();
}

#[tokio::test]
async fn periodic_sync_plays_distinct_entries_with_the_same_url() {
    let (manager, mut requests, server) = server(vec![
        Some((200, playlist("H0", "A", "BV-same"))),
        Some((200, playlist("H1", "B", "BV-same"))),
        Some((200, playlist("H2", "B", "BV-same"))),
    ])
    .await;
    let (played, mut received) = mpsc::unbounded_channel();
    let callback_manager = manager.clone();
    manager.start_periodic_update(&tokio::runtime::Handle::current(), move |url| {
        let manager = callback_manager.clone();
        let played = played.clone();
        Box::pin(async move {
            confirm_play(&manager, false).await;
            played.send(url).unwrap();
        })
    });
    for _ in 0..2 {
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), received.recv())
                .await
                .unwrap()
                .unwrap(),
            "BV-same"
        );
    }
    assert_eq!(progress(&manager, 1, 300).await, (1, 300));
    for _ in 0..3 {
        request(&mut requests).await;
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(400), received.recv())
            .await
            .is_err()
    );
    server.abort();
}

#[tokio::test]
async fn dropping_playback_state_cancels_its_retry_without_an_ownership_cycle() {
    let (manager, mut requests, server) = server(vec![Some((503, json!({})))]).await;
    let weak_gate = Arc::downgrade(&manager.progress_gate);
    manager.progress_gate.lock().await.auto_next.start_request(
        manager.clone(),
        "H0".into(),
        &tokio::runtime::Handle::current(),
    );
    request(&mut requests).await;
    drop(manager);
    assert!(weak_gate.upgrade().is_none());
    no_more_requests(&mut requests).await;
    server.abort();
}

#[tokio::test]
async fn heartbeat_sync_plays_missed_song_and_later_updates_do_not_replay_it() {
    let (manager, mut requests, server) = server(vec![
        Some((200, playlist("H0", "A", "BV-A"))),
        Some((200, playlist("H1", "B", "BV-B"))),
        Some((200, json!({"changed":false}))),
        Some((200, playlist("H2", "B", "BV-B"))),
    ])
    .await;
    let (played, mut received) = mpsc::unbounded_channel();
    let mut callback = |url| -> Pin<Box<dyn Future<Output = ()> + Send>> {
        let manager = manager.clone();
        let played = played.clone();
        Box::pin(async move {
            // Song sync must mask the old renderer position before dispatching play.
            assert_eq!(progress(&manager, 298, 300).await, (-1, -1));
            confirm_play(&manager, false).await;
            played.send(url).unwrap();
        })
    };
    let mut cached = None;
    manager
        .sync_playlist(&mut cached, &mut callback)
        .await
        .unwrap();
    assert_eq!(received.try_recv().unwrap(), "BV-A");
    // No WS notification: the heartbeat uses this same operation and must play B.
    manager
        .sync_playlist(&mut cached, &mut callback)
        .await
        .unwrap();
    assert_eq!(received.try_recv().unwrap(), "BV-B");
    assert_eq!(progress(&manager, 1, 180).await, (1, 180));
    // A later sync/reconnect and a queue-only update must not replay B.
    for _ in 0..2 {
        manager
            .sync_playlist(&mut cached, &mut callback)
            .await
            .unwrap();
    }
    assert!(received.try_recv().is_err());
    assert_eq!(manager.current_hash().await, "H2");
    for _ in 0..4 {
        request(&mut requests).await;
    }
    server.abort();
}

#[tokio::test]
async fn failed_sync_preserves_handled_song_and_next_sync_recovers() {
    let (manager, mut requests, server) = server(vec![
        Some((200, playlist("H0", "A", "BV-A"))),
        Some((503, json!({}))),
        Some((200, playlist("H1", "B", "BV-B"))),
    ])
    .await;
    let (played, mut received) = mpsc::unbounded_channel();
    let mut callback = |url| -> Pin<Box<dyn Future<Output = ()> + Send>> {
        played.send(url).unwrap();
        Box::pin(async {})
    };
    let mut cached = None;
    manager
        .sync_playlist(&mut cached, &mut callback)
        .await
        .unwrap();
    assert_eq!(received.try_recv().unwrap(), "BV-A");
    assert!(
        manager
            .sync_playlist(&mut cached, &mut callback)
            .await
            .is_err()
    );
    assert_eq!(cached, Some((Some("A".into()), "BV-A".into())));
    assert!(received.try_recv().is_err());
    manager
        .sync_playlist(&mut cached, &mut callback)
        .await
        .unwrap();
    assert_eq!(received.try_recv().unwrap(), "BV-B");
    for _ in 0..3 {
        request(&mut requests).await;
    }
    server.abort();
}


#[tokio::test]
async fn part_edit_preserves_title_and_sync_replays_same_id_only_once() {
    use crate::playlist::parts::CurrentSongSnapshot;
    let first = "bilibili://video/BV1LS4MzKE8y?page=0";
    let second = "bilibili://video/BV1LS4MzKE8y?page=1";
    let (manager, mut requests, server) = server(vec![
        Some((200, playlist("H0", "A", first))),
        Some((200, json!({"success":true,"hash":"H1"}))),
        Some((200, playlist("H1", "A", second))),
        Some((200, playlist("H1", "A", second))),
    ])
    .await;
    let (played, mut received) = mpsc::unbounded_channel();
    let mut callback = |url| -> Pin<Box<dyn Future<Output = ()> + Send>> {
        let manager = manager.clone();
        let played = played.clone();
        Box::pin(async move {
            assert_eq!(progress(&manager, 299, 300).await, (-1, -1));
            confirm_play(&manager, false).await;
            played.send(url).unwrap();
        })
    };
    let mut cached = None;
    manager
        .sync_playlist(&mut cached, &mut callback)
        .await
        .unwrap();
    assert_eq!(received.try_recv().unwrap(), "BV1LS4MzKE8y-page0");
    let selection = CurrentSongSnapshot {
        song_id: "A".into(),
        song_url: first.into(),
        room_hash: "H0".into(),
        song_title: "A".into(),
    };
    assert!(
        manager
            .edit_current_part(&selection, second, false)
            .await
            .unwrap()
    );
    assert_eq!(manager.current_hash().await, "H0");
    assert_eq!(progress(&manager, 12, 357).await, (12, 357));
    request(&mut requests).await;
    let body = request(&mut requests).await;
    assert_eq!(
        body,
        json!({"idArrayHash":"H0","song":{"id":"A","title":"A","url":second}})
    );
    manager
        .sync_playlist(&mut cached, &mut callback)
        .await
        .unwrap();
    assert_eq!(received.try_recv().unwrap(), "BV1LS4MzKE8y-page1");
    assert_eq!(progress(&manager, 1, 226).await, (1, 226));
    manager
        .sync_playlist(&mut cached, &mut callback)
        .await
        .unwrap();
    assert!(received.try_recv().is_err());
    server.abort();
}


#[tokio::test]
#[ignore = "manual live-room example; mutates the remote playlist"]
async fn test_playlist_manager() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== PlaylistManager 使用示例 ===");

    let mut manager = PlaylistManager::new("https://ktv.starfreedomx.top", "1111".to_string());

    println!("开始获取播放列表...");

    // --- 第一次操作 ---
    match manager.fetch_playlist().await {
        Ok(_) => {
            println!("✓ 成功获取播放列表");
            // 获取当前播放的歌曲
            {
                let song_playing = manager.get_song_playing().await;
                match &song_playing {
                    Some(url) => println!("当前播放的歌曲: {}", url),
                    None => println!("当前没有播放的歌曲"),
                }
            }
        }
        Err(e) => error!("✗ 获取播放列表失败: {}", e),
    }

    // --- 第二次操作 ---
    manager.next_song().await?;
    println!("请求下一首歌曲后播放列表状态:");

    // 获取当前播放的歌曲
    {
        let song_playing = manager.get_song_playing().await;
        match &song_playing {
            Some(url) => println!("当前播放的歌曲: {}", url),
            None => println!("当前没有播放的歌曲"),
        }
    }

    // --- 后台任务开始 ---
    manager.start_periodic_update(&tokio::runtime::Handle::current(), |url: String| {
        println!("Song singing changed to {}!", url);
        Box::pin(async {})
    });

    // 【关键点 3】：sleep 必须在“裸奔”状态下运行（不持有任何锁）
    // 此时 playlist 锁是空闲的，后台线程的 fetch_playlist 才能拿到锁并更新数据
    sleep(Duration::from_secs(5)).await;

    println!("5秒后播放列表状态:");

    // 获取当前播放的歌曲
    {
        let song_playing = manager.get_song_playing().await;
        match &song_playing {
            Some(url) => println!("当前播放的歌曲: {}", url),
            None => println!("当前没有播放的歌曲"),
        }
    }

    println!("=== 示例结束 ===");
    Ok(())
}
