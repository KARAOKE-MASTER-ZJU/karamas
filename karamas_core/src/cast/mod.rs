//! 设备能力、控制契约和协议实现。
pub mod bilibili;
pub mod dlna;
pub mod local;
pub mod progress;
mod request;

use async_trait::async_trait;
use std::fmt;

#[derive(Debug, Clone)]
pub struct SongRef(pub String);

#[derive(Debug, Clone, Copy, Default)]
pub struct Progress {
    pub current_secs: u32,
    pub total_secs: u32,
}

/// 投屏器维护的当前播放状态，包含回传校正后的进度。
#[derive(Debug, Clone, Copy)]
pub struct PlaybackSnapshot {
    pub progress: Progress,
    /// 播放/暂停状态；缓冲和定位不改变此值。
    pub is_playing: bool,
    /// None 表示当前没有可用的结束状态。
    pub ended: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct Capabilities {
    pub absolute_volume: bool,
    pub seek: bool,
    pub hardware_progress: bool,
}

/// B站清晰度。数值沿用 `/x/tv/stream/cmd` 接口定义的协议常量（qn），
/// 网络请求传递对应的 u32，Rust 内部用枚举校验合法档位。
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

/// 播放目标类型，与当前播放状态无关。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CasterKind {
    Dlna,
    Bilibili,
    Local,
}

/// Caster 抽象接口
#[async_trait]
pub trait Caster: Send + Sync {
    fn kind(&self) -> CasterKind;

    fn report_local_playback(&self, _report: local::LocalReport) -> Result<bool, CastError> {
        Err(CastError::Unsupported)
    }

    /// 停止设备状态订阅并关闭接收连接，不向设备发送停止播放命令。
    fn stop_status_subscription(&self) {}

    async fn play_song(&self, song: &SongRef) -> Result<(), CastError>;
    async fn resume(&self) -> Result<(), CastError>;
    async fn pause(&self) -> Result<(), CastError>;
    async fn stop(&self) -> Result<(), CastError>;
    async fn seek(&self, secs: u32) -> Result<(), CastError>;
    async fn get_progress(&self) -> Result<Progress, CastError>;

    /// 有状态回传的投屏器提供同步快照，其余协议沿用原有查询方式。
    fn playback_snapshot(&self) -> Option<PlaybackSnapshot> {
        None
    }

    /// 是否允许当前进度触发自动切歌；交互期间可暂时禁止。
    fn can_auto_advance(&self) -> bool {
        true
    }
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
