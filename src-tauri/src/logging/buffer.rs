use crate::models::LogLevel;
use serde::Serialize;
use std::{
    collections::VecDeque,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

const LOG_CAPACITY: usize = 10_000;

/// 记录时生成标识和时间，页面读取不改变原始记录。
#[derive(Clone, Serialize)]
pub struct LogEntry {
    pub id: u64,
    pub time: u64,
    pub level: LogLevel,
    pub target: String,
    pub message: String,
}

#[derive(Serialize)]
pub struct LogSnapshot {
    pub entries: Vec<LogEntry>,
    pub total: usize,
    pub retained: usize,
}

#[derive(Default)]
struct BufferState {
    entries: VecDeque<LogEntry>,
    next_id: u64,
}

/// 应用进程内的日志留存，与页面生命周期无关。
pub struct LogBuffer {
    state: Mutex<BufferState>,
    capacity: usize,
}

impl Default for LogBuffer {
    fn default() -> Self {
        Self::new(LOG_CAPACITY)
    }
}

impl LogBuffer {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            state: Mutex::new(BufferState::default()),
            capacity: capacity.max(1),
        }
    }

    pub fn write(&self, level: LogLevel, target: String, message: String) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.next_id += 1;
        let entry = LogEntry {
            id: state.next_id,
            time: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            level,
            target,
            message,
        };
        if state.entries.len() == self.capacity {
            state.entries.pop_front();
        }
        state.entries.push_back(entry);
    }

    /// 查询最近的匹配记录；limit 只限制返回量，不影响留存。
    pub fn snapshot(&self, minimum: LogLevel, query: &str, limit: Option<usize>) -> LogSnapshot {
        let query = query.trim().to_lowercase();
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let matches: Vec<_> = state
            .entries
            .iter()
            .filter(|entry| {
                entry.level >= minimum
                    && (query.is_empty()
                        || format!("[{}] {}", entry.target, entry.message)
                            .to_lowercase()
                            .contains(&query))
            })
            .collect();
        let total = matches.len();
        let start = total.saturating_sub(limit.unwrap_or(self.capacity).min(self.capacity));
        LogSnapshot {
            entries: matches[start..]
                .iter()
                .map(|entry| (*entry).clone())
                .collect(),
            total,
            retained: state.entries.len(),
        }
    }

    pub fn clear(&self) {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .entries
            .clear();
    }
}
