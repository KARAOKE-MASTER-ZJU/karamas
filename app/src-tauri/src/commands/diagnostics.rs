use crate::{error::CommandResult, state::BridgeState};
use karamas_core::diagnostics::{
    self, DeviceInfo, FunctionCheck, PlaybackTest, ServiceInfo, TestResponse,
};
use std::collections::BTreeMap;
use tauri::{State, ipc::Channel};

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
    let tester = state.diagnostic_player_for(location).await;
    Ok(tester
        .execute(test, tauri::async_runtime::handle().inner().clone())
        .await?)
}

#[tauri::command]
pub async fn diagnostic_functions<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, BridgeState>,
    location: String,
    song_url: String,
    on_check: Channel<FunctionCheck>,
) -> CommandResult<()> {
    super::session::http_url(&location)?;
    super::session::http_url(&song_url)?;
    crate::platform::ensure_network_access(&app).await?;
    state.initialize_storage().await?;
    let tester = state.diagnostic_player_for(location).await;
    tester
        .test_functions(
            &song_url,
            tauri::async_runtime::handle().inner().clone(),
            move |check| {
                let _ = on_check.send(check);
            },
        )
        .await?;
    Ok(())
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
