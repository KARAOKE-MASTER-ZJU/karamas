use casting_core::cast::{Capabilities, bilibili::BilibiliSession, dlna::DlnaDevice};
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
    pub title: Option<String>,
    pub current_secs: Option<u32>,
    pub total_secs: Option<u32>,
    /// 核心库记录的播放状态，不是设备状态的实时读回。
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
