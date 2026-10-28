//! Own one current song/quality representation. URLs carry a generation so an
//! old renderer request can never recreate a previous song's cache.
use crate::{
    cast::{MediaFormat, Quality, select_media_quality},
    media::cache::SongCache,
    media::mp4::dash,
    media::mp4::indexed::VirtualMp4,
    sources::bilibili::{BilibiliMedia, get_bilibili_media_with_formats, parse_bilibili_video},
};
use futures_util::StreamExt;
use std::{
    io,
    path::PathBuf,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{OnceCell, watch};

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
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
fn cache_root() -> io::Result<PathBuf> {
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

pub enum SessionMedia {
    Direct(String),
    Hls(crate::media::hls::Playlists),
    Seekable {
        mp4: Arc<VirtualMp4>,
        cache: Arc<SongCache>,
    },
}

pub struct MediaSession {
    pub generation: u64,
    pub song: String,
    pub quality: Quality,
    pub format: MediaFormat,
    allowed_formats: Vec<MediaFormat>,
    hls_direct: bool,
    resolved_quality: Mutex<Option<(Quality, MediaFormat)>>,
    stopped: watch::Sender<bool>,
    ready: OnceCell<Arc<SessionMedia>>,
    warmup: Mutex<Option<tokio::task::JoinHandle<()>>>,
}
impl MediaSession {
    pub fn resolved_quality(&self) -> (Quality, MediaFormat) {
        self.resolved_quality
            .lock()
            .unwrap()
            .unwrap_or((self.quality, self.format))
    }
    /// 生成包含当前会话代次的媒体路径。
    pub fn path(&self) -> String {
        let format = self.resolved_quality().1;
        if format == MediaFormat::Hls {
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
    fn cancel(&self) {
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

    /// 解析歌曲媒体源，按需准备可定位的 DASH 媒体。
    async fn resolve(&self, client: &reqwest::Client) -> io::Result<SessionMedia> {
        if crate::sources::youtube::is_youtube_url(&self.song) {
            let selected = select_media_quality(
                (self.quality, self.format),
                self.allowed_formats
                    .iter()
                    .copied()
                    .filter(|format| *format != MediaFormat::Mp4)
                    .flat_map(|format| {
                        [Quality::P720, Quality::P1080].map(|quality| (quality, format))
                    }),
            )
            .ok_or_else(|| io::Error::other("YouTube 没有符合目标的可选清晰度"))?;
            let max_height = match selected.0 {
                Quality::P1080 => 1080,
                _ => 720,
            };
            let tracks = crate::sources::youtube::resolve(&self.song, max_height).await?;
            log::info!(target: "DLNA1080", "YouTube 双轨就绪: generation={}, duration={}s", self.generation, tracks.duration_secs);
            let media = self
                .prepare_dash(
                    client,
                    &tracks.video_url,
                    &tracks.audio_url,
                    tracks.hls_tracks.as_ref(),
                    selected.1,
                )
                .await?;
            *self.resolved_quality.lock().unwrap() = Some(selected);
            return Ok(media);
        }
        let bilibili = parse_bilibili_video(&self.song);
        if bilibili.is_err()
            && (self.song.starts_with("http://") || self.song.starts_with("https://"))
        {
            *self.resolved_quality.lock().unwrap() = Some((self.quality, MediaFormat::Mp4));
            return Ok(SessionMedia::Direct(self.song.clone()));
        }
        let (bvid, page) = bilibili.map_err(io::Error::other)?;
        let resolved = get_bilibili_media_with_formats(
            &bvid,
            Some(page),
            self.quality.as_qn(),
            self.format,
            &self.allowed_formats,
        )
        .await
        .map_err(io::Error::other)?;
        let media = match resolved.media {
            BilibiliMedia::Direct { url } => SessionMedia::Direct(url),
            BilibiliMedia::Dash {
                video_url,
                audio_url,
                ..
            } => {
                self.prepare_dash(
                    client,
                    &video_url,
                    &audio_url,
                    resolved.hls_tracks.as_ref(),
                    resolved.format,
                )
                .await?
            }
        };
        *self.resolved_quality.lock().unwrap() = Some((resolved.quality, resolved.format));
        Ok(media)
    }

    /// 构建 DASH 媒体的 MP4 索引和歌曲缓存。
    async fn prepare_dash(
        &self,
        client: &reqwest::Client,
        video_url: &str,
        audio_url: &str,
        hls_tracks: Option<&(crate::media::hls::Track, crate::media::hls::Track)>,
        format: MediaFormat,
    ) -> io::Result<SessionMedia> {
        if format == MediaFormat::Hls {
            let (video, audio) =
                hls_tracks.ok_or_else(|| io::Error::other("媒体源缺少 HLS 所需的分段信息"))?;
            return Ok(SessionMedia::Hls(
                crate::media::hls::Playlists::prepare(client, video, audio, !self.hls_direct).await?,
            ));
        }
        let prepared = tokio::time::timeout(
            Duration::from_secs(60),
            dash::prepare(client, video_url, audio_url),
        )
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "MP4 preparation timeout"))??;
        let cache = Arc::new(SongCache::new(
            &cache_root()?,
            client.clone(),
            prepared.sources,
            256 * 1024 * 1024,
        )?);
        Ok(SessionMedia::Seekable {
            mp4: Arc::new(prepared.mp4),
            cache,
        })
    }
}

#[derive(Default)]
pub struct MediaSessions {
    current: Mutex<Option<Arc<MediaSession>>>,
}
impl MediaSessions {
    /// 获取尚未失效的当前媒体会话。
    pub fn current(&self) -> Option<Arc<MediaSession>> {
        self.current
            .lock()
            .ok()?
            .as_ref()
            .filter(|s| !s.is_stopped())
            .cloned()
    }
    /// 按保存的偏好创建媒体会话；实际选档由当前曲目的可用点决定。
    pub fn activate_with_format(
        &self,
        song: &str,
        quality: Quality,
        format: MediaFormat,
    ) -> Arc<MediaSession> {
        self.activate_with_formats(
            song,
            quality,
            format,
            &[MediaFormat::Mp4, MediaFormat::Dash, MediaFormat::Hls],
        )
    }

    /// 限定当前播放端允许的模式，避免回退到已关闭的模式。
    pub(crate) fn activate_with_formats(
        &self,
        song: &str,
        quality: Quality,
        format: MediaFormat,
        allowed_formats: &[MediaFormat],
    ) -> Arc<MediaSession> {
        self.activate(song, quality, format, allowed_formats, false)
    }

    pub fn activate_hls_with_format(
        &self,
        song: &str,
        quality: Quality,
        format: MediaFormat,
    ) -> Arc<MediaSession> {
        self.activate(
            song,
            quality,
            format,
            &[MediaFormat::Mp4, MediaFormat::Hls],
            true,
        )
    }

    fn activate(
        &self,
        song: &str,
        quality: Quality,
        format: MediaFormat,
        allowed_formats: &[MediaFormat],
        hls_direct: bool,
    ) -> Arc<MediaSession> {
        let session = Arc::new(MediaSession {
            generation: NEXT_GENERATION.fetch_add(1, Ordering::Relaxed),
            song: song.into(),
            quality,
            format,
            allowed_formats: allowed_formats.to_vec(),
            hls_direct,
            resolved_quality: Mutex::new(None),
            stopped: watch::channel(false).0,
            ready: OnceCell::new(),
            warmup: Mutex::new(None),
        });
        let previous = self.current.lock().unwrap().replace(session.clone());
        if let Some(previous) = previous {
            previous.cancel();
        }
        log::info!(target: "DLNA1080", "切换当前曲目媒体会话: generation={}, quality={}", session.generation, quality.label());
        session
    }
    /// 按代次获取仍有效的当前媒体会话。
    pub fn get(&self, generation: u64) -> Option<Arc<MediaSession>> {
        self.current
            .lock()
            .ok()?
            .as_ref()
            .filter(|s| s.generation == generation && !s.is_stopped())
            .cloned()
    }
    /// 移除并取消当前媒体会话。
    pub fn clear(&self) {
        if let Some(previous) = self.current.lock().unwrap().take() {
            previous.cancel();
        }
    }
}
impl Drop for MediaSessions {
    /// 销毁会话集合时清理当前会话。
    fn drop(&mut self) {
        self.clear();
    }
}

/// 从媒体路径中解析会话代次。
pub fn generation_from_path(path: &str) -> Option<u64> {
    let path = path.strip_prefix("__ktv_media/")?;
    if let Some((generation, name)) = path.split_once('/') {
        matches!(name, "master.m3u8" | "video.m3u8" | "audio.m3u8" | "video.mp4" | "audio.mp4")
            .then(|| generation.parse().ok())
            .flatten()
    } else {
        path.strip_suffix(".mp4")?.parse().ok()
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
    #[tokio::test]
    async fn repeated_prepare_shares_one_immutable_representation() {
        let sessions = MediaSessions::default();
        let s = sessions.activate_with_format(
            "https://example.invalid/test.mp4",
            Quality::P720,
            MediaFormat::Mp4,
        );
        let client = reqwest::Client::new();
        let (a, b) = tokio::join!(s.prepare(&client), s.prepare(&client));
        assert!(Arc::ptr_eq(&a.unwrap(), &b.unwrap()));
    }
}
