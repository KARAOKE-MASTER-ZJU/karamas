mod buffer;
mod export;
mod files;
mod panic;

pub use buffer::{LogBuffer, LogSnapshot};
pub use export::{ExportAction, can_share_logs, export_logs};
use flexi_logger::{
    DeferredNow,
    writers::{FileLogWriter, LogWriter},
};
use log::{Log, Metadata, Record};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, Runtime};

/// 通知查看页面重新读取日志，记录本身保存在 Rust。
pub const LOG_EVENT: &str = "casting://logs-changed";

struct AppLogger<R: Runtime> {
    app: AppHandle<R>,
    buffer: Arc<LogBuffer>,
    files: Option<FileLogWriter>,
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
        if let Some(files) = &self.files {
            if let Err(error) = files.write(&mut DeferredNow::new(), record) {
                eprintln!("写入日志文件失败：{error}");
            }
        }
        // 通知失败不影响留存，也不再记录日志，避免递归。
        let _ = self.app.emit(LOG_EVENT, ());
    }

    fn flush(&self) {
        if let Some(files) = &self.files {
            files::flush(files);
        }
    }
}

/// 注册内存和文件输出，默认记录 info 及更严重的日志。
pub fn initialize<R: Runtime>(app: AppHandle<R>) -> Result<(), log::SetLoggerError> {
    let buffer = Arc::new(LogBuffer::default());
    let directory = app.path().app_log_dir();
    let panic_path = directory.as_ref().ok().map(|path| path.join("panics.log"));
    let file_output = directory
        .map_err(|error| error.to_string())
        .and_then(|directory| {
            files::open(&directory, files::MAX_FILE_SIZE, files::MAX_ARCHIVED_FILES)
                .map_err(|error| error.to_string())
        });
    let (files, file_error) = match file_output {
        Ok(files) => (Some(files), None),
        Err(error) => (None, Some(error)),
    };
    log::set_boxed_logger(Box::new(AppLogger {
        app: app.clone(),
        buffer: buffer.clone(),
        files,
    }))?;
    app.manage(buffer);
    log::set_max_level(log::LevelFilter::Info);
    panic::install(panic_path);
    if let Some(error) = file_error {
        log::warn!("文件日志初始化失败，当前记录仅保留在内存：{error}");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
