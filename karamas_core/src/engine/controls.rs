use super::state::{ENGINE_STATE, EngineContext};
use crate::{cast, error::CoreError};
use std::sync::atomic::{AtomicBool, Ordering};

/// 等待点歌台确认下一首请求，设备播放由后续歌单同步处理。
pub async fn trigger_next_song() -> Result<(), CoreError> {
    switch_song(true).await
}

/// 等待点歌台确认上一首请求，设备播放由后续歌单同步处理。
pub async fn trigger_prev_song() -> Result<(), CoreError> {
    switch_song(false).await
}

async fn switch_song(next: bool) -> Result<(), CoreError> {
    let ctx = {
        let guard = ENGINE_STATE.read().map_err(|_| "Lock error")?;
        guard.as_ref().cloned().ok_or("Engine not initialized")?
    };
    let _control = ctx.begin_progress_control().await?;
    let mut pm = ctx.playlist_manager.clone();
    ctx.spawn_task(async move { pm.switch_song(next).await })?
        .await??;
    Ok(())
}

/// 跳转到指定时间点，单位秒
pub async fn jump_to_secs(target_secs: u32) -> Result<(), CoreError> {
    let ctx = {
        let guard = ENGINE_STATE.read().map_err(|_| "Lock error")?;
        guard.as_ref().cloned().ok_or("Engine not initialized")?
    };
    seek(&ctx, target_secs).await
}

pub(super) async fn seek(ctx: &EngineContext, target_secs: u32) -> Result<(), CoreError> {
    let _control = ctx.begin_progress_control().await?;
    let revision = ctx.playlist_manager.progress_gate.lock().await.revision();
    ctx.caster.seek(target_secs).await?;
    ctx.playlist_manager
        .progress_gate
        .lock()
        .await
        .seek_succeeded(revision, target_secs);
    Ok(())
}

/// 恢复播放，成功后返回 true。
pub async fn play_core() -> Result<bool, CoreError> {
    set_playing_core(true).await
}

/// 暂停播放，成功后返回 false。
pub async fn pause_core() -> Result<bool, CoreError> {
    set_playing_core(false).await
}

async fn set_playing_core(playing: bool) -> Result<bool, CoreError> {
    let ctx = {
        let guard = ENGINE_STATE.read().map_err(|_| "Lock error")?;
        guard.as_ref().cloned().ok_or("Engine not initialized")?
    };
    let _playback = ctx.playback_lock.lock().await;
    let _control = ctx.begin_progress_control().await?;
    let result = apply_playback_state(ctx.caster.as_ref(), &ctx.is_playing, playing).await?;
    if playing && ctx.caster.kind() == cast::CasterKind::Local {
        ctx.playlist_manager
            .progress_gate
            .lock()
            .await
            .confirm_resume();
    }
    Ok(result)
}

// 调用方持有 playback_lock；本地状态可能过期，始终向设备发送明确命令。
async fn apply_playback_state(
    caster: &dyn cast::Caster,
    is_playing: &AtomicBool,
    playing: bool,
) -> Result<bool, CoreError> {
    if playing {
        caster.resume().await?;
    } else {
        caster.pause().await?;
    }
    is_playing.store(playing, Ordering::SeqCst);
    Ok(playing)
}

/// 切换播放/暂停状态，返回切换后的状态
pub async fn toggle_pause_core() -> Result<bool, CoreError> {
    let ctx = {
        let guard = ENGINE_STATE.read().map_err(|_| "Lock error")?;
        guard.as_ref().cloned().ok_or("Engine not initialized")?
    };
    toggle_pause(&ctx).await
}

pub(super) async fn toggle_pause(ctx: &EngineContext) -> Result<bool, CoreError> {
    // 与新曲播放串行处理，避免播放确认覆盖刚完成的暂停状态。
    let _playback = ctx.playback_lock.lock().await;
    let target_state = !ctx.is_playing();
    let _control = ctx.begin_progress_control().await?;

    let result = apply_playback_state(ctx.caster.as_ref(), &ctx.is_playing, target_state).await?;
    if target_state && ctx.caster.kind() == cast::CasterKind::Local {
        ctx.playlist_manager
            .progress_gate
            .lock()
            .await
            .confirm_resume();
    }
    Ok(result)
}

/// 设置音量，返回设置后的值
pub async fn set_volume_core(volume: u32) -> Result<u32, CoreError> {
    let ctx = {
        let guard = ENGINE_STATE.read().map_err(|_| "Lock error")?;
        guard.as_ref().cloned().ok_or("Engine not initialized")?
    };
    let target = volume.clamp(0, 100);
    ctx.caster.set_volume(target).await?;
    Ok(target)
}

/// 获取当前音量，返回值范围 0~100；如果设备不支持获取音量，则返回 0。
pub async fn get_volume_core() -> Result<u32, CoreError> {
    let ctx = {
        let guard = ENGINE_STATE.read().map_err(|_| "Lock error")?;
        guard.as_ref().cloned().ok_or("Engine not initialized")?
    };
    let v = ctx.caster.get_volume().await?;
    Ok(v.unwrap_or(0))
}

/// 相对调高音量，返回设置后的值
pub async fn volume_up_core(step: u32) -> Result<(), CoreError> {
    let ctx = {
        let guard = ENGINE_STATE.read().map_err(|_| "Lock error")?;
        guard.as_ref().cloned().ok_or("Engine not initialized")?
    };
    ctx.caster.volume_up(step).await?;
    Ok(())
}

/// 相对调低音量，返回设置后的值
pub async fn volume_down_core(step: u32) -> Result<(), CoreError> {
    let ctx = {
        let guard = ENGINE_STATE.read().map_err(|_| "Lock error")?;
        guard.as_ref().cloned().ok_or("Engine not initialized")?
    };
    ctx.caster.volume_down(step).await?;
    Ok(())
}

/// 直接设置弹幕开关，返回设置后的状态。
pub async fn set_danmaku_core(on: bool) -> Result<bool, CoreError> {
    let ctx = {
        let guard = ENGINE_STATE.read().map_err(|_| "Lock error")?;
        guard.as_ref().cloned().ok_or("Engine not initialized")?
    };
    ctx.caster.set_danmaku(on).await?;
    // ctx.danmaku_on.store(on, Ordering::SeqCst);
    Ok(on)
}

/// 切换弹幕开关，返回切换后的状态。
pub async fn toggle_danmaku_core() -> Result<bool, CoreError> {
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
pub async fn set_quality_core(quality: cast::Quality) -> Result<cast::Quality, CoreError> {
    let ctx = {
        let guard = ENGINE_STATE.read().map_err(|_| "Lock error")?;
        guard.as_ref().cloned().ok_or("Engine not initialized")?
    };
    ctx.caster.set_quality(quality).await?;
    // ctx.quality_qn.store(quality.as_qn(), Ordering::SeqCst);
    Ok(quality)
}

/// 同时切换清晰度和取源方式，并返回生效后的选项。
pub async fn set_media_quality_core(
    quality: cast::Quality,
    format: cast::MediaFormat,
) -> Result<(cast::Quality, cast::MediaFormat), CoreError> {
    let ctx = {
        let guard = ENGINE_STATE.read().map_err(|_| "Lock error")?;
        guard.as_ref().cloned().ok_or("Engine not initialized")?
    };
    ctx.caster.set_media_quality(quality, format).await?;
    Ok((ctx.caster.get_quality()?, ctx.caster.get_media_format()))
}

/// 切到下一档清晰度（循环），返回切换后的值。
pub async fn cycle_quality_core() -> Result<cast::Quality, CoreError> {
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

/// 获取当前歌曲id，若未连接或无歌曲则返回空字符串。
pub async fn get_current_song_id_core() -> String {
    let ctx = ENGINE_STATE
        .read()
        .ok()
        .and_then(|state| state.as_ref().cloned());
    match ctx {
        Some(ctx) => ctx.playlist_manager.get_song_id().await.unwrap_or_default(),
        None => String::new(),
    }
}

/// 获取当前歌曲标题，若未连接或无歌曲则返回 "暂无歌曲"。
pub async fn get_current_song_title_core() -> String {
    let ctx = {
        let guard = ENGINE_STATE.read().ok();
        guard.and_then(|g| g.as_ref().cloned())
    };
    if let Some(ctx) = ctx {
        return ctx
            .playlist_manager
            .get_song_title()
            .await
            .unwrap_or_else(|| "暂无歌曲".to_string());
    }
    "未连接".to_string()
}
