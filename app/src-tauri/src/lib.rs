mod commands;
mod error;
mod logging;
mod models;
mod platform;
mod session_store;
mod state;
mod updater;

use state::BridgeState;
use tauri::Manager;

/// 启动 Tauri 应用，注册命令、接入状态和日志转发。
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let builder = tauri::Builder::default();

    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build());

    #[cfg(target_os = "android")]
    let builder = builder.plugin(platform::init());

    #[cfg(any(target_os = "android", target_os = "ios", target_os = "macos"))]
    let builder = builder.plugin(tauri_plugin_sharekit::init());

    builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            logging::initialize(app.handle().clone())?;
            app.manage(updater::UpdateState::default());
            app.manage(platform::playback_window::PlaybackWindowState::default());
            // 接入层只保存资源和调用协调状态，播放状态由核心库维护。
            app.manage(BridgeState::new(
                app.path().app_data_dir()?,
                app.path().app_cache_dir()?,
            ));
            Ok(())
        })
        .on_window_event(platform::playback_window::close_with_main)
        .on_page_load(platform::playback_window::on_page_load)
        .invoke_handler(commands::handler())
        .build(tauri::generate_context!())
        .expect("failed to build KARAM@S")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                karamas_core::reset_engine();
                // 退出回调不等待异步锁，能取得句柄时取消扫码任务。
                if let Some(state) = app.try_state::<BridgeState>()
                    && let Ok(mut login) = state.login.try_lock()
                    && let Some(task) = login.take()
                {
                    task.abort();
                }
                log::logger().flush();
            }
        });
}
