use crate::error::CommandResult;

#[cfg(target_os = "android")]
mod android;

#[cfg(target_os = "android")]
pub use android::{android_update_version, install_android_update};
#[cfg(target_os = "android")]
pub use android::{ensure_network_access, init, start_casting_service, stop_casting_service};

/// Android 返回桌面时保留 Activity 和正在投屏的会话。
#[tauri::command]
pub async fn move_to_background<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> CommandResult<()> {
    #[cfg(target_os = "android")]
    return android::move_to_background(&app).await;
    #[cfg(not(target_os = "android"))]
    let _ = app;
    #[cfg(not(target_os = "android"))]
    Ok(())
}

#[cfg(not(target_os = "android"))]
pub async fn start_casting_service<R: tauri::Runtime>(
    _app: &tauri::AppHandle<R>,
    _room_id: &str,
) -> CommandResult<()> {
    Ok(())
}

#[cfg(not(target_os = "android"))]
pub async fn stop_casting_service<R: tauri::Runtime>(
    _app: &tauri::AppHandle<R>,
) -> CommandResult<()> {
    Ok(())
}

#[cfg(not(target_os = "android"))]
pub async fn ensure_network_access<R: tauri::Runtime>(
    _app: &tauri::AppHandle<R>,
) -> CommandResult<()> {
    Ok(())
}

/// 局域网房间也需要先取得 Android 的本地网络权限。
pub async fn ensure_room_access<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    url: &url::Url,
) -> CommandResult<()> {
    let local = match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_private() || ip.is_loopback() || ip.is_link_local(),
        Some(url::Host::Ipv6(ip)) => {
            ip.is_loopback() || ip.is_unique_local() || ip.is_unicast_link_local()
        }
        Some(url::Host::Domain(host)) => !host.contains('.') || host.ends_with(".local"),
        None => false,
    };
    if local {
        ensure_network_access(app).await?;
    }
    #[cfg(target_os = "android")]
    if !local
        && let Some(host) = url.host_str()
        && let Ok(Ok(mut addresses)) = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            tokio::net::lookup_host((host, url.port_or_known_default().unwrap_or(80))),
        )
        .await
        && addresses.any(|address| match address.ip() {
            std::net::IpAddr::V4(ip) => ip.is_private() || ip.is_loopback() || ip.is_link_local(),
            std::net::IpAddr::V6(ip) => {
                ip.is_loopback() || ip.is_unique_local() || ip.is_unicast_link_local()
            }
        })
    {
        ensure_network_access(app).await?;
    }
    Ok(())
}
