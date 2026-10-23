mod buffer;

pub use buffer::{LogBuffer, LogSnapshot};
use log::{Log, Metadata, Record};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, Runtime};

/// 通知查看页面重新读取日志，记录本身保存在 Rust。
pub const LOG_EVENT: &str = "casting://logs-changed";

struct AppLogger<R: Runtime> {
    app: AppHandle<R>,
    buffer: Arc<LogBuffer>,
}

impl<R: Runtime> Log for AppLogger<R> {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        self.buffer.write(
            record.level().into(),
            record.target().to_owned(),
            record.args().to_string(),
        );
        // 通知失败不影响留存，也不再记录日志，避免递归。
        let _ = self.app.emit(LOG_EVENT, ());
    }

    fn flush(&self) {}
}

/// 注册应用日志器和内存留存，默认记录 info 及更严重的日志。
pub fn initialize<R: Runtime>(app: AppHandle<R>) -> Result<(), log::SetLoggerError> {
    let buffer = Arc::new(LogBuffer::default());
    log::set_boxed_logger(Box::new(AppLogger {
        app: app.clone(),
        buffer: buffer.clone(),
    }))?;
    app.manage(buffer);
    log::set_max_level(log::LevelFilter::Info);
    Ok(())
}

#[cfg(test)]
mod tests;
