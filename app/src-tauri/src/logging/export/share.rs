use crate::{
    error::{CommandError, CommandResult},
    models::SharePosition,
};
use std::{fs, path::Path, time::Duration};
use tauri::{Manager, Runtime};

pub(super) fn export_logs<R: Runtime>(
    app: tauri::AppHandle<R>,
    webview: tauri::Webview<R>,
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
    crate::platform::share::share_file(
        &app,
        &webview,
        path.to_string_lossy().into_owned(),
        position,
    )
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
