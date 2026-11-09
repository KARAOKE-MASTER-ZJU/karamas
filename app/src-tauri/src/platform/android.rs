use crate::error::{CommandError, CommandResult};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, plugin::PluginHandle};

struct AndroidPlatform<R: tauri::Runtime> {
    handle: PluginHandle<R>,
    playback_channel: tauri::ipc::Channel<serde_json::Value>,
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
        .js_init_script("window.__KARAMAS_PRESENTATION_SUPPORTED__ = true;".to_string())
        .setup(|app, api| {
            let handle =
                api.register_android_plugin("zju.idol.karamas", "CastingPlatformPlugin")?;
            app.manage(AndroidPlatform {
                handle,
                playback_channel: playback_channel(app),
            });
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

pub async fn open_playback_window<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    token: String,
    display_id: Option<String>,
) -> CommandResult<()> {
    playback_window_call(
        app,
        "openPlaybackWindow",
        serde_json::json!({
            "token": token, "displayId": display_id,
            "onMessage": app.state::<AndroidPlatform<R>>().playback_channel,
        }),
    )
    .await
}

fn playback_channel<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> tauri::ipc::Channel<serde_json::Value> {
    let handle = app.clone();
    tauri::ipc::Channel::new(move |body| {
        let tauri::ipc::InvokeResponseBody::Json(json) = body else {
            return Ok(());
        };
        let data: serde_json::Value = serde_json::from_str(&json)?;
        if data["type"] == "message" {
            let mut message = data["message"].clone();
            message["token"] = data["token"].clone();
            handle.emit_to("main", "playback-window-message", message)?;
        } else if data["type"] == "invoke" {
            let app = handle.clone();
            tauri::async_runtime::spawn(async move {
                let result = invoke_playback(&data).await;
                let reply = match result {
                    Ok(value) => serde_json::json!({"id": data["id"], "value": value}),
                    Err(error) => serde_json::json!({"id": data["id"], "error": error}),
                };
                let _ = playback_window_call::<_, ()>(
                    &app,
                    "sendPlaybackWindowMessage",
                    serde_json::json!({"message": {
                        "token": data["token"], "type": "reply", "payload": reply,
                    }}),
                )
                .await;
            });
        }
        Ok(())
    })
}

async fn invoke_playback(data: &serde_json::Value) -> CommandResult<bool> {
    match data["command"].as_str() {
        Some("report_local_playback") => {
            let report = serde_json::from_value(data["args"]["report"].clone())
                .map_err(|error| CommandError::new("LOCAL_REPORT", error.to_string()))?;
            crate::commands::playback::report_local_playback(report).await
        }
        Some("play") => crate::commands::playback::play().await,
        _ => Err(CommandError::new("WINDOW_ACCESS", "播放窗口命令无效。")),
    }
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
