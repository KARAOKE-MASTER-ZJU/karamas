use super::PlaylistManager;
use serde_json::json;

#[derive(Debug)]
pub(crate) enum SwitchSongError {
    Retryable(String),
    Rejected(String),
}

impl std::fmt::Display for SwitchSongError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Retryable(message) | Self::Rejected(message) => f.write_str(message),
        }
    }
}

impl PlaylistManager {
    /// Read song and hash from one authoritative room response, without consuming
    /// the sync loop's hash or suppressing its playback callback.
    pub(crate) async fn current_song_snapshot(
        &self,
    ) -> Result<crate::playlist::parts::CurrentSongSnapshot, crate::playlist::parts::PartError> {
        use crate::playlist::parts::{CurrentSongSnapshot, PartError};
        let json: serde_json::Value = self
            .client
            .get(format!(
                "{}/api/songListInfo",
                self.url.trim_end_matches('/')
            ))
            .query(&[("roomId", self.room_id.as_str())])
            .send()
            .await
            .map_err(|e| PartError::new("ROOM_ERROR", e.to_string()))?
            .error_for_status()
            .map_err(|e| PartError::new("ROOM_ERROR", e.to_string()))?
            .json()
            .await
            .map_err(|e| PartError::new("ROOM_ERROR", e.to_string()))?;
        let song = &json["list"]["singing"];
        if json["changed"].as_bool() != Some(true) || !json["list"].is_object() {
            return Err(PartError::new("ROOM_ERROR", "无效的房间响应"));
        }
        if song.is_null() {
            return Err(PartError::new("NO_CURRENT_SONG", "当前没有歌曲"));
        }
        let field = |value: &serde_json::Value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| PartError::new("ROOM_ERROR", "房间歌曲字段不完整"))
        };
        Ok(CurrentSongSnapshot {
            song_id: field(&song["id"])?,
            song_url: field(&song["url"])?,
            song_title: field(&song["title"])?,
            room_hash: field(&json["hash"])?,
        })
    }

    pub(crate) async fn edit_current_part(
        &self,
        song: &crate::playlist::parts::CurrentSongSnapshot,
        target_url: &str,
        unchanged: bool,
    ) -> Result<bool, crate::playlist::parts::PartError> {
        use crate::playlist::parts::PartError;
        // The room API atomically checks song ID and hash before publishing
        // the edit. A second client-side read would not close that race.
        if unchanged {
            return Ok(false);
        }
        let response = self
            .client
            .post(format!(
                "{}/api/editCurrentSong",
                self.url.trim_end_matches('/')
            ))
            .query(&[("roomId", &self.room_id)])
            .json(&json!({"idArrayHash": song.room_hash, "song": {
                "id": song.song_id, "title": song.song_title, "url": target_url,
            }}))
            .send()
            .await
            .map_err(|e| {
                PartError::new("RESULT_UNKNOWN", format!("提交结果未知，请刷新确认：{e}"))
            })?;
        let status = response.status();
        if !status.is_success() {
            return Err(PartError::new(
                if status.is_server_error() {
                    "RESULT_UNKNOWN"
                } else {
                    "ROOM_ERROR"
                },
                format!("修改分 P 请求失败：HTTP {status}，请刷新确认"),
            ));
        }
        let json: serde_json::Value = response.json().await.map_err(|e| {
            PartError::new("RESULT_UNKNOWN", format!("提交结果未知，请刷新确认：{e}"))
        })?;
        if json["success"].as_bool() == Some(true) {
            Ok(true)
        } else if json["success"].as_bool() == Some(false) {
            Err(PartError::new(
                if json["code"] == "REJECT" {
                    "STALE_CONTEXT"
                } else {
                    "ROOM_ERROR"
                },
                json["msg"].as_str().unwrap_or(if json["code"] == "REJECT" {
                    "歌单已变化，请重试"
                } else {
                    "修改被拒绝，请重试"
                }),
            ))
        } else {
            Err(PartError::new("RESULT_UNKNOWN", "无效的提交响应，请刷新确认"))
        }
    }

    pub async fn next_song(&mut self) -> Result<(), String> {
        self.switch_song(true).await
    }

    pub async fn prev_song(&mut self) -> Result<(), String> {
        self.switch_song(false).await
    }

    pub async fn switch_song(&mut self, next: bool) -> Result<(), String> {
        let hash = self.current_hash().await;
        self.switch_song_with_hash(next, &hash).await.map_err(|e| e.to_string())
    }

    pub(crate) async fn current_hash(&self) -> String {
        self.hash.lock().await.clone().unwrap_or_else(|| "EMPTY_LIST_HASH".into())
    }

    pub(crate) async fn switch_song_with_hash(
        &self,
        next: bool,
        hash: &str,
    ) -> Result<(), SwitchSongError> {
        Self::send_switch_request(self.switch_request(next, hash)).await
    }

    pub(crate) fn switch_request(&self, next: bool, hash: &str) -> reqwest::RequestBuilder {
        let url = format!("{}/api/{}Song?roomId={}", self.url, if next {"next"} else {"prev"}, self.room_id);
        self.client.post(url).json(&json!({"idArrayHash": hash}))
    }

    pub(crate) async fn send_switch_request(request: reqwest::RequestBuilder) -> Result<(), SwitchSongError> {
        let resp = request
            .send()
            .await
            .map_err(|e| SwitchSongError::Retryable(format!("发送请求失败: {}", e)))?;
        let status = resp.status();
        if status.is_server_error() || status.as_u16() == 408 || status.as_u16() == 429 {
            return Err(SwitchSongError::Retryable(format!("请求失败: HTTP {}", status)));
        }
        if !status.is_success() {
            return Err(SwitchSongError::Rejected(format!("请求失败: HTTP {}", status)));
        }
        let resp_json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| SwitchSongError::Retryable(format!("解析JSON失败: {}", e)))?;

        if !resp_json["success"].as_bool().unwrap_or(false) {
            return Err(SwitchSongError::Rejected(format!("请求失败: {}", resp_json)));
        }

        Ok(())
    }
}
