use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
    time::{SystemTime, UNIX_EPOCH},
};

static SESSION_DIR: OnceLock<String> = OnceLock::new();
pub const TOKEN_FILE: &str = "bili_session.json";

/// Bilibili 投屏会话
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BilibiliSession {
    pub access_token: String,
    pub mid: u64,
    #[serde(default)]
    pub expires_at: Option<u64>,
}

pub(super) fn now_ts() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

/// Initialize session directory (call from Android/CLI startup)
pub fn init_session_dir(dir: &str) {
    let _ = SESSION_DIR.set(dir.to_string());
    log::info!("[Bilibili] Session 目录已初始化: {}", dir);
}

/// 获取 session 文件路径
fn get_session_file() -> PathBuf {
    let dir = SESSION_DIR.get().map(|s| s.as_str()).unwrap_or(".");
    PathBuf::from(dir).join(TOKEN_FILE)
}

/// 删除 session 文件
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

/// 保存 session 到文件
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

/// 加载已保存的 session
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

/// 检查 session 是否过期
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

/// 清除 session 文件
pub fn clear_session() -> Result<(), String> {
    let path = get_session_file();
    remove_session_file(&path)?;
    log::info!("[Bilibili] Session 已清除: {}", path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_session_treats_missing_file_as_success() {
        let path = Path::new("/tmp/ktv-casting-nonexistent-session.json");
        assert!(remove_session_file(path).is_ok());
    }
}
