use log::{Log, Metadata, Record};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};

/// 前端通过事件订阅接收 Rust 日志。
pub const LOG_EVENT: &str = "casting://log";

/// 日志事件载荷，级别名称与前端 LogLevel 一致。
#[derive(Clone, Serialize)]
pub struct LogMessage {
    level: &'static str,
    target: String,
    message: String,
}

struct AppLogger<R: Runtime>(AppHandle<R>);

impl<R: Runtime> Log for AppLogger<R> {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Debug
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let level = match record.level() {
            log::Level::Error => "error",
            log::Level::Warn => "warning",
            log::Level::Info => "info",
            log::Level::Debug | log::Level::Trace => "debug",
        };
        let message = LogMessage {
            level,
            target: record.target().to_owned(),
            message: record.args().to_string(),
        };
        eprintln!("[{}] {}: {}", level, message.target, message.message);
        // 发送失败时不再记录日志，避免日志桥接递归。
        let _ = self.0.emit(LOG_EVENT, message);
    }

    fn flush(&self) {}
}

/// 将 Rust 日志转发为独立事件，不与界面提示条耦合。
pub fn initialize<R: Runtime>(app: AppHandle<R>) -> Result<(), log::SetLoggerError> {
    // log 注册器为进程全局资源，仅在应用初始化时注册。
    log::set_boxed_logger(Box::new(AppLogger(app)))?;
    log::set_max_level(log::LevelFilter::Debug);
    Ok(())
}
