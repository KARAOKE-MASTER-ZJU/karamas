use crate::{
    error::{CommandError, CommandResult},
    state::BridgeState,
};
use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime, ipc::Channel};
use tokio::sync::{Mutex, oneshot};

#[cfg(desktop)]
mod desktop;
#[cfg(desktop)]
use desktop as platform;
#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "android")]
use android as platform;

#[cfg(target_os = "ios")]
mod platform {
    use super::*;
    pub struct PendingUpdate;
    pub async fn check<R: Runtime>(
        _: &AppHandle<R>,
    ) -> CommandResult<Option<(UpdateInfo, PendingUpdate)>> {
        Err(failure("此平台通过应用商店更新"))
    }
    pub async fn download<R: Runtime>(
        _: &AppHandle<R>,
        _: &mut PendingUpdate,
        _: Channel<DownloadProgress>,
    ) -> CommandResult<()> {
        Err(failure("此平台通过应用商店更新"))
    }
    pub async fn install<R: Runtime>(_: &AppHandle<R>, _: &mut PendingUpdate) -> CommandResult<()> {
        Err(failure("此平台通过应用商店更新"))
    }
    pub async fn cleanup_download<R: Runtime>(_: &AppHandle<R>) {}
}

#[derive(Default)]
pub struct UpdateState {
    pending: Mutex<Option<platform::PendingUpdate>>,
    cancellation: std::sync::Mutex<Option<oneshot::Sender<()>>>,
}

#[derive(Clone, Serialize)]
pub struct UpdateInfo {
    pub version: String,
    pub notes: String,
}

#[derive(Clone, Serialize)]
pub struct DownloadProgress {
    pub downloaded: u64,
    pub total: Option<u64>,
}

fn failure(error: impl std::fmt::Display) -> CommandError {
    CommandError::new("UPDATE_ERROR", error.to_string())
}

pub async fn check<R: Runtime>(app: &AppHandle<R>) -> CommandResult<Option<UpdateInfo>> {
    let state = app.state::<UpdateState>();
    let mut pending = state
        .pending
        .try_lock()
        .map_err(|_| failure("更新操作正在进行中"))?;
    match platform::check(app).await {
        Ok(result) => {
            let info = result.as_ref().map(|(info, _)| info.clone());
            *pending = result.map(|(_, update)| update);
            Ok(info)
        }
        Err(error) => {
            log::warn!("检查更新失败：{error}");
            Err(error)
        }
    }
}

pub async fn download<R: Runtime>(
    app: &AppHandle<R>,
    progress: Channel<DownloadProgress>,
) -> CommandResult<()> {
    let state = app.state::<UpdateState>();
    let mut pending = state
        .pending
        .try_lock()
        .map_err(|_| failure("更新操作正在进行中"))?;
    let update = pending.as_mut().ok_or_else(|| failure("请先检查更新"))?;
    let (cancel, cancelled) = oneshot::channel();
    *state.cancellation.lock().map_err(failure)? = Some(cancel);
    // 通知前端下载已启动，此后才开放取消按钮。
    let _ = progress.send(DownloadProgress {
        downloaded: 0,
        total: None,
    });
    let result = tokio::select! {
        result = platform::download(app, update, progress) => result,
        _ = cancelled => Err(CommandError::new("UPDATE_CANCELLED", "已取消下载")),
    };
    state.cancellation.lock().map_err(failure)?.take();
    if result.is_err() {
        platform::cleanup_download(app).await;
    }
    result
}

pub fn cancel_download<R: Runtime>(app: &AppHandle<R>) -> CommandResult<()> {
    if let Some(cancel) = app
        .state::<UpdateState>()
        .cancellation
        .lock()
        .map_err(failure)?
        .take()
    {
        let _ = cancel.send(());
    }
    Ok(())
}

pub async fn install<R: Runtime>(app: &AppHandle<R>) -> CommandResult<()> {
    let state = app.state::<UpdateState>();
    let mut pending = state
        .pending
        .try_lock()
        .map_err(|_| failure("更新操作正在进行中"))?;
    let update = pending.as_mut().ok_or_else(|| failure("请先检查更新"))?;
    let bridge = app.state::<BridgeState>();
    let _lifecycle = bridge.lifecycle.lock().await;
    // 安装即将退出应用，在同一会话锁内停止后台任务。
    karamas_core::reset_engine();
    crate::platform::stop_casting_service(app).await?;
    log::logger().flush();
    platform::install(app, update).await
}
