use super::{playback::sample_progress, state::EngineContext};
use crate::{cast, error::CoreError};
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

/// 当前会话的播放状态。读取时不查询设备，也不触发切歌。
pub struct EnginePlaybackState {
    pub song_id: Option<String>,
    pub song_url: Option<String>,
    pub title: Option<String>,
    pub current_secs: Option<u32>,
    pub total_secs: Option<u32>,
    pub is_playing: bool,
    pub queued_count: usize,
    pub sung_count: usize,
    pub quality_qn: Option<u32>,
    pub danmaku: Option<bool>,
    pub capabilities: cast::Capabilities,
}

impl EngineContext {
    pub async fn current_progress(&self) -> (i32, i32) {
        let gate = self.playlist_manager.progress_gate.lock().await;
        if self.stopped.load(Ordering::SeqCst) {
            (-1, -1)
        } else {
            gate.cached_progress()
        }
    }

    pub async fn playback_state(&self) -> EnginePlaybackState {
        // 与歌单更新使用同一把锁，避免新歌信息配上上一首的进度。
        let gate = self.playlist_manager.progress_gate.lock().await;
        let (current, total) = if self.stopped.load(Ordering::SeqCst) {
            (-1, -1)
        } else {
            gate.cached_progress()
        };
        let song = gate.song();
        EnginePlaybackState {
            song_id: song.as_ref().and_then(|(id, _)| id.clone()),
            song_url: song.map(|(_, url)| url),
            title: self.playlist_manager.get_song_title().await,
            current_secs: u32::try_from(current).ok(),
            total_secs: u32::try_from(total).ok(),
            is_playing: self.is_playing(),
            queued_count: self.playlist_manager.get_queued_count().await,
            sung_count: self.playlist_manager.get_sung_count().await,
            quality_qn: self.caster.get_quality().ok().map(|q| q.as_qn()),
            danmaku: self.caster.get_danmaku().ok(),
            capabilities: self.caster.capabilities(),
        }
    }
}

/// 与房间同步一起由会话持有，宿主无须定期调用进度接口。
pub(super) fn start_monitor(ctx: &Arc<EngineContext>) -> Result<(), CoreError> {
    let session = Arc::downgrade(ctx);
    ctx.spawn_task(async move {
        let mut timer = tokio::time::interval(Duration::from_secs(1));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            timer.tick().await;
            let Some(ctx) = session.upgrade() else { break };
            if ctx.stopped.load(Ordering::SeqCst) {
                break;
            }
            sample_progress(&ctx).await;
        }
    })?;
    Ok(())
}
