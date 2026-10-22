pub mod bilibili_caster;
pub mod dlna_caster;
pub mod progress;

use async_trait::async_trait;
use std::fmt;

#[derive(Debug, Clone)]
pub struct SongRef(pub String);

#[derive(Debug, Clone, Copy, Default)]
pub struct Progress {
    pub current_secs: u32,
    pub total_secs: u32,
}

#[derive(Debug, Clone)]
pub struct Capabilities {
    pub absolute_volume: bool,
    pub seek: bool,
    pub hardware_progress: bool,
}

/// B站清晰度。数值是 B站 `/x/tv/stream/cmd` 接口本身定义的协议常量（qn），
/// 不是 ktv-casting 自己设计的格式，所以底层（JNI/网络请求）仍然传裸 u32，
/// 这个枚举只在 Rust 内部用来做类型安全（拒绝非法档位）。
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quality {
    P360 = 16,
    P480 = 32,
    P720 = 64,
    P1080 = 80,
    P1080H = 112,
    P1080P60 = 116,
    P4K = 120,
}

impl Quality {
    pub const ALL: [Quality; 7] = [
        Quality::P360,
        Quality::P480,
        Quality::P720,
        Quality::P1080,
        Quality::P1080H,
        Quality::P1080P60,
        Quality::P4K,
    ];

    pub fn as_qn(self) -> u32 {
        self as u32
    }

    pub fn from_qn(qn: u32) -> Option<Quality> {
        Self::ALL.into_iter().find(|q| q.as_qn() == qn)
    }

    pub fn label(self) -> &'static str {
        match self {
            Quality::P360 => "360P",
            Quality::P480 => "480P",
            Quality::P720 => "720P",
            Quality::P1080 => "1080P",
            Quality::P1080H => "1080PHigh",
            Quality::P1080P60 => "1080P60",
            Quality::P4K => "4K",
        }
    }

    /// 循环到下一档清晰度。
    pub fn next(self) -> Quality {
        let idx = Self::ALL.iter().position(|&q| q == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }
}

impl Default for Quality {
    fn default() -> Self {
        Quality::P1080
    }
}

#[derive(Debug)]
pub enum CastError {
    Unsupported,
    Device(String),
}

impl fmt::Display for CastError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CastError::Unsupported => write!(f, "not supported"),
            CastError::Device(s) => write!(f, "device error: {}", s),
        }
    }
}

impl std::error::Error for CastError {}

/// Retry control requests only on timeout, before receiving response headers.
/// Media preparation and response body reads stay outside this loop.
pub(crate) async fn retry_cast_request<T, F, Fut>(context: &str, mut send: F) -> Result<T, reqwest::Error>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, reqwest::Error>>,
{
    for attempt in 0..=2 {
        match send().await {
            Err(error) if attempt < 2 && error.is_timeout() => {
                log::warn!(
                    "{}：请求超时，1 秒后重发（{}/2），错误={}",
                    context,
                    attempt + 1,
                    error
                );
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
            result => return result,
        }
    }
    unreachable!("the third attempt always returns")
}

#[cfg(test)]
#[path = "../../test/cast_request.rs"]
mod request_tests;

#[async_trait]
pub trait Caster: Send + Sync {
    async fn play_song(&self, song: &SongRef) -> Result<(), CastError>;
    async fn resume(&self) -> Result<(), CastError>;
    async fn pause(&self) -> Result<(), CastError>;
    async fn stop(&self) -> Result<(), CastError>;
    async fn seek(&self, secs: u32) -> Result<(), CastError>;
    async fn get_progress(&self) -> Result<Progress, CastError>;
    async fn set_volume(&self, volume: u32) -> Result<(), CastError>;
    async fn get_volume(&self) -> Result<Option<u32>, CastError>;

    /// 相对调高音量。默认实现基于 get_volume/set_volume；不支持绝对音量的
    /// Caster（如 Bilibili 投屏）应覆盖此方法，改为发送设备原生的相对音量指令。
    async fn volume_up(&self, step: u32) -> Result<(), CastError> {
        let current = self.get_volume().await?.unwrap_or(0);
        self.set_volume((current + step).min(100)).await
    }

    /// 相对调低音量，参见 [`Caster::volume_up`]。
    async fn volume_down(&self, step: u32) -> Result<(), CastError> {
        let current = self.get_volume().await?.unwrap_or(0);
        self.set_volume(current.saturating_sub(step)).await
    }

    /// 弹幕开关。只有 Bilibili 投屏支持，其余 Caster 默认不支持。
    async fn set_danmaku(&self, _on: bool) -> Result<(), CastError> {
        Err(CastError::Unsupported)
    }
    fn get_danmaku(&self) -> Result<bool, CastError> {
        Err(CastError::Unsupported)
    }

    /// 切换清晰度。只有 Bilibili 投屏支持，其余 Caster 默认不支持。
    async fn set_quality(&self, _quality: Quality) -> Result<(), CastError> {
        Err(CastError::Unsupported)
    }
    fn get_quality(&self) -> Result<Quality, CastError> {
        Err(CastError::Unsupported)
    }

    fn capabilities(&self) -> Capabilities;
}
