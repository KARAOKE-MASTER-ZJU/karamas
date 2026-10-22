use crate::playlist::progress_gate::ProgressGate;
use log::{debug, info};
use reqwest::Client;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct PlaylistManager {
    pub(super) url: String,
    pub(super) room_id: String,
    pub(super) client: Client,
    pub(super) hash: Arc<Mutex<Option<String>>>,
    pub(super) song_playing: Arc<Mutex<Option<String>>>,
    pub(super) song_title: Arc<Mutex<Option<String>>>,
    pub(super) queued_count: Arc<Mutex<usize>>,
    pub(super) sung_count: Arc<Mutex<usize>>,
    pub(crate) progress_gate: Arc<Mutex<ProgressGate>>,
}

impl PlaylistManager {
    /// 创建指定房间的歌单管理器。
    pub fn new(url: &str, room_id: String) -> Self {
        // 在初始化时构建一次 Client
        let client = Client::builder()
            .use_rustls_tls()
            .timeout(crate::config::API_TIMEOUT)
            .build()
            .expect("构建 HTTP 客户端失败");
        Self {
            url: url.to_string(),
            room_id,
            client,
            hash: Arc::new(Mutex::new(None)),
            song_playing: Arc::new(Mutex::new(None)),
            song_title: Arc::new(Mutex::new(None)),
            queued_count: Arc::new(Mutex::new(0)),
            sung_count: Arc::new(Mutex::new(0)),
            progress_gate: Arc::new(Mutex::new(ProgressGate::default())),
        }
    }

    // 适配新的返回结构：
    // {
    //   changed: boolean,
    //   list: { queued: Song[]; singing: Song; sung: Song[] },
    //   hash: string
    // }
    // Song { id, title, url, addedBy? }
    /// 同步房间歌单，返回当前歌曲的 ID 和地址。
    pub(super) async fn fetch_playlist(&self) -> Result<Option<(Option<String>, String)>, String> {
        let last_hash = self
            .hash
            .lock()
            .await
            .clone()
            .unwrap_or_else(|| "EMPTY_LIST_HASH".into());

        let url = format!(
            "{}/api/songListInfo?roomId={}&lastHash={}",
            self.url, self.room_id, last_hash
        );

        debug!("正在获取播放列表: {}", url);

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("发送请求失败: {}", e))?;
        if !resp.status().is_success() {
            return Err(format!("请求失败，状态码: {}", resp.status()));
        }

        let resp_json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("解析JSON失败: {}", e))?;
        let changed: bool = resp_json["changed"].as_bool().unwrap_or(false);

        if !changed {
            debug!("播放列表未改变，跳过更新");
            return Ok(self.progress_gate.lock().await.song());
        }

        // 获取新的 hash 值
        let new_hash = resp_json["hash"]
            .as_str()
            .unwrap_or("EMPTY_LIST_HASH")
            .to_string();

        let extract_bv_function = |url: &str| {
            if let Ok((bvid, page)) = crate::sources::bilibili::parse_bilibili_video(url) {
                return format!("{bvid}-page{page}");
            }

            // 提取 bilibili://video/ 后面的部分
            if let Some(start) = url.find("bilibili://video/") {
                let after_prefix = &url[start + "bilibili://video/".len()..];
                after_prefix.to_string().replace("?", "-").replace("=", "") // 保持原有 bilibili 逻辑
            } else if url.starts_with("http") {
                // 如果是直接链接（如 eplus），不做任何替换，保持完整参数
                url.to_string()
            } else {
                url.to_string().replace("?", "-").replace("=", "")
            }
        };
        let singing_title: Option<String> = resp_json["list"]["singing"]["title"]
            .as_str()
            .map(|s| s.to_string());

        // 当前正在演唱的歌曲：list.singing.url
        let singing_url: Option<String> = resp_json["list"]["singing"]["url"]
            .as_str()
            .map(extract_bv_function);

        info!("新的hash: {}", new_hash);

        // 获取队列中还有多少首歌未播和多少歌已播
        let queued_count = resp_json["list"]["queued"].as_array().map_or(0, Vec::len);
        let sung_count = resp_json["list"]["sung"].as_array().map_or(0, Vec::len);

        // Mask old renderer progress before publishing the new song/hash. All
        // sync paths (including heartbeat) apply this same guard.
        let singing_id = resp_json["list"]["singing"]["id"]
            .as_str().map(str::to_owned);
        let mut gate = self.progress_gate.lock().await;
        gate.update_song(singing_id, singing_url.clone());
        // 更新状态
        *self.song_playing.lock().await = singing_url.clone();
        *self.song_title.lock().await = singing_title; // 更新标题
        *self.hash.lock().await = Some(new_hash);
        *self.queued_count.lock().await = queued_count;
        *self.sung_count.lock().await = sung_count;

        Ok(gate.song())
    }

    /// 读取最近同步的当前歌曲地址。
    pub async fn get_song_playing(&self) -> Option<String> {
        self.song_playing.lock().await.clone()
    }

    /// 读取最近同步的当前歌曲 ID。
    pub async fn get_song_id(&self) -> Option<String> {
        self.progress_gate.lock().await.song().and_then(|(id, _)| id)
    }

    /// 读取最近同步的当前歌曲标题。
    pub async fn get_song_title(&self) -> Option<String> {
        self.song_title.lock().await.clone()
    }

    /// 读取最近同步的待播歌曲数量。
    pub async fn get_queued_count(&self) -> usize {
        *self.queued_count.lock().await
    }

    /// 读取最近同步的已播歌曲数量。
    pub async fn get_sung_count(&self) -> usize {
        *self.sung_count.lock().await
    }
}

#[cfg(test)]
#[path = "tests/manager.rs"]
mod tests;
