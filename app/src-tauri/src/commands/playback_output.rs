use crate::{
    error::{CommandError, CommandResult},
    platform::playback_output,
};

fn authorize<R: tauri::Runtime>(webview: &tauri::Webview<R>) -> CommandResult<()> {
    if matches!(webview.label(), "main" | "playback-controller") {
        Ok(())
    } else {
        Err(CommandError::new("WINDOW_ACCESS", "播放窗口操作无效。"))
    }
}

#[tauri::command]
pub fn get_playback_output_capabilities<R: tauri::Runtime>(
    webview: tauri::Webview<R>,
) -> CommandResult<playback_output::OutputCapabilities> {
    authorize(&webview)?;
    Ok(playback_output::capabilities())
}

#[tauri::command]
pub fn get_playback_output<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    webview: tauri::Webview<R>,
) -> CommandResult<playback_output::OutputView> {
    authorize(&webview)?;
    Ok(playback_output::view(&app))
}

#[tauri::command]
pub async fn set_playback_output<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    webview: tauri::Webview<R>,
    output: String,
) -> CommandResult<()> {
    authorize(&webview)?;
    playback_output::set_output(&app, output).await
}

#[tauri::command]
pub fn focus_playback_output<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    webview: tauri::Webview<R>,
) -> CommandResult<()> {
    authorize(&webview)?;
    playback_output::focus(&app)
}

#[tauri::command]
pub fn playback_output_ready<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    webview: tauri::Webview<R>,
    request_id: u32,
) -> CommandResult<()> {
    if webview.label() != "main" {
        return Err(CommandError::new("WINDOW_ACCESS", "播放文档无效。"));
    }
    playback_output::prepared(&app, request_id);
    Ok(())
}

#[tauri::command]
pub fn playback_controller_ready<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    webview: tauri::Webview<R>,
) -> CommandResult<()> {
    if webview.label() != "playback-controller"
        && !(cfg!(target_os = "android") && webview.label() == "main")
    {
        return Err(CommandError::new("WINDOW_ACCESS", "控制文档无效。"));
    }
    playback_output::controller_ready(&app);
    Ok(())
}

#[tauri::command]
pub async fn send_android_controller_message<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    webview: tauri::Webview<R>,
    token: String,
    message: serde_json::Value,
) -> CommandResult<()> {
    authorize(&webview)?;
    #[cfg(target_os = "android")]
    return crate::platform::android::send_controller_message(&app, token, message).await;
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, token, message);
        Ok(())
    }
}

#[tauri::command]
pub fn android_playback_returned<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    webview: tauri::Webview<R>,
) -> CommandResult<()> {
    authorize(&webview)?;
    playback_output::returned(&app);
    Ok(())
}
