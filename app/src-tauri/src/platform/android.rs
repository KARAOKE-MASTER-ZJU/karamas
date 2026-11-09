use crate::error::{CommandError, CommandResult};
use serde::{Deserialize, Serialize};
use tauri::{Manager, plugin::PluginHandle};

struct AndroidPlatform<R: tauri::Runtime> {
    handle: PluginHandle<R>,
}

#[derive(Deserialize)]
struct PlaybackDisplays {
    displays: Vec<super::displays::PlaybackDisplay>,
}

pub async fn list_playback_displays<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> CommandResult<Vec<super::displays::PlaybackDisplay>> {
    let result: PlaybackDisplays = app
        .state::<AndroidPlatform<R>>()
        .handle
        .run_mobile_plugin_async("listPlaybackDisplays", ())
        .await
        .map_err(|error| CommandError::new("DISPLAY_DISCOVERY", error.to_string()))?;
    Ok(result.displays)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateVersion {
    version_code: u64,
}

pub async fn android_update_version<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> CommandResult<u64> {
    let result: UpdateVersion = app
        .state::<AndroidPlatform<R>>()
        .handle
        .run_mobile_plugin_async("updateVersion", ())
        .await
        .map_err(|error| CommandError::new("ANDROID_UPDATE", error.to_string()))?;
    Ok(result.version_code)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InstallUpdateArgs<'a> {
    path: &'a std::path::Path,
    version_code: u64,
}

pub async fn install_android_update<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    path: &std::path::Path,
    version_code: u64,
) -> CommandResult<()> {
    app.state::<AndroidPlatform<R>>()
        .handle
        .run_mobile_plugin_async::<()>("installUpdate", InstallUpdateArgs { path, version_code })
        .await
        .map_err(|error| CommandError::new("ANDROID_UPDATE", error.to_string()))
}

pub async fn move_to_background<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> CommandResult<()> {
    app.state::<AndroidPlatform<R>>()
        .handle
        .run_mobile_plugin_async::<()>("moveToBackground", ())
        .await
        .map_err(|error| CommandError::new("ANDROID_NAVIGATION", error.to_string()))
}

pub fn init<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("casting-platform")
        .setup(|app, api| {
            let handle =
                api.register_android_plugin("zju.idol.karamas", "CastingPlatformPlugin")?;
            app.manage(AndroidPlatform { handle });
            Ok(())
        })
        .build()
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct NetworkInfo {
    wifi: bool,
    vpn: bool,
    interface_name: Option<String>,
    addresses: Vec<String>,
}

pub async fn ensure_network_access<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> CommandResult<()> {
    let network = app.state::<AndroidPlatform<R>>();
    let info: NetworkInfo = network
        .handle
        .run_mobile_plugin_async("ensureAccess", ())
        .await
        .map_err(|error| CommandError::new("ANDROID_NETWORK", error.to_string()))?;
    log::info!(
        target: "ANDROID_NETWORK",
        "默认网络：Wi-Fi={}，VPN={}，接口={:?}，地址={:?}",
        info.wifi, info.vpn, info.interface_name, info.addresses
    );
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StartServiceArgs<'a> {
    room_id: &'a str,
}

pub async fn start_casting_service<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    room_id: &str,
) -> CommandResult<()> {
    app.state::<AndroidPlatform<R>>()
        .handle
        .run_mobile_plugin_async::<()>("startCasting", StartServiceArgs { room_id })
        .await
        .map_err(|error| CommandError::new("ANDROID_SERVICE", error.to_string()))
}

pub async fn stop_casting_service<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> CommandResult<()> {
    app.state::<AndroidPlatform<R>>()
        .handle
        .run_mobile_plugin_async::<()>("stopCasting", ())
        .await
        .map_err(|error| CommandError::new("ANDROID_SERVICE", error.to_string()))
}

pub async fn playback_window_call<R: tauri::Runtime, T: serde::de::DeserializeOwned>(
    app: &tauri::AppHandle<R>,
    command: &str,
    args: serde_json::Value,
) -> CommandResult<T> {
    app.state::<AndroidPlatform<R>>()
        .handle
        .run_mobile_plugin_async(command, args)
        .await
        .map_err(|error| CommandError::new("ANDROID_PRESENTATION", error.to_string()))
}

pub async fn set_playback_output<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    output: &str,
) -> CommandResult<()> {
    if output == "embedded" {
        return playback_window_call(
            app,
            "attachPlaybackOutput",
            serde_json::json!({ "output": output }),
        )
        .await;
    }
    let already_open = super::playback_output::view(app).output != "embedded";
    if !already_open {
        let (ready, receive) = tokio::sync::oneshot::channel();
        *app.state::<super::playback_output::PlaybackOutputState>()
            .controller_ready
            .lock()
            .unwrap() = Some(ready);
        playback_window_call::<_, ()>(
            app,
            "preparePlaybackOutput",
            serde_json::json!({ "output": output }),
        )
        .await?;
        super::playback_output::wait_ready(receive).await?;
        super::playback_output::prepare(app).await?;
    }
    playback_window_call(
        app,
        "attachPlaybackOutput",
        serde_json::json!({ "output": output }),
    )
    .await
}

pub async fn send_controller_message<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    token: String,
    message: serde_json::Value,
) -> CommandResult<()> {
    playback_window_call(
        app,
        "sendControllerMessage",
        serde_json::json!({ "token": token, "message": message }),
    )
    .await
}
