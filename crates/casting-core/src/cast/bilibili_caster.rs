use std::sync::{Arc, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
// use tokio::sync::RwLock;
use crate::bilibili_parser::{bv_to_aid, get_page_info};
// use crate::cast;
use super::progress::LocalProgressTracker;
use super::{Capabilities, CastError, Caster, Progress, Quality, SongRef};

static SESSION_DIR: OnceLock<String> = OnceLock::new();

const APPKEY: &str = "4409e2ce8ffd12b8";
const APP_SECRET: &str = "59b43e04ad6965f34319062b478f83dd";
const DEVICE_ID: &str = "5R84Y6H5NB1VSDWTM9B2G56";
const BUVID: &str = "CLI54H1B5DHS5GB4RS5GS5G6E";
const UA: &str = "Mozilla/5.0 BiliTV/1.8.5 os/android model/OPD2404 mobi_app/android_tv_yst build/108500 channel/dangbei innerVer/108500 osVer/16 network/2";
const PASSPORT_HOST: &str = "https://passport.snm0516.aisee.tv";
const API_HOST: &str = "https://api.bilibili.com";
pub const TOKEN_FILE: &str = "bili_session.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BilibiliSession {
    pub access_token: String,
    pub mid: u64,
    #[serde(default)]
    pub expires_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BilibiliDevice {
    pub name: String,
    pub brand: String,
    pub model: String,
    pub buvid: String,
}

impl std::fmt::Display for BilibiliDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({}-{}) [{}]", self.name, self.brand, self.model, self.buvid)
    }
}

fn now_ts() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

fn base_params() -> Vec<(String, String)> {
    vec![
        ("appkey".into(), APPKEY.into()),
        ("bili_local_id".into(), DEVICE_ID.into()),
        ("build".into(), "108500".into()),
        ("buvid".into(), BUVID.into()),
        ("channel".into(), "dangbei".into()),
        ("device_id".into(), DEVICE_ID.into()),
        ("extend".into(), r#"{"option":"4"}"#.into()),
        ("local_id".into(), BUVID.into()),
        ("mobi_app".into(), "android_tv_yst".into()),
        ("platform".into(), "android".into()),
        ("statistics".into(), r#"{"appId":"18","platform":"3","version":"108500"}"#.into()),
        ("ts".into(), now_ts().to_string()),
    ]
}

fn sign_params(params: &[(String, String)]) -> String {
    let mut sorted = params.to_vec();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let parts: Vec<String> = sorted
        .iter()
        .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
        .collect();
    let body = parts.join("&");
    let digest = md5::compute(format!("{}{}", body, APP_SECRET));
    format!("{}&sign={:x}", body, digest)
}

fn form_body(params: &[(&str, &str)]) -> String {
    params
        .iter()
        .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

fn bili_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(UA)
            .timeout(crate::API_TIMEOUT)
            .build()
            .expect("reqwest client")
    })
}

/// Login via QR code. `on_qr(url)` is called once with the QR URL to display.
pub async fn login_qr(on_qr: impl Fn(String) + Send + Sync) -> Result<BilibiliSession, String> {
    let client = bili_client();
    let params = base_params();
    let body = sign_params(&params);

    let json: Value = client
        .post(format!("{}/x/passport-tv-login/qrcode/auth_code", PASSPORT_HOST))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(|e| format!("auth_code: {}", e))?
        .json()
        .await
        .map_err(|e| format!("auth_code parse: {}", e))?;

    if json["code"].as_i64() != Some(0) {
        return Err(format!("auth_code error: {}", json["message"]));
    }
    let qr_url = json["data"]["url"].as_str().ok_or("no url")?.to_string();
    let auth_code = json["data"]["auth_code"].as_str().ok_or("no auth_code")?.to_string();

    on_qr(qr_url);

    loop {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        let mut poll_params = base_params();
        poll_params.push(("auth_code".into(), auth_code.clone()));
        let poll_body = sign_params(&poll_params);

        let poll: Value = client
            .post(format!("{}/x/passport-tv-login/qrcode/poll", PASSPORT_HOST))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(poll_body)
            .send()
            .await
            .map_err(|e| format!("poll: {}", e))?
            .json()
            .await
            .map_err(|e| format!("poll parse: {}", e))?;

        match poll["code"].as_i64() {
            Some(0) => {
                let token = poll["data"]["token_info"]["access_token"]
                    .as_str()
                    .ok_or("no access_token")?
                    .to_string();
                let mid = poll["data"]["mid"].as_u64().ok_or("no mid")?;
                let expires_in = poll["data"]["token_info"]["expires_in"]
                    .as_u64()
                    .unwrap_or(7776000);
                let expires_at = Some(now_ts() + expires_in);
                let session = BilibiliSession { access_token: token, mid, expires_at };
                save_session(&session)?;
                return Ok(session);
            }
            Some(86038) => return Err("QR code expired".into()),
            _ => log::debug!("bili poll: {} {}", poll["code"], poll["message"]),
        }
    }
}

/// Initialize session directory (call from Android/CLI startup)
pub fn init_session_dir(dir: &str) {
    let _ = SESSION_DIR.set(dir.to_string());
    log::info!("[Bilibili] Session 目录已初始化: {}", dir);
}

fn get_session_file() -> PathBuf {
    let dir = SESSION_DIR.get().map(|s| s.as_str()).unwrap_or(".");
    PathBuf::from(dir).join(TOKEN_FILE)
}

fn remove_session_file(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            log::info!("[Bilibili] Session 文件不存在，视为已清除: {}", path.display());
            Ok(())
        }
        Err(e) => {
            let err_msg = format!("清除 session 失败: {} (路径: {})", e, path.display());
            log::error!("[Bilibili] {}", err_msg);
            Err(err_msg)
        }
    }
}

fn is_auth_error(code: i64, message: &str) -> bool {
    code == -101
        || message.contains("未登录")
        || message.to_ascii_lowercase().contains("access_key")
        || message.to_ascii_lowercase().contains("token")
}

fn parse_devices_response(json: Value) -> Result<Vec<BilibiliDevice>, String> {
    if let Some(code) = json["code"].as_i64() {
        if code != 0 {
            let message = json["message"]
                .as_str()
                .or_else(|| json["msg"].as_str())
                .unwrap_or("unknown error");
            let prefix = if is_auth_error(code, message) {
                "auth expired"
            } else {
                "api error"
            };
            return Err(format!("{prefix}: code={code}, message={message}"));
        }
    }

    let Some(arr) = json["data"]["devices"].as_array() else {
        return Err("no devices array".into());
    };

    Ok(arr
        .iter()
        .filter_map(|d| {
            Some(BilibiliDevice {
                name: d["name"].as_str()?.into(),
                brand: d["brand"].as_str()?.into(),
                model: d["model"].as_str()?.into(),
                buvid: d["buvid"].as_str()?.into(),
            })
        })
        .collect())
}

pub fn save_session(session: &BilibiliSession) -> Result<(), String> {
    let json = serde_json::to_string_pretty(session).map_err(|e| e.to_string())?;
    let path = get_session_file();
    std::fs::write(&path, json).map_err(|e| {
        let err_msg = format!("保存 session 失败: {} (路径: {})", e, path.display());
        log::error!("[Bilibili] {}", err_msg);
        err_msg
    })?;
    log::info!("[Bilibili] Session 已保存: {}", path.display());
    Ok(())
}

/// Load saved session. Supports both our flat format and the Python nested format.
pub fn load_session() -> Option<BilibiliSession> {
    let path = get_session_file();
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => {
            log::info!("[Bilibili] 从 {} 读取 session", path.display());
            t
        }
        Err(e) => {
            log::debug!("[Bilibili] Session 文件不存在或无法读取: {} (路径: {})", e, path.display());
            return None;
        }
    };
    let v: Value = serde_json::from_str(&text).ok()?;
    // Flat format (our own): {"access_token": "...", "mid": 123, "expires_at": ...}
    if let (Some(t), Some(m)) = (v["access_token"].as_str(), v["mid"].as_u64()) {
        let expires_at = v["expires_at"].as_u64();
        log::info!("[Bilibili] 成功加载 session (UID: {}, expires_at: {:?})", m, expires_at);
        return Some(BilibiliSession { access_token: t.into(), mid: m, expires_at });
    }
    // Python format: {"data": {"token_info": {"access_token": "..."}, "mid": 123}}
    if let (Some(t), Some(m)) = (
        v["data"]["token_info"]["access_token"].as_str(),
        v["data"]["mid"].as_u64(),
    ) {
        log::info!("[Bilibili] 成功加载 session (Python 格式, UID: {})", m);
        return Some(BilibiliSession { access_token: t.into(), mid: m, expires_at: None });
    }
    log::warn!("[Bilibili] Session 格式无法识别");
    None
}

pub fn is_session_expired(session: &BilibiliSession) -> bool {
    match session.expires_at {
        Some(expires_at) => {
            let current_ts = now_ts();
            let expired = current_ts >= expires_at;
            if expired {
                log::warn!("[Bilibili] Token 已过期: now={}, expires_at={}", current_ts, expires_at);
            }
            expired
        }
        None => false,
    }
}

pub fn clear_session() -> Result<(), String> {
    let path = get_session_file();
    remove_session_file(&path)?;
    log::info!("[Bilibili] Session 已清除: {}", path.display());
    Ok(())
}

pub async fn list_devices(session: &BilibiliSession) -> Result<Vec<BilibiliDevice>, String> {
    let endpoint = format!("{}/x/tv/projection/devices", API_HOST);
    let started = Instant::now();
    let response = bili_client()
        .get(&endpoint)
        .query(&[
            ("access_key", session.access_token.as_str()),
            ("appkey", APPKEY),
        ])
        .send()
        .await
        .map_err(|error| devices_request_failure("发送请求", &endpoint, started, None, error))?;
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .map_err(|error| devices_request_failure("读取响应", &endpoint, started, Some(status), error))?;
    let json: Value = serde_json::from_slice(&bytes).map_err(|error| format!(
        "[Bilibili] 获取设备列表失败：步骤=解析JSON响应，请求=GET {endpoint}，耗时={}ms，超时设置={}ms，HTTP={}，错误链={error}",
        started.elapsed().as_millis(), crate::API_TIMEOUT.as_millis(), status.as_u16(),
    ))?;

    parse_devices_response(json)
}

fn devices_request_failure(
    stage: &str,
    endpoint: &str,
    started: Instant,
    status: Option<reqwest::StatusCode>,
    error: reqwest::Error,
) -> String {
    // reqwest's URL contains access_key; remove it before formatting the error chain.
    let error = error.without_url();
    let mut causes = vec![error.to_string()];
    let mut source = std::error::Error::source(&error);
    while let Some(cause) = source {
        causes.push(cause.to_string());
        source = cause.source();
    }
    let http = status.map(|status| format!("，HTTP={}", status.as_u16())).unwrap_or_default();
    format!(
        "[Bilibili] 获取设备列表失败：步骤={stage}，请求=GET {endpoint}，耗时={}ms，超时设置={}ms{http}，is_timeout={}，is_connect={}，is_request={}，is_body={}，is_decode={}，错误链={}",
        started.elapsed().as_millis(),
        crate::API_TIMEOUT.as_millis(),
        error.is_timeout(),
        error.is_connect(),
        error.is_request(),
        error.is_body(),
        error.is_decode(),
        causes.join(" -> "),
    )
}

pub struct BilibiliCaster {
    access_token: String,
    mid: u64,
    device_buvid: String,
    progress: Arc<LocalProgressTracker>,
    // 保存配置项
    quality: AtomicU32, // 使用 AtomicU32 保存清晰度的数值表示
    danmaku: AtomicBool,
}

impl BilibiliCaster {
    pub fn new(session: BilibiliSession, device_buvid: String) -> Self {
        Self {
            access_token: session.access_token,
            mid: session.mid,
            device_buvid,
            progress: LocalProgressTracker::new(),
            quality: AtomicU32::new(Quality::default().as_qn()) ,
            danmaku: AtomicBool::new(false),
        }
    }

    async fn send_cmd(
        &self,
        command: u32,
        oid: u64,
        extra_override: Option<Value>,
        seek_ts: u32,
    ) -> Result<(), CastError> {
        let device_info = serde_json::json!({
            "ott_buvid": self.device_buvid,
            "ott_mid": self.mid,
            "ott_build": 108500,
            "ott_mobi_app": "android_tv_yst"
        })
        .to_string();

        let mut full_extra = serde_json::json!({
            "accessKey": self.access_token.as_str(),
            "autoNext": false, "biz_id": 6213332, "oid": oid,
            "userDesireSpeed": 1.0, "type": 101, "userVipInfo": 1
        });
        if let (Some(obj), Some(extra)) = (full_extra.as_object_mut(), extra_override.as_ref().and_then(|v| v.as_object())) {
            for (k, v) in extra {
                obj.insert(k.clone(), v.clone());
            }
        }

        let cmd_str = command.to_string();
        let seek_str = seek_ts.to_string();
        let ts_str = now_ts().to_string();

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
            ("video_type", "4"),
            ("room_id", "0"),
            ("mobi_app", "android"),
            ("platform", "android"),
            ("s_locale", "zh-Hans_CN"),
        ];

        // 如果存在cid，作为顶级参数加入
        if let Some(ref cid) = cid_str {
            params.insert(8, ("cid", cid.as_str()));
        }

        let body = form_body(&params);

        let endpoint = format!("{}/x/tv/stream/cmd", API_HOST);
        let context = format!("[Bilibili] {}：command={}，设备={}，端点={}", command_name(command), command, self.device_buvid, endpoint);
        let send = || async {
            bili_client()
                .post(&endpoint)
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(body.clone())
                .send()
                .await
        };
        let response = if command == 1 {
            super::retry_cast_request(&context, send).await
        } else {
            send().await
        }
        .map_err(|error| control_failure(&context, "发送请求", None, error))?;
        read_control_response(&context, command, response).await
    }
}

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

fn control_failure(context: &str, stage: &str, status: Option<reqwest::StatusCode>, error: impl std::fmt::Display) -> CastError {
    let http = status.map(|status| format!("，HTTP={}", status.as_u16())).unwrap_or_default();
    let message = format!("{}失败：步骤={}{}，错误={}", context, stage, http, error);
    log::warn!("{}", message);
    CastError::Device(message)
}

fn parse_song_ref(s: &str) -> (String, u32) {
    if let Some((bv, page_str)) = s.split_once("-page") {
        (bv.to_string(), page_str.parse().unwrap_or(0))
    } else {
        (s.to_string(), 0)
    }
}

#[async_trait]
impl Caster for BilibiliCaster {
    async fn play_song(&self, song: &SongRef) -> Result<(), CastError> {
        let (bvid, page) = parse_song_ref(&song.0);
        if !bvid.starts_with("BV") {
            return Err(CastError::Unsupported);
        }
        log::info!("[Bilibili] play_song: song_ref={}, parsed bvid={}, page={}", song.0, bvid, page);

        let aid = bv_to_aid(&bvid);
        log::debug!("[Bilibili] BV to AID: {} -> {}", bvid, aid);

        let (cid, duration) = get_page_info(&bvid, page).await.map_err(|error| {
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
        let extra = serde_json::json!({ "oid": aid, "cid": cid, "type": 101, "quality": current_quality, "danmaku_switch": current_danmaku });

        log::info!("[Bilibili] sending play command: aid={}, cid={}, page={}, extra={}", aid, cid, page, extra);
        self.send_cmd(1, aid, Some(extra), 0).await?;
        // The cloud API response is our only playback acknowledgement. Start the
        // simulated clock only after success, including clearing stale duration.
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

    async fn resume(&self) -> Result<(), CastError> {
        self.progress.resume().await;
        self.send_cmd(6, 0, None, 0).await
    }

    async fn pause(&self) -> Result<(), CastError> {
        self.progress.pause().await;
        self.send_cmd(5, 0, None, 0).await
    }

    async fn stop(&self) -> Result<(), CastError> {
        self.send_cmd(7, 0, None, 0).await
    }

    async fn seek(&self, secs: u32) -> Result<(), CastError> {
        self.progress.seek(secs).await;
        self.send_cmd(4, 0, None, secs).await
    }

    async fn get_progress(&self) -> Result<Progress, CastError> {
        Ok(self.progress.get_progress().await)
    }

    async fn set_volume(&self, _volume: u32) -> Result<(), CastError> {
        Err(CastError::Unsupported)
    }

    async fn get_volume(&self) -> Result<Option<u32>, CastError> {
        Ok(None)
    }

    // B站投屏没有"绝对音量"接口，只有设备侧按固定步长调整的相对指令：
    // command=8 音量+，command=12 音量-（与 main.py 的 cmd 8 / 12 一致）。
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
}

#[cfg(test)]
mod tests {
    use super::{parse_devices_response, read_control_response, remove_session_file};
    use serde_json::json;
    use std::path::Path;

    #[test]
    fn clear_session_treats_missing_file_as_success() {
        let path = Path::new("/tmp/ktv-casting-nonexistent-session.json");
        assert!(remove_session_file(path).is_ok());
    }

    #[test]
    fn parse_devices_response_surfaces_auth_expiry() {
        let err = parse_devices_response(json!({
            "code": -101,
            "message": "账号未登录"
        }))
        .unwrap_err();

        assert!(err.starts_with("auth expired:"));
        assert!(err.contains("账号未登录"));
    }

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
