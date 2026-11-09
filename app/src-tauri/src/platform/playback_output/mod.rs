use crate::error::{CommandError, CommandResult};
use serde::Serialize;
use std::sync::{
    Mutex,
    atomic::{AtomicU32, Ordering},
};
use tauri::{Emitter, Manager};
use tokio::sync::oneshot;

#[cfg(desktop)]
mod desktop;

#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum OutputTarget {
    Embedded,
    Window,
    Screen {
        display: super::displays::PlaybackDisplay,
        index: usize,
    },
}

// 初始化查询和变更通知共用完整的输出快照。
#[derive(Clone, Serialize)]
pub struct OutputView {
    pub target: OutputTarget,
    pub switching: bool,
}

#[derive(Serialize)]
pub struct OutputCapabilities {
    pub window: bool,
    pub screen: bool,
}

pub fn capabilities() -> OutputCapabilities {
    OutputCapabilities {
        window: cfg!(desktop),
        screen: cfg!(any(desktop, target_os = "android")),
    }
}

pub struct PlaybackOutputState {
    view: Mutex<OutputView>,
    operation: tokio::sync::Mutex<()>,
    next_request: AtomicU32,
    prepared: Mutex<Option<(u32, oneshot::Sender<()>)>>,
    pub(super) controller_ready: Mutex<Option<oneshot::Sender<()>>>,
}

impl Default for PlaybackOutputState {
    fn default() -> Self {
        Self {
            view: Mutex::new(OutputView {
                target: OutputTarget::Embedded,
                switching: false,
            }),
            operation: tokio::sync::Mutex::new(()),
            next_request: AtomicU32::new(1),
            prepared: Mutex::new(None),
            controller_ready: Mutex::new(None),
        }
    }
}

pub fn view<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> OutputView {
    app.state::<PlaybackOutputState>()
        .view
        .lock()
        .unwrap()
        .clone()
}

fn publish<R: tauri::Runtime>(app: &tauri::AppHandle<R>, next: OutputView) {
    *app.state::<PlaybackOutputState>().view.lock().unwrap() = next.clone();
    if let Err(error) = app.emit("playback-output-changed", next) {
        log::error!("广播播放输出状态失败：{error}");
    }
}

pub fn prepared<R: tauri::Runtime>(app: &tauri::AppHandle<R>, request_id: u32) {
    let state = app.state::<PlaybackOutputState>();
    let mut pending = state.prepared.lock().unwrap();
    if pending.as_ref().is_some_and(|(id, _)| *id == request_id) {
        if let Some((_, ready)) = pending.take() {
            let _ = ready.send(());
        }
    }
}

pub fn controller_ready<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    if let Some(ready) = app
        .state::<PlaybackOutputState>()
        .controller_ready
        .lock()
        .unwrap()
        .take()
    {
        let _ = ready.send(());
    }
}

pub(super) async fn prepare<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> CommandResult<()> {
    let (send, receive) = oneshot::channel();
    let request_id = app
        .state::<PlaybackOutputState>()
        .next_request
        .fetch_add(1, Ordering::Relaxed);
    *app.state::<PlaybackOutputState>().prepared.lock().unwrap() = Some((request_id, send));
    app.emit_to(
        tauri::EventTarget::webview("main"),
        "playback-output-prepare",
        request_id,
    )
    .map_err(window_error)?;
    wait_ready(receive).await
}

pub(super) async fn wait_ready(receive: oneshot::Receiver<()>) -> CommandResult<()> {
    tokio::time::timeout(std::time::Duration::from_secs(20), receive)
        .await
        .map_err(|_| CommandError::new("WINDOW_TIMEOUT", "播放界面准备超时，请重试。"))?
        .map_err(|_| CommandError::new("WINDOW_CLOSED", "播放界面已关闭。"))
}

fn window_error(error: tauri::Error) -> CommandError {
    CommandError::new("PLAYBACK_WINDOW", error.to_string())
}

pub fn create_main<R: tauri::Runtime>(app: &tauri::App<R>) -> tauri::Result<()> {
    let config = &app.config().app.windows[0];
    #[cfg(desktop)]
    {
        // 宿主窗口与播放文档分别创建，便于迁移同一个 WebView。
        let window = tauri::window::WindowBuilder::from_config(app, config)?.build()?;
        let size = window.inner_size()?;
        window.add_child(
            tauri::webview::WebviewBuilder::new("main", tauri::WebviewUrl::default()).auto_resize(),
            tauri::PhysicalPosition::new(0, 0),
            size,
        )?;
    }
    #[cfg(not(desktop))]
    tauri::WebviewWindowBuilder::from_config(app, config)?.build()?;
    Ok(())
}

pub async fn set_output<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    output: String,
) -> CommandResult<()> {
    if output != "embedded" && output != "window" && !output.starts_with("screen:") {
        return Err(CommandError::new("WINDOW_OUTPUT", "播放位置无效。"));
    }
    let supported = capabilities();
    if (output == "window" && !supported.window)
        || (output.starts_with("screen:") && !supported.screen)
    {
        return Err(CommandError::new(
            "WINDOW_UNSUPPORTED",
            "当前平台不支持所选播放位置。",
        ));
    }
    let state = app.state::<PlaybackOutputState>();
    let _operation = state.operation.lock().await;
    publish(
        app,
        OutputView {
            switching: true,
            ..view(app)
        },
    );
    let result = async {
        let target = match output.as_str() {
            "embedded" => OutputTarget::Embedded,
            "window" => OutputTarget::Window,
            _ => {
                let (index, display) = super::displays::list_playback_displays(app.clone())
                    .await?
                    .into_iter()
                    .enumerate()
                    .find(|(_, display)| display.id == output)
                    .ok_or_else(|| {
                        CommandError::new("DISPLAY_UNAVAILABLE", "所选屏幕已变化，请刷新屏幕列表。")
                    })?;
                OutputTarget::Screen { display, index }
            }
        };
        #[cfg(desktop)]
        let result = desktop::set_output(app, &target).await;
        #[cfg(target_os = "android")]
        let result = super::android::set_playback_output(app, &output).await;
        #[cfg(not(any(desktop, target_os = "android")))]
        let result = Err(CommandError::new(
            "WINDOW_UNSUPPORTED",
            "当前平台不支持独立播放窗口。",
        ));
        result.map(|_| target)
    }
    .await;
    #[cfg(desktop)]
    if result.is_err() {
        desktop::recover(app);
    }
    let mut next = view(app);
    if let Ok(target) = &result {
        next.target = target.clone();
    }
    next.switching = false;
    publish(app, next);
    result.map(|_| ())
}

pub fn focus<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> CommandResult<()> {
    #[cfg(desktop)]
    if let Some(window) = app.get_window("playback") {
        window
            .unminimize()
            .and_then(|_| window.show())
            .and_then(|_| window.set_focus())
            .map_err(window_error)?;
    }
    #[cfg(target_os = "android")]
    {
        let _ = app;
    }
    Ok(())
}

pub fn on_window_event<R: tauri::Runtime>(window: &tauri::Window<R>, event: &tauri::WindowEvent) {
    #[cfg(desktop)]
    desktop::on_window_event(window, event);
    #[cfg(not(desktop))]
    let _ = (window, event);
}

pub fn returned<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    publish(
        app,
        OutputView {
            target: OutputTarget::Embedded,
            switching: false,
        },
    );
}
