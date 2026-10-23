use std::{
    backtrace::Backtrace,
    fs::OpenOptions,
    io::{self, Write},
    path::{Path, PathBuf},
};

/// panic 独立追加，避免依赖普通日志器及其锁，并保留已有 hook。
pub(super) fn install(path: Option<PathBuf>) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|location| {
                format!(
                    "{}:{}:{}",
                    location.file(),
                    location.line(),
                    location.column()
                )
            })
            .unwrap_or_else(|| "<unknown>".to_owned());
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
            .unwrap_or("<unknown panic payload>");
        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("<unnamed>");
        let backtrace = Backtrace::force_capture();
        let text = format!(
            "[{}] pid={} thread='{thread_name}' at {location}: {message}\n{backtrace}\n----\n",
            chrono::Utc::now().to_rfc3339(),
            std::process::id()
        );
        if let Some(path) = path.as_deref() {
            if let Err(error) = append_record(path, &text) {
                eprintln!("写入 panic 记录失败：{error}\n{text}");
            }
        } else {
            eprintln!("{text}");
        }
        previous(info);
    }));
}

fn append_record(path: &Path, text: &str) -> io::Result<()> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(text.as_bytes())?;
    file.flush()?;
    file.sync_data()
}
