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
