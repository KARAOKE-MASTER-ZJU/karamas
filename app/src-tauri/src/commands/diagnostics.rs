use crate::{error::CommandResult, state::BridgeState};
use karamas_core::diagnostics::{
    self, DeviceInfo, PlaybackTest, PlaybackTester, ServiceInfo, TestResponse,
};
use std::collections::BTreeMap;
use std::sync::Arc;
use tauri::State;

#[tauri::command]
pub async fn diagnostic_device<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    location: String,
) -> CommandResult<DeviceInfo> {
    super::session::http_url(&location)?;
    crate::platform::ensure_network_access(&app).await?;
    Ok(diagnostics::device_info(&location).await?)
}

#[tauri::command]
pub async fn diagnostic_service<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    location: String,
    service_id: String,
) -> CommandResult<ServiceInfo> {
    super::session::http_url(&location)?;
    crate::platform::ensure_network_access(&app).await?;
    Ok(diagnostics::service_info(&location, &service_id).await?)
}

#[tauri::command]
pub async fn diagnostic_query<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    location: String,
    service_id: String,
    action: String,
    args: BTreeMap<String, String>,
) -> CommandResult<TestResponse> {
    super::session::http_url(&location)?;
    crate::platform::ensure_network_access(&app).await?;
    Ok(diagnostics::query(&location, &service_id, &action, args).await?)
}

#[tauri::command]
pub async fn diagnostic_playback<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, BridgeState>,
    location: String,
    test: PlaybackTest,
) -> CommandResult<Vec<TestResponse>> {
    super::session::http_url(&location)?;
    crate::platform::ensure_network_access(&app).await?;
    state.initialize_storage().await?;
    let tester = {
        let mut slot = state.diagnostic_player.lock().await;
        if slot
            .as_ref()
            .is_none_or(|tester| tester.location != location)
        {
            *slot = Some(Arc::new(PlaybackTester::new(location)));
        }
        slot.as_ref().expect("已初始化测试播放器").clone()
    };
    Ok(tester
        .execute(test, tauri::async_runtime::handle().inner().clone())
        .await?)
}

#[tauri::command]
pub async fn diagnostic_latency<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    url: String,
    lan: bool,
) -> CommandResult<TestResponse> {
    let address = super::session::http_url(&url)?;
    crate::platform::ensure_room_access(&app, &address).await?;
    Ok(diagnostics::latency(&url, lan).await?)
}
