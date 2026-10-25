use crate::{
    playlist::{PlaylistManager, parts::PartError},
    sources::bilibili::{self, BilibiliPart},
};
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentBilibiliParts {
    pub song_id: String,
    pub song_url: String,
    pub bvid: String,
    pub current_page_index: u32,
    pub parts: Vec<BilibiliPart>,
}

fn manager() -> Result<PlaylistManager, PartError> {
    crate::engine::ENGINE_STATE
        .read()
        .ok()
        .and_then(|state| state.as_ref().map(|ctx| ctx.playlist_manager.clone()))
        .ok_or_else(|| PartError::new("NOT_CONNECTED", "尚未连接房间"))
}

pub async fn get_current_bilibili_parts_core() -> Result<CurrentBilibiliParts, PartError> {
    let manager = manager()?;
    let song = manager.current_song_snapshot().await?;
    let (bvid, current_page_index) = bilibili::parse_bilibili_video(&song.song_url)
        .map_err(|e| PartError::new("UNSUPPORTED", e))?;
    let parts = bilibili::get_bilibili_parts(&bvid)
        .await
        .map_err(|e| PartError::new("UPSTREAM_ERROR", e))?;
    if parts.get(current_page_index as usize).is_none() {
        return Err(PartError::new("INVALID_PART", "当前分 P 已不存在"));
    }
    Ok(CurrentBilibiliParts {
        song_id: song.song_id,
        // 与播放状态使用同一地址格式，供前端校验返回结果所属的分 P。
        song_url: format!("{bvid}-page{current_page_index}"),
        bvid,
        current_page_index,
        parts,
    })
}

/// Returns true when submitted, false when already on the selected part.
pub async fn select_current_bilibili_part_core(
    song_id: &str,
    page_index: u32,
) -> Result<bool, PartError> {
    let manager = manager()?;
    let song = manager.current_song_snapshot().await?;
    if song_id != song.song_id {
        return Err(PartError::new(
            "STALE_CONTEXT",
            "当前歌曲已切换，请重新选择分 P",
        ));
    }
    let (bvid, current) = bilibili::parse_bilibili_video(&song.song_url)
        .map_err(|e| PartError::new("UNSUPPORTED", e))?;
    let parts = bilibili::get_bilibili_parts(&bvid)
        .await
        .map_err(|e| PartError::new("UPSTREAM_ERROR", e))?;
    if parts.get(page_index as usize).is_none() {
        return Err(PartError::new("INVALID_PART", "分 P 超出范围"));
    }
    let url = format!("bilibili://video/{bvid}?page={page_index}");
    manager
        .edit_current_part(&song, &url, page_index == current)
        .await
}
