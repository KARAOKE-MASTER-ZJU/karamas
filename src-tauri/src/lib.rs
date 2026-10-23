mod commands;
mod error;
mod logging;
mod models;
mod state;

use state::BridgeState;
use tauri::Manager;

/// 启动 Tauri 应用，注册命令、接入状态和日志转发。
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // 接入层只保存资源和调用协调状态，播放状态由核心库维护。
            app.manage(BridgeState::new(
                app.path().app_data_dir()?,
                app.path().app_cache_dir()?,
            ));
            logging::initialize(app.handle().clone())?;
            Ok(())
        })
        .invoke_handler(commands::handler())
        .build(tauri::generate_context!())
        .expect("failed to build KTV Casting")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                casting_core::reset_engine();
                // 退出回调不等待异步锁，能取得句柄时取消扫码任务。
                if let Some(state) = app.try_state::<BridgeState>()
                    && let Ok(mut login) = state.login.try_lock()
                    && let Some(task) = login.take()
                {
                    task.abort();
                }
            }
        });
}
