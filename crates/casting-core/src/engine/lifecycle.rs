use super::{
    playback::{play_and_confirm, prepare_duration},
    state::{ENGINE_STATE, EngineContext},
};
use crate::cast::dlna::settings::set_dlna_quality;
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
) -> Result<(), CoreError> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    info!("开始初始化核心引擎: {}, Room: {}", loc_str, room_id);

    reset_engine();

    // 每次进入 DLNA 会话都从已验证最稳定的 720P 开始。静态清晰度状态会
    // 跨引擎重建存活，若不显式重置会让上一会话的 1080P 误成为默认值。
    set_dlna_quality(cast::Quality::P720)?;
    info!(target: "DLNA1080", "DLNA 新会话默认清晰度: 720P");

    let handle = rt.clone();
    let (controller, device, local_ip_addr, port, cache, shared_state) =
        connect_dlna_device(loc_str, handle).await?;

    let caster: Arc<dyn cast::Caster> =
        Arc::new(DlnaCaster::new(controller, device, local_ip_addr, port)
            .with_sessions(shared_state.media_sessions.clone()));

    connect_room(base_url_str, room_id, caster, local_ip_addr, port, cache, rt).await?;

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

    let target_ip = loc_str
        .split('/')
        .nth(2)
        .and_then(|hp| hp.split(':').next())
        .unwrap_or("127.0.0.1");

    let (local_ip_addr, port, cache, shared_state) =
        crate::media::server::start(target_ip, handle)?;

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
) -> Result<(), CoreError> {
    info!("开始连接房间: {}", room_id);

    let pm = PlaylistManager::new(&base_url_str, room_id);

    let caster_cb = Arc::clone(&caster);
    let cache_cb = Arc::clone(&cache);
    let gate_cb = Arc::clone(&pm.progress_gate);
    let playback_lock = Arc::new(Mutex::new(()));
    let playback_cb = Arc::clone(&playback_lock);
    let sync_task = pm.start_sync(&rt, move |video_url| {
        let c = Arc::clone(&caster_cb);
        let cache = Arc::clone(&cache_cb);
        let gate = Arc::clone(&gate_cb);
        let playback = Arc::clone(&playback_cb);
        Box::pin(async move {
            let _playback = playback.lock().await;
            let Some(token) = gate.lock().await.play_token(&video_url) else { return };
            info!("通知设备准备拉取路径: {}", video_url);
            // 先写入 BV 的完整时长，再通知 DLNA 设备拉流。DASH/fMP4 的 moov
            // 可能只给 renderer 暴露首个分片（约 5 秒）的临时时长；若先播放，
            // Android 的自动切歌逻辑会把该临时时长误认为歌曲已结束。
            prepare_duration(&video_url, &cache).await;

            play_and_confirm(&*c, &video_url, token, &gate).await;
        })
    });

    let ctx = Arc::new(EngineContext {
        caster,
        playlist_manager: pm,
        duration_cache: cache,
        local_ip: local_ip_addr,
        server_port: port,
        is_playing: AtomicBool::new(true),
        playback_lock,
        // danmaku_on: AtomicBool::new(false),
        // quality_qn: AtomicU32::new(cast::Quality::default().as_qn()),
        rt,
        tasks: std::sync::Mutex::new(vec![sync_task.abort_handle()]),
        stopped: AtomicBool::new(false),
    });

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
    rt: tokio::runtime::Handle,
) -> Result<(), CoreError> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    info!("开始初始化Bilibili引擎, Room: {}", room_id);

    reset_engine();

    let caster: Arc<dyn cast::Caster> = Arc::new(
        cast::bilibili::BilibiliCaster::new(session, device_buvid)
    );
    let cache = Arc::new(Mutex::new(std::collections::HashMap::new()));
    connect_room(base_url_str, room_id, caster, "127.0.0.1".parse().unwrap(), 0, cache, rt).await
}
