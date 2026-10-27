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
    on_local_command: tauri::ipc::Channel<karamas_core::cast::local::LocalCommand>,
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
    // 在仍可见时启动前台服务，避免连接设备期间切后台后无法再启动。
    crate::platform::start_casting_service(&app, &room_id).await?;
    let result = match target {
        CastTarget::Local => {
            let channel = on_local_command;
            karamas_core::start_local_engine_core(
                server_url,
                room_id,
                std::sync::Arc::new(move |command| {
                    channel.send(command).map_err(|error| error.to_string())
                }),
                runtime,
            )
            .await
        }
        CastTarget::Dlna { location } => {
            karamas_core::start_engine_core(server_url, room_id, location, runtime).await
        }
        CastTarget::Bilibili {
            session,
            device_buvid,
            sender_buvid,
        } => {
            karamas_core::start_bilibili_engine_core(
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
        karamas_core::reset_engine();
        if let Err(error) = crate::platform::stop_casting_service(&app).await {
            log::warn!("启动失败后停止 Android 投屏服务失败：{}", error.message);
        }
    }
    Ok(result?)
}

/// 重置当前会话，释放核心库持有的后台任务和媒体服务。
#[tauri::command]
pub async fn disconnect_casting<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, BridgeState>,
) -> CommandResult<()> {
    let _lifecycle = state.lifecycle.lock().await;
    karamas_core::reset_engine();
    crate::platform::stop_casting_service(&app).await
}

/// 设置 DLNA 后续媒体加载使用的直连模式。
#[tauri::command]
pub async fn set_dlna_direct(enabled: bool) {
    karamas_core::set_dlna_direct(enabled);
}

/// 读取 DLNA 直连设置。
#[tauri::command]
pub async fn get_dlna_direct() -> bool {
    karamas_core::get_dlna_direct()
}
