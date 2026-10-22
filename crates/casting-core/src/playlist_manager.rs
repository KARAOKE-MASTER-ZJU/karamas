use futures_util::{SinkExt, StreamExt};
use log::{debug, error, info, warn};
use reqwest::Client;
use serde_json::json;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use std::{env, future::Future};
use tokio::sync::Mutex;
#[cfg(test)]
use tokio::time::sleep;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use url::Url;

#[derive(Clone)]
pub struct PlaylistManager {
    url: String,
    room_id: String,
    client: Client,
    hash: Arc<Mutex<Option<String>>>,
    song_playing: Arc<Mutex<Option<String>>>,
    song_title: Arc<Mutex<Option<String>>>,
    queued_count: Arc<Mutex<usize>>,
    sung_count: Arc<Mutex<usize>>,
    pub(crate) progress_gate: Arc<Mutex<crate::progress_gate::ProgressGate>>,
}

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
    pub fn new(url: &str, room_id: String) -> Self {
        // 在初始化时构建一次 Client
        let client = Client::builder()
            .use_rustls_tls()
            .timeout(crate::API_TIMEOUT)
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
            progress_gate: Arc::new(Mutex::new(crate::progress_gate::ProgressGate::default())),
        }
    }

    // 适配新的返回结构：
    // {
    //   changed: boolean,
    //   list: { queued: Song[]; singing: Song; sung: Song[] },
    //   hash: string
    // }
    // Song { id, title, url, addedBy? }
    async fn fetch_playlist(&self) -> Result<Option<(Option<String>, String)>, String> {
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
            if let Ok((bvid, page)) = crate::bilibili_parser::parse_bilibili_video(url) {
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

    // 根据环境变量切换同步驱动（WS / POLLING）
    // 环境变量：KTV_SYNC_MODE = "WS" 或 "POLLING"（不区分大小写），默认为 WS
    pub fn start_sync<F>(
        &self,
        runtime: &tokio::runtime::Handle,
        f_on_update: F,
    ) -> tokio::task::JoinHandle<()>
    where
        F: Fn(String) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + 'static,
    {
        let mode = env::var("KTV_SYNC_MODE").unwrap_or_else(|_| "WS".to_string());
        info!("播放列表同步模式: {}", mode);
        if mode.to_uppercase() != "POLLING" {
            self.start_ws_update(runtime, f_on_update)
        } else {
            self.start_periodic_update(runtime, f_on_update)
        }
    }

    async fn sync_playlist<F>(
        &self,
        song_playing_cached: &mut Option<(Option<String>, String)>,
        f_on_update: &mut F,
    ) -> Result<(), String>
    where
        F: Fn(String) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send,
    {
        let song = self.fetch_playlist().await?;
        if song != *song_playing_cached {
            if let Some((_, url)) = &song {
                f_on_update(url.clone()).await;
            }
            *song_playing_cached = song;
        }
        Ok(())
    }

    fn start_ws_update<F>(
        &self,
        runtime: &tokio::runtime::Handle,
        mut f_on_update: F,
    ) -> tokio::task::JoinHandle<()>
    where
        F: Fn(String) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + 'static,
    {
        let self_clone = self.clone();
        runtime.spawn(async move {
            /*
               这是维护WebSocket连接的循环
               负责连接、重连
            */
            let interval_secs = env::var("KEEP_ALIVE_INTERVAL")
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(30);
            info!("心跳间隔: {} 秒", interval_secs);
            // Keep the last handled song across reconnects as well as heartbeat/WS updates.
            let mut song_playing_cached = None;
            loop {
                // 构造 WS URL （将 http(s) -> ws(s)）
                let nickname = env::var("KTV_NICKNAME").unwrap_or_default();
                let mut ws_url = format!(
                    "{}/api/ws?roomId={}&nickname={}",
                    self_clone.url.trim_end_matches('/'),
                    self_clone.room_id,
                    urlencoding::encode(&nickname)
                );
                if let Ok(mut parsed) = Url::parse(&ws_url) {
                    let _ = match parsed.scheme() {
                        "https" => parsed.set_scheme("wss"),
                        "http" => parsed.set_scheme("ws"),
                        _ => Ok(()),
                    };
                    ws_url = parsed.to_string();
                }

                info!("WebSocket Connecting: {}", ws_url);

                // 调试用代码
                /* let url = match url::Url::parse(&ws_url) {
                    Ok(u) => u,
                    Err(e) => {
                        error!("URL 解析失败: {}", e);
                        break;
                    }
                };
                let host = url.host_str().unwrap_or_default().to_string();
                let port = url.port_or_known_default().unwrap_or(443);

                // 异步 DNS 解析
                let addrs = match tokio::task::spawn_blocking(move || {
                    use std::net::ToSocketAddrs;
                    (host.as_str(), port).to_socket_addrs()
                })
                .await
                {
                    Ok(Ok(addr_iter)) => addr_iter,
                    _ => {
                        error!("DNS 解析失败");
                        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                        continue;
                    }
                };

                let addr = match addrs.into_iter().next() {
                    Some(a) => a,
                    None => {
                        error!("DNS 未找到记录");
                        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                        continue;
                    }
                };

                info!("DNS 解析成功: {}", addr);

                // 建立 TCP 连接
                let stream = match tokio::time::timeout(
                    tokio::time::Duration::from_secs(10),
                    tokio::net::TcpStream::connect(addr),
                )
                .await
                {
                    Ok(Ok(s)) => s,
                    _ => {
                        error!("TCP 连接超时或拒绝");
                        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                        continue;
                    }
                };

                info!("TCP 已连接，准备 WS 握手...");

                // WebSocket 握手

                // 加载 webpki 根证书
                let mut root_store = rustls::RootCertStore::empty();
                root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

                // 2. 创建配置 (支持 TLS 1.2 和 1.3)
                let config = rustls::ClientConfig::builder()
                    .with_root_certificates(root_store)
                    .with_no_client_auth();

                let connector = tokio_tungstenite::Connector::Rustls(std::sync::Arc::new(config));

                let (mut ws_stream, _) = match tokio::time::timeout(
                    tokio::time::Duration::from_secs(10),
                    tokio_tungstenite::client_async_tls_with_config(
                        ws_url.clone(),
                        stream,
                        None,
                        Some(connector),
                    ),
                )
                .await
                {
                    Ok(Ok(val)) => val,
                    Ok(Err(e)) => {
                        error!("WS 握手报错: {:?}", e);
                        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                        continue;
                    }
                    Err(_) => {
                        error!("WS 握手超时");
                        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                        continue;
                    }
                }; */

                let (ws_stream, _) = match connect_async(ws_url).await {
                    Ok(val) => val,
                    Err(e) => {
                        error!("连接 WebSocket 失败: {}", e);
                        // 避免紧密重连循环，等待后重试
                        tokio::time::sleep(Duration::from_secs(3)).await;
                        continue;
                    }
                };

                info!("WebSocket connected for room {}", self_clone.room_id);

                if let Err(e) = self_clone.sync_playlist(&mut song_playing_cached, &mut f_on_update).await {
                    error!("初始化拉取失败: {}", e);
                }
                let (mut write, mut read) = ws_stream.split();

                // 引入心跳计时器
                let mut heartbeat = tokio::time::interval(Duration::from_secs(interval_secs));
                // 设置为延迟触发模式，避免不必要的积压和短间隔
                heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                // 立即触发第一次心跳
                heartbeat.tick().await;

                loop {
                    tokio::select! {
                        // 分支 A：定时发送心跳
                        _ = heartbeat.tick() => {
                            // 发送 WebSocket 协议层 Ping (维持 WS 长连接)
                            if let Err(e) = write.send(Message::Ping(vec![].into())).await {
                                warn!("发送 WS 心跳失败: {}, 准备重连", e);
                                break;
                            }

                            // Reuse HTTP connections and recover missed WS updates through
                            // the same serial sync path, including device playback.
                            if let Err(e) = self_clone.sync_playlist(&mut song_playing_cached, &mut f_on_update).await {
                                debug!("心跳同步歌单失败: {}", e);
                            }
                        }

                        // 分支 B：接收 WS 消息
                        msg = read.next() => {
                            let msg = match msg {
                                Some(Ok(m)) => m,
                                Some(Err(e)) => { error!("WS 读取错误: {}", e); break; },
                                None => { info!("WS 连接关闭"); break; }
                            };

                            match msg {
                                Message::Text(text) => {
                                    let v = match serde_json::from_str::<serde_json::Value>(&text) {
                                        Ok(v) => v,
                                        Err(e) => { error!("解析 JSON 失败: {}", e); continue; },
                                    };
                                    if v["type"].as_str().unwrap_or("") != "UPDATE" { continue; }
                                    info!("收到 WS UPDATE 消息: {:?}", v);
                                    let incoming_hash = v["hash"].as_str().unwrap_or("");
                                    let current_hash = self_clone.hash.lock().await.clone().unwrap_or_default();
                                    if incoming_hash == current_hash { continue; }

                                    debug!("[WS UPDATE]: {} -> {}", current_hash, incoming_hash);
                                    if let Err(e) = self_clone.sync_playlist(&mut song_playing_cached, &mut f_on_update).await {
                                        error!("WS 同步歌单失败: {}", e);
                                    }
                                }
                                Message::Ping(p) => {
                                    let _ = write.send(Message::Pong(p)).await;
                                }
                                Message::Close(_) => { info!("服务器关闭连接"); break; }
                                _ => {}
                            }
                        }
                    }
                }

                // 等待并重连
                tokio::time::sleep(Duration::from_secs(3)).await;
                debug!("重连 WS...");
            }
        })
    }

    pub fn start_periodic_update<F>(
        &self,
        runtime: &tokio::runtime::Handle,
        mut f_on_update: F,
    ) -> tokio::task::JoinHandle<()>
    where
        F: Fn(String) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + 'static,
    {
        let self_clone = self.clone();
        runtime.spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(300));
            let mut song_playing = None;
            loop {
                interval.tick().await;
                if let Err(e) = self_clone.sync_playlist(&mut song_playing, &mut f_on_update).await {
                    error!("定时更新播放列表失败: {}", e);
                }
            }
        })
    }

    /// Read song and hash from one authoritative room response, without consuming
    /// the sync loop's hash or suppressing its playback callback.
    pub(crate) async fn current_song_snapshot(
        &self,
    ) -> Result<crate::bilibili_parts::CurrentSongSnapshot, crate::bilibili_parts::PartError> {
        use crate::bilibili_parts::{CurrentSongSnapshot, PartError};
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
        song: &crate::bilibili_parts::CurrentSongSnapshot,
        target_url: &str,
        unchanged: bool,
    ) -> Result<bool, crate::bilibili_parts::PartError> {
        use crate::bilibili_parts::PartError;
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

    pub async fn get_song_playing(&self) -> Option<String> {
        self.song_playing.lock().await.clone()
    }

    pub async fn get_song_id(&self) -> Option<String> {
        self.progress_gate.lock().await.song().and_then(|(id, _)| id)
    }

    pub async fn get_song_title(&self) -> Option<String> {
        self.song_title.lock().await.clone()
    }

    pub async fn get_queued_count(&self) -> usize {
        *self.queued_count.lock().await
    }

    pub async fn get_sung_count(&self) -> usize {
        *self.sung_count.lock().await
    }
}

#[cfg(test)]
#[path = "../test/playlist_manager.rs"]
mod automatic_next_tests;

#[tokio::test]
#[ignore = "manual live-room example; mutates the remote playlist"]
async fn test_playlist_manager() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== PlaylistManager 使用示例 ===");

    let mut manager = PlaylistManager::new("https://ktv.starfreedomx.top", "1111".to_string());

    println!("开始获取播放列表...");

    // --- 第一次操作 ---
    match manager.fetch_playlist().await {
        Ok(_) => {
            println!("✓ 成功获取播放列表");
            // 获取当前播放的歌曲
            {
                let song_playing = manager.get_song_playing().await;
                match &song_playing {
                    Some(url) => println!("当前播放的歌曲: {}", url),
                    None => println!("当前没有播放的歌曲"),
                }
            }
        }
        Err(e) => error!("✗ 获取播放列表失败: {}", e),
    }

    // --- 第二次操作 ---
    manager.next_song().await?;
    println!("请求下一首歌曲后播放列表状态:");

    // 获取当前播放的歌曲
    {
        let song_playing = manager.get_song_playing().await;
        match &song_playing {
            Some(url) => println!("当前播放的歌曲: {}", url),
            None => println!("当前没有播放的歌曲"),
        }
    }

    // --- 后台任务开始 ---
    manager.start_periodic_update(&tokio::runtime::Handle::current(), |url: String| {
        println!("Song singing changed to {}!", url);
        Box::pin(async {})
    });

    // 【关键点 3】：sleep 必须在“裸奔”状态下运行（不持有任何锁）
    // 此时 playlist 锁是空闲的，后台线程的 fetch_playlist 才能拿到锁并更新数据
    sleep(Duration::from_secs(5)).await;

    println!("5秒后播放列表状态:");

    // 获取当前播放的歌曲
    {
        let song_playing = manager.get_song_playing().await;
        match &song_playing {
            Some(url) => println!("当前播放的歌曲: {}", url),
            None => println!("当前没有播放的歌曲"),
        }
    }

    println!("=== 示例结束 ===");
    Ok(())
}
