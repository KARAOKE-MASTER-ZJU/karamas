//! Bounded, per-song source-block cache. A new song gets a new instance and
//! cancels the old one. The caller chooses an app-private writable directory.
//! Files and their directory are removed only after active readers release them.
use bytes::{Bytes, BytesMut};
use futures_util::StreamExt;
use std::{
    collections::{HashMap, VecDeque},
    io,
    ops::Range,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Mutex, OnceCell, Semaphore, watch};

pub const BLOCK_BYTES: u64 = 1024 * 1024;
pub const DEFAULT_CACHE_BLOCK_BYTES: u64 = 128 * 1024;
const HOT_BLOCKS: usize = 4;
type Key = (usize, u64);
static DIRECTORY_ID: AtomicU64 = AtomicU64::new(0);

pub struct RemoteSource {
    pub url: String,
    pub len: u64,
}
struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.0);
    }
}
struct Staging(PathBuf);
impl Drop for Staging {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
struct Block {
    path: PathBuf,
    ready: OnceCell<()>,
    accessed: AtomicU64,
    _directory: Arc<Directory>,
}
impl Drop for Block {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub struct SongCache {
    client: reqwest::Client,
    sources: Vec<RemoteSource>,
    directory: Arc<Directory>,
    blocks: Mutex<HashMap<Key, Arc<Block>>>,
    hot: Mutex<VecDeque<(Key, Bytes)>>,
    block_limit: usize,
    block_bytes: u64,
    hot_limit: usize,
    parallel: Arc<Semaphore>,
    cancelled: watch::Sender<bool>,
    stream_epoch: watch::Sender<u64>,
    stream_id: AtomicU64,
    tick: AtomicU64,
    requests: AtomicU64,
    hits: AtomicU64,
}

fn error(kind: io::ErrorKind, message: &str) -> io::Error {
    io::Error::new(kind, message)
}
fn cancelled() -> io::Error {
    error(io::ErrorKind::Interrupted, "song cache cancelled")
}

impl SongCache {
    pub fn new(
        parent: &Path,
        client: reqwest::Client,
        sources: Vec<RemoteSource>,
        capacity: u64,
    ) -> io::Result<Self> {
        Self::new_with_block_size(parent, client, sources, capacity, DEFAULT_CACHE_BLOCK_BYTES)
    }

    pub fn new_with_block_size(
        parent: &Path,
        client: reqwest::Client,
        sources: Vec<RemoteSource>,
        capacity: u64,
        block_bytes: u64,
    ) -> io::Result<Self> {
        if !(16 * 1024..=BLOCK_BYTES).contains(&block_bytes) || !block_bytes.is_power_of_two() {
            return Err(error(
                io::ErrorKind::InvalidInput,
                "invalid cache block size",
            ));
        }
        let block_limit = usize::try_from(capacity / block_bytes)
            .map_err(|_| error(io::ErrorKind::InvalidInput, "cache capacity too large"))?;
        if block_limit == 0 || sources.is_empty() || sources.iter().any(|s| s.len == 0) {
            return Err(error(
                io::ErrorKind::InvalidInput,
                "empty cache capacity or source",
            ));
        }
        // create_dir (not create_dir_all) refuses to adopt another session's directory.
        let directory = loop {
            let n = DIRECTORY_ID.fetch_add(1, Ordering::Relaxed);
            let path = parent.join(format!("ktv-song-{}-{n}", std::process::id()));
            match std::fs::create_dir(&path) {
                Ok(()) => break Arc::new(Directory(path)),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        };
        let (cancelled, _) = watch::channel(false);
        Ok(Self {
            client,
            sources,
            directory,
            blocks: Mutex::new(HashMap::new()),
            hot: Mutex::new(VecDeque::new()),
            block_limit,
            block_bytes,
            hot_limit: ((HOT_BLOCKS as u64 * BLOCK_BYTES) / block_bytes) as usize,
            parallel: Arc::new(Semaphore::new(4)),
            cancelled,
            stream_epoch: watch::channel(0).0,
            stream_id: AtomicU64::new(0),
            tick: AtomicU64::new(0),
            requests: AtomicU64::new(0),
            hits: AtomicU64::new(0),
        })
    }

    pub fn cancel(&self) {
        self.cancelled.send_replace(true);
        self.interrupt_streams();
    }
    /// A deliberate seek makes existing response streams obsolete, but keeps
    /// their completed cache blocks. New HTTP requests subscribe to the new epoch.
    pub fn interrupt_streams(&self) {
        self.retire_streams(self.stream_cutoff());
    }
    pub fn stream_cutoff(&self) -> u64 {
        self.stream_id.load(Ordering::SeqCst)
    }
    pub fn retire_streams(&self, cutoff: u64) {
        self.stream_epoch.send_modify(|epoch| *epoch = (*epoch).max(cutoff));
    }
    pub fn stream_epoch(&self) -> (u64, watch::Receiver<u64>) {
        let receiver = self.stream_epoch.subscribe();
        let id = self.stream_id.fetch_add(1, Ordering::SeqCst) + 1;
        (id, receiver)
    }
    pub fn stats(&self) -> (u64, u64) {
        (
            self.requests.load(Ordering::Relaxed),
            self.hits.load(Ordering::Relaxed),
        )
    }

    pub(crate) fn warmup_blocks(&self, parts: Vec<crate::seekable_mp4::ReadPart>) -> Vec<(usize, Range<u64>)> {
        let mut seen = std::collections::HashSet::new();
        let mut reads = Vec::new();
        for part in parts {
            if let crate::seekable_mp4::ReadPart::Source { source, range } = part {
                if range.is_empty() { continue; }
                for block in range.start / self.block_bytes..=(range.end - 1) / self.block_bytes {
                    if seen.insert((source, block)) {
                        let start = block * self.block_bytes;
                        reads.push((source, start..(start + self.block_bytes).min(self.sources[source].len)));
                    }
                }
            }
        }
        reads
    }

    /// Read at most 1MiB of data. Cross-block requests are split internally.
    /// Larger virtual responses should stream these pieces instead of accumulating.
    pub async fn read(&self, source: usize, range: Range<u64>) -> io::Result<Bytes> {
        let remote = self
            .sources
            .get(source)
            .ok_or_else(|| error(io::ErrorKind::InvalidInput, "unknown source"))?;
        if range.start > range.end
            || range.end > remote.len
            || range.end - range.start > BLOCK_BYTES
        {
            return Err(error(
                io::ErrorKind::InvalidInput,
                "invalid or oversized cache read",
            ));
        }
        let mut stop = self.cancelled.subscribe();
        if *stop.borrow() {
            return Err(cancelled());
        }
        tokio::select! {
            biased;
            _ = stop.changed() => Err(cancelled()),
            result = async {
                if range.is_empty() { return Ok(Bytes::new()); }
                let first = range.start/self.block_bytes;
                let last = (range.end-1)/self.block_bytes;
                if first == last {
                    let bytes = self.block((source, first)).await?;
                    let base = first*self.block_bytes;
                    return Ok(bytes.slice((range.start-base) as usize..(range.end-base) as usize));
                }
                // A large keyframe can span several cache blocks. Fetch those
                // concurrently instead of adding a network RTT for each block.
                // Tiny test/cache configurations cannot pin more blocks than fit.
                let mut reads = futures_util::stream::iter(first..=last).map(|number| async move {
                    let bytes = self.block((source, number)).await?;
                    let base = number*self.block_bytes;
                    let start = range.start.max(base)-base;
                    let end = range.end.min(base+self.block_bytes)-base;
                    Ok::<_, io::Error>(bytes.slice(start as usize..end as usize))
                }).buffered(4.min(self.block_limit));
                let mut output = BytesMut::with_capacity((range.end - range.start) as usize);
                while let Some(bytes) = reads.next().await { output.extend_from_slice(&bytes?); }
                Ok(output.freeze())
            } => result,
        }
    }

    async fn block(&self, key: Key) -> io::Result<Bytes> {
        {
            let mut hot = self.hot.lock().await;
            if let Some(index) = hot.iter().position(|(k, _)| *k == key) {
                let entry = hot.remove(index).unwrap();
                let bytes = entry.1.clone();
                hot.push_back(entry);
                self.hits.fetch_add(1, Ordering::Relaxed);
                return Ok(bytes);
            }
        }
        let block = {
            let mut blocks = self.blocks.lock().await;
            if let Some(block) = blocks.get(&key) {
                Arc::clone(block)
            } else {
                if blocks.len() >= self.block_limit {
                    let oldest = blocks
                        .iter()
                        .filter(|(_, b)| Arc::strong_count(b) == 1)
                        .min_by_key(|(_, b)| b.accessed.load(Ordering::Relaxed))
                        .map(|(k, _)| *k);
                    let Some(oldest) = oldest else {
                        return Err(error(
                            io::ErrorKind::WouldBlock,
                            "cache capacity pinned by active readers",
                        ));
                    };
                    blocks.remove(&oldest);
                    self.hot.lock().await.retain(|(k, _)| *k != oldest);
                }
                let block = Arc::new(Block {
                    path: self.directory.0.join(format!("{}-{}", key.0, key.1)),
                    ready: OnceCell::new(),
                    accessed: AtomicU64::new(0),
                    _directory: Arc::clone(&self.directory),
                });
                blocks.insert(key, Arc::clone(&block));
                block
            }
        };
        block
            .accessed
            .store(self.tick.fetch_add(1, Ordering::Relaxed), Ordering::Relaxed);
        let fetched = std::sync::atomic::AtomicBool::new(false);
        block
            .ready
            .get_or_try_init(|| async {
                let permit = self
                    .parallel
                    .clone()
                    .acquire_owned()
                    .await
                    .map_err(|_| cancelled())?;
                let remote = &self.sources[key.0];
                let start = key.1 * self.block_bytes;
                let end = start.saturating_add(self.block_bytes).min(remote.len);
                self.requests.fetch_add(1, Ordering::Relaxed);
                let fetch_started = std::time::Instant::now();
                let bytes = fetch_range(&self.client, &remote.url, start..end, remote.len).await?;
                log::trace!(target: "DLNA1080_CACHE", "缓存未命中下载: source={}, offset={}, bytes={}, fetch_ms={}", key.0, start, bytes.len(), fetch_started.elapsed().as_millis());
                // A dropped async filesystem write can continue on Tokio's worker.
                // Publish atomically from a unique staging file; keep the block pinned
                // until that worker finishes, even if the initiating read is cancelled.
                let writer = Arc::clone(&block);
                tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    let n = DIRECTORY_ID.fetch_add(1, Ordering::Relaxed);
                    let staging = Staging(writer.path.with_extension(format!("pending-{n}")));
                    std::fs::write(&staging.0, &bytes)?;
                    std::fs::rename(&staging.0, &writer.path)
                })
                .await
                .map_err(io::Error::other)??;
                fetched.store(true, Ordering::Relaxed);
                Ok::<(), io::Error>(())
            })
            .await?;
        if !fetched.load(Ordering::Relaxed) {
            self.hits.fetch_add(1, Ordering::Relaxed);
        }
        let bytes = Bytes::from(tokio::fs::read(&block.path).await?);
        let expected = (self.sources[key.0].len - key.1 * self.block_bytes).min(self.block_bytes);
        if bytes.len() as u64 != expected {
            return Err(error(
                io::ErrorKind::InvalidData,
                "cached block length mismatch",
            ));
        }
        let mut hot = self.hot.lock().await;
        hot.retain(|(k, _)| *k != key);
        hot.push_back((key, bytes.clone()));
        while hot.len() > self.hot_limit.min(self.block_limit) {
            hot.pop_front();
        }
        Ok(bytes)
    }
}

/// Strict byte-range fetch, shared by index loading and media caching. Rejects
/// ignored ranges, mismatched file sizes and truncated/oversized responses.
pub async fn fetch_range(
    client: &reqwest::Client,
    url: &str,
    range: Range<u64>,
    total: u64,
) -> io::Result<Bytes> {
    if range.start >= range.end || range.end > total || range.end - range.start > 16 * BLOCK_BYTES {
        return Err(error(io::ErrorKind::InvalidInput, "invalid upstream range"));
    }
    let response = client
        .get(url)
        .header("User-Agent", crate::upstream::user_agent(url))
        .header("Referer", crate::upstream::referer(url))
        .header("Accept-Encoding", "identity")
        .header("Range", format!("bytes={}-{}", range.start, range.end - 1))
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| io::Error::other(e.without_url()))?;
    let expected = format!("bytes {}-{}/{}", range.start, range.end - 1, total);
    if !response.status().is_success() {
        return Err(io::Error::other(format!(
            "upstream HTTP {} while requesting {expected}",
            response.status()
        )));
    }
    if response.status() != reqwest::StatusCode::PARTIAL_CONTENT
        || response
            .headers()
            .get("Content-Range")
            .and_then(|v| v.to_str().ok())
            != Some(expected.as_str())
    {
        return Err(error(
            io::ErrorKind::InvalidData,
            "upstream did not return the exact requested range",
        ));
    }
    let mut result = BytesMut::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| io::Error::other(e.without_url()))?;
        if result.len() as u64 + chunk.len() as u64 > range.end - range.start {
            return Err(error(
                io::ErrorKind::InvalidData,
                "oversized upstream range",
            ));
        }
        result.extend_from_slice(&chunk);
    }
    if result.len() as u64 != range.end - range.start {
        return Err(error(
            io::ErrorKind::UnexpectedEof,
            "truncated upstream range",
        ));
    }
    Ok(result.freeze())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    #[test]
    fn retiring_old_streams_does_not_cancel_new_ranges_or_the_cache() {
        let cache = SongCache::new(&std::env::temp_dir(), reqwest::Client::new(), vec![
            RemoteSource { url: "http://unused.invalid/video".into(), len: 1024 },
        ], DEFAULT_CACHE_BLOCK_BYTES).unwrap();
        let (old_id, old) = cache.stream_epoch();
        let cutoff = cache.stream_cutoff();
        let (new_id, new) = cache.stream_epoch();
        cache.retire_streams(cutoff);
        assert!(*old.borrow() >= old_id);
        assert!(*new.borrow() < new_id);
        assert!(!*cache.cancelled.borrow());
        cache.retire_streams(0); // A late completion cannot move the cutoff back.
        assert!(*old.borrow() >= old_id);
    }

    #[test]
    fn warmup_deduplicates_interleaved_blocks_in_first_use_order() {
        use crate::seekable_mp4::ReadPart::Source;
        let block = DEFAULT_CACHE_BLOCK_BYTES;
        let cache = SongCache::new(&std::env::temp_dir(), reqwest::Client::new(), vec![
            RemoteSource { url: "http://unused.invalid/video".into(), len: block * 3 },
            RemoteSource { url: "http://unused.invalid/audio".into(), len: block + 7 },
        ], block * 8).unwrap();
        let reads = cache.warmup_blocks(vec![
            Source { source: 0, range: block - 1..block + 5 },
            Source { source: 1, range: block..block + 7 },
            Source { source: 0, range: block..block + 6 },
        ]);
        assert_eq!(reads, vec![(0, 0..block), (0, block..block * 2), (1, block..block + 7)]);
        assert_eq!(cache.stats(), (0, 0));
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
        assert_eq!(cache.stats().0, 1);
        assert_eq!(std::fs::read_dir(&path).unwrap().count(), 1);
        drop(cache);
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn cross_block_reads_eviction_and_capacity_are_exact() {
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
        assert_eq!(cache.blocks.lock().await.len(), 1);
        assert_eq!(cache.hot.lock().await.len(), 1);
        let disk: u64 = std::fs::read_dir(&cache.directory.0)
            .unwrap()
            .map(|f| f.unwrap().metadata().unwrap().len())
            .sum();
        assert!(disk <= BLOCK_BYTES);
        assert_eq!(cache.read(0, 0..10).await.unwrap().as_ref(), &data[..10]);
        assert_eq!(server.requests.load(Ordering::Relaxed), 4);
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
        let result = tokio::time::timeout(Duration::from_millis(200), read)
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
        assert!(cache.read(0, 0..5).await.is_err());
        assert_eq!(server.requests.load(Ordering::Relaxed), 2);
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
        let result =
            tokio::time::timeout(Duration::from_secs(2), cache.read(0, 0..data.len() as u64))
                .await
                .unwrap()
                .unwrap();
        assert_eq!(result, data);
        assert_eq!(server.requests.load(Ordering::Relaxed), 2);
    }
}
