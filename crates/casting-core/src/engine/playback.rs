use super::state::{ENGINE_STATE, EngineContext};
use crate::{cast, playlist::progress_gate};
use log::{debug, info, warn};
use std::{sync::atomic::Ordering, time::Duration};
use tokio::sync::Mutex;

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
    if ctx.stopped.load(Ordering::SeqCst) {
        return (-1, -1);
    }
    let (current, total) = gate.filter(revision, current, total);
    // Validate the sample and consume once under the same lock as song updates
    // and play confirmations. An old poll cannot trigger after a new playback.
    if trigger_next && gate.auto_next.observe(current, total) {
        info!("歌曲即将结束，启动一次自动切歌任务");
        gate.auto_next.start_request(ctx.playlist_manager.clone(), hash, &ctx.rt);
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
