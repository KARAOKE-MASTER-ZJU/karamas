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
        if token.is_empty()
            || token.len() > 64
            || !token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(CommandError::new("WINDOW_TOKEN", "播放窗口标识无效。"));
        }
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
