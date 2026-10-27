use crate::{cast, error::CoreError};
use std::sync::atomic::Ordering;

// 清晰度与取源方式放在同一份状态中，切换时一起更新。
static DLNA_MEDIA_QUALITY: std::sync::Mutex<(cast::Quality, cast::MediaFormat)> =
    std::sync::Mutex::new((cast::Quality::P720, cast::MediaFormat::Auto));
static DLNA_DIRECT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 设置 DLNA 直连模式
pub fn set_dlna_direct(enabled: bool) {
    DLNA_DIRECT.store(enabled, Ordering::Relaxed);
    if enabled {
        let mut selected = DLNA_MEDIA_QUALITY.lock().unwrap();
        *selected = (cast::Quality::P720, cast::MediaFormat::Auto);
    }
}

/// 获取 DLNA 直连模式
pub fn get_dlna_direct() -> bool {
    DLNA_DIRECT.load(Ordering::Relaxed)
}

/// 设置 DLNA 清晰度
pub fn set_dlna_quality(quality: cast::Quality) -> Result<(), CoreError> {
    set_dlna_media_quality(quality, cast::MediaFormat::Auto)
}

/// 获取 DLNA 清晰度
pub fn get_dlna_quality() -> cast::Quality {
    get_dlna_media_quality().0
}

pub fn get_dlna_media_quality() -> (cast::Quality, cast::MediaFormat) {
    *DLNA_MEDIA_QUALITY.lock().unwrap()
}

pub fn set_dlna_media_quality(
    quality: cast::Quality,
    format: cast::MediaFormat,
) -> Result<(), CoreError> {
    if get_dlna_direct() && format == cast::MediaFormat::Dash {
        return Err("混流需要启用 DLNA 媒体代理".into());
    }
    *DLNA_MEDIA_QUALITY.lock().unwrap() = (quality, format);
    Ok(())
}
