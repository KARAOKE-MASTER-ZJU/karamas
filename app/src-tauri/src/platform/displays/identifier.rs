use crate::error::{CommandError, CommandResult};
use std::time::Duration;
use tauri::{PhysicalPosition, PhysicalSize, WebviewWindow};

fn identify_error(error: tauri::Error) -> CommandError {
    CommandError::new("DISPLAY_IDENTIFY", error.to_string())
}

fn create_identifier<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    monitor: &tauri::window::Monitor,
    number: usize,
) -> CommandResult<WebviewWindow<R>> {
    let area = monitor.work_area();
    let scale = monitor.scale_factor();
    let size = PhysicalSize::new(
        ((160.0 * scale).round() as u32).min(area.size.width),
        ((160.0 * scale).round() as u32).min(area.size.height),
    );
    let margin = (24.0 * scale).round() as u32;
    let position = PhysicalPosition::new(
        area.position.x + margin.min(area.size.width.saturating_sub(size.width)) as i32,
        area.position.y + margin.min(area.size.height.saturating_sub(size.height)) as i32,
    );
    tauri::WebviewWindowBuilder::new(
        app,
        format!("screen-identifier-{number}"),
        tauri::WebviewUrl::App(format!("screen-identifier.html?number={number}").into()),
    )
    .title(format!("屏幕 {number}"))
    .visible(false)
    .decorations(false)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .closable(false)
    .focused(false)
    .focusable(false)
    .always_on_top(true)
    .visible_on_all_workspaces(true)
    .skip_taskbar(true)
    .shadow(false)
    .inner_size(160.0, 160.0)
    .background_color(tauri::window::Color(27, 28, 32, 255))
    .on_page_load(move |window, payload| {
        if payload.event() == tauri::webview::PageLoadEvent::Finished {
            let result = window
                .set_size(size)
                .and_then(|_| window.set_position(position))
                .and_then(|_| window.show())
                .and_then(|_| window.set_ignore_cursor_events(true));
            if let Err(error) = result {
                log::error!("显示屏幕编号失败：{error}");
            }
        }
    })
    .build()
    .map_err(identify_error)
}

pub async fn identify<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    display_ids: Vec<String>,
) -> CommandResult<()> {
    let monitors = app.available_monitors().map_err(identify_error)?;
    let current_ids: Vec<_> = monitors
        .iter()
        .enumerate()
        .map(|(index, monitor)| super::monitor_id(index, monitor))
        .collect();
    if display_ids.is_empty() || display_ids != current_ids {
        return Err(CommandError::new(
            "DISPLAY_UNAVAILABLE",
            "屏幕已变化，请刷新屏幕列表后重试。",
        ));
    }
    let mut windows = Vec::with_capacity(monitors.len());
    let result = monitors
        .iter()
        .enumerate()
        .try_for_each(|(index, monitor)| {
            create_identifier(app, monitor, index + 1).map(|window| windows.push(window))
        });
    if result.is_ok() {
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
    for window in windows {
        if let Err(error) = window.destroy() {
            log::error!("关闭屏幕编号失败：{error}");
        }
    }
    result
}
