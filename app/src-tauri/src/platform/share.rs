use crate::{
    error::{CommandError, CommandResult},
    models::{ShareEdge, SharePosition},
};

pub fn share_file<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    webview: &tauri::Webview<R>,
    path: String,
    position: Option<SharePosition>,
) -> CommandResult<bool> {
    #[cfg(target_os = "macos")]
    {
        use objc2::{AnyThread, rc::Retained, runtime::AnyObject};
        use objc2_app_kit::{NSSharingServicePicker, NSView};
        use objc2_foundation::{NSArray, NSPoint, NSRect, NSRectEdge, NSSize, NSString, NSURL};
        let _ = app;
        webview
            .with_webview(move |view| unsafe {
                let native = &*(view.inner() as *const NSView);
                let url: Retained<AnyObject> =
                    Retained::cast_unchecked(NSURL::fileURLWithPath(&NSString::from_str(&path)));
                let items = NSArray::from_retained_slice(&[url]);
                let picker =
                    NSSharingServicePicker::initWithItems(NSSharingServicePicker::alloc(), &items);
                let (x, y, edge) = position
                    .map(|value| {
                        (
                            value.x,
                            value.y,
                            match value.preferred_edge {
                                Some(ShareEdge::Top) => NSRectEdge::NSMaxYEdge,
                                Some(ShareEdge::Left) => NSRectEdge::NSMinXEdge,
                                Some(ShareEdge::Right) => NSRectEdge::NSMaxXEdge,
                                _ => NSRectEdge::NSMinYEdge,
                            },
                        )
                    })
                    .unwrap_or((0.0, 0.0, NSRectEdge::NSMinYEdge));
                picker.showRelativeToRect_ofView_preferredEdge(
                    NSRect::new(NSPoint::new(x, y), NSSize::new(1.0, 1.0)),
                    native,
                    edge,
                );
            })
            .map_err(|error| CommandError::new("LOG_SHARE", error.to_string()))?;
        Ok(true)
    }
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        use tauri::Manager;
        use tauri_plugin_sharekit::{RectEdge, ShareExt, ShareFileOptions};
        let window = app
            .get_webview_window(webview.label())
            .ok_or_else(|| CommandError::new("LOG_SHARE", "分享窗口已关闭。"))?;
        match app.share().share_file(
            window,
            path,
            ShareFileOptions {
                mime_type: Some("text/plain".into()),
                title: Some("分享日志文件".into()),
                position: position.map(|value| tauri_plugin_sharekit::SharePosition {
                    x: value.x,
                    y: value.y,
                    preferred_edge: value.preferred_edge.map(|edge| match edge {
                        ShareEdge::Top => RectEdge::Top,
                        ShareEdge::Bottom => RectEdge::Bottom,
                        ShareEdge::Left => RectEdge::Left,
                        ShareEdge::Right => RectEdge::Right,
                    }),
                }),
            },
        ) {
            Ok(()) => Ok(true),
            Err(error) if error.to_string() == "Share cancelled" => Ok(false),
            Err(error) => Err(CommandError::new("LOG_SHARE", error.to_string())),
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "android", target_os = "ios")))]
    {
        let _ = (app, webview, path, position);
        Err(CommandError::new("LOG_SHARE", "当前平台不支持系统分享。"))
    }
}
