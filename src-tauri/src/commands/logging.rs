use crate::models::LogLevel;

/// 设置后续日志的记录级别。
#[tauri::command]
pub fn set_log_level(level: LogLevel) {
    log::set_max_level(level.into());
}
