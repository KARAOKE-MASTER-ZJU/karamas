#[cfg(not(target_os = "android"))]
use crate::error::CommandError;
use crate::error::CommandResult;
use serde::{Deserialize, Serialize};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackDisplay {
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub built_in: Option<bool>,
}

/// 原生接口提供的显示器名称和内置屏标记。
#[cfg(desktop)]
#[derive(Clone)]
struct DisplayMetadata {
    name: String,
    built_in: Option<bool>,
}

/// 枚举可用屏幕并返回显示信息。
#[tauri::command]
pub async fn list_playback_displays<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> CommandResult<Vec<PlaybackDisplay>> {
    #[cfg(target_os = "android")]
    {
        return super::android::list_playback_displays(&app).await;
    }

    #[cfg(desktop)]
    {
        let monitors = app
            .available_monitors()
            .map_err(|error| CommandError::new("DISPLAY_DISCOVERY", error.to_string()))?;

        // 各平台按传入的屏幕顺序返回元数据；无法匹配的屏幕保留 None。
        #[cfg(target_os = "macos")]
        let metadata = macos::metadata(&app, &monitors).await?;
        #[cfg(target_os = "windows")]
        let metadata = windows::metadata(&monitors)?;
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let metadata: Vec<Option<DisplayMetadata>> = vec![None; monitors.len()];

        Ok(monitors
            .into_iter()
            .zip(metadata)
            .enumerate()
            .map(|(index, (monitor, native))| {
                let source = monitor.name().cloned().unwrap_or_default();
                let name = native
                    .as_ref()
                    .map(|metadata| metadata.name.trim())
                    .filter(|name| !name.is_empty())
                    .unwrap_or(source.trim());
                PlaybackDisplay {
                    id: monitor_id(index, &monitor),
                    name: readable_name(name),
                    width: monitor.size().width,
                    height: monitor.size().height,
                    built_in: native.and_then(|metadata| metadata.built_in),
                }
            })
            .collect())
    }

    #[cfg(not(any(desktop, target_os = "android")))]
    {
        let _ = app;
        Err(CommandError::new(
            "DISPLAY_UNSUPPORTED",
            "当前平台暂不支持检测屏幕。",
        ))
    }
}

#[cfg(desktop)]
fn readable_name(name: &str) -> String {
    if let Some(number) = name.strip_prefix(r"\\.\DISPLAY")
        && !number.is_empty()
        && number.bytes().all(|byte| byte.is_ascii_digit())
    {
        return format!("Display {number}");
    }
    name.to_owned()
}

#[cfg(desktop)]
pub(super) fn monitor_id(index: usize, monitor: &tauri::window::Monitor) -> String {
    let name = monitor.name().map(String::as_str).unwrap_or_default();
    let position = monitor.position();
    format!("screen:{index}:{name}:{}:{}", position.x, position.y)
}
