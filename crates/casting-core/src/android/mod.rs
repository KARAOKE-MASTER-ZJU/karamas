//! 旧 Android JNI 接入，负责原生导出、状态适配和日志转发。
mod bilibili;
mod discovery;
mod engine;
mod logging;
mod playback;
mod playlist;

pub use bilibili::{BILI_AUTH_STATE, BiliAuthState};

use std::sync::OnceLock;

fn engine_runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| tokio::runtime::Runtime::new().expect("无法创建 JNI 运行时"))
}
