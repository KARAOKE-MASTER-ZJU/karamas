use super::*;
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[test]
fn retiring_old_streams_keeps_new_streams_active() {
    let cache = SongCache::new(
        &std::env::temp_dir(),
        reqwest::Client::new(),
        vec![RemoteSource {
            url: "http://unused.invalid/video".into(),
            len: 1024,
        }],
        BLOCK_BYTES,
    )
    .unwrap();
    let (old_id, old) = cache.stream_epoch();
    let cutoff = cache.stream_cutoff();
    let (new_id, new) = cache.stream_epoch();
    cache.retire_streams(cutoff);
    assert!(*old.borrow() >= old_id);
    assert!(*new.borrow() < new_id);
    cache.retire_streams(0); // A late completion cannot move the cutoff back.
    assert!(*old.borrow() >= old_id);
}

struct Origin {
    url: String,
    requests: Arc<AtomicU64>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Origin {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn origin(data: Bytes, status: u16, delay: bool) -> Origin {
    origin_with_gate(data, status, delay, None).await
}

async fn origin_with_gate(
    data: Bytes,
    status: u16,
    delay: bool,
    gate: Option<Arc<tokio::sync::Barrier>>,
) -> Origin {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/media", listener.local_addr().unwrap());
    let requests = Arc::new(AtomicU64::new(0));
    let count = requests.clone();
    let task = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let data = data.clone();
            let count = count.clone();
            let gate = gate.clone();
            tokio::spawn(async move {
                let mut request = Vec::new();
                let mut chunk = [0; 1024];
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    let n = socket.read(&mut chunk).await.unwrap();
                    if n == 0 {
                        return;
                    }
                    request.extend_from_slice(&chunk[..n]);
                }
                count.fetch_add(1, Ordering::Relaxed);
                if let Some(gate) = gate {
                    gate.wait().await;
                }
                if delay {
                    std::future::pending::<()>().await;
                }
                let request = String::from_utf8(request).unwrap().to_ascii_lowercase();
                let range = request
                    .lines()
                    .find_map(|l| l.strip_prefix("range: bytes="))
                    .unwrap();
                let (start, end) = range.split_once('-').unwrap();
                let start: usize = start.parse().unwrap();
                let end: usize = end.parse().unwrap();
                let response = format!(
                    "HTTP/1.1 {status} OK\r\nContent-Range: bytes {start}-{end}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    data.len(),
                    end - start + 1
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.write_all(&data[start..=end]).await;
            });
        }
    });
    Origin {
        url,
        requests,
        task,
    }
}

fn cache(server: &Origin, length: u64, blocks: u64) -> SongCache {
    SongCache::new_with_block_size(
        &std::env::temp_dir(),
        reqwest::Client::new(),
        vec![RemoteSource {
            url: server.url.clone(),
            len: length,
        }],
        blocks * BLOCK_BYTES,
        BLOCK_BYTES,
    )
    .unwrap()
}

#[tokio::test]
async fn concurrent_reads_share_download_and_cleanup_directory() {
    let server = origin(Bytes::from_static(b"0123456789"), 206, false).await;
    let cache = Arc::new(cache(&server, 10, 2));
    let path = cache.directory.0.clone();
    let mut tasks = Vec::new();
    for _ in 0..16 {
        let c = cache.clone();
        tasks.push(tokio::spawn(async move { c.read(0, 2..7).await.unwrap() }));
    }
    for task in tasks {
        assert_eq!(task.await.unwrap().as_ref(), b"23456");
    }
    assert_eq!(server.requests.load(Ordering::Relaxed), 1);
    drop(cache);
    assert!(!path.exists());
}

#[tokio::test]
async fn cross_block_reads_respect_disk_capacity() {
    let data: Vec<u8> = (0..BLOCK_BYTES as usize * 3 + 7)
        .map(|i| (i % 251) as u8)
        .collect();
    let server = origin(Bytes::from(data.clone()), 206, false).await;
    let cache = cache(&server, data.len() as u64, 1);
    let r = BLOCK_BYTES - 3..BLOCK_BYTES + 5;
    assert_eq!(
        cache.read(0, r.clone()).await.unwrap().as_ref(),
        &data[r.start as usize..r.end as usize]
    );
    cache
        .read(0, 2 * BLOCK_BYTES..2 * BLOCK_BYTES + 10)
        .await
        .unwrap();
    let disk: u64 = std::fs::read_dir(&cache.directory.0)
        .unwrap()
        .map(|f| f.unwrap().metadata().unwrap().len())
        .sum();
    assert!(disk <= BLOCK_BYTES);
    assert_eq!(cache.read(0, 0..10).await.unwrap().as_ref(), &data[..10]);
}

#[tokio::test]
async fn cancellation_interrupts_inflight_network_and_rejects_later_reads() {
    let server = origin(Bytes::from_static(b"0123456789"), 206, true).await;
    let cache = Arc::new(cache(&server, 10, 1));
    let path = cache.directory.0.clone();
    let reader = cache.clone();
    let read = tokio::spawn(async move { reader.read(0, 0..10).await });
    tokio::time::timeout(Duration::from_secs(2), async {
        while server.requests.load(Ordering::Relaxed) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    cache.cancel();
    let result = tokio::time::timeout(Duration::from_secs(2), read)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Interrupted);
    assert_eq!(
        cache.read(0, 0..1).await.unwrap_err().kind(),
        io::ErrorKind::Interrupted
    );
    drop(cache);
    assert!(!path.exists());
}

#[tokio::test]
async fn ignored_range_is_never_cached_as_valid_data() {
    let server = origin(Bytes::from_static(b"0123456789"), 200, false).await;
    let cache = cache(&server, 10, 1);
    assert!(cache.read(0, 0..5).await.is_err());
    let failed_requests = server.requests.load(Ordering::Relaxed);
    assert!(cache.read(0, 0..5).await.is_err());
    assert!(server.requests.load(Ordering::Relaxed) > failed_requests);
    assert_eq!(std::fs::read_dir(&cache.directory.0).unwrap().count(), 0);
}

#[tokio::test]
async fn keyframe_spanning_blocks_fetches_them_concurrently() {
    let block_bytes = 64 * 1024;
    let data = Bytes::from(vec![37; block_bytes as usize * 2]);
    // Neither response can finish until BOTH requests reach the server.
    // A serial cross-block reader deadlocks and fails the bounded test.
    let server = origin_with_gate(
        data.clone(),
        206,
        false,
        Some(Arc::new(tokio::sync::Barrier::new(2))),
    )
    .await;
    let cache = SongCache::new_with_block_size(
        &std::env::temp_dir(),
        reqwest::Client::new(),
        vec![RemoteSource {
            url: server.url.clone(),
            len: data.len() as u64,
        }],
        BLOCK_BYTES,
        block_bytes,
    )
    .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(2), cache.read(0, 0..data.len() as u64))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result, data);
}
