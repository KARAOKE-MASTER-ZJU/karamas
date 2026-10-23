//! 单会话生命周期、播放协调和面向宿主的控制接口。
mod controls;
mod lifecycle;
mod parts;
mod playback;
mod state;

pub use controls::{
    cycle_quality_core, get_current_song_id_core, get_current_song_title_core, get_danmaku_core,
    get_quality_core, get_volume_core, jump_to_secs, pause_core, play_core, set_danmaku_core,
    set_quality_core, set_volume_core, toggle_danmaku_core, toggle_pause_core, trigger_next_song,
    trigger_prev_song, volume_down_core, volume_up_core,
};
pub use lifecycle::{
    connect_dlna_device, connect_room, reset_engine, start_bilibili_engine_core, start_engine_core,
};
pub use parts::{
    CurrentBilibiliParts, get_current_bilibili_parts_core, select_current_bilibili_part_core,
};
pub use playback::{
    get_current_progress, get_total_duration, poll_playback_progress, recast_current_song,
};
pub use state::{ENGINE_STATE, EngineContext};
