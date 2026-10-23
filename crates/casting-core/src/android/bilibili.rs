use jni::sys::{jboolean, jint, jstring};
use jni::{
    JNIEnv,
    objects::{JClass, JString},
};
use std::sync::RwLock;

/// JNI 登录状态，供 Android 界面轮询。
pub static BILI_AUTH_STATE: RwLock<BiliAuthState> = RwLock::new(BiliAuthState::Idle);

pub enum BiliAuthState {
    Idle,
    AwaitingScan { qr_url: String },
    Success(crate::cast::bilibili::BilibiliSession),
    Error(String),
}

fn set_bili_auth_state(next: BiliAuthState) {
    if let Ok(mut state) = BILI_AUTH_STATE.write() {
        *state = next;
    }
}

fn reset_bili_auth_state() {
    set_bili_auth_state(BiliAuthState::Idle);
}

pub(super) fn current_bilibili_session() -> Option<crate::cast::bilibili::BilibiliSession> {
    let state = BILI_AUTH_STATE.read().ok()?;
    match &*state {
        BiliAuthState::Success(session) => Some(session.clone()),
        _ => None,
    }
}

/// 启动B站扫码登录（异步后台）。登录状态通过 getBilibiliAuthState() 查询。
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_startBilibiliQrLogin(
    mut env: JNIEnv,
    _class: JClass,
    sender_buvid: JString,
) {
    use crate::cast::bilibili::{login_qr, save_session};

    let sender_buvid: String = env.get_string(&sender_buvid).unwrap().into();

    reset_bili_auth_state();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let result = login_qr(&sender_buvid, |qr_url| {
                set_bili_auth_state(BiliAuthState::AwaitingScan { qr_url });
            })
            .await
            .and_then(|session| {
                save_session(&session)?;
                Ok(session)
            });
            match result {
                Ok(session) => set_bili_auth_state(BiliAuthState::Success(session)),
                Err(e) => set_bili_auth_state(BiliAuthState::Error(e)),
            }
        });
    });
}

/// 返回当前的扫码 URL（可用于在 Android 侧渲染二维码）。
/// 未就绪时返回空字符串。
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_getBilibiliQrUrl(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let url = if let Ok(s) = BILI_AUTH_STATE.read() {
        match &*s {
            BiliAuthState::AwaitingScan { qr_url } => qr_url.clone(),
            _ => String::new(),
        }
    } else {
        String::new()
    };
    env.new_string(url).unwrap().into_raw()
}

/// 登录状态：0=等待扫码，1=成功，-1=失败/过期，-2=未开始
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_getBilibiliLoginStatus(
    _env: JNIEnv,
    _class: JClass,
) -> jint {
    if let Ok(s) = BILI_AUTH_STATE.read() {
        match &*s {
            BiliAuthState::Idle => -2,
            BiliAuthState::AwaitingScan { .. } => 0,
            BiliAuthState::Success(_) => 1,
            BiliAuthState::Error(_) => -1,
        }
    } else {
        -1
    }
}

/// 列出在线的B站投屏设备，返回 JSON 字符串（数组）。
/// 需要已登录（getBilibiliLoginStatus() == 1）。
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_listBilibiliDevices(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    use crate::cast::bilibili::{clear_session, list_devices};

    let session = current_bilibili_session();
    let json = if let Some(session) = session {
        let rt = tokio::runtime::Runtime::new().unwrap();
        match rt.block_on(list_devices(&session)) {
            Ok(devs) => serde_json::to_string(&devs).unwrap_or_else(|_| "[]".into()),
            Err(e) => {
                if e.starts_with("auth expired:") {
                    log::warn!("[Bilibili] 设备列表鉴权失效，清理当前会话: {}", e);
                    let _ = clear_session();
                    set_bili_auth_state(BiliAuthState::Error("session expired".into()));
                }
                log::error!("listBilibiliDevices: {}", e);
                "[]".into()
            }
        }
    } else {
        "[]".into()
    };

    env.new_string(json).unwrap().into_raw()
}

/// 检查 B 站 token 是否已过期。返回 true 表示已过期，false 表示有效或无法判断。
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_isBilibiliSessionExpired(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    use crate::cast::bilibili::{is_session_expired, load_session};

    if let Some(session) = load_session() {
        if is_session_expired(&session) {
            log::warn!("[Bilibili] Token 已过期，清除 session");
            let _ = crate::cast::bilibili::clear_session();
            set_bili_auth_state(BiliAuthState::Error("session expired".into()));
            return jboolean::from(true);
        }
    }
    jboolean::from(false)
}

/// 清除保存的 B 站 session。
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_clearBilibiliSession(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    use crate::cast::bilibili::clear_session;

    match clear_session() {
        Ok(_) => {
            reset_bili_auth_state();
            jboolean::from(true)
        }
        Err(e) => {
            log::error!("[Bilibili] 清除 session 失败: {}", e);
            jboolean::from(false)
        }
    }
}

/// 返回当前登录 Session 的 JSON 字符串，供 Android 持久化到 EncryptedSharedPreferences。
/// 未登录时返回空字符串。
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_getBilibiliSessionJson(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let json = current_bilibili_session()
        .and_then(|session| serde_json::to_string(&session).ok())
        .unwrap_or_default();
    env.new_string(json).unwrap().into_raw()
}

/// 从 Android 传入的 JSON 字符串恢复 Session（应用启动时调用）。
/// 成功返回 1，JSON 无效返回 0。
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_restoreBilibiliSession(
    mut env: JNIEnv,
    _class: JClass,
    json: JString,
) -> jint {
    use crate::cast::bilibili::{clear_session, is_session_expired, save_session, BilibiliSession};

    let json_str: String = env.get_string(&json).unwrap().into();
    if let Ok(session) = serde_json::from_str::<BilibiliSession>(&json_str) {
        if is_session_expired(&session) {
            log::warn!("[Bilibili] 忽略已过期的恢复会话");
            let _ = clear_session();
            reset_bili_auth_state();
            return 0;
        }
        if let Err(e) = save_session(&session) {
            log::error!("[Bilibili] 恢复会话落盘失败: {}", e);
            reset_bili_auth_state();
            return 0;
        }

        set_bili_auth_state(BiliAuthState::Success(session));
        return 1;
    }

    0
}
