use crate::{
    error::{CommandError, CommandResult},
    models::{PlaybackQuality, PlaybackState, RecastResult},
    state::current_engine,
};
use std::sync::Arc;

/// 读取核心库最近的播放状态；未连接时返回 null。
#[tauri::command]
pub async fn poll_playback() -> CommandResult<Option<PlaybackState>> {
    let ctx = match current_engine() {
        Ok(ctx) => ctx,
        Err(error) if error.code == "NOT_CONNECTED" => return Ok(None),
        Err(error) => return Err(error),
    };
    let playback = ctx.playback_state().await;
    let result = PlaybackState {
        song_id: playback.song_id,
        song_url: playback.song_url,
        title: playback.title,
        added_by: playback.added_by,
        playlist_sync_error: playback.playlist_sync_error,
        current_secs: playback.current_secs,
        total_secs: playback.total_secs,
        is_playing: playback.is_playing,
        queued_count: playback.queued_count,
        sung_count: playback.sung_count,
        quality_qn: playback.quality_qn,
        quality_format: playback.quality_format,
        danmaku: playback.danmaku,
        capabilities: playback.capabilities.into(),
    };
    // 重连期间不把旧会话的结果返回给界面。
    if !current_engine().is_ok_and(|current| Arc::ptr_eq(&current, &ctx)) {
        return Ok(None);
    }
    Ok(Some(result))
}

/// 恢复播放，成功后返回 true。
#[tauri::command]
pub async fn play() -> CommandResult<bool> {
    Ok(karamas_core::play_core().await?)
}

/// 暂停播放，成功后返回 false。
#[tauri::command]
pub async fn pause() -> CommandResult<bool> {
    Ok(karamas_core::pause_core().await?)
}

/// 切换播放状态，返回是否正在播放。
#[tauri::command]
pub async fn toggle_pause() -> CommandResult<bool> {
    Ok(karamas_core::toggle_pause_core().await?)
}

/// 等待点歌台确认切换下一首的请求。
#[tauri::command]
pub async fn next_song() -> CommandResult<()> {
    Ok(karamas_core::trigger_next_song().await?)
}

/// 等待点歌台确认切换上一首的请求。
#[tauri::command]
pub async fn previous_song() -> CommandResult<()> {
    Ok(karamas_core::trigger_prev_song().await?)
}

/// 重新投屏当前曲目，并等待核心库返回结果。
#[tauri::command]
pub async fn recast_current_song() -> CommandResult<RecastResult> {
    let ctx = current_engine()?;
    match karamas_core::recast_current_song(&ctx).await {
        1 => Ok(RecastResult::Played),
        0 => Ok(RecastResult::NoCurrentSong),
        -2 => Ok(RecastResult::Busy),
        _ => Err(CommandError::new(
            "PLAYBACK_FAILED",
            "重新投屏失败或超时，请查看日志",
        )),
    }
}

/// 跳转到指定秒数。
#[tauri::command]
pub async fn seek(seconds: u32) -> CommandResult<()> {
    Ok(karamas_core::jump_to_secs(seconds).await?)
}

/// 设置绝对音量，返回设置后的值。
#[tauri::command]
pub async fn set_volume(volume: u32) -> CommandResult<u32> {
    Ok(karamas_core::set_volume_core(volume).await?)
}

/// 读取支持绝对音量的设备音量。
#[tauri::command]
pub async fn get_volume() -> CommandResult<u32> {
    Ok(karamas_core::get_volume_core().await?)
}

/// 调高或调低音量，步长限定在有效音量范围内。
#[tauri::command]
pub async fn change_volume(step: u32, increase: bool) -> CommandResult<()> {
    if step > 100 {
        return Err(CommandError::new(
            "INVALID_ARGUMENT",
            "音量步长必须在 0 到 100 之间",
        ));
    }
    if increase {
        karamas_core::volume_up_core(step).await?;
    } else {
        karamas_core::volume_down_core(step).await?;
    }
    Ok(())
}

/// 设置设备弹幕状态。
#[tauri::command]
pub async fn set_danmaku(enabled: bool) -> CommandResult<bool> {
    Ok(karamas_core::set_danmaku_core(enabled).await?)
}

/// 校验协议清晰度数值后调用核心库。
#[tauri::command]
pub async fn set_quality(
    qn: u32,
    format: Option<karamas_core::cast::MediaFormat>,
) -> CommandResult<PlaybackQuality> {
    let quality = karamas_core::cast::Quality::from_qn(qn)
        .ok_or_else(|| CommandError::new("INVALID_ARGUMENT", "不支持的清晰度"))?;
    let (quality, format) =
        karamas_core::set_media_quality_core(quality, format.unwrap_or_default()).await?;
    Ok(PlaybackQuality {
        qn: quality.as_qn(),
        format,
    })
}

/// 查询指定 B站视频分 P 的清晰度列表。
#[tauri::command]
pub async fn get_bilibili_qualities(
    song_url: String,
) -> CommandResult<Vec<karamas_core::sources::bilibili::BilibiliQuality>> {
    karamas_core::sources::bilibili::get_bilibili_qualities(&song_url)
        .await
        .map_err(|message| CommandError::new("UPSTREAM_ERROR", message))
}

/// 查询 DLNA 和本机播放可用的 MP4、DASH 清晰度。
#[tauri::command]
pub async fn get_bilibili_media_qualities(
    song_url: String,
) -> CommandResult<Vec<karamas_core::sources::bilibili::BilibiliMediaQuality>> {
    karamas_core::sources::bilibili::get_bilibili_media_qualities(&song_url)
        .await
        .map_err(|message| CommandError::new("UPSTREAM_ERROR", message))
}

/// 查询指定 B站视频的默认封面。
#[tauri::command]
pub async fn get_bilibili_cover(song_url: String) -> CommandResult<String> {
    karamas_core::sources::bilibili::get_bilibili_cover(&song_url)
        .await
        .map_err(|message| CommandError::new("UPSTREAM_ERROR", message))
}

/// 查询当前歌曲可选的 B站分 P。
#[tauri::command]
pub async fn get_current_bilibili_parts() -> CommandResult<karamas_core::CurrentBilibiliParts> {
    Ok(karamas_core::get_current_bilibili_parts_core().await?)
}

/// 提交分 P 修改，返回是否已提交。
#[tauri::command]
pub async fn select_current_bilibili_part(song_id: String, page_index: u32) -> CommandResult<bool> {
    Ok(karamas_core::select_current_bilibili_part_core(&song_id, page_index).await?)
}

/// 回传本机播放器的操作确认与真实进度，旧会话消息由核心丢弃。
#[tauri::command]
pub async fn report_local_playback(
    report: karamas_core::cast::local::LocalReport,
) -> CommandResult<bool> {
    Ok(current_engine()?.report_local_playback(report).await?)
}
