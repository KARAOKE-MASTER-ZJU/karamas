use crate::{
    error::CommandResult,
    logging::{ExportAction, LOG_EVENT, LogBuffer, LogSnapshot},
    models::LogLevel,
};
use std::sync::Arc;
use tauri::{Emitter, Runtime, State};

/// 设置后续日志的记录级别。
#[tauri::command]
pub fn set_log_level(level: LogLevel) {
    log::set_max_level(level.into());
}

/// 前端日志也通过应用日志器记录，使用相同级别和留存规则。
#[tauri::command]
pub fn write_log(level: LogLevel, message: String) {
    log::log!(target: "frontend", log::Level::from(level), "{}", message);
}

/// 按展示条件读取留存日志，不改变记录级别。
#[tauri::command]
pub async fn get_logs(
    buffer: State<'_, Arc<LogBuffer>>,
    level: LogLevel,
    query: String,
    limit: Option<usize>,
) -> CommandResult<LogSnapshot> {
    Ok(buffer.snapshot(level, &query, limit))
}

/// 清空记录源，并通知正在查看日志的页面。
#[tauri::command]
pub fn clear_logs<R: Runtime>(app: tauri::AppHandle<R>, buffer: State<'_, Arc<LogBuffer>>) {
    buffer.clear();
    let _ = app.emit(LOG_EVENT, ());
}

#[tauri::command]
pub fn can_share_logs() -> bool {
    crate::logging::can_share_logs()
}

#[tauri::command]
pub async fn export_logs<R: Runtime>(
    app: tauri::AppHandle<R>,
    window: tauri::WebviewWindow<R>,
    buffer: State<'_, Arc<LogBuffer>>,
    action: ExportAction,
    position: Option<tauri_plugin_sharekit::SharePosition>,
) -> CommandResult<bool> {
    crate::logging::export_logs(app, window, buffer.inner().clone(), action, position).await
}
