use super::LogBuffer;
use crate::{
    error::{CommandError, CommandResult},
    models::LogLevel,
};
use chrono::{DateTime, Local};
use serde::Deserialize;
use std::{
    fs,
    io::Write,
    path::Path,
    sync::Arc,
    time::{Duration, SystemTime},
};
use tauri::{Manager, Runtime};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_fs::{FsExt, OpenOptions};
use tauri_plugin_sharekit::{ShareExt, ShareFileOptions, SharePosition};

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
                if !can_share_logs() {
                    return Err(CommandError::new(
                        "UNSUPPORTED_PLATFORM",
                        "当前平台不支持系统分享，请使用保存日志",
                    ));
                }
                let cache = app
                    .path()
                    .app_cache_dir()
                    .map_err(|error| CommandError::new("LOG_SHARE", error.to_string()))?;
                let directory = cache.join("log-exports");
                fs::create_dir_all(&directory)
                    .map_err(|error| CommandError::new("LOG_SHARE", error.to_string()))?;
                // 插件会再复制一份到缓存根目录；源文件放在子目录，避免复制到自身后被截断。
                remove_old_exports(&directory);
                remove_old_exports(&cache);
                let path = directory.join(filename);
                fs::write(&path, text)
                    .map_err(|error| CommandError::new("LOG_SHARE", error.to_string()))?;
                match app.share().share_file(
                    window,
                    path.to_string_lossy().into_owned(),
                    ShareFileOptions {
                        mime_type: Some("text/plain".into()),
                        title: Some("分享日志文件".into()),
                        position,
                    },
                ) {
                    Ok(()) => Ok(true),
                    // ShareKit 0.3 用错误字符串表示用户取消。
                    Err(error)
                        if matches!(
                            error.to_string().as_str(),
                            "Share cancelled" | "Windows API error: Share cancelled"
                        ) =>
                    {
                        Ok(false)
                    }
                    Err(error) => Err(CommandError::new("LOG_SHARE", error.to_string())),
                }
            }
        }
    })
    .await
    .map_err(|error| CommandError::new("LOG_EXPORT", error.to_string()))?
}

fn remove_old_exports(directory: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("karamas-logs-")
            && name.ends_with(".txt")
            && entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .ok()
                .and_then(|time| time.elapsed().ok())
                .is_some_and(|age| age > Duration::from_secs(86400))
        {
            let _ = fs::remove_file(entry.path());
        }
    }
}
