//! B 站会话加密存储和旧明文文件迁移。
mod file;
mod legacy;

use crate::error::{CommandError, CommandResult};
use file::Store;
use karamas_core::cast::bilibili::BilibiliSession;
use std::path::Path;

fn decode(value: &str) -> CommandResult<BilibiliSession> {
    let session: BilibiliSession = serde_json::from_str(value)
        .map_err(|_| CommandError::new("SESSION_FORMAT", "保存的登录会话格式无效，请重新登录。"))?;
    if session.access_token.is_empty() {
        return Err(CommandError::new(
            "SESSION_FORMAT",
            "登录凭据为空，请重新登录。",
        ));
    }
    Ok(session)
}

async fn write_verified(store: &Store, session: &BilibiliSession) -> CommandResult<()> {
    let value = serde_json::to_string(session)
        .map_err(|_| CommandError::new("SESSION_FORMAT", "无法编码登录会话。"))?;
    store.write(value.clone()).await?;
    if store.read().await?.as_deref() != Some(value.as_str()) {
        return Err(CommandError::new(
            "STORAGE_ERROR",
            "登录会话写入后校验失败，请重试。",
        ));
    }
    Ok(())
}

pub async fn load(data_dir: &Path) -> CommandResult<Option<BilibiliSession>> {
    let store = Store::new(data_dir);
    if let Some(value) = store.read().await? {
        let session = decode(&value)?;
        legacy::remove(data_dir).await?;
        return Ok(Some(session));
    }
    // TODO(next-major): 移除明文会话迁移分支和 legacy 模块，仅保留加密会话读取。
    let Some(session) = legacy::read(data_dir).await? else {
        return Ok(None);
    };
    write_verified(&store, &session).await?;
    legacy::remove(data_dir).await?;
    log::info!("[Bilibili] 旧会话已迁入加密存储，明文文件已删除");
    Ok(Some(session))
}

pub async fn save(data_dir: &Path, session: &BilibiliSession) -> CommandResult<()> {
    let store = Store::new(data_dir);
    write_verified(&store, session).await?;
    legacy::remove(data_dir).await
}

pub async fn clear(data_dir: &Path) -> CommandResult<()> {
    let store = Store::new(data_dir);
    // 退出时一并清理迁移源。
    legacy::remove(data_dir).await?;
    store.delete().await
}
