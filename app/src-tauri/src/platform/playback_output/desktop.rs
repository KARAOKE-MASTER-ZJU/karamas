use super::{PlaybackOutputState, prepare, publish, wait_ready, window_error};
use crate::error::{CommandError, CommandResult};
use tauri::{Manager, PhysicalPosition};

fn output_window<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> CommandResult<tauri::Window<R>> {
    if let Some(window) = app.get_window("playback") {
        return Ok(window);
    }
    tauri::window::WindowBuilder::new(app, "playback")
        .title("KARAM@S · 播放窗口")
        .inner_size(960.0, 540.0)
        .min_inner_size(320.0, 180.0)
        .visible(false)
        .background_color(tauri::window::Color(0, 0, 0, 255))
        .build()
        .map_err(window_error)
}

async fn ensure_controller<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> CommandResult<()> {
    if app.get_webview("playback-controller").is_some() {
        return Ok(());
    }
    let main = app
        .get_window("main")
        .ok_or_else(|| CommandError::new("WINDOW_CLOSED", "主窗口已关闭。"))?;
    let (ready, receive) = tokio::sync::oneshot::channel();
    *app.state::<PlaybackOutputState>()
        .controller_ready
        .lock()
        .unwrap() = Some(ready);
    let size = main.inner_size().map_err(window_error)?;
    let controller = main
        .add_child(
            tauri::webview::WebviewBuilder::new(
                "playback-controller",
                tauri::WebviewUrl::App("index.html?playback-controller".into()),
            )
            .auto_resize(),
            PhysicalPosition::new(0, 0),
            size,
        )
        .map_err(window_error)?;
    controller.hide().map_err(window_error)?;
    if let Err(error) = wait_ready(receive).await {
        let _ = controller.close();
        return Err(error);
    }
    Ok(())
}

async fn move_document<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    window: &tauri::Window<R>,
) -> CommandResult<()> {
    let handle = app.clone();
    let window = window.clone();
    let (send, receive) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let result = (|| {
            let document = handle
                .get_webview("main")
                .ok_or_else(|| CommandError::new("WINDOW_CLOSED", "播放文档已关闭。"))?;
            // macOS 的 inner_size 可能取首个 WebView 的尺寸，因此在迁入前读取目标窗口尺寸。
            let size = window.inner_size().map_err(window_error)?;
            document.set_auto_resize(false).map_err(window_error)?;
            if document.window().label() != window.label() {
                document.reparent(&window).map_err(window_error)?;
            }
            document
                .set_bounds(tauri::Rect {
                    position: PhysicalPosition::new(0, 0).into(),
                    size: size.into(),
                })
                .map_err(window_error)?;
            document.set_auto_resize(true).map_err(window_error)
        })();
        let _ = send.send(result);
    })
    .map_err(window_error)?;
    receive
        .await
        .map_err(|_| CommandError::new("WINDOW_CLOSED", "播放窗口已关闭。"))?
}

fn display<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    id: &str,
) -> CommandResult<tauri::window::Monitor> {
    app.available_monitors()
        .map_err(window_error)?
        .into_iter()
        .enumerate()
        .find(|(index, monitor)| crate::platform::displays::monitor_id(*index, monitor) == id)
        .map(|(_, monitor)| monitor)
        .ok_or_else(|| CommandError::new("DISPLAY_UNAVAILABLE", "所选屏幕已变化，请刷新屏幕列表。"))
}

pub(super) async fn set_output<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    target: &str,
) -> CommandResult<()> {
    let monitor = if target.starts_with("screen:") {
        Some(display(app, target)?)
    } else {
        None
    };
    let main = app
        .get_window("main")
        .ok_or_else(|| CommandError::new("WINDOW_CLOSED", "主窗口已关闭。"))?;
    if target == "embedded" {
        if let Some(output) = app.get_window("playback") {
            move_document(app, &main).await?;
            // 主界面只在文档已归位后恢复；控制文档盖住这一步的纯视频布局。
            publish(app, target.into(), true);
            if let Some(controller) = app.get_webview("playback-controller") {
                controller.close().map_err(window_error)?;
            }
            output.destroy().map_err(window_error)?;
        }
        main.set_focus().map_err(window_error)?;
        return Ok(());
    }
    let output = output_window(app)?;
    let embedded = app
        .get_webview("main")
        .is_some_and(|document| document.window().label() == "main");
    if embedded {
        ensure_controller(app).await?;
        prepare(app).await?;
        move_document(app, &output).await?;
        // 播放文档随宿主迁移，媒体实例持续保留。
        publish(app, "window".into(), true);
        if let Some(controller) = app.get_webview("playback-controller") {
            controller.show().map_err(window_error)?;
        }
    }
    output.set_fullscreen(false).map_err(window_error)?;
    if let Some(monitor) = monitor {
        output
            .set_position(*monitor.position())
            .map_err(window_error)?;
        output.show().map_err(window_error)?;
        output.set_fullscreen(true).map_err(window_error)?;
    } else {
        output.show().map_err(window_error)?;
    }
    output
        .unminimize()
        .and_then(|_| output.set_focus())
        .map_err(window_error)
}

pub(super) fn on_window_event<R: tauri::Runtime>(
    window: &tauri::Window<R>,
    event: &tauri::WindowEvent,
) {
    if window.label() == "playback" {
        match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let app = window.app_handle().clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = super::set_output(&app, "embedded".into()).await {
                        log::error!("返回内嵌播放失败：{}", error.message);
                    }
                });
            }
            tauri::WindowEvent::Resized(_) => {
                if !window.is_fullscreen().unwrap_or(false) {
                    let app = window.app_handle();
                    let state = app.state::<PlaybackOutputState>();
                    let current = state.view.lock().unwrap().clone();
                    if current.output.starts_with("screen:") && !current.switching {
                        publish(app, "window".into(), false);
                    }
                }
            }
            _ => {}
        }
    } else if window.label() == "main" && matches!(event, tauri::WindowEvent::Destroyed) {
        if let Some(output) = window.app_handle().get_window("playback") {
            let _ = output.destroy();
        }
    }
}

// 切换失败时，按 WebView 实际所在的窗口恢复界面状态。
pub(super) fn recover<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    let detached = app
        .get_webview("main")
        .is_some_and(|document| document.window().label() == "playback");
    if detached {
        let current = super::view(app);
        publish(
            app,
            if current.output == "embedded" {
                "window".into()
            } else {
                current.output
            },
            false,
        );
        if let Some(controller) = app.get_webview("playback-controller") {
            let _ = controller.show();
        }
        if let Some(output) = app.get_window("playback") {
            let _ = output.show();
        }
    } else {
        if let Some(controller) = app.get_webview("playback-controller") {
            let _ = controller.close();
        }
        if let Some(output) = app.get_window("playback") {
            let _ = output.destroy();
        }
        publish(app, "embedded".into(), false);
    }
}
