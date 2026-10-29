use crate::error::{CommandError, CommandResult};
use std::sync::Mutex;
#[cfg(desktop)]
use tauri::Emitter;
use tauri::Manager;

#[derive(Default)]
pub struct PlaybackWindowState(Mutex<Option<String>>);

fn authorize<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>) -> CommandResult<()> {
    if window.label() != "main" {
        return Err(CommandError::new(
            "WINDOW_ACCESS",
            "请从主窗口操作播放窗口。",
        ));
    }
    Ok(())
}

#[tauri::command(async)]
pub fn open_playback_window<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    window: tauri::WebviewWindow<R>,
    token: String,
    display_id: Option<String>,
) -> CommandResult<()> {
    authorize(&window)?;
    #[cfg(not(desktop))]
    {
        let _ = (app, token, display_id);
        Err(CommandError::new(
            "WINDOW_UNSUPPORTED",
            "当前平台暂不支持独立播放窗口。",
        ))
    }
    #[cfg(desktop)]
    {
        if token.is_empty()
            || token.len() > 64
            || !token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(CommandError::new("WINDOW_TOKEN", "播放窗口标识无效。"));
        }
        let display = resolve_display(&app, display_id.as_deref())?;
        {
            let state = app.state::<PlaybackWindowState>();
            let mut current = state.0.lock().unwrap();
            if current.is_some() || app.get_webview_window("playback").is_some() {
                return Err(CommandError::new("WINDOW_BUSY", "播放窗口已打开。"));
            }
            *current = Some(token.clone());
        }
        let script = format!(
            "window.__KARAMAS_PLAYBACK_WINDOW__ = {};",
            serde_json::to_string(&token).unwrap()
        );
        let result = tauri::WebviewWindowBuilder::new(
            &app,
            "playback",
            tauri::WebviewUrl::App("index.html".into()),
        )
        .visible(false)
        .title("KARAM@S · 播放窗口")
        .inner_size(960.0, 540.0)
        .min_inner_size(320.0, 180.0)
        .background_color(tauri::window::Color(0, 0, 0, 255))
        .initialization_script(script)
        .build();
        match result {
            Ok(player) => {
                let handle = app.clone();
                player.on_window_event(move |event| {
                    let kind = match event {
                        tauri::WindowEvent::CloseRequested { api, .. } => {
                            api.prevent_close();
                            "closing"
                        }
                        tauri::WindowEvent::Destroyed => {
                            let state = handle.state::<PlaybackWindowState>();
                            let mut current = state.0.lock().unwrap();
                            if current.as_deref() == Some(&token) {
                                *current = None;
                            }
                            "closed"
                        }
                        _ => return,
                    };
                    let _ = handle.emit_to(
                        "main",
                        "playback-window-message",
                        serde_json::json!({
                            "token": token, "type": kind,
                        }),
                    );
                });
                if let Err(error) = place_window(&player, display.as_ref()) {
                    let _ = player.destroy();
                    return Err(error);
                }
                Ok(())
            }
            Err(error) => {
                *app.state::<PlaybackWindowState>().0.lock().unwrap() = None;
                Err(CommandError::new("WINDOW_CREATE", error.to_string()))
            }
        }
    }
}

#[tauri::command]
pub fn close_playback_window<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    window: tauri::WebviewWindow<R>,
    token: String,
) -> CommandResult<bool> {
    authorize(&window)?;
    if app
        .state::<PlaybackWindowState>()
        .0
        .lock()
        .unwrap()
        .as_deref()
        != Some(&token)
    {
        return Ok(false);
    }
    if let Some(player) = app.get_webview_window("playback") {
        player
            .destroy()
            .map_err(|error| CommandError::new("WINDOW_CLOSE", error.to_string()))?;
        return Ok(true);
    }
    Ok(false)
}

#[tauri::command]
pub fn focus_playback_window<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    window: tauri::WebviewWindow<R>,
    token: String,
) -> CommandResult<()> {
    authorize(&window)?;
    #[cfg(not(desktop))]
    {
        let _ = (app, token);
        Err(CommandError::new(
            "WINDOW_UNSUPPORTED",
            "当前平台暂不支持独立播放窗口。",
        ))
    }
    #[cfg(desktop)]
    {
        if app
            .state::<PlaybackWindowState>()
            .0
            .lock()
            .unwrap()
            .as_deref()
            != Some(&token)
        {
            return Err(CommandError::new("WINDOW_CLOSED", "播放窗口已关闭。"));
        }
        if let Some(player) = app.get_webview_window("playback") {
            player
                .unminimize()
                .and_then(|_| player.set_focus())
                .map_err(|error| CommandError::new("WINDOW_FOCUS", error.to_string()))?;
        }
        Ok(())
    }
}

pub fn close_with_main<R: tauri::Runtime>(window: &tauri::Window<R>, event: &tauri::WindowEvent) {
    if window.label() == "main" && matches!(event, tauri::WindowEvent::Destroyed) {
        if let Some(player) = window.app_handle().get_webview_window("playback") {
            let _ = player.destroy();
        }
    }
}

pub fn on_page_load<R: tauri::Runtime>(
    webview: &tauri::Webview<R>,
    payload: &tauri::webview::PageLoadPayload<'_>,
) {
    if webview.label() == "main" && payload.event() == tauri::webview::PageLoadEvent::Started {
        if let Some(player) = webview.app_handle().get_webview_window("playback") {
            let _ = player.destroy();
        }
    }
}

#[cfg(desktop)]
fn resolve_display<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    id: Option<&str>,
) -> CommandResult<Option<tauri::window::Monitor>> {
    let Some(id) = id else {
        return Ok(None);
    };
    let monitors = app
        .available_monitors()
        .map_err(|error| CommandError::new("DISPLAY_DISCOVERY", error.to_string()))?;
    monitors
        .into_iter()
        .enumerate()
        .find(|(index, monitor)| super::displays::monitor_id(*index, monitor) == id)
        .map(|(_, monitor)| Some(monitor))
        .ok_or_else(|| CommandError::new("DISPLAY_UNAVAILABLE", "所选屏幕已变化，请刷新屏幕列表。"))
}

#[cfg(desktop)]
fn place_window<R: tauri::Runtime>(
    player: &tauri::WebviewWindow<R>,
    display: Option<&tauri::window::Monitor>,
) -> CommandResult<()> {
    let place = || -> tauri::Result<()> {
        player.unminimize()?;
        if let Some(display) = display {
            player.set_size(*display.size())?;
            player.set_position(*display.position())?;
            player.set_fullscreen(true)?;
        }
        player.show()?;
        player.set_focus()
    };
    place().map_err(|error| CommandError::new("WINDOW_DISPLAY", error.to_string()))
}
