use super::LogBuffer;
use crate::models::LogLevel;

#[test]
fn display_limits_and_filters_do_not_discard_retained_logs() {
    let buffer = LogBuffer::new(500);
    buffer.write(LogLevel::Info, "casting".into(), "Original needle".into());
    for _ in 0..250 {
        buffer.write(LogLevel::Debug, "poll".into(), "detail".into());
    }
    let page = buffer.snapshot(LogLevel::Trace, "", Some(200));
    assert_eq!(page.entries.len(), 200);
    assert_eq!(page.total, 251);
    let info = buffer.snapshot(LogLevel::Info, "NEEDLE", Some(200));
    assert_eq!(info.entries.len(), 1);
    assert_eq!(info.entries[0].id, 1);
    assert_eq!(info.retained, 251);
    assert_eq!(
        buffer.snapshot(LogLevel::Trace, "", None).entries.len(),
        251
    );
}

#[test]
fn retention_capacity_and_clear_are_independent_of_queries() {
    let buffer = LogBuffer::new(3);
    for _ in 0..4 {
        buffer.write(LogLevel::Info, "casting".into(), "event".into());
    }
    let snapshot = buffer.snapshot(LogLevel::Info, "", None);
    assert_eq!(
        snapshot
            .entries
            .iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>(),
        vec![2, 3, 4]
    );
    buffer.clear();
    assert_eq!(buffer.snapshot(LogLevel::Trace, "", None).retained, 0);
    buffer.write(LogLevel::Error, "casting".into(), "new event".into());
    assert_eq!(buffer.snapshot(LogLevel::Trace, "", None).entries[0].id, 5);
}

struct LogDirectory(std::path::PathBuf);

impl LogDirectory {
    fn new() -> Self {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("ktv-casting-logs-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for LogDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn panic_record_is_appended_before_process_exit() {
    const CHILD_PATH: &str = "KTV_CASTING_PANIC_TEST_PATH";
    if let Some(path) = std::env::var_os(CHILD_PATH) {
        std::panic::set_hook(Box::new(|_| eprintln!("previous-hook-called")));
        super::panic::install(Some(path.into()));
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            hook(info);
            // 直接退出，不依赖展开栈或析构刷新文件。
            std::process::exit(73);
        }));
        panic!("panic-persistence-marker");
    }

    let directory = LogDirectory::new();
    let path = directory.0.join("panics.log");
    std::fs::write(&path, "existing record\n").unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "logging::tests::panic_record_is_appended_before_process_exit",
            "--nocapture",
        ])
        .env(CHILD_PATH, &path)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(73));
    let text = std::fs::read_to_string(path).unwrap();
    assert!(text.starts_with("existing record\n"));
    assert!(text.contains("panic-persistence-marker"));
    assert!(text.contains("logging/tests.rs:"));
    assert!(text.contains("pid="));
    assert!(text.contains("thread='"));
    assert!(text.contains("std::panicking"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("previous-hook-called"));
}
