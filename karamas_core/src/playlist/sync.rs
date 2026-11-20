use super::PlaylistManager;
use crate::engine::remote::{RemoteControl, RemoteStatus};
use futures_util::{SinkExt, StreamExt};
use log::{debug, error, info, warn};
use serde_json::Value;
use std::sync::Arc;
use std::{env, future::Future, pin::Pin, time::Duration};
use tokio::sync::{Notify, mpsc, watch};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use url::Url;

const PLAYLIST_RETRIES: usize = 3;
const PLAYLIST_RETRY_INTERVAL: Duration = Duration::from_secs(1);

/// 遥控接入点：注册帧随连接发送，命令读入后交给引擎，状态帧写回连接。
pub struct RemoteSync {
    pub registration: String,
    pub commands: mpsc::UnboundedSender<Value>,
    pub playback: watch::Receiver<Option<(String, String)>>,
    pub enabled: watch::Receiver<bool>,
    pub(crate) control: Arc<RemoteControl>,
}

impl PlaylistManager {
    // 根据环境变量切换同步驱动（WS / POLLING）
    // 环境变量：KTV_SYNC_MODE = "WS" 或 "POLLING"（不区分大小写），默认为 WS
    /// 根据同步模式启动房间歌单同步任务。
    pub fn start_sync<F>(
        &self,
        runtime: &tokio::runtime::Handle,
        f_on_update: F,
        remote: Option<RemoteSync>,
    ) -> tokio::task::JoinHandle<()>
    where
        F: Fn(String) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + 'static,
    {
        let mode = env::var("KTV_SYNC_MODE").unwrap_or_else(|_| "WS".to_string());
        info!("播放列表同步模式: {}", mode);
        if mode.to_uppercase() != "POLLING" {
            self.start_ws_update(runtime, f_on_update, remote)
        } else {
            if remote.as_ref().is_some_and(|remote| *remote.control.enabled.borrow()) {
                warn!("HTTP 轮询模式不承载遥控协议，网页遥控不可用");
            }
            self.start_periodic_update(runtime, f_on_update)
        }
    }

    /// 同步一次歌单；当前歌曲变化且有 URL 时回调，无 URL 时保持播放。
    pub(crate) async fn sync_playlist<F, Fut>(
        &self,
        f_on_update: F,
        report_error: bool,
    ) -> Result<(), String>
    where
        F: FnOnce(String) -> Fut,
        Fut: Future<Output = ()>,
    {
        // 手动刷新与后台同步串行，共用同一份已处理歌曲记录。
        let mut last_handled_song = self.last_handled_song.lock().await;
        let mut retries = 0;
        let song = loop {
            match self.fetch_playlist().await {
                Ok(song) => {
                    *self.sync_error.lock().await = None;
                    break song;
                }
                Err(error) if retries < PLAYLIST_RETRIES => {
                    retries += 1;
                    warn!("拉取歌单失败，1 秒后重试（{retries}/{PLAYLIST_RETRIES}）：{error}");
                    tokio::time::sleep(PLAYLIST_RETRY_INTERVAL).await;
                }
                Err(error) => {
                    let message = format!(
                        "拉取歌单失败，重试 {PLAYLIST_RETRIES} 次后仍未成功，请检查网络或稍后重连。详情：{error}"
                    );
                    if report_error {
                        *self.sync_error.lock().await = Some(message.clone());
                    }
                    return Err(message);
                }
            }
        };
        if song != *last_handled_song {
            if let Some((_, url)) = &song {
                f_on_update(url.clone()).await;
            }
            *last_handled_song = song;
        }
        Ok(())
    }

    /// 通过 WebSocket 通知和心跳同步歌单，断线后自动重连。
    fn start_ws_update<F>(
        &self,
        runtime: &tokio::runtime::Handle,
        mut f_on_update: F,
        remote: Option<RemoteSync>,
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
            let sync_requested = Notify::new();
            // 歌单与起播仍串行处理，耗时操作不挡住同一连接上的遥控消息。
            let playlist_sync = async {
                loop {
                    sync_requested.notified().await;
                    if let Err(error) = self_clone.sync_playlist(&mut f_on_update, true).await {
                        error!("同步歌单失败: {}", error);
                    }
                }
            };
            tokio::pin!(playlist_sync);
            // 遥控通道跨重连复用；注册帧每次连接后重发。
            let (mut connection, command_tx, mut playback_rx, mut enabled_rx) = match remote {
                Some(remote) => (
                    Some(RemoteConnection::new(remote.registration, remote.control)),
                    Some(remote.commands),
                    Some(remote.playback),
                    Some(remote.enabled),
                ),
                None => (None, None, None, None),
            };
            loop {
                if let Some(connection) = &connection {
                    connection.control.status.send_replace(RemoteStatus::Disconnected);
                }
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

                sync_requested.notify_one();
                let (mut write, mut read) = ws_stream.split();

                if let Some(connection) = &mut connection {
                    connection.phase = RemotePhase::Disabled;
                    connection.control.status.send_replace(RemoteStatus::Disabled);
                    if let Some(frame) = connection.update() {
                        if let Err(error) = write.send(Message::Text(frame.into())).await {
                            warn!("发送遥控注册失败: {}，准备重连", error);
                            continue;
                        }
                    }
                }

                // 引入心跳计时器
                let mut heartbeat = tokio::time::interval(Duration::from_secs(interval_secs));
                // 设置为延迟触发模式，避免不必要的积压和短间隔
                heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                // 立即触发第一次心跳
                heartbeat.tick().await;

                loop {
                    tokio::select! {
                        _ = &mut playlist_sync => break,
                        changed = remote_enabled_changed(&mut enabled_rx) => {
                            if !changed { break; }
                            if let Some(connection) = &mut connection {
                                if let Some(frame) = connection.update() {
                                    if let Err(error) = write.send(Message::Text(frame.into())).await {
                                        warn!("更新网页遥控失败: {}", error);
                                        break;
                                    }
                                }
                            }
                        }
                        // 只发送当前已注册会话的最新快照。
                        frame = next_remote_frame(&mut playback_rx) => {
                            if let Some((session_id, frame)) = frame {
                                if connection.as_ref().is_some_and(|connection| connection.control.accepts_session(&session_id)) {
                                    if let Err(error) = write.send(Message::Text(frame.into())).await {
                                        warn!("发送遥控状态失败: {}，准备重连", error);
                                        break;
                                    }
                                }
                            }
                        }

                        // 分支 A：定时发送心跳
                        _ = heartbeat.tick() => {
                            // 发送 WebSocket 协议层 Ping (维持 WS 长连接)
                            if let Err(e) = write.send(Message::Ping(vec![].into())).await {
                                warn!("发送 WS 心跳失败: {}, 准备重连", e);
                                break;
                            }

                            sync_requested.notify_one();
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
                                    let kind = v["type"].as_str().unwrap_or("");
                                    if kind != "UPDATE" {
                                        if let Some(connection) = &mut connection {
                                            if let Some(frame) = connection.receive(&v) {
                                                if let Err(error) = write.send(Message::Text(frame.into())).await {
                                                    warn!("更新网页遥控失败: {}", error);
                                                    break;
                                                }
                                            }
                                            if kind == "remote:command"
                                                && v["sessionId"].as_str().is_some_and(|id| connection.control.accepts_session(id))
                                            {
                                                if let Some(commands) = &command_tx {
                                                    let _ = commands.send(v);
                                                }
                                            }
                                        }
                                        continue;
                                    }
                                    info!("收到 WS UPDATE 消息: {:?}", v);
                                    let incoming_hash = v["hash"].as_str().unwrap_or("");
                                    let current_hash = self_clone.hash.lock().await.clone().unwrap_or_default();
                                    if incoming_hash == current_hash { continue; }

                                    debug!("[WS UPDATE]: {} -> {}", current_hash, incoming_hash);
                                    sync_requested.notify_one();
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

                if let Some(connection) = &connection {
                    connection.control.status.send_replace(RemoteStatus::Disconnected);
                }
                // 等待并重连
                tokio::time::sleep(Duration::from_secs(3)).await;
                debug!("重连 WS...");
            }
        })
    }

    /// 启动定时轮询歌单的后台任务。
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
            loop {
                interval.tick().await;
                if let Err(e) = self_clone.sync_playlist(&mut f_on_update, true).await {
                    error!("定时更新播放列表失败: {}", e);
                }
            }
        })
    }
}

/// 等待遥控变化；没有接入遥控时不参与 select。
async fn remote_enabled_changed(rx: &mut Option<watch::Receiver<bool>>) -> bool {
    match rx {
        Some(rx) => rx.changed().await.is_ok(),
        None => std::future::pending().await,
    }
}

async fn next_remote_frame(
    rx: &mut Option<watch::Receiver<Option<(String, String)>>>,
) -> Option<(String, String)> {
    match rx {
        Some(rx) => {
            if rx.changed().await.is_err() {
                return std::future::pending().await;
            }
            rx.borrow_and_update().clone()
        }
        None => std::future::pending().await,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RemotePhase {
    Disabled,
    Registering,
    Registered,
    Unregistering,
}

struct RemoteConnection {
    registration: String,
    control: Arc<RemoteControl>,
    phase: RemotePhase,
}

impl RemoteConnection {
    fn new(registration: String, control: Arc<RemoteControl>) -> Self {
        Self {
            registration,
            control,
            phase: RemotePhase::Disabled,
        }
    }

    fn update(&mut self) -> Option<String> {
        let enabled = *self.control.enabled.borrow();
        if enabled && self.phase == RemotePhase::Disabled {
            self.phase = RemotePhase::Registering;
            self.control.status.send_replace(RemoteStatus::Pending);
            Some(self.registration.clone())
        } else if !enabled
            && matches!(
                self.phase,
                RemotePhase::Registering | RemotePhase::Registered
            )
        {
            self.phase = RemotePhase::Unregistering;
            self.control.status.send_replace(RemoteStatus::Pending);
            Some(serde_json::json!({ "v": 1, "type": "remote:unregister" }).to_string())
        } else {
            if !enabled && self.phase == RemotePhase::Disabled {
                self.control.status.send_replace(RemoteStatus::Disabled);
            }
            None
        }
    }

    fn receive(&mut self, message: &Value) -> Option<String> {
        match message["type"].as_str() {
            Some("remote:registered") if self.phase == RemotePhase::Registering => {
                if let Some(session_id) = message["sessionId"].as_str().filter(|id| !id.is_empty())
                {
                    self.phase = RemotePhase::Registered;
                    if *self.control.enabled.borrow() {
                        self.control
                            .status
                            .send_replace(RemoteStatus::Registered(session_id.to_owned()));
                    }
                }
                self.update()
            }
            Some("remote:unregistered") if self.phase == RemotePhase::Unregistering => {
                self.phase = RemotePhase::Disabled;
                self.control.status.send_replace(RemoteStatus::Disabled);
                self.update()
            }
            Some("remote:error") => {
                let message = message["msg"].as_str().unwrap_or("网页遥控请求失败");
                warn!("网页遥控请求失败: {}", message);
                self.control
                    .status
                    .send_replace(RemoteStatus::Failed(message.to_owned()));
                None
            }
            _ => None,
        }
    }
}
