use crate::cast::dlna_caster::DlnaCaster;
use crate::dlna_controller::{DlnaController, DlnaDevice};
use crate::playlist_manager::PlaylistManager;
use actix_web::{App, HttpServer, web};
use log::{info, debug, warn};
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::sync::Notify;

// ── 超时常量 ──────────────────────────────────────────────
// 所有可能阻塞 App 主线程（JNI block_on）或 CLI 键盘事件的
// 同步等待都必须在此阈值内返回，否则会触发 Android ANR（5s）。
pub const ANR_DEADLINE: Duration = Duration::from_secs(5);

/// DLNA SOAP 请求超时。局域网内正常响应 <100ms，3s 留有充足余量。
pub const SOAP_TIMEOUT: Duration = Duration::from_secs(3);
const _: () = assert!(SOAP_TIMEOUT.as_secs() < ANR_DEADLINE.as_secs(),
    "SOAP_TIMEOUT 必须小于 ANR_DEADLINE (5s)，否则设备不响应时无法在 ANR 前返回");

/// B站 API / 点歌台 HTTP 请求超时。
pub const API_TIMEOUT: Duration = Duration::from_secs(3);
const _: () = assert!(API_TIMEOUT.as_secs() < ANR_DEADLINE.as_secs(),
    "API_TIMEOUT 必须小于 ANR_DEADLINE (5s)，否则 B 站 API 不响应时无法在 ANR 前返回");

/// 媒体代理上游连接超时。此路径不在 block_on 链路上，可设得更宽松。
pub const PROXY_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

#[cfg(target_os = "android")]
pub mod android;

pub mod bilibili_parser;
pub mod bilibili_parts;
pub use bilibili_parts::{get_current_bilibili_parts_core, select_current_bilibili_part_core};
pub mod cast;
pub mod dlna_controller;
pub mod fmp4_mux;
pub mod seekable_mp4;
pub mod song_cache;
pub mod virtual_mp4_http;
pub mod dash_index;
pub mod media_session;
pub mod media_server;
pub mod mp4_util;
pub mod playlist_manager;
pub mod youtube_parser;
pub mod upstream;
mod auto_next;
mod progress_gate;
#[cfg(test)]
#[path = "../test/playback.rs"]
mod playback_tests;

pub static ENGINE_STATE: RwLock<Option<Arc<EngineContext>>> = RwLock::new(None);

/// DLNA 媒体代理清晰度。720P 保持旧的完整 MP4 路径，1080P 启用 Rust fMP4 混流。
static DLNA_QUALITY_QN: AtomicU32 = AtomicU32::new(64);
static DLNA_DIRECT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Changes apply to the next media load (including recast).
pub fn set_dlna_direct(enabled: bool) {
    DLNA_DIRECT.store(enabled, Ordering::Relaxed);
    if enabled {
        DLNA_QUALITY_QN.store(64, Ordering::Relaxed);
    }
}

pub fn get_dlna_direct() -> bool {
    DLNA_DIRECT.load(Ordering::Relaxed)
}

pub fn set_dlna_quality(quality: cast::Quality) -> Result<(), &'static str> {
    if get_dlna_direct() && quality == cast::Quality::P1080 {
        return Err("DLNA 直连模式不支持 B站 1080P，请先启用媒体代理");
    }
    match quality {
        cast::Quality::P720 | cast::Quality::P1080 => {
            DLNA_QUALITY_QN.store(quality.as_qn(), Ordering::Relaxed);
            Ok(())
        }
        _ => Err("DLNA 仅支持 720P 和 1080P(beta)"),
    }
}

pub fn get_dlna_quality() -> cast::Quality {
    cast::Quality::from_qn(DLNA_QUALITY_QN.load(Ordering::Relaxed))
        .unwrap_or(cast::Quality::P720)
}

/// DLNA 媒体代理服务器的关闭信号。
static MEDIA_SERVER_SHUTDOWN: RwLock<Option<Arc<Notify>>> = RwLock::new(None);

/// Shared state for the Bilibili QR login flow (polled by Android UI / CLI).
pub static BILI_AUTH_STATE: RwLock<BiliAuthState> = RwLock::new(BiliAuthState::Idle);

pub enum BiliAuthState {
    Idle,
    AwaitingScan { qr_url: String },
    Success(crate::cast::bilibili_caster::BilibiliSession),
    Error(String),
}

pub struct EngineContext {
    pub caster: Arc<dyn cast::Caster>,
    pub playlist_manager: PlaylistManager,
    pub duration_cache: Arc<Mutex<std::collections::HashMap<String, u32>>>,
    pub local_ip: std::net::IpAddr,
    pub server_port: u16,
    pub is_playing: AtomicBool,
    playback_lock: Arc<Mutex<()>>,
    /// 仅 Bilibili 投屏使用：本地跟踪的弹幕/清晰度状态（设备侧没有读回接口）。
    // pub danmaku_on: AtomicBool,
    // pub quality_qn: AtomicU32,
    pub rt: tokio::runtime::Runtime,
}

pub struct SharedState {
    pub duration_cache: Arc<Mutex<std::collections::HashMap<String, u32>>>,
    pub eplus_auth: Arc<tokio::sync::Mutex<Option<String>>>,
    pub media_sessions: Arc<media_session::MediaSessions>,
}

pub(crate) fn get_best_local_ip(target_device_ip: &str) -> String {
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

pub fn reset_engine() {
    if let Ok(mut guard) = ENGINE_STATE.write() {
        *guard = None;
    }
    if let Ok(mut guard) = MEDIA_SERVER_SHUTDOWN.write() {
        if let Some(notify) = guard.take() {
            notify.notify_one();
        }
    }
}

pub async fn get_current_progress() -> (i32, i32) {
    let ctx = {
        let guard = ENGINE_STATE.read().ok();
        guard.and_then(|g| g.as_ref().cloned())
    };
    let Some(ctx) = ctx else { return (-1, -1) };

    read_progress(&ctx, false).await
}

/// Poll progress and run the automatic end trigger. Manual next-song stays separate.
pub async fn poll_playback_progress() -> (i32, i32) {
    let ctx = ENGINE_STATE.read().ok().and_then(|guard| guard.as_ref().cloned());
    let Some(ctx) = ctx else { return (-1, -1) };
    read_progress(&ctx, true).await
}

async fn read_progress(ctx: &EngineContext, trigger_next: bool) -> (i32, i32) {
    let gate = ctx.playlist_manager.progress_gate.lock().await;
    let revision = gate.revision();
    let hash = ctx.playlist_manager.current_hash().await;
    let playing = ctx.playlist_manager.get_song_playing().await;
    let cached_total = match playing.as_ref() {
        Some(song) => *ctx.duration_cache.lock().await.get(song).unwrap_or(&0),
        None => 0,
    };
    drop(gate);

    let (current, total) = match ctx.caster.get_progress().await {
        Ok(p) => {
            debug!(
                "progress: curr={} device_total={} cached_total={} playing={:?}",
                p.current_secs, p.total_secs, cached_total, playing
            );
            let total = if cached_total > 0 { cached_total } else { p.total_secs } as i32;
            (p.current_secs as i32, total)
        }
        Err(_) => (-1, -1),
    };
    let mut gate = ctx.playlist_manager.progress_gate.lock().await;
    let (current, total) = gate.filter(revision, current, total);
    // Validate the sample and consume once under the same lock as song updates
    // and play confirmations. An old poll cannot trigger after a new playback.
    if trigger_next && gate.auto_next.observe(current, total) {
        info!("歌曲即将结束，启动一次自动切歌任务");
        gate.auto_next.start_request(ctx.playlist_manager.clone(), hash, ctx.rt.handle());
    }
    (current, total)
}

pub fn trigger_next_song() {
    if let Ok(guard) = ENGINE_STATE.read() {
        if let Some(ctx) = guard.as_ref() {
            let ctx_task = Arc::clone(ctx);
            ctx.rt.spawn(async move {
                let mut pm = ctx_task.playlist_manager.clone();
                if let Err(e) = pm.next_song().await {
                    log::warn!("切下一首失败: {}", e);
                }
            });
        }
    }
}
pub fn trigger_prev_song() {
    if let Ok(guard) = ENGINE_STATE.read() {
        if let Some(ctx) = guard.as_ref() {
            let ctx_task = Arc::clone(ctx);
            ctx.rt.spawn(async move {
                let mut pm = ctx_task.playlist_manager.clone();
                if let Err(e) = pm.prev_song().await {
                    log::warn!("切上一首失败: {}", e);
                }
            });
        }
    }
}

pub async fn jump_to_secs(target_secs: u32) -> Result<(), Box<dyn std::error::Error>> {
    let ctx = {
        let guard = ENGINE_STATE.read().map_err(|_| "Lock error")?;
        guard.as_ref().cloned().ok_or("Engine not initialized")?
    };
    ctx.caster.seek(target_secs).await.map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
}

/// 1 = confirmed, 0 = no current song, -1 = failed/timed out/superseded, -2 = busy.
pub async fn recast_current_song(ctx: &EngineContext) -> i32 {
    let result = recast_with_caster(
        &*ctx.caster, &ctx.playlist_manager.progress_gate,
        &ctx.duration_cache, &ctx.playback_lock,
    ).await;
    if result == 1 {
        ctx.is_playing.store(true, Ordering::SeqCst);
    }
    result
}

async fn recast_with_caster(
    caster: &dyn cast::Caster,
    gate: &Mutex<progress_gate::ProgressGate>,
    cache: &Mutex<std::collections::HashMap<String, u32>>,
    playback_lock: &Mutex<()>,
) -> i32 {
    let Ok(_playback) = playback_lock.try_lock() else { return -2 };
    let Some((song, token)) = gate.lock().await.begin_recast() else { return 0 };
    // YouTube extraction (30s) and indexed DASH preparation (60s) precede
    // the device commands. Allow that shared cold-start path to finish.
    match tokio::time::timeout(Duration::from_secs(120), async {
        prepare_duration(&song, cache).await;
        play_and_confirm(caster, &song, token, gate).await
    }).await {
        Ok(true) => 1,
        Ok(false) => -1,
        Err(_) => {
            warn!("重新投屏超时");
            -1
        }
    }
}

pub async fn start_engine_core(
    base_url_str: String,
    room_id: String,
    loc_str: String,
    rt: tokio::runtime::Runtime,
) -> Result<(), Box<dyn std::error::Error>> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    info!("开始初始化核心引擎: {}, Room: {}", loc_str, room_id);

    if let Ok(mut guard) = ENGINE_STATE.write() {
        if guard.is_some() {
            info!("检测到旧引擎正在运行，正在重置...");
            *guard = None;
        }
    }

    // 每次进入 DLNA 会话都从已验证最稳定的 720P 开始。静态清晰度状态会
    // 跨引擎重建存活，若不显式重置会让上一会话的 1080P 误成为默认值。
    set_dlna_quality(cast::Quality::P720).expect("720P is a supported DLNA quality");
    info!(target: "DLNA1080", "DLNA 新会话默认清晰度: 720P");

    let handle = rt.handle().clone();
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
    Box<dyn std::error::Error>,
> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    info!("开始连接DLNA设备: {}", loc_str);

    let controller = DlnaController::new();
    let uri = loc_str.parse().expect("解析 URL 失败");
    let device_obj = rupnp::Device::from_url(uri).await.expect("连接设备失败");

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

    let cache = Arc::new(Mutex::new(std::collections::HashMap::new()));
    let shared_state = web::Data::new(SharedState {
        duration_cache: cache.clone(),
        eplus_auth: Arc::new(tokio::sync::Mutex::new(None)),
        media_sessions: Arc::new(media_session::MediaSessions::default()),
    });
    // 通知旧服务器关闭（防止端口冲突）
    if let Ok(mut guard) = MEDIA_SERVER_SHUTDOWN.write() {
        if let Some(notify) = guard.take() {
            notify.notify_one();
        }
    }

    // 优先使用已验证的 8080，被占用时回退到内核分配
    let (listener, port) = match std::net::TcpListener::bind("0.0.0.0:8080") {
        Ok(l) => {
            info!("媒体服务器绑定端口 8080");
            (l, 8080)
        }
        Err(_) => {
            warn!("端口 8080 被占用（可能被 Kodi WebUI 等服务使用），由内核分配可用端口");
            let l = std::net::TcpListener::bind("0.0.0.0:0")?;
            let p = l.local_addr()?.port();
            warn!("媒体服务器改用端口 {}", p);
            (l, p)
        }
    };

    let shutdown = Arc::new(Notify::new());
    let shutdown_clone = shutdown.clone();
    if let Ok(mut guard) = MEDIA_SERVER_SHUTDOWN.write() {
        *guard = Some(shutdown);
    }

    let shared_state_clone = shared_state.clone();
    handle.spawn(async move {
        info!("正在启动媒体服务器...");
        let app_factory = move || {
            // 媒体代理 client：只限制连接建立阶段，避免上游连接挂起导致设备拉流卡死
            // （流式响应本身不能设整体 timeout，否则长视频会被掐断）
            let proxy_client = reqwest::Client::builder()
                .connect_timeout(PROXY_CONNECT_TIMEOUT)
                .build()
                .expect("创建媒体代理 HTTP 客户端失败");
            App::new()
                .app_data(web::Data::new(proxy_client))
                .app_data(shared_state_clone.clone())
                .service(media_server::proxy_handler)
        };
        let server = HttpServer::new(app_factory)
            .workers(1)
            .listen(listener)
            .expect("使用预绑定 listener 不应失败")
            .run();
        tokio::select! {
            _ = server => {},
            _ = shutdown_clone.notified() => {
                info!("媒体服务器收到关闭信号，正在停止...");
            },
        }
    });

    let local_ip_addr: std::net::IpAddr = get_best_local_ip(target_ip).parse().unwrap();
    Ok((controller, device, local_ip_addr, port, cache, shared_state))
}

pub async fn connect_room(
    base_url_str: String,
    room_id: String,
    caster: Arc<dyn cast::Caster>,
    local_ip_addr: std::net::IpAddr,
    port: u16,
    cache: Arc<Mutex<std::collections::HashMap<String, u32>>>,
    rt: tokio::runtime::Runtime,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("开始连接房间: {}", room_id);

    let pm = PlaylistManager::new(&base_url_str, room_id);

    let caster_cb = Arc::clone(&caster);
    let cache_cb = Arc::clone(&cache);
    let gate_cb = Arc::clone(&pm.progress_gate);
    let playback_lock = Arc::new(Mutex::new(()));
    let playback_cb = Arc::clone(&playback_lock);
    pm.start_sync(move |video_url| {
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
    });

    if let Ok(mut guard) = ENGINE_STATE.write() {
        *guard = Some(ctx);
        info!("房间连接成功");
    }

    Ok(())
}

async fn prepare_duration(song: &str, cache: &Mutex<std::collections::HashMap<String, u32>>) {
    if let Some((bvid, page)) = extract_bvid(song) {
        if let Ok((_, duration)) = crate::bilibili_parser::get_page_info(&bvid, page).await {
            if duration > 0 {
                cache.lock().await.insert(song.to_owned(), duration);
                info!(target: "DLNA1080", "播放前预填充 BV 视频时长: {},p{} -> {}s", bvid, page+1, duration);
            }
        }
    }
}

async fn play_and_confirm(
    caster: &dyn cast::Caster,
    song: &str,
    token: u64,
    gate: &Mutex<progress_gate::ProgressGate>,
) -> bool {
    if gate.lock().await.revision() != token { return false; }
    match caster.play_song(&cast::SongRef(song.to_owned())).await {
        Ok(()) => {
            gate.lock().await.play_succeeded(
                token, caster.capabilities().hardware_progress, std::time::Instant::now(),
            )
        }
        Err(error) => {
            warn!("投屏失败：曲目={}，错误={}", song, error);
            false
        }
    }
}

fn extract_bvid(video_url: &str) -> Option<(String,u32)> {
    if let Some((bv, page)) = video_url.split_once("-page") {
        if bv.starts_with("BV") {
            return Some((bv.to_string(),page.parse::<u32>().unwrap_or_default()));
        }
    }
    None
}

pub async fn get_total_duration() -> u32 {
    if let Ok(guard) = ENGINE_STATE.read() {
        if let Some(ctx) = guard.as_ref() {
            if let Some(playing) = ctx.playlist_manager.get_song_playing().await {
                if let Some(&d) = ctx.duration_cache.lock().await.get(&playing) {
                    return d;
                }
            }
        }
    }
    0
}

pub async fn toggle_pause_core() -> Result<bool, Box<dyn std::error::Error>> {
    let target_state = {
        let guard = ENGINE_STATE.read().map_err(|_| "Lock error")?;
        let ctx = guard.as_ref().ok_or("Engine not initialized")?;
        !ctx.is_playing.load(Ordering::SeqCst)
    };

    let ctx = {
        let guard = ENGINE_STATE.read().map_err(|_| "Lock error")?;
        guard.as_ref().cloned().ok_or("Engine not initialized")?
    };

    if target_state {
        ctx.caster.resume().await.map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
    } else {
        ctx.caster.pause().await.map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
    }
    ctx.is_playing.store(target_state, Ordering::SeqCst);

    Ok(target_state)
}

pub async fn set_volume_core(volume: u32) -> Result<u32, Box<dyn std::error::Error>> {
    let ctx = {
        let guard = ENGINE_STATE.read().map_err(|_| "Lock error")?;
        guard.as_ref().cloned().ok_or("Engine not initialized")?
    };
    let target = volume.clamp(0, 100);
    ctx.caster.set_volume(target).await.map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
    Ok(target)
}

pub async fn get_volume_core() -> Result<u32, Box<dyn std::error::Error>> {
    let ctx = {
        let guard = ENGINE_STATE.read().map_err(|_| "Lock error")?;
        guard.as_ref().cloned().ok_or("Engine not initialized")?
    };
    let v = ctx.caster.get_volume().await.map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
    Ok(v.unwrap_or(0))
}

pub async fn volume_up_core(step: u32) -> Result<(), Box<dyn std::error::Error>> {
    let ctx = {
        let guard = ENGINE_STATE.read().map_err(|_| "Lock error")?;
        guard.as_ref().cloned().ok_or("Engine not initialized")?
    };
    ctx.caster.volume_up(step).await.map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
    Ok(())
}

pub async fn volume_down_core(step: u32) -> Result<(), Box<dyn std::error::Error>> {
    let ctx = {
        let guard = ENGINE_STATE.read().map_err(|_| "Lock error")?;
        guard.as_ref().cloned().ok_or("Engine not initialized")?
    };
    ctx.caster.volume_down(step).await.map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
    Ok(())
}

/// 直接设置弹幕开关，返回设置后的状态。
pub async fn set_danmaku_core(on: bool) -> Result<bool, Box<dyn std::error::Error>> {
    let ctx = {
        let guard = ENGINE_STATE.read().map_err(|_| "Lock error")?;
        guard.as_ref().cloned().ok_or("Engine not initialized")?
    };
    ctx.caster.set_danmaku(on).await.map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
    // ctx.danmaku_on.store(on, Ordering::SeqCst);
    Ok(on)
}

/// 切换弹幕开关，返回切换后的状态。
pub async fn toggle_danmaku_core() -> Result<bool, Box<dyn std::error::Error>> {
    set_danmaku_core(!get_danmaku_core()).await
}

/// 当前本地跟踪的弹幕状态（设备侧无读回接口）。
pub fn get_danmaku_core() -> bool {
    if let Ok(guard) = ENGINE_STATE.read() {
        if let Some(ctx) = guard.as_ref() {
            // return ctx.danmaku_on.load(Ordering::SeqCst);
            return ctx.caster.get_danmaku().unwrap_or_default();
        }
    }
    false
}

/// 直接设置清晰度，返回设置后的值。
pub async fn set_quality_core(quality: cast::Quality) -> Result<cast::Quality, Box<dyn std::error::Error>> {
    let ctx = {
        let guard = ENGINE_STATE.read().map_err(|_| "Lock error")?;
        guard.as_ref().cloned().ok_or("Engine not initialized")?
    };
    ctx.caster.set_quality(quality).await.map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
    // ctx.quality_qn.store(quality.as_qn(), Ordering::SeqCst);
    Ok(quality)
}

/// 切到下一档清晰度（循环），返回切换后的值。
pub async fn cycle_quality_core() -> Result<cast::Quality, Box<dyn std::error::Error>> {
    set_quality_core(get_quality_core().next()).await
}

/// 当前本地跟踪的清晰度（设备侧无读回接口）。
pub fn get_quality_core() -> cast::Quality {
    if let Ok(guard) = ENGINE_STATE.read() {
        if let Some(ctx) = guard.as_ref() {
            // return cast::Quality::from_qn(ctx.quality_qn.load(Ordering::SeqCst)).unwrap_or_default();
            return ctx.caster.get_quality().unwrap_or_default();
        }
    }
    cast::Quality::default()
}

pub async fn start_bilibili_engine_core(
    base_url_str: String,
    room_id: String,
    session: cast::bilibili_caster::BilibiliSession,
    device_buvid: String,
    rt: tokio::runtime::Runtime,
) -> Result<(), Box<dyn std::error::Error>> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    info!("开始初始化Bilibili引擎, Room: {}", room_id);

    if let Ok(mut guard) = ENGINE_STATE.write() {
        if guard.is_some() {
            *guard = None;
        }
    }

    let caster: Arc<dyn cast::Caster> = Arc::new(
        cast::bilibili_caster::BilibiliCaster::new(session, device_buvid)
    );
    let cache = Arc::new(Mutex::new(std::collections::HashMap::new()));
    connect_room(base_url_str, room_id, caster, "127.0.0.1".parse().unwrap(), 0, cache, rt).await
}

pub async fn discover_devices_core() -> Vec<DlnaDevice> {
    DlnaController::new()
        .discover_devices()
        .await
        .unwrap_or_default()
}

pub async fn discover_device_from_url_core(url: String) -> Result<DlnaDevice, String> {
    DlnaController::new()
        .get_device_from_url(&url)
        .await
        .map_err(|e| e.to_string())
}

/// Last synchronized current-song ID; empty when disconnected or no song exists.
pub async fn get_current_song_id_core() -> String {
    let ctx = ENGINE_STATE.read().ok().and_then(|state| state.as_ref().cloned());
    match ctx {
        Some(ctx) => ctx.playlist_manager.get_song_id().await.unwrap_or_default(),
        None => String::new(),
    }
}

pub async fn get_current_song_title_core() -> String {
    if let Ok(guard) = ENGINE_STATE.read() {
        if let Some(ctx) = guard.as_ref() {
            return ctx.playlist_manager.get_song_title().await
                .unwrap_or_else(|| "暂无歌曲".to_string());
        }
    }
    "未连接".to_string()
}
