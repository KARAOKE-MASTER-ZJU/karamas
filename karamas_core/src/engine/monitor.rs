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
    pub added_by: Option<String>,
    pub playlist_sync_error: Option<String>,
    pub current_secs: Option<u32>,
    pub total_secs: Option<u32>,
    pub is_playing: bool,
    pub queued_count: usize,
    pub sung_count: usize,
    pub quality_qn: Option<u32>,
    pub quality_format: cast::MediaFormat,
    pub danmaku: Option<bool>,
    pub capabilities: cast::Capabilities,
}

impl EngineContext {
    /// 原生控件的状态回传与进度采样串行，防止旧的末尾进度触发切歌。
    pub async fn report_local_playback(
        &self,
        report: cast::local::LocalReport,
    ) -> Result<bool, CoreError> {
        // 控制命令可能持有 progress_control 等待确认，确认回包不能再等待同一把锁。
        if report.acknowledged {
            return Ok(self.caster.report_local_playback(report)?);
        }
        let mut control = self.progress_control.lock().await;
        if self.stopped.load(Ordering::SeqCst) {
            return Ok(false);
        }
        let position = report.current_secs as u32;
        let settled = !report.seeking && report.error.is_none();
        let playing = !report.paused && !report.ended && report.error.is_none();
        if !self.caster.report_local_playback(report)? {
            return Ok(false);
        }
        *control += 1;
        self.is_playing.store(playing, Ordering::SeqCst);
        let mut gate = self.playlist_manager.progress_gate.lock().await;
        if playing && settled {
            gate.confirm_resume();
        }
        if settled {
            let revision = gate.revision();
            gate.seek_succeeded(revision, position);
        }
        Ok(true)
    }

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
            added_by: self.playlist_manager.get_song_added_by().await,
            playlist_sync_error: self.playlist_manager.get_sync_error().await,
            current_secs: u32::try_from(current).ok(),
            total_secs: u32::try_from(total).ok(),
            is_playing: self.is_playing(),
            queued_count: self.playlist_manager.get_queued_count().await,
            sung_count: self.playlist_manager.get_sung_count().await,
            quality_qn: self.caster.get_quality().ok().map(|q| q.as_qn()),
            quality_format: self.caster.get_media_format(),
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
