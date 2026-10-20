//! Current-song part selection. The room remains the source of truth.
use crate::{
    bilibili_parser::{self, BilibiliPart},
    playlist_manager::PlaylistManager,
};
use serde::Serialize;

#[derive(Debug)]
pub(crate) struct CurrentSongSnapshot {
    pub song_id: String,
    pub song_url: String,
    pub song_title: String,
    pub room_hash: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentBilibiliParts {
    pub song_id: String,
    pub bvid: String,
    pub current_page_index: u32,
    pub parts: Vec<BilibiliPart>,
}

#[derive(Debug, Serialize)]
pub struct PartError {
    pub code: &'static str,
    pub message: String,
}

impl PartError {
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for PartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for PartError {}

fn manager() -> Result<PlaylistManager, PartError> {
    crate::ENGINE_STATE
        .read()
        .ok()
        .and_then(|state| state.as_ref().map(|ctx| ctx.playlist_manager.clone()))
        .ok_or_else(|| PartError::new("NOT_CONNECTED", "尚未连接房间"))
}

pub async fn get_current_bilibili_parts_core() -> Result<CurrentBilibiliParts, PartError> {
    let manager = manager()?;
    let song = manager.current_song_snapshot().await?;
    let (bvid, current_page_index) = bilibili_parser::parse_bilibili_video(&song.song_url)
        .map_err(|e| PartError::new("UNSUPPORTED", e))?;
    let parts = bilibili_parser::get_bilibili_parts(&bvid)
        .await
        .map_err(|e| PartError::new("UPSTREAM_ERROR", e))?;
    if parts.get(current_page_index as usize).is_none() {
        return Err(PartError::new("INVALID_PART", "当前分 P 已不存在"));
    }
    Ok(CurrentBilibiliParts {
        song_id: song.song_id,
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
    let (bvid, current) = bilibili_parser::parse_bilibili_video(&song.song_url)
        .map_err(|e| PartError::new("UNSUPPORTED", e))?;
    let parts = bilibili_parser::get_bilibili_parts(&bvid)
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
