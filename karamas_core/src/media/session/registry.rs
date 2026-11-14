//! 当前会话的切换与失效、代次标识和代理资源路径。
use super::MediaSession;
use crate::{
    cast::{MediaFormat, Quality},
    sources::bilibili::parse_bilibili_video,
};
use std::{
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicI32, AtomicU64, Ordering},
    },
};
use tokio::sync::{OnceCell, watch};

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

pub struct MediaSessions {
    current: Mutex<Option<Arc<MediaSession>>>,
    proxy_token: String,
    audio_delay_ms: AtomicI32,
}
impl Default for MediaSessions {
    fn default() -> Self {
        Self {
            current: Mutex::new(None),
            proxy_token: uuid::Uuid::new_v4().simple().to_string(),
            audio_delay_ms: AtomicI32::new(0),
        }
    }
}
impl MediaSessions {
    pub fn set_audio_delay_ms(&self, milliseconds: i32) -> io::Result<()> {
        if !(-5000..=5000).contains(&milliseconds) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "声音偏移须在 -5000 到 5000 毫秒之间",
            ));
        }
        self.audio_delay_ms.store(milliseconds, Ordering::Relaxed);
        Ok(())
    }

    /// 为代理资源添加当前连接的访问 token，保留资源路径和查询参数。
    pub fn proxy_path(&self, resource: &str) -> String {
        format!("{}/{resource}", self.proxy_token)
    }

    /// 校验路径前缀，返回原来的资源路径。
    pub(crate) fn resource_path<'a>(&self, path: &'a str) -> Option<&'a str> {
        let (token, resource) = path.split_once('/')?;
        (token == self.proxy_token).then_some(resource)
    }

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

    pub fn activate_local_with_format(
        &self,
        song: &str,
        quality: Quality,
        format: MediaFormat,
    ) -> Arc<MediaSession> {
        self.activate(
            song,
            quality,
            format,
            &[MediaFormat::Mp4, MediaFormat::Dash, MediaFormat::Hls],
            false,
            true,
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
        self.activate(song, quality, format, allowed_formats, false, false)
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
            false,
        )
    }

    fn activate(
        &self,
        song: &str,
        quality: Quality,
        format: MediaFormat,
        allowed_formats: &[MediaFormat],
        hls_direct: bool,
        local: bool,
    ) -> Arc<MediaSession> {
        let audio_delay_ms = self.audio_delay_ms.load(Ordering::Relaxed);
        let remux = audio_delay_ms != 0
            && (parse_bilibili_video(song).is_ok()
                || crate::sources::youtube::is_youtube_url(song));
        let session = Arc::new(MediaSession {
            generation: NEXT_GENERATION.fetch_add(1, Ordering::Relaxed),
            song: song.into(),
            quality,
            format: if remux { MediaFormat::Dash } else { format },
            allowed_formats: if remux {
                vec![MediaFormat::Dash]
            } else {
                allowed_formats.to_vec()
            },
            hls_direct: hls_direct && !remux,
            local,
            resolved_quality: Mutex::new(None),
            stopped: watch::channel(false).0,
            ready: OnceCell::new(),
            live_source: OnceCell::new(),
            warmup: Mutex::new(None),
            audio_delay_ms,
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
    let (generation, _) = path.split_once('/').or_else(|| path.split_once('.'))?;
    generation.parse().ok()
}
