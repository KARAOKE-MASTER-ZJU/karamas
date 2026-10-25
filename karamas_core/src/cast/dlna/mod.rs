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

pub async fn discover_device_from_url_core(url: String) -> Result<DlnaDevice, CoreError> {
    Ok(DlnaController::new().get_device_from_url(&url).await?)
}
