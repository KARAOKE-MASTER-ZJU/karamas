//! 小电视广播接收与当前播放状态同步。
mod playback;
mod proto;

use playback::PlaybackTimeline;

use crate::cast::CastError;
use base64::{Engine, engine::general_purpose::STANDARD_NO_PAD};
use futures_util::{Stream, StreamExt};
use prost::Message;
use proto::*;
use std::{
    collections::HashSet,
    sync::{Arc, Mutex, RwLock},
    time::Duration,
};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{
    Request,
    metadata::{MetadataMap, MetadataValue},
    transport::{ClientTlsConfig, Endpoint},
};
use uuid::Uuid;

const HOST: &str = "https://broadcast.chat.bilibili.com:7824";
const TUNNEL: &str = "/bilibili.broadcast.v1.BroadcastTunnel/CreateTunnel";
const BROADCAST: &str = "/bilibili.broadcast.v1.Broadcast/";
const PROJ: &str = "/bilibili.broadcast.message.tv.Tv/Proj";
const PROJ_CORE: &str = "/bilibili.broadcast.message.tv.Tv/ProjCore";
const START_TIMEOUT: Duration = Duration::from_secs(15);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(25);
pub(super) const MOBILE_BUILD: i32 = 8940300;

#[derive(Clone)]
pub(super) struct PlaySession {
    pub id: String,
    pub aid: u64,
    pub cid: u64,
    timeline: PlaybackTimeline,
}

/// 使用本应用持久化的发送端身份，广播重连和引擎重建均复用该标识。
pub(super) struct BroadcastReceiver {
    pub buvid: String,
    #[cfg(test)]
    test_host: Option<String>,
    active: Arc<RwLock<Option<PlaySession>>>,
    worker: Mutex<Option<Worker>>,
    stop: watch::Sender<bool>,
}

struct Worker(tokio::task::JoinHandle<()>);

impl Drop for Worker {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl BroadcastReceiver {
    pub fn new(buvid: String) -> Self {
        let (stop, _) = watch::channel(false);
        Self {
            buvid,
            #[cfg(test)]
            test_host: None,
            active: Arc::new(RwLock::new(None)),
            worker: Mutex::new(None),
            stop,
        }
    }

    pub fn begin_play(&self, aid: u64, cid: u64) -> PlaySession {
        let session = PlaySession {
            id: Uuid::new_v4().to_string(),
            aid,
            cid,
            timeline: PlaybackTimeline::default(),
        };
        *self.active.write().unwrap_or_else(|e| e.into_inner()) = Some(session.clone());
        session
    }

    pub fn clear_play(&self, id: &str) {
        let mut active = self.active.write().unwrap_or_else(|e| e.into_inner());
        if active.as_ref().is_some_and(|session| session.id == id) {
            *active = None;
        }
    }

    pub fn confirm_play(&self, id: &str, duration: u32) {
        let mut active = self.active.write().unwrap_or_else(|e| e.into_inner());
        if let Some(session) = active.as_mut().filter(|session| session.id == id) {
            session.timeline.confirm_play(duration);
        }
    }

    pub fn playback_snapshot(&self) -> Option<crate::cast::PlaybackSnapshot> {
        self.active
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|session| session.timeline.snapshot())
    }

    pub fn update_local_playback(&self, position: Option<u32>, playing: Option<bool>) {
        let mut active = self.active.write().unwrap_or_else(|e| e.into_inner());
        if let Some(session) = active.as_mut() {
            session.timeline.update(position, playing);
        }
    }

    pub fn stop_status_subscription(&self) {
        self.stop.send_replace(true);
    }

    pub fn session_id(&self) -> Option<String> {
        self.active
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|session| session.id.clone())
    }

    /// 在后台建立订阅，立即返回；断流后可再次调用以重连。
    pub fn start(&self, token: &str, mid: u64) {
        let token = token.to_owned();
        let buvid = self.buvid.clone();
        let active = self.active.clone();
        let host = HOST.to_owned();
        #[cfg(test)]
        let host = self.test_host.clone().unwrap_or(host);
        self.spawn_worker(move |ready| async move {
            let headers = headers(&token, mid, &buvid).map_err(|error| error.to_string())?;
            connect(host, headers, active, ready).await
        });
    }

    fn spawn_worker<F>(&self, connection: impl FnOnce(oneshot::Sender<()>) -> F + Send + 'static)
    where
        F: std::future::Future<Output = Result<(), String>> + Send + 'static,
    {
        // 锁只保护任务句柄，不跨网络请求或订阅等待持有。
        let mut worker = self.worker.lock().unwrap_or_else(|e| e.into_inner());
        if *self.stop.borrow()
            || worker
                .as_ref()
                .is_some_and(|worker| !worker.0.is_finished())
        {
            return;
        }
        let mut stop = self.stop.subscribe();
        *worker = Some(Worker(tokio::spawn(async move {
            if *stop.borrow() {
                return;
            }
            let receive = async move {
                let (ready_tx, ready_rx) = oneshot::channel();
                let connection = connection(ready_tx);
                tokio::pin!(connection);
                tokio::select! {
                    result = &mut connection => return result,
                    ready = tokio::time::timeout(START_TIMEOUT, ready_rx) => {
                        ready.map_err(|_| "小电视广播订阅超时（15 秒）".to_string())?
                            .map_err(|_| "小电视广播订阅未完成".to_string())?;
                    }
                }
                connection.await
            };
            tokio::select! {
                biased;
                _ = stop.changed() => {
                    log::info!(target: "BILI_BROADCAST", "小电视广播连接已关闭");
                }
                result = receive => {
                    if let Err(error) = result {
                        log::warn!(target: "BILI_BROADCAST", "小电视广播接收失败：{}；下次播放时重连", error);
                    }
                }
            }
        })));
    }
}

impl Drop for BroadcastReceiver {
    fn drop(&mut self) {
        self.stop_status_subscription();
    }
}

fn headers(token: &str, mid: u64, buvid: &str) -> Result<MetadataMap, CastError> {
    let mut headers = MetadataMap::new();
    let mut eid = mid.to_string().into_bytes();
    for (i, byte) in eid.iter_mut().enumerate() {
        *byte ^= b"ad1va46a7lza"[i % 12];
    }
    for (key, value) in [
        ("authorization", format!("identify_v1 {token}")),
        ("buvid", buvid.to_owned()),
        ("x-bili-mid", mid.to_string()),
        ("x-bili-aurora-eid", STANDARD_NO_PAD.encode(eid)),
    ] {
        let mut value = MetadataValue::try_from(value.as_str())
            .map_err(|_| CastError::Device(format!("小电视广播请求头格式错误：{key}")))?;
        value.set_sensitive(true);
        headers.insert(key, value);
    }
    let device = Device {
        build: MOBILE_BUILD,
        buvid: buvid.into(),
        mobi_app: "android".into(),
        platform: "android".into(),
        device: "phone".into(),
        channel: "bili".into(),
    };
    let metadata = Metadata {
        access_key: token.into(),
        mobi_app: "android".into(),
        device: "phone".into(),
        build: MOBILE_BUILD,
        channel: "bili".into(),
        buvid: buvid.into(),
        platform: "android".into(),
    };
    for (key, bytes) in [
        ("x-bili-device-bin", device.encode_to_vec()),
        ("x-bili-metadata-bin", metadata.encode_to_vec()),
        (
            "x-bili-local-bin",
            Locale {
                timezone: "Asia/Shanghai".into(),
            }
            .encode_to_vec(),
        ),
        ("x-bili-network-bin", Network { r#type: 1 }.encode_to_vec()),
    ] {
        // tonic 负责 -bin 的传输编码，这里传原始 protobuf 字节。
        let mut value = MetadataValue::from_bytes(&bytes);
        value.set_sensitive(true);
        headers.insert_bin(key, value);
    }
    Ok(headers)
}

async fn connect(
    host: String,
    headers: MetadataMap,
    active: Arc<RwLock<Option<PlaySession>>>,
    ready: oneshot::Sender<()>,
) -> Result<(), String> {
    let endpoint = Endpoint::from_shared(host.clone())
        .map_err(|error| format!("广播地址无效：{error}"))?
        .connect_timeout(START_TIMEOUT)
        .tls_config(ClientTlsConfig::new().with_webpki_roots())
        .map_err(|_| "配置 TLS 失败".to_string())?;
    log::info!(target: "BILI_BROADCAST", "正在连接小电视广播：{}", host);
    let channel = endpoint
        .connect()
        .await
        .map_err(|error| format!("建立 TLS 连接失败：{error}"))?;
    let mut client = tonic::client::Grpc::new(channel);
    client
        .ready()
        .await
        .map_err(|error| format!("广播通道未就绪：{error}"))?;
    let (tx, rx) = mpsc::channel(32);
    let mut sender = FrameSender { tx, sequence: 0 };
    sender
        .send(
            "Auth",
            "AuthReq",
            AuthReq {
                guid: Uuid::new_v4().to_string(),
                conn_id: Uuid::new_v4().to_string(),
                last_msg_id: 0,
            },
        )
        .await?;
    let mut request = Request::new(ReceiverStream::new(rx));
    *request.metadata_mut() = headers;
    let response = client
        .streaming(
            request,
            tonic::codegen::http::uri::PathAndQuery::from_static(TUNNEL),
            tonic_prost::ProstCodec::<BroadcastFrame, BroadcastFrame>::default(),
        )
        .await
        .map_err(|status| format!("创建广播双向流失败：{}", status.code()))?;
    receive(response.into_inner(), sender, active, ready, |event| {
        log::debug!(target: "BILI_BROADCAST",
            "电视状态回传：状态={}，cmd={}，位置秒={:?}，总时长毫秒={:?}，aid={}，cid={}，bizSessionId={}",
            event.name, event.command, event.position_secs, event.duration_ms, event.aid, event.cid, event.session);
    }).await
}

/// 唯一发送者串行处理订阅、ACK 和心跳，保持 sequence 单调递增。
struct FrameSender {
    tx: mpsc::Sender<BroadcastFrame>,
    sequence: i64,
}

impl FrameSender {
    async fn send(
        &mut self,
        method: &str,
        message: &str,
        body: impl Message,
    ) -> Result<i64, String> {
        let sequence = self.sequence;
        self.sequence += 1;
        let frame = BroadcastFrame {
            target_path: format!("{BROADCAST}{method}"),
            body: Some(prost_types::Any {
                type_url: format!("type.googleapis.com/bilibili.broadcast.v1.{message}"),
                value: body.encode_to_vec(),
            }),
            options: Some(FrameOption {
                message_id: sequence,
                sequence,
                timestamp: chrono::Utc::now().timestamp_millis(),
                ..Default::default()
            }),
        };
        tokio::time::timeout(Duration::from_secs(5), self.tx.send(frame))
            .await
            .map_err(|_| "广播发送队列阻塞".to_string())?
            .map_err(|_| "广播发送通道已关闭".to_string())?;
        Ok(sequence)
    }
}

async fn receive(
    mut incoming: impl Stream<Item = Result<BroadcastFrame, tonic::Status>> + Unpin,
    mut sender: FrameSender,
    active: Arc<RwLock<Option<PlaySession>>>,
    ready: oneshot::Sender<()>,
    mut on_event: impl FnMut(DeviceEvent),
) -> Result<(), String> {
    let mut ready = Some(ready);
    let mut authed = false;
    let mut pending = HashSet::new();
    let mut heartbeat = tokio::time::interval_at(
        tokio::time::Instant::now() + HEARTBEAT_INTERVAL,
        HEARTBEAT_INTERVAL,
    );
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        let frame = tokio::select! {
            frame = incoming.next() => frame.ok_or_else(|| "服务端关闭了广播流".to_string())?
                .map_err(|status| format!("广播流错误：{}", status.code()))?,
            _ = heartbeat.tick() => {
                sender.send("Heartbeat", "HeartbeatReq", HeartbeatReq {}).await?;
                continue;
            }
        };
        let options = frame.options.unwrap_or_default();
        if let Some(status) = options.status.as_ref().filter(|status| status.code != 0) {
            return Err(format!("广播业务响应失败：code={}", status.code));
        }
        if options.is_ack {
            sender
                .send(
                    "MessageAck",
                    "MessageAckReq",
                    MessageAckReq {
                        ack_id: options.message_id,
                        ack_origin: options.ack_origin,
                        target_path: frame.target_path.clone(),
                        msg_type: options.msg_type,
                    },
                )
                .await?;
        }
        match frame.target_path.as_str() {
            path if path == format!("{BROADCAST}Auth") && !authed => {
                authed = true;
                log::info!(target: "BILI_BROADCAST", "小电视广播鉴权成功，开始订阅 Proj / ProjCore");
                for topic in [PROJ, PROJ_CORE] {
                    let sequence = sender
                        .send(
                            "Subscribe",
                            "TargetPath",
                            TargetPath {
                                target_paths: vec![topic.into()],
                            },
                        )
                        .await?;
                    pending.insert(sequence);
                }
            }
            path if path == format!("{BROADCAST}Subscribe") => {
                if pending.remove(&options.sequence) && pending.is_empty() {
                    log::info!(target: "BILI_BROADCAST", "小电视广播订阅就绪：Proj / ProjCore");
                    if let Some(ready) = ready.take() {
                        let _ = ready.send(());
                    }
                }
            }
            PROJ | PROJ_CORE if ready.is_none() => {
                let event = {
                    // 校验会话与更新状态共用一把锁，切歌后不能写入旧曲目的回传。
                    let mut session = active.write().unwrap_or_else(|e| e.into_inner());
                    let event = decode_event(frame.body, session.as_ref());
                    if let (Ok(Some(event)), Some(session)) = (&event, session.as_mut()) {
                        session.timeline.apply(event);
                    }
                    event
                };
                match event {
                    Ok(Some(event)) => on_event(event),
                    Ok(None) => {}
                    Err(error) => {
                        log::warn!(target: "BILI_BROADCAST", "忽略无效电视状态：{}", error)
                    }
                }
            }
            _ => {}
        }
    }
}

#[derive(Debug, PartialEq)]
struct DeviceEvent {
    name: &'static str,
    command: i64,
    position_secs: Option<u64>,
    duration_ms: Option<u64>,
    aid: i64,
    cid: i64,
    session: String,
}

fn decode_event(
    body: Option<prost_types::Any>,
    active: Option<&PlaySession>,
) -> Result<Option<DeviceEvent>, &'static str> {
    let Some(active) = active else {
        return Ok(None);
    };
    let body = body.ok_or("缺少 protobuf 消息体")?;
    if body.type_url.rsplit('/').next() != Some("bilibili.broadcast.message.tv.ProjReply") {
        return Err("消息类型不是 ProjReply");
    }
    let message = ProjReply::decode(body.value.as_slice()).map_err(|_| "无法解码 ProjReply")?;
    #[derive(serde::Deserialize)]
    struct Extra {
        #[serde(rename = "bizSessionId")]
        session: Option<String>,
        duration: Option<u64>,
    }
    let extra: Extra =
        serde_json::from_str(&message.extra).map_err(|_| "extra 不是有效状态 JSON")?;
    if extra.session.as_deref() != Some(&active.id) {
        return Ok(None);
    }
    // 部分控制通知省略 aid/cid；提供标识时必须与当前曲目匹配。
    if (message.aid != 0 && u64::try_from(message.aid).ok() != Some(active.aid))
        || (message.cid != 0 && u64::try_from(message.cid).ok() != Some(active.cid))
    {
        return Ok(None);
    }
    let name = match message.cmd_type {
        4 => "进度",
        5 => "暂停",
        6 => "恢复",
        7 => "停止",
        13 => "时长",
        _ => return Ok(None),
    };
    Ok(Some(DeviceEvent {
        name,
        command: message.cmd_type,
        // 暂停/恢复通知可能不带 seek_ts，不能把 protobuf 默认零值当成回到开头。
        position_secs: if message.cmd_type == 4 {
            u64::try_from(message.seek_ts).ok()
        } else {
            None
        },
        duration_ms: extra.duration.filter(|duration| *duration > 0),
        aid: message.aid,
        cid: message.cid,
        session: active.id.clone(),
    }))
}

#[cfg(test)]
mod tests;
