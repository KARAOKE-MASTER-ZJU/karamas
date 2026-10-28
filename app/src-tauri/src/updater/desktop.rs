use super::*;
use std::time::Duration;
use tauri_plugin_updater::{Update, UpdaterExt};

pub struct PendingUpdate {
    update: Update,
    bytes: Option<Vec<u8>>,
}

pub async fn cleanup_download<R: Runtime>(_: &AppHandle<R>) {}

pub async fn check<R: Runtime>(
    app: &AppHandle<R>,
) -> CommandResult<Option<(UpdateInfo, PendingUpdate)>> {
    let configured = app
        .config()
        .plugins
        .0
        .get("updater")
        .and_then(|config| config.get("pubkey"))
        .and_then(|key| key.as_str())
        .is_some_and(|key| !key.is_empty());
    if !configured {
        return Err(failure("当前构建未配置应用更新，请使用正式发布版"));
    }
    let update = app
        .updater_builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(failure)?
        .check()
        .await
        .map_err(failure)?;
    Ok(update.map(|update| {
        let info = UpdateInfo {
            version: update.version.clone(),
            notes: update.body.clone().unwrap_or_default(),
        };
        (
            info,
            PendingUpdate {
                update,
                bytes: None,
            },
        )
    }))
}

pub async fn download<R: Runtime>(
    _: &AppHandle<R>,
    pending: &mut PendingUpdate,
    progress: Channel<DownloadProgress>,
) -> CommandResult<()> {
    if pending.bytes.is_some() {
        return Ok(());
    }
    let mut downloaded = 0;
    // 检查请求与安装包下载采用不同超时，避免大文件被检查超时截断。
    pending.update.timeout = Some(Duration::from_secs(15 * 60));
    let bytes = pending
        .update
        .download(
            |length, total| {
                downloaded += length as u64;
                let _ = progress.send(DownloadProgress { downloaded, total });
            },
            || {},
        )
        .await
        .map_err(failure)?;
    pending.bytes = Some(bytes);
    Ok(())
}

pub async fn install<R: Runtime>(
    app: &AppHandle<R>,
    pending: &mut PendingUpdate,
) -> CommandResult<()> {
    let bytes = pending
        .bytes
        .as_ref()
        .ok_or_else(|| failure("请先下载更新"))?
        .clone();
    let update = pending.update.clone();
    tauri::async_runtime::spawn_blocking(move || update.install(bytes))
        .await
        .map_err(failure)?
        .map_err(failure)?;
    // Windows 由安装器重启；macOS 和 Linux 在替换完成后重启。
    #[cfg(not(target_os = "windows"))]
    app.restart();
    #[cfg(target_os = "windows")]
    let _ = app;
    #[allow(unreachable_code)]
    Ok(())
}
