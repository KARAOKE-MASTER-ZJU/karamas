//! DLNA 设备发现、协议控制和投屏实现。
mod caster;
mod controller;
pub mod settings;

pub use caster::DlnaCaster;
pub use controller::{DlnaController, DlnaDevice};

use crate::error::CoreError;

/// 无设备时返回空列表，搜索失败时返回错误。
pub async fn discover_devices_core() -> Result<Vec<DlnaDevice>, CoreError> {
    Ok(DlnaController::new().discover_devices().await?)
}

/// 搜索过程中逐个通知可用设备，完成后返回完整列表。
pub async fn discover_devices_with_updates_core(
    on_device: impl FnMut(&DlnaDevice),
) -> Result<Vec<DlnaDevice>, CoreError> {
    Ok(DlnaController::new()
        .discover_devices_with_updates(on_device)
        .await?)
}

pub async fn discover_device_from_url_core(url: String) -> Result<DlnaDevice, CoreError> {
    Ok(DlnaController::new().get_device_from_url(&url).await?)
}
