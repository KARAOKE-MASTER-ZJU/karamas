use super::DisplayMetadata;
use crate::error::{CommandError, CommandResult};
use core_graphics::display::CGDisplay;
use objc2::MainThreadMarker;
use objc2_app_kit::NSScreen;
use objc2_foundation::{NSNumber, NSString};
use tauri::{PhysicalPosition, PhysicalSize, window::Monitor};

/// 在主线程读取 AppKit 屏幕元数据。
pub(super) async fn metadata<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    monitors: &[Monitor],
) -> CommandResult<Vec<Option<DisplayMetadata>>> {
    if let Some(marker) = MainThreadMarker::new() {
        return collect_metadata(marker, monitors);
    }
    let monitors = monitors.to_vec();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let result = MainThreadMarker::new()
            .ok_or_else(|| CommandError::new("DISPLAY_DISCOVERY", "读取屏幕信息需要在主线程执行。"))
            .and_then(|marker| collect_metadata(marker, &monitors));
        let _ = sender.send(result);
    })
    .map_err(|error| CommandError::new("DISPLAY_DISCOVERY", error.to_string()))?;
    receiver
        .await
        .map_err(|error| CommandError::new("DISPLAY_DISCOVERY", error.to_string()))?
}

fn display_id(screen: &NSScreen) -> Option<u32> {
    let value = screen
        .deviceDescription()
        .objectForKey(&NSString::from_str("NSScreenNumber"))?;
    let number = value.downcast::<NSNumber>().ok()?;
    Some(number.unsignedIntValue())
}

fn collect_metadata(
    marker: MainThreadMarker,
    monitors: &[Monitor],
) -> CommandResult<Vec<Option<DisplayMetadata>>> {
    let screens = NSScreen::screens(marker)
        .iter()
        .map(|screen| {
            let id = display_id(&screen).ok_or_else(|| {
                CommandError::new("DISPLAY_DISCOVERY", "无法读取显示器标识，请刷新重试。")
            })?;
            let display = CGDisplay::new(id);
            let bounds = display.bounds();
            let scale = screen.backingScaleFactor();
            // 与 Tauri/tao 的 macOS 显示器坐标换算一致，兼容不同屏幕的缩放比例。
            let position =
                PhysicalPosition::from_logical::<_, f64>((bounds.origin.x, bounds.origin.y), scale);
            let size = PhysicalSize::from_logical::<_, f64>(
                (display.pixels_wide() as f64, display.pixels_high() as f64),
                scale,
            );
            Ok((
                position,
                size,
                DisplayMetadata {
                    name: screen.localizedName().to_string(),
                    built_in: Some(display.is_builtin()),
                },
            ))
        })
        .collect::<CommandResult<Vec<_>>>()?;
    Ok(monitors
        .iter()
        .map(|monitor| {
            screens
                .iter()
                .find(|(position, size, _)| {
                    position == monitor.position() && size == monitor.size()
                })
                .map(|(_, _, metadata)| metadata.clone())
        })
        .collect())
}
