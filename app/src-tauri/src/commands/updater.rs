use crate::{
    error::CommandResult,
    updater::{self, DownloadProgress, UpdateInfo},
};
use tauri::{AppHandle, Runtime, ipc::Channel};

#[tauri::command]
pub fn update_supported() -> bool {
    cfg!(any(desktop, target_os = "android"))
}

#[tauri::command]
pub async fn check_update<R: Runtime>(app: AppHandle<R>) -> CommandResult<Option<UpdateInfo>> {
    updater::check(&app).await
}

#[tauri::command]
pub async fn download_update<R: Runtime>(
    app: AppHandle<R>,
    on_progress: Channel<DownloadProgress>,
) -> CommandResult<()> {
    updater::download(&app, on_progress).await
}

#[tauri::command]
pub fn cancel_update_download<R: Runtime>(app: AppHandle<R>) -> CommandResult<()> {
    updater::cancel_download(&app)
}

#[tauri::command]
pub async fn install_update<R: Runtime>(app: AppHandle<R>) -> CommandResult<()> {
    updater::install(&app).await
}
