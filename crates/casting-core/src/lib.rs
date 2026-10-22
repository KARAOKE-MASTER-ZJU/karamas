//! 投屏核心。宿主通过引擎接口组合设备协议、房间同步和媒体服务。
pub mod cast;
pub mod config;
pub mod engine;
mod error;
pub mod media;
pub mod playlist;
pub mod sources;

#[cfg(target_os = "android")]
pub mod android;

pub use cast::dlna::{
    discover_device_from_url_core, discover_devices_core,
    settings::{get_dlna_direct, get_dlna_quality, set_dlna_direct, set_dlna_quality},
};
pub use config::{ANR_DEADLINE, API_TIMEOUT, PROXY_CONNECT_TIMEOUT, SOAP_TIMEOUT};
pub use engine::{
    CurrentBilibiliParts, ENGINE_STATE, EngineContext, connect_dlna_device, connect_room,
    cycle_quality_core, get_current_bilibili_parts_core, get_current_progress,
    get_current_song_id_core, get_current_song_title_core, get_danmaku_core, get_quality_core,
    get_total_duration, get_volume_core, jump_to_secs, poll_playback_progress, recast_current_song,
    reset_engine, select_current_bilibili_part_core, set_danmaku_core, set_quality_core,
    set_volume_core, start_bilibili_engine_core, start_engine_core, toggle_danmaku_core,
    toggle_pause_core, trigger_next_song, trigger_prev_song, volume_down_core, volume_up_core,
};
pub use error::CoreError;
pub use media::server::SharedState;
