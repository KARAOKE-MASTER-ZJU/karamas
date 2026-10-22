use crate::{
    error::{CommandError, CommandResult},
    models::DlnaDeviceInfo,
};
use casting_core::cast::bilibili::{self, BilibiliDevice, BilibiliSession};

/// 搜索局域网 DLNA 设备，保留空列表和搜索失败的区别。
#[tauri::command]
pub async fn discover_dlna_devices() -> CommandResult<Vec<DlnaDeviceInfo>> {
    Ok(casting_core::discover_devices_core()
        .await?
        .into_iter()
        .map(Into::into)
        .collect())
}

/// 通过设备描述文件地址获取 DLNA 设备。
#[tauri::command]
pub async fn discover_dlna_device(url: String) -> CommandResult<DlnaDeviceInfo> {
    super::session::http_url(&url)?;
    Ok(casting_core::discover_device_from_url_core(url)
        .await?
        .into())
}

/// 使用调用方提供的会话查询 B站在线投屏设备。
#[tauri::command]
pub async fn list_bilibili_devices(session: BilibiliSession) -> CommandResult<Vec<BilibiliDevice>> {
    bilibili::list_devices(&session)
        .await
        .map_err(|e| CommandError::new("BILIBILI_ERROR", e))
}
