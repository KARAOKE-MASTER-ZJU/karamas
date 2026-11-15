use super::{
    controls, monitor::EnginePlaybackState, playback::recast_current_song, state::EngineContext,
};
use crate::{
    cast::{
        MediaFormat, Quality,
        dlna::settings::{get_dlna_direct, get_dlna_hls_direct},
    },
    error::CoreError,
    playlist::RemoteSync,
    sources::bilibili::BilibiliMediaQuality,
};
use log::{debug, info, warn};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex, Weak},
    time::Duration,
};
use tokio::sync::{Notify, mpsc, watch};
/// 无变化时的完整上报间隔，覆盖设备侧变化等没有事件来源的情况。
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);

/// 状态变化唤醒；控制入口与起播流程触发，避免用轮询猜变化。
static WAKE: Mutex<Option<Arc<Notify>>> = Mutex::new(None);

/// 通知遥控任务立即上报一次状态。
pub(crate) fn wake() {
    let wake = WAKE
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    if let Some(wake) = wake {
        wake.notify_one();
    }
}

/// 宿主传入的客户端标识；名字由用户在设置里决定。
pub struct RemoteClient {
    pub enabled: bool,
    pub app_id: String,
    pub name: String,
}

/// 注册时上报的执行端身份。
pub struct RemoteIdentity {
    pub enabled: bool,
    pub app_id: String,
    pub name: String,
    pub mode: &'static str,
    pub target_name: String,
}

impl RemoteIdentity {
    pub fn new(
        enabled: bool,
        app_id: String,
        name: String,
        mode: &'static str,
        target_name: String,
    ) -> Self {
        // 名字只用于展示：留空回退到平台名，超出协议上限就截断。
        let name = name.trim();
        let name = if name.is_empty() {
            default_remote_name()
        } else {
            name.chars().take(80).collect()
        };
        Self {
            enabled,
            app_id,
            name,
            mode,
            target_name,
        }
    }
}

/// 未设置设备名称时上报的名字。
pub fn default_remote_name() -> String {
    format!("{} 投屏", platform_label())
}

fn platform_label() -> &'static str {
    match std::env::consts::OS {
        "android" => "Android",
        "ios" => "iOS",
        "macos" => "macOS",
        "windows" => "Windows",
        "linux" => "Linux",
        _ => "设备",
    }
}

fn registration_frame(identity: &RemoteIdentity) -> String {
    json!({
        "v": 1,
        "type": "remote:register",
        "appId": identity.app_id,
        "name": identity.name,
        "target": { "name": identity.target_name, "mode": identity.mode },
    })
    .to_string()
}

#[derive(Default)]
struct QualityCache {
    song: Option<String>,
    items: Vec<BilibiliMediaQuality>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RemoteStatus {
    Disconnected,
    Pending,
    Disabled,
    Registered(String),
    Failed(String),
}

pub(crate) struct RemoteControl {
    pub enabled: watch::Sender<bool>,
    pub status: watch::Sender<RemoteStatus>,
}

impl RemoteControl {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled: watch::channel(enabled).0,
            status: watch::channel(RemoteStatus::Disconnected).0,
        }
    }

    pub fn accepts_session(&self, session_id: &str) -> bool {
        *self.enabled.borrow()
            && matches!(&*self.status.borrow(), RemoteStatus::Registered(current) if current == session_id)
    }

    async fn set_enabled(&self, enabled: bool) -> Result<(), CoreError> {
        let mut status = self.status.subscribe();
        if *self.enabled.borrow() == enabled
            && matches!(&*status.borrow(), RemoteStatus::Registered(_) if enabled)
        {
            return Ok(());
        }
        let disconnected = *status.borrow() == RemoteStatus::Disconnected;
        if !disconnected {
            self.status.send_replace(RemoteStatus::Pending);
        }
        self.enabled.send_replace(enabled);
        if disconnected {
            return Ok(());
        }
        let result = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match status.borrow_and_update().clone() {
                    RemoteStatus::Disconnected => return Ok(()),
                    RemoteStatus::Disabled if !enabled => return Ok(()),
                    RemoteStatus::Registered(_) if enabled => return Ok(()),
                    RemoteStatus::Failed(message) => return Err(CoreError::from(message)),
                    _ => {}
                }
                status
                    .changed()
                    .await
                    .map_err(|_| CoreError::from("遥控任务已结束"))?;
            }
        })
        .await
        .unwrap_or_else(|_| Err("等待网页遥控服务端确认超时".into()));
        if result.is_err() && enabled {
            self.status.send_replace(RemoteStatus::Pending);
            self.enabled.send_replace(false);
        }
        result
    }
}

/// 修改当前会话的遥控开关；未连接时由宿主保存偏好。
pub async fn set_remote_control_enabled_core(enabled: bool) -> Result<(), CoreError> {
    let ctx = super::state::ENGINE_STATE
        .read()
        .map_err(|_| "无法读取引擎状态")?
        .clone();
    let Some(ctx) = ctx else { return Ok(()) };
    let control = ctx
        .remote_control
        .as_ref()
        .ok_or("当前会话未接入网页遥控")?;
    if enabled
        && std::env::var("KTV_SYNC_MODE").is_ok_and(|mode| mode.eq_ignore_ascii_case("POLLING"))
    {
        return Err("HTTP 轮询模式不支持网页遥控".into());
    }
    control.set_enabled(enabled).await
}

/// 启动遥控任务，返回歌单同步循环使用的通道。
pub(crate) fn start_remote(
    ctx: &Arc<EngineContext>,
    identity: RemoteIdentity,
) -> Result<RemoteSync, CoreError> {
    let control = ctx
        .remote_control
        .as_ref()
        .ok_or("当前会话未接入网页遥控")?
        .clone();
    let (command_tx, mut command_rx) = mpsc::unbounded_channel::<Value>();
    // 断线或停用期间只保留最新快照，不积压过期状态。
    let (playback_tx, playback_rx) = watch::channel(None);
    let wake = Arc::new(Notify::new());
    *WAKE.lock().unwrap_or_else(|error| error.into_inner()) = Some(wake.clone());
    let session = Arc::downgrade(ctx);
    let commands_control = control.clone();
    ctx.spawn_task(async move {
        let mut status = commands_control.status.subscribe();
        loop {
            tokio::select! {
                biased;
                changed = status.changed() => {
                    if changed.is_err() { break; }
                    if !matches!(&*status.borrow(), RemoteStatus::Registered(_)) {
                        while command_rx.try_recv().is_ok() {}
                    }
                }
                command = command_rx.recv() => {
                    let Some(command) = command else { break };
                    if !command["sessionId"].as_str().is_some_and(|id| commands_control.accepts_session(id)) {
                        continue;
                    }
                    let Some(ctx) = session.upgrade() else { break };
                    execute(&ctx, &command).await;
                }
            }
        }
    })?;

    let session: Weak<EngineContext> = Arc::downgrade(ctx);
    let reporting_control = control.clone();
    let mode = identity.mode;
    ctx.spawn_task(async move {
        let mut status = reporting_control.status.subscribe();
        let mut seq = 0_u64;
        let mut cache = QualityCache::default();
        loop {
            let current = status.borrow_and_update().clone();
            if let RemoteStatus::Registered(session_id) = current {
                let report = async {
                    // 首帧只读取已有状态，清晰度查询随后补齐。
                    {
                        let Some(ctx) = session.upgrade() else { return };
                        let state = ctx.playback_state().await;
                        if cache.song != state.song_url {
                            cache = QualityCache::default();
                        }
                        seq += 1;
                        playback_tx.send_replace(Some((
                            session_id.clone(),
                            playback_frame(&state, mode, &cache, seq),
                        )));
                    }
                    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
                    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                    heartbeat.tick().await;
                    loop {
                        {
                            let Some(ctx) = session.upgrade() else { return };
                            let state = ctx.playback_state().await;
                            refresh_qualities(&mut cache, state.song_url.as_deref()).await;
                            let state = ctx.playback_state().await;
                            if cache.song != state.song_url {
                                cache = QualityCache::default();
                            }
                            seq += 1;
                            playback_tx.send_replace(Some((
                                session_id.clone(),
                                playback_frame(&state, mode, &cache, seq),
                            )));
                        }
                        tokio::select! {
                            _ = wake.notified() => {}
                            _ = heartbeat.tick() => {}
                        }
                    }
                };
                tokio::select! {
                    biased;
                    _ = status.changed() => {}
                    _ = report => { break; }
                }
                playback_tx.send_replace(None);
            } else if status.changed().await.is_err() {
                break;
            }
        }
        info!("遥控任务结束");
    })?;

    Ok(RemoteSync {
        registration: registration_frame(&identity),
        commands: command_tx,
        playback: playback_rx,
        enabled: control.enabled.subscribe(),
        control,
    })
}

async fn execute(ctx: &Arc<EngineContext>, message: &Value) {
    let command = message["command"].as_str().unwrap_or_default();
    let payload = &message["payload"];
    let result = match command {
        "play" => controls::play_core().await.map(|_| ()),
        "pause" => controls::pause_core().await.map(|_| ()),
        "seek" => match seconds(payload) {
            Some(seconds) => controls::jump_to_secs(seconds).await,
            None => return warn!("遥控 seek 缺少秒数"),
        },
        "setVolume" => match payload["volume"].as_u64() {
            Some(volume) => controls::set_volume_core(volume.min(100) as u32)
                .await
                .map(|_| ()),
            None => return warn!("遥控 setVolume 缺少音量"),
        },
        "changeVolume" => match payload["increase"].as_bool() {
            Some(true) => controls::volume_up_core(5).await,
            Some(false) => controls::volume_down_core(5).await,
            None => return warn!("遥控 changeVolume 缺少方向"),
        },
        "setQuality" => set_quality(payload).await,
        "recast" => {
            recast_current_song(ctx).await;
            Ok(())
        }
        other => return debug!("忽略未知遥控命令: {}", other),
    };
    if let Err(error) = result {
        warn!("遥控命令 {} 执行失败: {}", command, error);
    }
}

fn seconds(payload: &Value) -> Option<u32> {
    let seconds = payload["seconds"].as_f64()?;
    if !seconds.is_finite() || seconds < 0.0 {
        return None;
    }
    Some(seconds.min(u32::MAX as f64) as u32)
}

async fn set_quality(payload: &Value) -> Result<(), CoreError> {
    let qn = payload["qn"]
        .as_u64()
        .and_then(|qn| u32::try_from(qn).ok())
        .ok_or("遥控 setQuality 缺少清晰度")?;
    let quality = Quality::from_qn(qn).ok_or("不支持的清晰度")?;
    match payload["format"].as_str() {
        Some(format) => {
            let format: MediaFormat = serde_json::from_value(Value::String(format.to_owned()))
                .map_err(|_| CoreError::from("不支持的取源方式"))?;
            controls::set_media_quality_core(quality, format)
                .await
                .map(|_| ())
        }
        None => controls::set_quality_core(quality).await.map(|_| ()),
    }
}

fn playback_frame(
    state: &EnginePlaybackState,
    mode: &str,
    cache: &QualityCache,
    seq: u64,
) -> String {
    let qualities = quality_options(
        mode,
        cache,
        state.song_url.as_deref(),
        state.quality_qn.map(|qn| (qn, state.quality_format)),
    );
    let quality_supported = !qualities.is_empty();

    json!({
        "v": 1,
        "type": "remote:playback",
        "playback": {
            "seq": seq,
            "position": state.current_secs,
            "duration": state.total_secs,
            "volume": state.volume,
            "isPlaying": state.is_playing,
            "qualities": qualities,
            "capabilities": {
                "absoluteVolume": state.capabilities.absolute_volume,
                "seek": state.capabilities.seek,
                "quality": quality_supported,
            },
        },
    })
    .to_string()
}

/// 清晰度来自视频源，与投屏设备无关；按歌曲缓存，切歌时才重新查询。
async fn refresh_qualities(cache: &mut QualityCache, song: Option<&str>) {
    if cache.song.as_deref() == song {
        return;
    }
    let items = match song {
        Some(song) => match crate::sources::bilibili::get_bilibili_media_qualities(song).await {
            Ok(items) => items,
            Err(error) => {
                debug!("查询遥控清晰度失败: {}", error);
                Vec::new()
            }
        },
        None => Vec::new(),
    };
    // 查询取消时不记为已缓存，下次启用仍能补齐清晰度。
    cache.song = song.map(str::to_owned);
    cache.items = items;
}

fn quality_options(
    mode: &str,
    cache: &QualityCache,
    song: Option<&str>,
    current: Option<(u32, Option<MediaFormat>)>,
) -> Vec<Value> {
    if mode == "bilibili" {
        return cache
            .items
            .iter()
            .map(|quality| {
                let mut option = json!({ "qn": quality.qn, "label": quality.label });
                if current.map(|(qn, _)| qn) == Some(quality.qn) {
                    option["now"] = Value::Bool(true);
                }
                option
            })
            .collect();
    }

    let formats = split_formats(mode);
    let mut options = Vec::new();
    for quality in &cache.items {
        if quality.direct_mp4 == Some(true) {
            options.push(quality_option(
                quality.qn,
                Some(MediaFormat::Mp4),
                &quality.label,
                current,
            ));
        }
        if quality.dash {
            for format in &formats {
                options.push(quality_option(
                    quality.qn,
                    Some(*format),
                    &quality.label,
                    current,
                ));
            }
        }
    }
    // YouTube 没有分档查询接口，沿用双轨取源的固定选项。
    if options.is_empty() && is_mux_capable(mode) && song.is_some_and(is_youtube) {
        for (qn, label) in [(64u32, "720P准高清"), (80u32, "1080P高清")] {
            for format in &formats {
                options.push(quality_option(qn, Some(*format), label, current));
            }
        }
    }
    options
}

fn quality_option(
    qn: u32,
    format: Option<MediaFormat>,
    label: &str,
    current: Option<(u32, Option<MediaFormat>)>,
) -> Value {
    let mut option = json!({ "qn": qn, "label": label });
    if let Some(format) = format {
        option["format"] = json!(format_name(format));
    }
    let is_now = current.is_some_and(|(current_qn, current_format)| {
        current_qn == qn && format.map_or(true, |format| current_format == Some(format))
    });
    if is_now {
        option["now"] = Value::Bool(true);
    }
    option
}

/// DLNA / 本机可用的取源方式，与播放页的选项规则一致。
fn split_formats(mode: &str) -> Vec<MediaFormat> {
    match mode {
        "local" => vec![MediaFormat::Hls, MediaFormat::Dash],
        "dlna" if !get_dlna_direct() => {
            if get_dlna_hls_direct() {
                vec![MediaFormat::Hls, MediaFormat::Dash]
            } else {
                vec![MediaFormat::Dash]
            }
        }
        "dlna" => {
            if get_dlna_hls_direct() {
                vec![MediaFormat::Hls]
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    }
}

fn is_mux_capable(mode: &str) -> bool {
    mode == "local" || !get_dlna_direct() || get_dlna_hls_direct()
}

fn format_name(format: MediaFormat) -> &'static str {
    match format {
        MediaFormat::Mp4 => "mp4",
        MediaFormat::Dash => "dash",
        MediaFormat::Hls => "hls",
    }
}

fn is_youtube(url: &str) -> bool {
    let url = url
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .to_ascii_lowercase();
    [
        "www.youtube.com/",
        "m.youtube.com/",
        "youtube.com/",
        "youtu.be/",
    ]
    .iter()
    .any(|prefix| url.starts_with(prefix))
}
