use crate::{cast, error::CoreError};
use std::sync::atomic::{AtomicU32, Ordering};

/// DLNA 媒体代理清晰度。720P 保持旧的完整 MP4 路径，1080P 启用 Rust fMP4 混流。
static DLNA_QUALITY_QN: AtomicU32 = AtomicU32::new(64);
static DLNA_DIRECT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Changes apply to the next media load (including recast).
pub fn set_dlna_direct(enabled: bool) {
    DLNA_DIRECT.store(enabled, Ordering::Relaxed);
    if enabled {
        DLNA_QUALITY_QN.store(64, Ordering::Relaxed);
    }
}

pub fn get_dlna_direct() -> bool {
    DLNA_DIRECT.load(Ordering::Relaxed)
}

pub fn set_dlna_quality(quality: cast::Quality) -> Result<(), CoreError> {
    if get_dlna_direct() && quality == cast::Quality::P1080 {
        return Err("DLNA 直连模式不支持 B站 1080P，请先启用媒体代理".into());
    }
    match quality {
        cast::Quality::P720 | cast::Quality::P1080 => {
            DLNA_QUALITY_QN.store(quality.as_qn(), Ordering::Relaxed);
            Ok(())
        }
        _ => Err("DLNA 仅支持 720P 和 1080P(beta)".into()),
    }
}

pub fn get_dlna_quality() -> cast::Quality {
    cast::Quality::from_qn(DLNA_QUALITY_QN.load(Ordering::Relaxed))
        .unwrap_or(cast::Quality::P720)
}
