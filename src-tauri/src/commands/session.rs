use crate::{
    error::{CommandError, CommandResult},
    models::CastTarget,
    state::BridgeState,
};
use tauri::State;

/// 校验边界输入，避免把非 HTTP 地址交给设备或房间接口。
pub(super) fn http_url(value: &str) -> CommandResult<url::Url> {
    let url = url::Url::parse(value)
        .map_err(|_| CommandError::new("INVALID_ARGUMENT", "请输入有效的 HTTP 或 HTTPS 地址"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(CommandError::new(
            "INVALID_ARGUMENT",
            "请输入有效的 HTTP 或 HTTPS 地址",
        ));
    }
    Ok(url)
}

/// 按指定协议启动当前投屏会话，复用 Tauri 的运行时。
#[tauri::command]
pub async fn start_casting<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, BridgeState>,
    server_url: String,
    room_id: String,
    target: CastTarget,
) -> CommandResult<()> {
    let url = http_url(&server_url)?;
    if url.query().is_some() || url.fragment().is_some() || room_id.trim().is_empty() {
        return Err(CommandError::new(
            "INVALID_ARGUMENT",
            "服务器地址不能带查询参数或片段，房间号不能为空",
        ));
    }
    if let CastTarget::Dlna { location } = &target {
        http_url(location)?;
        crate::platform::ensure_network_access(&app).await?;
    }
    crate::platform::ensure_room_access(&app, &url).await?;
    // 核心库只有一份全局会话，启动期间不能并发启动或断开。
    let _lifecycle = state.lifecycle.lock().await;
    state.initialize_storage().await?;
    // 只传递 Handle，运行时的创建和关闭由 Tauri 负责。
    let runtime = tauri::async_runtime::handle().inner().clone();
    let server_url = url.as_str().trim_end_matches('/').to_owned();
    let room_id = room_id.trim().to_owned();
    let result = match target {
        CastTarget::Dlna { location } => {
            casting_core::start_engine_core(server_url, room_id, location, runtime).await
        }
        CastTarget::Bilibili {
            session,
            device_buvid,
            sender_buvid,
        } => {
            casting_core::start_bilibili_engine_core(
                server_url,
                room_id,
                session,
                device_buvid,
                sender_buvid,
                runtime,
            )
            .await
        }
    };
    // 启动失败时清理已创建的部分资源，避免媒体服务遗留。
    if result.is_err() {
        casting_core::reset_engine();
    }
    Ok(result?)
}

/// 重置当前会话，释放核心库持有的后台任务和媒体服务。
#[tauri::command]
pub async fn disconnect_casting(state: State<'_, BridgeState>) -> CommandResult<()> {
    let _lifecycle = state.lifecycle.lock().await;
    casting_core::reset_engine();
    Ok(())
}

/// 设置 DLNA 后续媒体加载使用的直连模式。
#[tauri::command]
pub async fn set_dlna_direct(enabled: bool) {
    casting_core::set_dlna_direct(enabled);
}

/// 读取 DLNA 直连设置。
#[tauri::command]
pub async fn get_dlna_direct() -> bool {
    casting_core::get_dlna_direct()
}
