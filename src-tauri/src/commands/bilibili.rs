use crate::{
    error::{CommandError, CommandResult},
    state::BridgeState,
};
use karamas_core::cast::bilibili::{self, BilibiliSession};
use tauri::{State, ipc::Channel};

/// 通过调用专属通道返回二维码，登录成功只返回会话。
#[tauri::command]
pub async fn login_bilibili(
    state: State<'_, BridgeState>,
    on_qr: Channel<String>,
    sender_buvid: String,
) -> CommandResult<BilibiliSession> {
    let mut slot = state.login.lock().await;
    if slot.as_ref().is_some_and(|task| !task.is_finished()) {
        return Err(CommandError::new("BUSY", "已有扫码登录正在进行"));
    }
    // 保留取消句柄，供关闭扫码流程时中止登录等待。
    let task = tauri::async_runtime::handle().inner().spawn(async move {
        bilibili::login_qr(&sender_buvid, move |url| {
            let _ = on_qr.send(url);
        })
        .await
    });
    let id = task.id();
    *slot = Some(task.abort_handle());
    // 等待登录期间释放锁，让取消命令仍能取得任务句柄。
    drop(slot);
    let result = task.await;
    let mut slot = state.login.lock().await;
    // 旧任务结束时不能清除取消后新建的登录任务。
    if slot.as_ref().is_some_and(|task| task.id() == id) {
        *slot = None;
    }
    drop(slot);
    result
        .map_err(|e| {
            CommandError::new(
                if e.is_cancelled() {
                    "CANCELLED"
                } else {
                    "TASK_FAILED"
                },
                e.to_string(),
            )
        })?
        .map_err(|e| CommandError::new("BILIBILI_ERROR", e))
}

/// 取消当前扫码登录，不清除已有会话文件。
#[tauri::command]
pub async fn cancel_bilibili_login(state: State<'_, BridgeState>) -> CommandResult<()> {
    if let Some(task) = state.login.lock().await.take() {
        task.abort();
    }
    Ok(())
}

/// 读取调用方明确保存过的会话，不隐式登录或清理。
#[tauri::command]
pub async fn load_bilibili_session(
    state: State<'_, BridgeState>,
) -> CommandResult<Option<BilibiliSession>> {
    state.initialize_storage().await?;
    let _io = state.session_io.lock().await;
    tauri::async_runtime::spawn_blocking(bilibili::load_session)
        .await
        .map_err(|e| CommandError::new("TASK_FAILED", e.to_string()))
}

/// 按调用方要求保存会话。
#[tauri::command]
pub async fn save_bilibili_session(
    state: State<'_, BridgeState>,
    session: BilibiliSession,
) -> CommandResult<()> {
    state.initialize_storage().await?;
    let _io = state.session_io.lock().await;
    tauri::async_runtime::spawn_blocking(move || bilibili::save_session(&session))
        .await
        .map_err(|e| CommandError::new("TASK_FAILED", e.to_string()))?
        .map_err(|e| CommandError::new("STORAGE_ERROR", e))
}

/// 按调用方要求清除保存的会话，不修改当前投屏连接。
#[tauri::command]
pub async fn clear_bilibili_session(state: State<'_, BridgeState>) -> CommandResult<()> {
    state.initialize_storage().await?;
    let _io = state.session_io.lock().await;
    tauri::async_runtime::spawn_blocking(bilibili::clear_session)
        .await
        .map_err(|e| CommandError::new("TASK_FAILED", e.to_string()))?
        .map_err(|e| CommandError::new("STORAGE_ERROR", e))
}
