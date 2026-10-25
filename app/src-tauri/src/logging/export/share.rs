use crate::{
    error::{CommandError, CommandResult},
    models::{ShareEdge, SharePosition},
};
use std::{fs, path::Path, time::Duration};
use tauri::{Manager, Runtime};
use tauri_plugin_sharekit::{RectEdge, ShareExt, ShareFileOptions};

pub(super) fn export_logs<R: Runtime>(
    app: tauri::AppHandle<R>,
    window: tauri::WebviewWindow<R>,
    text: String,
    filename: String,
    position: Option<SharePosition>,
) -> CommandResult<bool> {
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
    fs::write(&path, text).map_err(|error| CommandError::new("LOG_SHARE", error.to_string()))?;
    match app.share().share_file(
        window,
        path.to_string_lossy().into_owned(),
        ShareFileOptions {
            mime_type: Some("text/plain".into()),
            title: Some("分享日志文件".into()),
            position: position.map(|position| tauri_plugin_sharekit::SharePosition {
                x: position.x,
                y: position.y,
                preferred_edge: position.preferred_edge.map(|edge| match edge {
                    ShareEdge::Top => RectEdge::Top,
                    ShareEdge::Bottom => RectEdge::Bottom,
                    ShareEdge::Left => RectEdge::Left,
                    ShareEdge::Right => RectEdge::Right,
                }),
            }),
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
