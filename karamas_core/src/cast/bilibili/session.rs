use serde::{Deserialize, Serialize};
use std::{
    sync::RwLock,
    time::{SystemTime, UNIX_EPOCH},
};

static SESSION: RwLock<Option<BilibiliSession>> = RwLock::new(None);

/// Bilibili 投屏会话，持久化由宿主负责。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BilibiliSession {
    pub access_token: String,
    pub mid: u64,
    #[serde(default)]
    pub expires_at: Option<u64>,
}

pub(super) fn now_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// 更新当前登录会话，供媒体解析使用；退出登录时传入 None。
pub fn set_session(session: Option<BilibiliSession>) {
    *SESSION.write().unwrap() = session;
}

/// 读取当前登录会话。啊
pub fn load_session() -> Option<BilibiliSession> {
    SESSION.read().unwrap().clone()
}

/// 检查 session 是否过期
pub fn is_session_expired(session: &BilibiliSession) -> bool {
    match session.expires_at {
        Some(expires_at) => {
            let current_ts = now_ts();
            let expired = current_ts >= expires_at;
            if expired {
                log::warn!(
                    "[Bilibili] Token 已过期: now={}, expires_at={}",
                    current_ts,
                    expires_at
                );
            }
            expired
        }
        None => false,
    }
}
