//! 单个媒体会话的生命周期、准备结果共享和 seek 预取。
use crate::{
    cast::{MediaFormat, Quality},
    media::{cache::SongCache, mp4::indexed::VirtualMp4},
};
use futures_util::StreamExt;
use std::{
    io,
    path::PathBuf,
    sync::{Arc, Mutex, RwLock},
    time::Instant,
};
use tokio::sync::{OnceCell, watch};

mod registry;
mod resolve;
pub use registry::{MediaSessions, generation_from_path};

static CACHE_ROOT: RwLock<Option<PathBuf>> = RwLock::new(None);

/// 设置并创建媒体缓存根目录。
pub fn init_cache_dir(path: &str) -> io::Result<()> {
    let root = PathBuf::from(path).join("ktv-media-cache");
    std::fs::create_dir_all(&root)?;
    *CACHE_ROOT
        .write()
        .map_err(|_| io::Error::other("cache root lock poisoned"))? = Some(root);
    Ok(())
}
/// 获取当前媒体缓存根目录。
pub(super) fn cache_root() -> io::Result<PathBuf> {
    if let Some(root) = CACHE_ROOT
        .read()
        .map_err(|_| io::Error::other("cache root lock poisoned"))?
        .clone()
    {
        return Ok(root);
    }
    #[cfg(target_os = "android")]
    return Err(io::Error::other(
        "Android media cache directory is not initialized",
    ));
    #[cfg(not(target_os = "android"))]
    Ok(std::env::temp_dir())
}
/// 构建媒体会话已失效的错误。
fn stopped() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "obsolete media session")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PlaybackMediaType {
    Native,
    Hls,
    Flv,
}

pub enum SessionMedia {
    Direct(String),
    Mp4(String),
    Flv {
        url: url::Url,
        filesize: Option<u64>,
    },
    Hls(crate::media::hls::Playlists),
    Live(crate::media::live::LiveMedia),
    Seekable {
        mp4: Arc<VirtualMp4>,
        cache: Arc<SongCache>,
    },
}

impl SessionMedia {
    pub fn playback_type(&self) -> PlaybackMediaType {
        match self {
            Self::Direct(_) | Self::Mp4(_) | Self::Seekable { .. } => PlaybackMediaType::Native,
            Self::Hls(_) | Self::Live(_) => PlaybackMediaType::Hls,
            Self::Flv { .. } => PlaybackMediaType::Flv,
        }
    }
}

pub struct MediaSession {
    pub generation: u64,
    pub song: String,
    pub quality: Quality,
    pub format: MediaFormat,
    allowed_formats: Vec<MediaFormat>,
    hls_direct: bool,
    local: bool,
    resolved_quality: Mutex<Option<(Quality, MediaFormat)>>,
    stopped: watch::Sender<bool>,
    ready: OnceCell<Arc<SessionMedia>>,
    live_source: OnceCell<Option<crate::sources::live::Resolved>>,
    warmup: Mutex<Option<tokio::task::JoinHandle<()>>>,
    audio_delay_ms: i32,
}
impl MediaSession {
    pub fn audio_delay_ms(&self) -> Option<i32> {
        let media = self.ready.get()?;
        match media.as_ref() {
            SessionMedia::Seekable { .. } => Some(self.audio_delay_ms),
            SessionMedia::Mp4(_) => Some(0),
            SessionMedia::Hls(_) => Some(if self.hls_direct { 0 } else { self.audio_delay_ms }),
            SessionMedia::Live(media) if media.supports_audio_delay() => Some(self.audio_delay_ms),
            _ => None,
        }
    }

    /// 返回当前会话识别出的直播属性；尚未识别时返回 false。
    pub fn is_live(&self) -> bool {
        self.live_source.get().is_some_and(Option::is_some)
            || self
                .ready
                .get()
                .is_some_and(|media| matches!(media.as_ref(), SessionMedia::Live(_)))
    }

    pub fn resolved_quality(&self) -> (Quality, MediaFormat) {
        self.resolved_quality
            .lock()
            .unwrap()
            .unwrap_or((self.quality, self.format))
    }
    /// 生成包含当前会话代次的媒体路径。
    pub fn path(&self) -> String {
        let format = self.resolved_quality().1;
        if self.local && format != MediaFormat::Hls {
            format!("__ktv_media/{}/source", self.generation)
        } else if format == MediaFormat::Hls {
            format!("__ktv_media/{}/master.m3u8", self.generation)
        } else {
            format!("__ktv_media/{}.mp4", self.generation)
        }
    }
    /// 检查当前媒体会话是否已失效。
    pub fn is_stopped(&self) -> bool {
        *self.stopped.borrow()
    }

    /// 等待会话取消，停止旧曲目的分片下载。
    pub async fn cancelled(&self) {
        let mut stop = self.stopped.subscribe();
        while !*stop.borrow_and_update() {
            if stop.changed().await.is_err() {
                break;
            }
        }
    }
    /// 取消当前会话的媒体准备、预取和缓存读取。
    pub(crate) fn cancel(&self) {
        if let Some(task) = self.warmup.lock().unwrap().take() {
            task.abort();
        }
        self.stopped.send_replace(true);
        if let Some(media) = self.ready.get() {
            if let SessionMedia::Seekable { cache, .. } = media.as_ref() {
                cache.cancel();
            }
        }
    }

    /// Overlap cold-range fetching with the renderer processing the Seek
    /// command. Only the latest target is warmed; no whole-song predownload.
    pub fn warm_seek(&self, seconds: u32) {
        let mut slot = self.warmup.lock().unwrap();
        if let Some(task) = slot.take() {
            task.abort();
        }
        if self.is_stopped() {
            return;
        }
        let Some(media) = self.ready.get() else {
            return;
        };
        let SessionMedia::Seekable { mp4, cache } = media.as_ref() else {
            return;
        };
        let Ok(parts) = mp4.read_plan(mp4.seek_warmup_range(seconds)) else {
            return;
        };
        let cache = cache.clone();
        // Deduplicate and queue source blocks in presentation order. Small
        // samples from the same block must not occupy all the warm-up slots.
        let reads = cache.warmup_blocks(parts);
        *slot = Some(tokio::spawn(async move {
            let started = Instant::now();
            let mut reads = futures_util::stream::iter(reads)
                .map(|(source, range)| {
                    let cache = cache.clone();
                    async move { cache.read(source, range).await }
                })
                .buffered(4);
            while let Some(result) = reads.next().await {
                if let Err(error) = result {
                    if error.kind() != io::ErrorKind::Interrupted {
                        log::debug!(target: "DLNA1080", "定位预取未完成: target={}s, error={}", seconds, error);
                    }
                    return;
                }
            }
            log::debug!(target: "DLNA1080", "定位窗口预取完成: target={}s, elapsed_ms={}", seconds, started.elapsed().as_millis());
        }));
    }

    /// 获取可定位媒体的缓存和当前响应流编号。
    pub fn seek_stream_cutoff(&self) -> Option<(Arc<SongCache>, u64)> {
        let SessionMedia::Seekable { cache, .. } = self.ready.get()?.as_ref() else {
            return None;
        };
        Some((cache.clone(), cache.stream_cutoff()))
    }

    /// Share the complete preparation across probing/streaming requests. No
    /// mutex is held across network I/O and cancellation interrupts preparation.
    pub async fn prepare(&self, client: &reqwest::Client) -> io::Result<Arc<SessionMedia>> {
        let mut stop = self.stopped.subscribe();
        if *stop.borrow() {
            return Err(stopped());
        }
        let started = Instant::now();
        let result = tokio::select! {
            biased;
            _ = stop.changed() => return Err(stopped()),
            result = self.ready.get_or_try_init(|| async {
                let media = self.resolve(client).await?;
                if self.is_stopped() { return Err(stopped()); }
                log::info!(target: "DLNA1080", "当前曲目媒体就绪: generation={}, quality={}, preparation_ms={}", self.generation, self.quality.label(), started.elapsed().as_millis());
                Ok::<_, io::Error>(Arc::new(media))
            }) => result,
        }?;
        // Cancellation may race with committing OnceCell. Always cancel the
        // newly committed cache as well, so no old-generation stream survives.
        if self.is_stopped() {
            self.cancel();
            return Err(stopped());
        }
        Ok(result.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn old_paths_cannot_reactivate_a_song_after_switching() {
        let sessions = MediaSessions::default();
        let first = sessions.activate_with_format(
            "https://example.invalid/one.mp4",
            Quality::P720,
            MediaFormat::Mp4,
        );
        let second = sessions.activate_with_format(
            "https://example.invalid/two.mp4",
            Quality::P1080,
            MediaFormat::Dash,
        );
        assert!(sessions.get(first.generation).is_none());
        assert!(first.is_stopped());
        assert_eq!(
            first
                .prepare(&reqwest::Client::new())
                .await
                .err()
                .unwrap()
                .kind(),
            io::ErrorKind::Interrupted
        );
        assert!(sessions.get(second.generation).is_some());
        assert_eq!(
            generation_from_path(&second.path()),
            Some(second.generation)
        );
        sessions.clear();
        assert!(second.is_stopped());
    }
}
