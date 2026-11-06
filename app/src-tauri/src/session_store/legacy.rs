//! TODO(next-major): 下次主版本更新移除本模块及所有调用；仅用于迁移旧明文会话。
use super::decode;
use crate::error::{CommandError, CommandResult};
use karamas_core::cast::bilibili::BilibiliSession;
use std::{io::ErrorKind, path::Path};

const FILE: &str = "bili_session.json";

pub(super) async fn read(data_dir: &Path) -> CommandResult<Option<BilibiliSession>> {
    let value = match tokio::fs::read_to_string(data_dir.join(FILE)).await {
        Ok(value) => value,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(_) => {
            return Err(CommandError::new(
                "STORAGE_ERROR",
                "无法读取旧登录会话，请检查应用数据目录的权限。",
            ));
        }
    };
    // 兼容旧应用和历史 Python 会话格式。
    if let Ok(session) = decode(&value) {
        return Ok(Some(session));
    }
    let value: serde_json::Value = serde_json::from_str(&value)
        .map_err(|_| CommandError::new("SESSION_FORMAT", "旧登录会话格式无效，请重新登录。"))?;
    match (
        value["data"]["token_info"]["access_token"].as_str(),
        value["data"]["mid"].as_u64(),
    ) {
        (Some(token), Some(mid)) if !token.is_empty() => Ok(Some(BilibiliSession {
            access_token: token.to_owned(),
            mid,
            expires_at: None,
        })),
        _ => Err(CommandError::new(
            "SESSION_FORMAT",
            "旧登录会话格式无效，请重新登录。",
        )),
    }
}

pub(super) async fn remove(data_dir: &Path) -> CommandResult<()> {
    match tokio::fs::remove_file(data_dir.join(FILE)).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(_) => Err(CommandError::new(
            "STORAGE_ERROR",
            "无法删除旧明文会话，请检查应用数据目录的权限。",
        )),
    }
}
