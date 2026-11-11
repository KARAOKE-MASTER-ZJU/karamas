use super::{
    monitor::start_monitor,
    playback::update_playback,
    remote::{self, RemoteClient, RemoteIdentity},
    state::{ENGINE_STATE, EngineContext},
};
use crate::cast::dlna::settings::{reset_dlna_media_quality, set_dlna_media_quality};
use crate::cast::{Caster, MediaFormat, Quality};
use crate::{
    cast::{
        self,
        dlna::{DlnaCaster, DlnaController, DlnaDevice},
    },
    error::CoreError,
    media::server::SharedState,
    playlist::PlaylistManager,
};
use actix_web::web;
use log::info;
use std::sync::{Arc, atomic::AtomicBool};
use tokio::sync::Mutex;

pub fn reset_engine() {
    let previous = ENGINE_STATE.write().ok().and_then(|mut guard| guard.take());
    if let Some(ctx) = previous {
        ctx.stop_tasks();
    }
    crate::media::server::stop();
}

pub async fn start_engine_core(
    base_url_str: String,
    room_id: String,
    loc_str: String,
    rt: tokio::runtime::Handle,
    initial_quality: (Quality, MediaFormat),
    remote: Option<RemoteClient>,
) -> Result<(), CoreError> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    info!("开始初始化核心引擎: {}, Room: {}", loc_str, room_id);

    reset_engine();

    reset_dlna_media_quality();
    set_dlna_media_quality(initial_quality.0, initial_quality.1)?;

    let handle = rt.clone();
    let (controller, device, local_ip_addr, port, cache, shared_state) =
        connect_dlna_device(loc_str, handle).await?;

    let target_name = device.friendly_name.clone();
    let caster: Arc<dyn cast::Caster> =
        Arc::new(DlnaCaster::new(controller, device, local_ip_addr, port)
            .with_sessions(shared_state.media_sessions.clone()));
    let identity =
        remote.map(|client| RemoteIdentity::new(client.app_id, client.name, "dlna", target_name));

    connect_room(
        base_url_str,
        room_id,
        caster,
        local_ip_addr,
        port,
        cache,
        rt,
        identity,
    )
    .await?;

    info!("Rust Engine 已初始化，设备连接成功");
    Ok(())
}

pub async fn connect_dlna_device(
    loc_str: String,
    handle: tokio::runtime::Handle,
) -> Result<
    (
        DlnaController,
        DlnaDevice,
        std::net::IpAddr,
        u16,
        Arc<Mutex<std::collections::HashMap<String, u32>>>,
        web::Data<SharedState>,
    ),
    CoreError,
> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    info!("开始连接DLNA设备: {}", loc_str);

    let controller = DlnaController::new();
    let uri = loc_str.parse()?;
    let device_obj = rupnp::Device::from_url(uri).await?;

    let device = DlnaDevice {
        friendly_name: device_obj.friendly_name().to_string(),
        location: loc_str.clone(),
        device: device_obj,
        services: vec![],
    };

    let target_url = url::Url::parse(&loc_str)?;
    let host = target_url.host_str().ok_or("设备地址缺少主机名")?;
    let target_ip = tokio::net::lookup_host((host, target_url.port_or_known_default().unwrap_or(80)))
        .await?
        .find(|address| address.is_ipv4())
        .ok_or("设备地址未解析到 IPv4 地址")?
        .ip()
        .to_string();

    let (local_ip_addr, port, cache, shared_state) =
        crate::media::server::start(&target_ip, handle)?;

    Ok((controller, device, local_ip_addr, port, cache, shared_state))
}

pub async fn connect_room(
    base_url_str: String,
    room_id: String,
    caster: Arc<dyn cast::Caster>,
    local_ip_addr: std::net::IpAddr,
    port: u16,
    cache: Arc<Mutex<std::collections::HashMap<String, u32>>>,
    rt: tokio::runtime::Handle,
    remote_identity: Option<RemoteIdentity>,
) -> Result<(), CoreError> {
    info!("开始连接房间: {}", room_id);

    let pm = PlaylistManager::new(&base_url_str, room_id);

    let ctx = Arc::new(EngineContext {
        caster,
        playlist_manager: pm,
        duration_cache: cache,
        local_ip: local_ip_addr,
        server_port: port,
        is_playing: AtomicBool::new(true),
        playback_lock: Arc::new(Mutex::new(())),
        progress_control: Mutex::new(0),
        // danmaku_on: AtomicBool::new(false),
        // quality_qn: AtomicU32::new(cast::Quality::default().as_qn()),
        rt,
        tasks: std::sync::Mutex::new(vec![]),
        stopped: AtomicBool::new(false),
    });

    // 遥控与歌单同步共用同一条 WS 连接。
    let remote_sync = match remote_identity {
        Some(identity) => Some(remote::start_remote(&ctx, identity)?),
        None => None,
    };

    // 同步任务只弱引用会话，避免阻止引擎释放。
    let session = Arc::downgrade(&ctx);
    let sync_task = ctx.playlist_manager.start_sync(
        &ctx.rt,
        move |video_url| {
            let session = session.clone();
            Box::pin(async move {
                let Some(ctx) = session.upgrade() else { return };
                update_playback(&ctx, video_url).await;
            })
        },
        remote_sync,
    );
    ctx.tasks.lock().map_err(|_| "Lock error")?.push(sync_task.abort_handle());
    start_monitor(&ctx)?;

    let mut guard = ENGINE_STATE.write().map_err(|_| "Lock error")?;
    *guard = Some(ctx);
    info!("房间连接成功");

    Ok(())
}

pub async fn start_bilibili_engine_core(
    base_url_str: String,
    room_id: String,
    session: cast::bilibili::BilibiliSession,
    device_buvid: String,
    sender_buvid: String,
    rt: tokio::runtime::Handle,
    initial_quality: Quality,
    remote: Option<RemoteClient>,
) -> Result<(), CoreError> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    info!("开始初始化Bilibili引擎, Room: {}", room_id);

    reset_engine();

    let caster = Arc::new(cast::bilibili::BilibiliCaster::new(session, device_buvid, sender_buvid));
    caster.initialize_quality(initial_quality);
    caster.start_broadcast();
    let cache = Arc::new(Mutex::new(std::collections::HashMap::new()));
    let identity = remote.map(|client| {
        RemoteIdentity::new(client.app_id, client.name, "bilibili", "小电视".to_owned())
    });
    connect_room(
        base_url_str,
        room_id,
        caster,
        "127.0.0.1".parse().unwrap(),
        0,
        cache,
        rt,
        identity,
    )
    .await
}

/// 启动本机播放；宿主负责转发操作和回传播放器状态。
pub async fn start_local_engine_core(
    base_url: String,
    room_id: String,
    send: crate::cast::local::LocalCommandSink,
    rt: tokio::runtime::Handle,
    initial_quality: (Quality, MediaFormat),
    remote: Option<RemoteClient>,
) -> Result<(), CoreError> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reset_engine();
    let (ip, port, cache, state) = crate::media::server::start_local(rt.clone())?;
    let caster = Arc::new(crate::cast::local::LocalCaster::new(
        port,
        state.media_sessions.clone(),
        send,
    ));
    caster
        .set_media_quality(initial_quality.0, initial_quality.1)
        .await?;
    let identity = remote.map(|client| {
        RemoteIdentity::new(client.app_id, client.name, "local", "本机播放".to_owned())
    });
    connect_room(base_url, room_id, caster, ip, port, cache, rt, identity).await
}
