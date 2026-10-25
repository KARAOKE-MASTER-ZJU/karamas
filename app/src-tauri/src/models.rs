use karamas_core::cast::{Capabilities, bilibili::BilibiliSession, dlna::DlnaDevice};
use serde::{Deserialize, Serialize};

/// 仅传输设备标识，不将 UPnP 协议对象暴露给前端。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DlnaDeviceInfo {
    pub name: String,
    pub location: String,
}

impl From<DlnaDevice> for DlnaDeviceInfo {
    fn from(device: DlnaDevice) -> Self {
        Self {
            name: device.friendly_name,
            location: device.location,
        }
    }
}

/// 启动会话所需的目标设备参数。
#[derive(Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CastTarget {
    Dlna {
        location: String,
    },
    Bilibili {
        /// 嵌套会话沿用核心库字段名，如 access_token，不受外层 camelCase 影响。
        session: BilibiliSession,
        device_buvid: String,
        sender_buvid: String,
    },
}

/// 设备支持的控制能力，供界面选择可用控件。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCapabilities {
    /// 是否支持读取和设置绝对音量。
    pub absolute_volume: bool,
    pub seek: bool,
    /// 是否从设备查询真实进度；否则由核心库本地计时。
    pub hardware_progress: bool,
}

impl From<Capabilities> for DeviceCapabilities {
    fn from(value: Capabilities) -> Self {
        Self {
            absolute_volume: value.absolute_volume,
            seek: value.seek,
            hardware_progress: value.hardware_progress,
        }
    }
}

/// 当前播放状态；歌单字段来自最近同步，未取得有效进度时使用 null。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackState {
    pub song_id: Option<String>,
    /// 包含分 P 信息，供界面在同一歌曲切换分 P 时刷新清晰度。
    pub song_url: Option<String>,
    pub title: Option<String>,
    pub added_by: Option<String>,
    pub current_secs: Option<u32>,
    pub total_secs: Option<u32>,
    /// 当前播放状态，B站投屏由设备回传同步。
    pub is_playing: bool,
    pub queued_count: usize,
    pub sung_count: usize,
    /// B站协议清晰度数值，查询不可用或失败时为 null。
    pub quality_qn: Option<u32>,
    /// 弹幕状态，设备不支持或查询失败时为 null。
    pub danmaku: Option<bool>,
    pub capabilities: DeviceCapabilities,
}

/// 重新投屏的正常结果；失败和超时通过命令错误返回。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RecastResult {
    /// 播放请求已成功确认。
    Played,
    NoCurrentSong,
    /// 当前已有播放操作正在处理。
    Busy,
}

/// 日志级别按详细程度到严重程度排序。
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    #[serde(rename = "warning")]
    Warn,
    Error,
}

impl From<LogLevel> for log::Level {
    fn from(level: LogLevel) -> Self {
        match level {
            LogLevel::Trace => Self::Trace,
            LogLevel::Debug => Self::Debug,
            LogLevel::Info => Self::Info,
            LogLevel::Warn => Self::Warn,
            LogLevel::Error => Self::Error,
        }
    }
}

impl From<LogLevel> for log::LevelFilter {
    fn from(level: LogLevel) -> Self {
        log::Level::from(level).to_level_filter()
    }
}

impl From<log::Level> for LogLevel {
    fn from(level: log::Level) -> Self {
        match level {
            log::Level::Trace => Self::Trace,
            log::Level::Debug => Self::Debug,
            log::Level::Info => Self::Info,
            log::Level::Warn => Self::Warn,
            log::Level::Error => Self::Error,
        }
    }
}
