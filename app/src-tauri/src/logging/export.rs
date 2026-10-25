use super::LogBuffer;
use crate::{
    error::{CommandError, CommandResult},
    models::{LogLevel, SharePosition},
};
use chrono::{DateTime, Local};
use serde::Deserialize;
use std::{
    io::Write,
    sync::Arc,
    time::{Duration, SystemTime},
};
use tauri::Runtime;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_fs::{FsExt, OpenOptions};

#[cfg(any(target_os = "android", target_os = "ios", target_os = "macos"))]
mod share;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExportAction {
    Save,
    Share,
}

pub fn can_share_logs() -> bool {
    // ShareKit 0.3 的 Windows 取消回调可能不返回，暂不开放；Linux 没有系统分享实现。
    cfg!(any(
        target_os = "android",
        target_os = "ios",
        target_os = "macos"
    ))
}

pub async fn export_logs<R: Runtime>(
    app: tauri::AppHandle<R>,
    window: tauri::WebviewWindow<R>,
    buffer: Arc<LogBuffer>,
    action: ExportAction,
    position: Option<SharePosition>,
) -> CommandResult<bool> {
    tauri::async_runtime::spawn_blocking(move || {
        let text = buffer
            .snapshot(LogLevel::Trace, "", None)
            .entries
            .iter()
            .map(|entry| {
                let time: DateTime<Local> =
                    (SystemTime::UNIX_EPOCH + Duration::from_millis(entry.time)).into();
                format!(
                    "{} [{}] [{}] {}",
                    time.format("%Y-%m-%d %H:%M:%S%.3f %:z"),
                    log::Level::from(entry.level).as_str().to_ascii_lowercase(),
                    entry.target,
                    entry.message
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let filename = Local::now()
            .format("karamas-logs-%Y%m%d-%H%M%S-%9f.txt")
            .to_string();
        match action {
            ExportAction::Save => {
                let dialog = app
                    .dialog()
                    .file()
                    .set_title("保存日志")
                    .set_file_name(&filename)
                    .add_filter("日志文件", &["txt"]);
                #[cfg(desktop)]
                let dialog = dialog.set_parent(&window);
                let Some(path) = dialog.blocking_save_file() else {
                    return Ok(false);
                };
                let mut options = OpenOptions::new();
                options.read(false).write(true).create(true).truncate(true);
                app.fs()
                    .open(path, options)
                    .and_then(|mut file| file.write_all(text.as_bytes()))
                    .map_err(|error| CommandError::new("LOG_SAVE", error.to_string()))?;
                Ok(true)
            }
            ExportAction::Share => {
                #[cfg(any(target_os = "android", target_os = "ios", target_os = "macos"))]
                {
                    share::export_logs(app, window, text, filename, position)
                }
                #[cfg(not(any(target_os = "android", target_os = "ios", target_os = "macos")))]
                {
                    let _ = position;
                    Err(CommandError::new(
                        "UNSUPPORTED_PLATFORM",
                        "当前平台不支持系统分享，请使用保存日志",
                    ))
                }
            }
        }
    })
    .await
    .map_err(|error| CommandError::new("LOG_EXPORT", error.to_string()))?
}
