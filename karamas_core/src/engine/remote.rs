//! 网页遥控：执行端注册、命令执行和播放状态上报。

use super::{controls, playback::recast_current_song, state::EngineContext};
use crate::{
    cast::{
        dlna::settings::{get_dlna_direct, get_dlna_hls_direct},
        MediaFormat, Quality,
    },
    error::CoreError,
    playlist::RemoteSync,
    sources::bilibili::BilibiliMediaQuality,
};
use log::{debug, info, warn};
use serde_json::{json, Value};
use std::{
    sync::{Arc, Weak},
    time::Duration,
};
use tokio::sync::mpsc;

/// 状态采样间隔；命令执行后立即补一帧。
const PLAYBACK_INTERVAL: Duration = Duration::from_secs(5);

/// 注册时上报的执行端身份。
pub struct RemoteIdentity {
    pub app_id: String,
    pub mode: &'static str,
    pub target_name: String,
}

impl RemoteIdentity {
    pub fn new(app_id: String, mode: &'static str, target_name: String) -> Self {
        Self {
            app_id,
            mode,
            target_name,
        }
    }
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
        "name": format!("{} 投屏", platform_label()),
        "target": { "name": identity.target_name, "mode": identity.mode },
    })
    .to_string()
}

#[derive(Default)]
struct QualityCache {
    song: Option<String>,
    items: Vec<BilibiliMediaQuality>,
}

/// 启动遥控任务，返回歌单同步循环使用的通道。
pub(crate) fn start_remote(
    ctx: &Arc<EngineContext>,
    identity: RemoteIdentity,
) -> Result<RemoteSync, CoreError> {
    let (command_tx, mut command_rx) = mpsc::unbounded_channel();
    let (playback_tx, playback_rx) = mpsc::unbounded_channel();
    let session: Weak<EngineContext> = Arc::downgrade(ctx);
    let mode = identity.mode;

    ctx.spawn_task(async move {
        let mut seq = 0_u64;
        let mut cache = QualityCache::default();
        let mut timer = tokio::time::interval(PLAYBACK_INTERVAL);
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        timer.tick().await;
        loop {
            let ctx = tokio::select! {
                Some(command) = command_rx.recv() => {
                    let Some(ctx) = session.upgrade() else { break };
                    execute(&ctx, &command).await;
                    ctx
                }
                _ = timer.tick() => {
                    let Some(ctx) = session.upgrade() else { break };
                    ctx
                }
            };
            seq += 1;
            let Some(frame) = playback_frame(&ctx, mode, &mut cache, seq).await else {
                continue;
            };
            if playback_tx.send(frame).is_err() {
                break;
            }
        }
        info!("遥控任务结束");
    })?;

    Ok(RemoteSync {
        registration: registration_frame(&identity),
        commands: command_tx,
        playback: playback_rx,
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

async fn playback_frame(
    ctx: &Arc<EngineContext>,
    mode: &str,
    cache: &mut QualityCache,
    seq: u64,
) -> Option<String> {
    let state = ctx.playback_state().await;
    let volume = ctx.caster.get_volume().await.ok().flatten();
    refresh_qualities(cache, state.song_url.as_deref()).await;
    let qualities = quality_options(
        mode,
        cache,
        state.song_url.as_deref(),
        state.quality_qn.map(|qn| (qn, state.quality_format)),
    );
    let quality_supported = !qualities.is_empty();

    Some(
        json!({
            "v": 1,
            "type": "remote:playback",
            "playback": {
                "seq": seq,
                "position": state.current_secs,
                "duration": state.total_secs,
                "volume": volume,
                "isPlaying": state.is_playing,
                "qualities": qualities,
                "capabilities": {
                    "absoluteVolume": state.capabilities.absolute_volume,
                    "seek": state.capabilities.seek,
                    "quality": quality_supported,
                },
            },
        })
        .to_string(),
    )
}

/// 清晰度来自视频源，与投屏设备无关；按歌曲缓存，切歌时才重新查询。
async fn refresh_qualities(cache: &mut QualityCache, song: Option<&str>) {
    if cache.song.as_deref() == song {
        return;
    }
    cache.song = song.map(str::to_owned);
    cache.items.clear();
    let Some(song) = song else { return };
    match crate::sources::bilibili::get_bilibili_media_qualities(song).await {
        Ok(items) => cache.items = items,
        Err(error) => debug!("查询遥控清晰度失败: {}", error),
    }
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
        let label: String = quality.label.split_whitespace().collect();
        if quality.direct_mp4 == Some(true) {
            options.push(quality_option(
                quality.qn,
                Some(MediaFormat::Mp4),
                &label,
                current,
            ));
        }
        if quality.dash {
            for format in &formats {
                options.push(quality_option(quality.qn, Some(*format), &label, current));
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
    let text = match format {
        Some(format) => format!("[{}] {}", format_name(format).to_uppercase(), label),
        None => label.to_owned(),
    };
    let mut option = json!({ "qn": qn, "label": text });
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
