//! Bilibili 投屏器
use super::{
    broadcast::{BroadcastReceiver, MOBILE_BUILD},
    client::{API_HOST, APPKEY, bili_client, form_body},
    session::{BilibiliSession, now_ts},
};

// 解析歌曲引用，返回 BV 号和页码
use crate::{
    cast::{
        Capabilities, CastError, Caster, Progress, Quality, SongRef, progress::LocalProgressTracker,
    },
    sources::bilibili::{bv_to_aid, get_page_info},
};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU32, Ordering},
};

// Bilibili 投屏器
pub struct BilibiliCaster {
    access_token: String,
    mid: u64,
    device_buvid: String,
    progress: Arc<LocalProgressTracker>,
    broadcast: BroadcastReceiver,
    session_id: String,
    #[cfg(test)]
    test_api: Option<(String, (u64, u32))>,
    // 保存配置项
    quality: AtomicU32, // 使用 AtomicU32 保存清晰度的数值表示
    danmaku: AtomicBool,
}

// Bilibili 投屏器实现
impl BilibiliCaster {
    pub fn new(session: BilibiliSession, device_buvid: String, buvid: String) -> Self {
        Self {
            access_token: session.access_token,
            mid: session.mid,
            device_buvid,
            progress: LocalProgressTracker::new(),
            broadcast: BroadcastReceiver::new(buvid),
            session_id: uuid::Uuid::new_v4().to_string(),
            #[cfg(test)]
            test_api: None,
            quality: AtomicU32::new(Quality::default().as_qn()) ,
            danmaku: AtomicBool::new(false),
        }
    }

    #[cfg(test)]
    pub(super) fn with_test_transport(
        mut self,
        endpoint: String,
        page_info: (u64, u32),
        broadcast: BroadcastReceiver,
    ) -> Self {
        self.test_api = Some((endpoint, page_info));
        self.broadcast = broadcast;
        self
    }

    async fn page_info(&self, bvid: &str, page: u32) -> Result<(u64, u32), String> {
        #[cfg(test)]
        if let Some((_, info)) = &self.test_api {
            return Ok(*info);
        }
        get_page_info(bvid, page).await
    }

    /// 后台启动状态订阅，播放命令不等待订阅结果。
    pub fn start_broadcast(&self) {
        self.broadcast.start(&self.access_token, self.mid);
    }

    /// 按普通 UGC 云投屏格式构建命令，携带广播接收端和播放会话标识。
    fn command_body(
        &self,
        command: u32,
        oid: u64,
        extra_override: Option<Value>,
        seek_ts: u32,
    ) -> String {
        let device_info = serde_json::json!({
            "pink_mobi_app": "android",
            "pink_build": MOBILE_BUILD,
            "pink_buvid": self.broadcast.buvid,
            "ott_buvid": self.device_buvid,
            "ott_mid": self.mid,
            "ott_build": 108500,
            "ott_mobi_app": "android_tv_yst"
        })
        .to_string();

        // 构建基础 extra 参数
        let mut full_extra = serde_json::json!({
            "accessKey": self.access_token.as_str(),
            "buvid": self.broadcast.buvid,
            "sessionId": self.session_id,
            "mobileVersion": MOBILE_BUILD,
            "autoNext": false, "biz_id": 0, "oid": 0,
            "userDesireSpeed": 1.0, "type": 0, "desc": 0, "proj_type": 1,
            "danmakuSwitchSave": false
        });
        if let Some(id) = self.broadcast.session_id() {
            full_extra["bizSessionId"] = id.into();
        }
        if let (Some(obj), Some(extra)) = (full_extra.as_object_mut(), extra_override.as_ref().and_then(|v| v.as_object())) {
            for (k, v) in extra {
                obj.insert(k.clone(), v.clone());
            }
        }

        let cmd_str = command.to_string();
        let seek_str = seek_ts.to_string();
        let ts_str = now_ts().to_string();
        let aid_str = oid.to_string();
        let build_str = MOBILE_BUILD.to_string();

        // 提取cid作为顶级参数（如果存在）
        let cid_str = full_extra.get("cid")
            .and_then(|v| v.as_u64())
            .map(|c| c.to_string());

        // 从extra中移除cid，避免重复传递
        if let Some(obj) = full_extra.as_object_mut() {
            obj.remove("cid");
        }

        let extra_str = full_extra.to_string();

        let mut logged_extra = full_extra.clone();
        if let Some(extra) = logged_extra.as_object_mut() {
            extra.remove("accessKey");
        }
        log::info!("[Bilibili] send_cmd: command={}, oid={}, cid={:?}, extra={}", command, oid, cid_str, logged_extra);

        // 构建form参数，cid作为顶级参数（如果存在）
        let mut params = vec![
            ("access_key", self.access_token.as_str()),
            ("appkey", APPKEY),
            ("command", &cmd_str),
            ("ott_buvid", &self.device_buvid),
            ("ts", &ts_str),
            ("device_info", &device_info),
            ("extra", &extra_str),
            ("seek_ts", &seek_str),
            ("video_type", "0"),
            ("aid", &aid_str),
            ("epid", "0"),
            ("season_id", "0"),
            ("room_id", ""),
            ("by_code", "false"),
            ("danmakuSwitchSave", "false"),
            ("build", &build_str),
            ("mobi_app", "android"),
            ("platform", "android"),
            ("s_locale", "zh-Hans_CN"),
        ];

        // 如果存在cid，作为顶级参数加入
        if let Some(ref cid) = cid_str {
            params.insert(8, ("cid", cid.as_str()));
        }

        form_body(&params)
    }

    /// 发送 HTTP 控制命令，广播状态由独立连接接收。
    async fn send_cmd(
        &self,
        command: u32,
        oid: u64,
        extra_override: Option<Value>,
        seek_ts: u32,
    ) -> Result<(), CastError> {
        let body = self.command_body(command, oid, extra_override, seek_ts);

        // 发送请求
        let endpoint = format!("{}/x/tv/stream/cmd", API_HOST);
        #[cfg(test)]
        let endpoint = self
            .test_api
            .as_ref()
            .map(|(endpoint, _)| endpoint.clone())
            .unwrap_or(endpoint);
        let context = format!("[Bilibili] {}：command={}，设备={}，端点={}", command_name(command), command, self.device_buvid, endpoint);
        let send = || async {
            bili_client()
                .post(&endpoint)
                .header("Content-Type", "application/x-www-form-urlencoded")
                .header("User-Agent", "Mozilla/5.0 BiliDroid/8.94.0 (android)")
                .body(body.clone())
                .send()
                .await
        };
        let response = if command == 1 {
            crate::cast::request::retry_cast_request(&context, send).await
        } else {
            send().await
        }
        .map_err(|error| control_failure(&context, "发送请求", None, error))?;
        read_control_response(&context, command, response).await
    }
}

// 读取控制响应，解析JSON并检查业务响应码
async fn read_control_response(context: &str, command: u32, response: reqwest::Response) -> Result<(), CastError> {
    let status = response.status();
    let bytes = response.bytes().await
        .map_err(|error| control_failure(context, "读取响应", Some(status), error))?;
    let resp: Value = serde_json::from_slice(&bytes)
        .map_err(|error| control_failure(context, "解析JSON响应", Some(status), error))?;
    let message = resp["message"].as_str()
        .or_else(|| resp["msg"].as_str())
        .unwrap_or("未提供说明");

    log::info!("[Bilibili] send_cmd response: command={}, code={}, message={}", command, resp["code"], message);
    if resp["code"].as_i64() != Some(0) {
        Err(control_failure(context, "接口业务响应", Some(status), format!(
            "业务错误={}，说明={}", resp["code"], message
        )))
    } else {
        Ok(())
    }
}

/// 根据 command 值返回对应的操作名称，用于日志记录和错误信息中
fn command_name(command: u32) -> &'static str {
    match command {
        1 => "播放歌曲",
        4 => "定位",
        5 => "暂停",
        6 => "继续播放",
        7 => "停止播放",
        8 => "调高音量",
        9 => "设置弹幕",
        10 => "设置清晰度",
        12 => "调低音量",
        _ => "控制指令",
    }
}

/// 处理控制指令失败的情况，记录日志并返回 CastError
fn control_failure(context: &str, stage: &str, status: Option<reqwest::StatusCode>, error: impl std::fmt::Display) -> CastError {
    let http = status.map(|status| format!("，HTTP={}", status.as_u16())).unwrap_or_default();
    let message = format!("{}失败：步骤={}{}，错误={}", context, stage, http, error);
    log::warn!("{}", message);
    CastError::Device(message)
}

/// 解析歌曲引用，返回 BV 号和页码
fn parse_song_ref(s: &str) -> (String, u32) {
    if let Some((bv, page_str)) = s.split_once("-page") {
        (bv.to_string(), page_str.parse().unwrap_or(0))
    } else {
        (s.to_string(), 0)
    }
}

/// 投屏 Bilibili 视频
#[async_trait]
impl Caster for BilibiliCaster {
    
    /// 播放指定歌曲
    async fn play_song(&self, song: &SongRef) -> Result<(), CastError> {
        let (bvid, page) = parse_song_ref(&song.0);
        if !bvid.starts_with("BV") {
            return Err(CastError::Unsupported);
        }
        log::info!("[Bilibili] play_song: song_ref={}, parsed bvid={}, page={}", song.0, bvid, page);

        let aid = bv_to_aid(&bvid);
        log::debug!("[Bilibili] BV to AID: {} -> {}", bvid, aid);

        let (cid, duration) = self.page_info(&bvid, page).await.map_err(|error| {
            let message = format!("[Bilibili] 获取曲目信息失败：BV={}，page={}，错误={}", bvid, page, error);
            log::error!("{}", message);
            CastError::Device(message)
        })?;
        log::info!("[Bilibili] get_page_info: page={} -> cid={}, duration={}", page, cid, duration);

        // 获取 quality 读锁并拿到枚举，然后转为 qn 数值
        let current_quality = self.quality.load(Ordering::Relaxed);
        // 读取 AtomicBool 的当前值
        let current_danmaku = self.danmaku.load(Ordering::Relaxed);

        // extra的cid会被解析提取到外层
        self.start_broadcast();
        let session = self.broadcast.begin_play(aid, cid);
        let extra = serde_json::json!({ "cid": cid, "bizSessionId": session.id, "quality": current_quality, "danmaku_switch": current_danmaku });

        log::info!("[Bilibili] sending play command: aid={}, cid={}, page={}, extra={}", aid, cid, page, extra);
        if let Err(error) = self.send_cmd(1, aid, Some(extra), 0).await {
            self.broadcast.clear_play(&session.id);
            return Err(error);
        }
        // 本轮仅记录广播回传；进度和播放确认仍沿用 HTTP 成功后的本地计时。
        self.progress.start(duration).await;

        // 已保存状态，无需再次设置
        // 每次开始投屏都重置为默认状态：弹幕关闭、清晰度 1080P。
        // 设备侧没有读回接口，所以这里只能假定默认值，不要求成功才算播放成功。
        //if let Err(e) = self.set_danmaku(false).await {
        //    log::warn!("[Bilibili] 设置默认弹幕状态失败: {}", e);
        //}
        //if let Err(e) = self.set_quality(Quality::default()).await {
        //    log::warn!("[Bilibili] 设置默认清晰度失败: {}", e);
        //}

        Ok(())
    }

    /// 继续播放
    async fn resume(&self) -> Result<(), CastError> {
        self.progress.resume().await;
        self.send_cmd(6, 0, None, 0).await
    }

    /// 暂停播放
    async fn pause(&self) -> Result<(), CastError> {
        self.progress.pause().await;
        self.send_cmd(5, 0, None, 0).await
    }

    /// 停止播放
    async fn stop(&self) -> Result<(), CastError> {
        self.send_cmd(7, 0, None, 0).await
    }

    /// 定位到指定时间点（秒）
    async fn seek(&self, secs: u32) -> Result<(), CastError> {
        self.progress.seek(secs).await;
        self.send_cmd(4, 0, None, secs).await
    }

    /// 获取当前播放进度（秒），返回 Progress 结构体
    async fn get_progress(&self) -> Result<Progress, CastError> {
        Ok(self.progress.get_progress().await)
    }

    /// 设置音量（0-100），B站投屏没有绝对音量接口，返回 CastError::Unsupported
    async fn set_volume(&self, _volume: u32) -> Result<(), CastError> {
        Err(CastError::Unsupported)
    }

    /// 获取当前音量（0-100），B站投屏没有绝对音量接口，返回 None
    async fn get_volume(&self) -> Result<Option<u32>, CastError> {
        Ok(None)
    }

    // B站投屏没有"绝对音量"接口，只有设备侧按固定步长调整的相对指令：
    // command=8 音量+，command=12 音量-（与 main.py 的 cmd 8 / 12 一致）。
    /// 增大音量
    async fn volume_up(&self, _step: u32) -> Result<(), CastError> {
        self.send_cmd(8, 0, None, 0).await
    }

    async fn volume_down(&self, _step: u32) -> Result<(), CastError> {
        self.send_cmd(12, 0, None, 0).await
    }

    // command=9：弹幕开关，与 main.py 的 cmd 9 一致。
    async fn set_danmaku(&self, on: bool) -> Result<(), CastError> {
        self.send_cmd(9, 0, Some(serde_json::json!({ "danmaku_switch": on })), 0).await?;
        self.danmaku.store(on, Ordering::Relaxed);
        Ok(())
    }
    fn get_danmaku(&self) -> Result<bool, CastError> {
        Ok(self.danmaku.load(Ordering::Relaxed))
    }

    // command=10：切换清晰度，与 main.py 的 cmd 10 一致。
    async fn set_quality(&self, quality: Quality) -> Result<(), CastError> {
        let extra = serde_json::json!({
            "qn": { "currentQn": { "quality": quality.as_qn() }, "userDesireQn": 0 }
        });
        self.send_cmd(10, 0, Some(extra), 0).await?;
        self.quality.store(quality.as_qn(), Ordering::Relaxed);
        Ok(())
    }

    /// 获取当前清晰度
    fn get_quality(&self) -> Result<Quality, CastError> {
        Ok(Quality::from_qn(self.quality.load(Ordering::Relaxed)).unwrap_or_default())
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            absolute_volume: false,
            seek: true,
            hardware_progress: false,
        }
    }

    fn stop_status_subscription(&self) {
        self.broadcast.stop_status_subscription();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn control_errors_preserve_stage_http_status_and_business_message() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for (status, body, declared_length, expected) in [
            (200, r#"{"code":-101,"message":"账号未登录"}"#, None, "步骤=接口业务响应，HTTP=200，错误=业务错误=-101，说明=账号未登录"),
            (500, r#"{"code":-400,"msg":"参数错误"}"#, None, "步骤=接口业务响应，HTTP=500，错误=业务错误=-400，说明=参数错误"),
            (502, "bad gateway", None, "步骤=解析JSON响应，HTTP=502"),
            (200, "{", Some(100), "步骤=读取响应，HTTP=200"),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("http://{}/x/tv/stream/cmd", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                stream.read(&mut request).await.unwrap();
                let response = format!("HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", declared_length.unwrap_or(body.len()));
                stream.write_all(response.as_bytes()).await.unwrap();
            });
            let context = format!("[Bilibili] 播放歌曲：command=1，设备=test-device，端点={endpoint}");
            let response = reqwest::Client::builder().no_proxy().build().unwrap()
                .post(&endpoint).send().await.unwrap();
            let error = read_control_response(&context, 1, response).await.unwrap_err().to_string();
            server.await.unwrap();
            assert!(error.contains(&context), "{error}");
            assert!(error.contains(expected), "{error}");
        }
    }
}
