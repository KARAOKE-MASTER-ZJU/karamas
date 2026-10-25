pub mod bilibili;
pub mod devices;
pub mod logging;
pub mod playback;
pub mod room;
pub mod session;

/// 注册应用实际提供的 IPC 命令。
pub fn handler<R: tauri::Runtime>() -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static
{
    tauri::generate_handler![
        crate::platform::move_to_background,
        logging::set_log_level,
        logging::write_log,
        logging::get_logs,
        logging::clear_logs,
        logging::export_logs,
        logging::can_share_logs,
        room::enter_room,
        devices::discover_dlna_devices,
        devices::discover_dlna_device,
        devices::list_bilibili_devices,
        session::start_casting,
        session::disconnect_casting,
        session::get_dlna_direct,
        session::set_dlna_direct,
        playback::poll_playback,
        playback::play,
        playback::pause,
        playback::toggle_pause,
        playback::next_song,
        playback::previous_song,
        playback::recast_current_song,
        playback::seek,
        playback::get_volume,
        playback::set_volume,
        playback::change_volume,
        playback::set_danmaku,
        playback::set_quality,
        playback::get_bilibili_qualities,
        playback::get_current_bilibili_parts,
        playback::select_current_bilibili_part,
        bilibili::login_bilibili,
        bilibili::cancel_bilibili_login,
        bilibili::load_bilibili_session,
        bilibili::save_bilibili_session,
        bilibili::clear_bilibili_session,
    ]
}
