use flexi_logger::{
    Cleanup, Criterion, DeferredNow, FileSpec, FlexiLoggerError, Naming, WriteMode,
    writers::{FileLogWriter, LogWriter},
};
use log::Record;
use std::{
    io::{self, Write},
    path::Path,
    time::Duration,
};

pub(super) const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;
pub(super) const MAX_ARCHIVED_FILES: usize = 4;

/// 写入器只负责文件格式、缓冲和轮转，不另设日志级别。
pub(super) fn open(
    directory: &Path,
    max_size: u64,
    max_archived_files: usize,
) -> Result<FileLogWriter, FlexiLoggerError> {
    FileLogWriter::builder(
        FileSpec::default()
            .directory(directory)
            .basename("ktv-casting")
            .suppress_timestamp(),
    )
    .append()
    .format(format_record)
    .write_mode(WriteMode::BufferAndFlushWith(
        64 * 1024,
        Duration::from_millis(500),
    ))
    .rotate(
        Criterion::Size(max_size),
        Naming::Numbers,
        Cleanup::KeepLogFiles(max_archived_files),
    )
    .try_build()
}

fn format_record(
    out: &mut dyn Write,
    now: &mut DeferredNow,
    record: &Record<'_>,
) -> io::Result<()> {
    write!(
        out,
        "[{}] [{}] [{}] {}",
        now.format("%Y-%m-%d %H:%M:%S%.3f %:z"),
        record.level().as_str().to_ascii_lowercase(),
        record.target(),
        record.args()
    )
}

pub(super) fn flush(writer: &FileLogWriter) {
    if let Err(error) = writer.flush() {
        eprintln!("刷新日志文件失败：{error}");
    }
}
