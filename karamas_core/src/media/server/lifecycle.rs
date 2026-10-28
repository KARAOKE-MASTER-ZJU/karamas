use super::{SharedState, proxy_handler};
use crate::{error::CoreError, media::session};
use actix_web::{App, HttpServer, web};
use log::{info, warn};
use std::{
    net::Ipv4Addr,
    sync::{Arc, RwLock},
};
use tokio::sync::{Mutex, Notify};

static MEDIA_SERVER_SHUTDOWN: RwLock<Option<Arc<Notify>>> = RwLock::new(None);

/// 优先选择与目标设备最接近的本机 IPv4 地址。
fn get_best_local_ip(target_device_ip: &str) -> String {
    let interfaces = local_ip_address::list_afinet_netifas().unwrap_or_default();
    let target_u32 = target_device_ip.parse::<Ipv4Addr>().map(u32::from).ok();
    if let Some(target) = target_u32 {
        let best = interfaces
            .iter()
            .filter_map(|(name, ip)| {
                if let std::net::IpAddr::V4(v4) = ip {
                    let m_bits = (target ^ u32::from(*v4)).leading_zeros();
                    Some((m_bits, ip.to_string(), name))
                } else {
                    None
                }
            })
            .max_by_key(|(bits, _, _)| *bits);
        if let Some((_, ip_str, _)) = best {
            return ip_str;
        }
    }
    local_ip_address::local_ip()
        .map(|ip| ip.to_string())
        .unwrap_or_else(|_| "127.0.0.1".to_string())
}

/// 启动本地媒体代理，返回监听地址和共享状态。
pub(crate) fn start(
    target_device_ip: &str,
    handle: tokio::runtime::Handle,
) -> Result<
    (
        std::net::IpAddr,
        u16,
        Arc<Mutex<std::collections::HashMap<String, u32>>>,
        web::Data<SharedState>,
    ),
    CoreError,
> {
    start_with_binding(Some(target_device_ip), handle)
}

/// 本机播放只监听回环地址，端口由系统分配。
pub(crate) fn start_local(
    handle: tokio::runtime::Handle,
) -> Result<
    (
        std::net::IpAddr,
        u16,
        Arc<Mutex<std::collections::HashMap<String, u32>>>,
        web::Data<SharedState>,
    ),
    CoreError,
> {
    start_with_binding(None, handle)
}

fn start_with_binding(
    target_device_ip: Option<&str>,
    handle: tokio::runtime::Handle,
) -> Result<
    (
        std::net::IpAddr,
        u16,
        Arc<Mutex<std::collections::HashMap<String, u32>>>,
        web::Data<SharedState>,
    ),
    CoreError,
> {
    let cache = Arc::new(Mutex::new(std::collections::HashMap::new()));
    let shared_state = web::Data::new(SharedState {
        duration_cache: cache.clone(),
        eplus_auth: Arc::new(tokio::sync::Mutex::new(None)),
        media_sessions: Arc::new(session::MediaSessions::default()),
    });
    // 通知旧服务器关闭（防止端口冲突）
    {
        let mut guard = MEDIA_SERVER_SHUTDOWN.write().map_err(|_| "Lock error")?;
        if let Some(notify) = guard.take() {
            notify.notify_one();
        }
    }

    // 优先使用已验证的 8080，被占用时回退到内核分配
    let (listener, port) = match std::net::TcpListener::bind(if target_device_ip.is_some() {
        "0.0.0.0:8080"
    } else {
        "127.0.0.1:0"
    }) {
        Ok(l) => {
            info!("媒体服务器监听地址: {}", l.local_addr()?);
            let port = l.local_addr()?.port();
            (l, port)
        }
        Err(error) if target_device_ip.is_none() => return Err(error.into()),
        Err(_) => {
            warn!("端口 8080 被占用（可能被 Kodi WebUI 等服务使用），由内核分配可用端口");
            let l = std::net::TcpListener::bind("0.0.0.0:0")?;
            let p = l.local_addr()?.port();
            warn!("媒体服务器改用端口 {}", p);
            (l, p)
        }
    };

    let local_ip_addr: std::net::IpAddr = target_device_ip
        .map(get_best_local_ip)
        .unwrap_or_else(|| "127.0.0.1".into())
        .parse()?;
    // 媒体代理只限制连接超时，避免截断长视频的流式响应。
    let proxy_client = crate::media::upstream::client()?;
    let shared_state_clone = shared_state.clone();
    let app_factory = move || {
        App::new()
            .app_data(web::Data::new(proxy_client.clone()))
            .app_data(shared_state_clone.clone())
            .service(proxy_handler)
    };
    let server = HttpServer::new(app_factory)
        .disable_signals()
        .workers(1)
        .listen(listener)?
        .run();

    let shutdown = Arc::new(Notify::new());
    let shutdown_clone = shutdown.clone();
    {
        let mut guard = MEDIA_SERVER_SHUTDOWN.write().map_err(|_| "Lock error")?;
        *guard = Some(shutdown);
    }

    let media_sessions = shared_state.media_sessions.clone();
    handle.spawn(async move {
        info!("正在启动媒体服务器...");
        let server_handle = server.handle();
        tokio::pin!(server);
        tokio::select! {
            _ = &mut server => {},
            _ = shutdown_clone.notified() => {
                info!("媒体服务器收到关闭信号，正在停止...");
                media_sessions.clear();
                drop(server_handle.stop(false));
                let _ = server.await;
            },
        }
    });

    Ok((local_ip_addr, port, cache, shared_state))
}

/// 通知当前媒体代理停止。
pub(crate) fn stop() {
    if let Ok(mut guard) = MEDIA_SERVER_SHUTDOWN.write() {
        if let Some(notify) = guard.take() {
            notify.notify_one();
        }
    }
}
