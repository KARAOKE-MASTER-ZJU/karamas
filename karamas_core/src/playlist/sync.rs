use super::PlaylistManager;
use futures_util::{SinkExt, StreamExt};
use log::{debug, error, info, warn};
use std::{env, future::Future, pin::Pin, time::Duration};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use url::Url;

const PLAYLIST_RETRIES: usize = 3;
const PLAYLIST_RETRY_INTERVAL: Duration = Duration::from_secs(1);

impl PlaylistManager {
    // 根据环境变量切换同步驱动（WS / POLLING）
    // 环境变量：KTV_SYNC_MODE = "WS" 或 "POLLING"（不区分大小写），默认为 WS
    /// 根据同步模式启动房间歌单同步任务。
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

    /// 同步一次歌单，并在当前歌曲变化时调用播放回调。
    pub(super) async fn sync_playlist<F>(
        &self,
        song_playing_cached: &mut Option<(Option<String>, String)>,
        f_on_update: &mut F,
    ) -> Result<(), String>
    where
        F: Fn(String) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send,
    {
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
                    *self.sync_error.lock().await = Some(message.clone());
                    return Err(message);
                }
            }
        };
        if song != *song_playing_cached {
            if let Some((_, url)) = &song {
                f_on_update(url.clone()).await;
            }
            *song_playing_cached = song;
        }
        Ok(())
    }

    /// 通过 WebSocket 通知和心跳同步歌单，断线后自动重连。
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
            let mut song_playing = None;
            loop {
                interval.tick().await;
                if let Err(e) = self_clone.sync_playlist(&mut song_playing, &mut f_on_update).await {
                    error!("定时更新播放列表失败: {}", e);
                }
            }
        })
    }
}
