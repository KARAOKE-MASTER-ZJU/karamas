use crate::{
    cast::{MediaFormat, Quality},
    error::CoreError,
};
use std::sync::atomic::{AtomicBool, Ordering};

struct DlnaQuality {
    preferred: (Quality, MediaFormat),
    actual: Option<(Quality, MediaFormat)>,
}

static DLNA_QUALITY: std::sync::Mutex<DlnaQuality> = std::sync::Mutex::new(DlnaQuality {
    preferred: (Quality::P720, MediaFormat::Mp4),
    actual: None,
});
static DLNA_DIRECT: AtomicBool = AtomicBool::new(false);

/// 代理开关只影响可用点，不覆盖保存的清晰度偏好。
pub fn set_dlna_direct(enabled: bool) {
    DLNA_DIRECT.store(enabled, Ordering::Relaxed);
}

pub fn get_dlna_direct() -> bool {
    DLNA_DIRECT.load(Ordering::Relaxed)
}

pub fn reset_dlna_media_quality() {
    *DLNA_QUALITY.lock().unwrap() = DlnaQuality {
        preferred: (Quality::P720, MediaFormat::Mp4),
        actual: None,
    };
}

pub fn get_dlna_media_quality() -> (Quality, MediaFormat) {
    DLNA_QUALITY.lock().unwrap().preferred
}

pub fn set_dlna_media_quality(quality: Quality, format: MediaFormat) -> Result<(), CoreError> {
    DLNA_QUALITY.lock().unwrap().preferred = (quality, format);
    Ok(())
}

pub fn get_dlna_actual_quality() -> (Quality, MediaFormat) {
    let quality = DLNA_QUALITY.lock().unwrap();
    quality.actual.unwrap_or(quality.preferred)
}

pub fn set_dlna_actual_quality(quality: (Quality, MediaFormat)) {
    DLNA_QUALITY.lock().unwrap().actual = Some(quality);
}
