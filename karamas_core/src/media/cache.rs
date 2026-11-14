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
};
use tokio::sync::{Mutex, OnceCell, Semaphore, watch};

pub use super::upstream::fetch_range;

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
    /// 删除已释放的歌曲缓存目录。
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.0);
    }
}
struct Staging(PathBuf);
impl Drop for Staging {
    /// 删除未发布的缓存临时文件。
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
    /// 删除已释放的缓存块文件。
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

/// 构建指定类型的缓存 I/O 错误。
fn error(kind: io::ErrorKind, message: &str) -> io::Error {
    io::Error::new(kind, message)
}
/// 构建缓存读取已取消的错误。
fn cancelled() -> io::Error {
    error(io::ErrorKind::Interrupted, "song cache cancelled")
}

impl SongCache {
    /// 使用默认块大小创建歌曲缓存。
    pub fn new(
        parent: &Path,
        client: reqwest::Client,
        sources: Vec<RemoteSource>,
        capacity: u64,
    ) -> io::Result<Self> {
        Self::new_with_block_size(parent, client, sources, capacity, DEFAULT_CACHE_BLOCK_BYTES)
    }

    /// 按指定块大小和容量创建歌曲缓存。
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

    /// 取消当前歌曲的缓存读取和响应流。
    pub fn cancel(&self) {
        self.cancelled.send_replace(true);
        self.interrupt_streams();
    }
    /// A deliberate seek makes existing response streams obsolete, but keeps
    /// their completed cache blocks. New HTTP requests subscribe to the new epoch.
    pub fn interrupt_streams(&self) {
        self.retire_streams(self.stream_cutoff());
    }
    /// 读取当前响应流编号，供定位操作确定中断范围。
    pub fn stream_cutoff(&self) -> u64 {
        self.stream_id.load(Ordering::SeqCst)
    }
    /// 使指定编号及之前的响应流失效。
    pub fn retire_streams(&self, cutoff: u64) {
        self.stream_epoch.send_modify(|epoch| *epoch = (*epoch).max(cutoff));
    }
    /// 注册响应流，并返回编号和失效通知订阅。
    pub fn stream_epoch(&self) -> (u64, watch::Receiver<u64>) {
        let receiver = self.stream_epoch.subscribe();
        let id = self.stream_id.fetch_add(1, Ordering::SeqCst) + 1;
        (id, receiver)
    }
    /// 读取缓存下载请求数和命中数。
    pub fn stats(&self) -> (u64, u64) {
        (
            self.requests.load(Ordering::Relaxed),
            self.hits.load(Ordering::Relaxed),
        )
    }

    /// 将媒体读取范围转换为去重后的预取块列表。
    pub(crate) fn warmup_blocks(&self, parts: Vec<crate::media::mp4::indexed::ReadPart>) -> Vec<(usize, Range<u64>)> {
        let mut seen = std::collections::HashSet::new();
        let mut reads = Vec::new();
        for part in parts {
            if let crate::media::mp4::indexed::ReadPart::Source { source, range } = part {
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

    /// 读取一个缓存块，未命中时下载并保存。
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

#[cfg(test)]
mod tests;
