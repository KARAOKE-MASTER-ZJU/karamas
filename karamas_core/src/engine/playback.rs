use super::state::{ENGINE_STATE, EngineContext};
use crate::{cast, playlist::progress_gate};
use log::{info, trace, warn};
use std::{sync::atomic::Ordering, time::Duration};
use tokio::sync::Mutex;

/// 读取会话任务最近取得的有效进度。
pub async fn get_current_progress() -> (i32, i32) {
    let ctx = {
        let guard = ENGINE_STATE.read().ok();
        guard.and_then(|g| g.as_ref().cloned())
    };
    let Some(ctx) = ctx else { return (-1, -1) };

    ctx.current_progress().await
}

/// 当前歌曲有 URL 时开始播放；无 URL 时保持已有播放。
pub(super) async fn update_playback(ctx: &EngineContext, video_url: String) {
    let _playback = ctx.playback_lock.lock().await;
    let gate = &ctx.playlist_manager.progress_gate;
    let Some(token) = gate.lock().await.play_token(&video_url) else {
        return;
    };
    info!("通知设备准备拉取路径: {}", video_url);
    // 先准备完整时长，再通知设备播放，避免临时时长影响自动切歌。
    prepare_duration(&video_url, &ctx.duration_cache).await;
    if play_and_confirm(&*ctx.caster, &video_url, token, gate).await {
        ctx.is_playing.store(true, Ordering::SeqCst);
    }
}

pub(super) async fn sample_progress(ctx: &EngineContext) -> (i32, i32) {
    let Ok(control) = ctx.progress_control.try_lock() else {
        return (-1, -1);
    };
    let control_revision = *control;
    let Ok(playback) = ctx.playback_lock.try_lock() else {
        return (-1, -1);
    };
    let gate = ctx.playlist_manager.progress_gate.lock().await;
    if ctx.stopped.load(Ordering::SeqCst) {
        return (-1, -1);
    }
    if gate.song().is_none() {
        return (-1, -1);
    }
    let revision = gate.revision();
    let hash = ctx.playlist_manager.current_hash().await;
    let playing = ctx.playlist_manager.get_song_playing().await;
    let cached_total = match playing.as_ref() {
        Some(song) => *ctx.duration_cache.lock().await.get(song).unwrap_or(&0),
        None => 0,
    };
    drop(gate);
    drop(playback);
    drop(control);

    let snapshot = ctx.caster.playback_snapshot();
    let progress = match snapshot {
        Some(state) => Ok(state.progress),
        None => ctx.caster.get_progress().await,
    };
    let (current, total) = match progress {
        Ok(p) => {
            trace!(
                "progress: curr={} device_total={} cached_total={} playing={:?}",
                p.current_secs, p.total_secs, cached_total, playing
            );
            let total = if snapshot.is_some() && p.total_secs > 0 {
                p.total_secs
            } else if cached_total > 0 {
                cached_total
            } else {
                p.total_secs
            } as i32;
            (p.current_secs as i32, total)
        }
        Err(_) => (-1, -1),
    };
    let Ok(control) = ctx.progress_control.try_lock() else {
        return (-1, -1);
    };
    if *control != control_revision {
        return (-1, -1);
    }
    let Ok(_playback) = ctx.playback_lock.try_lock() else {
        return (-1, -1);
    };
    let mut gate = ctx.playlist_manager.progress_gate.lock().await;
    if ctx.stopped.load(Ordering::SeqCst) {
        return (-1, -1);
    }
    let (current, total) = gate.filter(revision, current, total);
    // Validate the sample and consume once under the same lock as song updates
    // and play confirmations. An old poll cannot trigger after a new playback.
    // 保留提前切歌的时间余量；本地暂停、缓冲和失联时不按进度推进。
    let should_advance = if snapshot.is_some_and(|state| state.ended == Some(true)) {
        current >= 0 && total > 0 && gate.auto_next.observe_finished()
    } else if ctx.caster.kind() == cast::CasterKind::Local
        && !snapshot.is_some_and(|state| state.is_playing)
    {
        false
    } else {
        gate.auto_next.observe(current, total)
    };
    if should_advance {
        info!("歌曲即将结束，启动一次自动切歌任务");
        gate.auto_next
            .start_request(ctx.playlist_manager.clone(), hash, &ctx.rt);
    }
    (current, total)
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

pub(super) async fn prepare_duration(song: &str, cache: &Mutex<std::collections::HashMap<String, u32>>) {
    if let Some((bvid, page)) = extract_bvid(song) {
        if let Ok((_, duration)) = crate::sources::bilibili::get_page_info(&bvid, page).await {
            if duration > 0 {
                cache.lock().await.insert(song.to_owned(), duration);
                info!(target: "DLNA1080", "播放前预填充 BV 视频时长: {},p{} -> {}s", bvid, page+1, duration);
            }
        }
    }
}

pub(super) async fn play_and_confirm(
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
            if caster.kind() == cast::CasterKind::Local {
                gate.lock().await.play_failed(token);
            }
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
    let ctx = {
        let guard = ENGINE_STATE.read().ok();
        guard.and_then(|g| g.as_ref().cloned())
    };
    if let Some(ctx) = ctx {
        if let Some(playing) = ctx.playlist_manager.get_song_playing().await {
            if let Some(&d) = ctx.duration_cache.lock().await.get(&playing) {
                return d;
            }
        }
    }
    0
}

#[cfg(test)]
mod tests;
