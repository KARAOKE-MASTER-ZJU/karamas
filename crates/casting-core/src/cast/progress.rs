//! 投屏进度条
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;

use super::Progress;

pub struct LocalProgressTracker {
    total_secs: AtomicU32,
    start_offset: AtomicU32,
    started_at: Mutex<Option<Instant>>,
    paused_at: Mutex<Option<u32>>,
}

/// 本地进度条追踪器
impl LocalProgressTracker {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            total_secs: AtomicU32::new(0),
            start_offset: AtomicU32::new(0),
            started_at: Mutex::new(None),
            paused_at: Mutex::new(None),
        })
    }

    /// 开始播放，设置总时长
    pub async fn start(&self, total_secs: u32) {
        self.total_secs.store(total_secs, Ordering::Relaxed);
        self.start_offset.store(0, Ordering::Relaxed);
        *self.started_at.lock().await = Some(Instant::now());
        *self.paused_at.lock().await = None;
    }

    /// 停止播放，重置状态
    pub async fn pause(&self) {
        let curr = self.running_secs().await;
        *self.paused_at.lock().await = Some(curr);
    }

    /// 恢复播放，继续计时
    pub async fn resume(&self) {
        let pos = self.paused_at.lock().await.take();
        if let Some(p) = pos {
            self.start_offset.store(p, Ordering::Relaxed);
            *self.started_at.lock().await = Some(Instant::now());
        }
    }

    /// 停止播放，重置状态
    pub async fn seek(&self, secs: u32) {
        self.start_offset.store(secs, Ordering::Relaxed);
        *self.started_at.lock().await = Some(Instant::now());
        *self.paused_at.lock().await = None;
    }

    /// 获取当前播放时间（秒）
    async fn running_secs(&self) -> u32 {
        let offset = self.start_offset.load(Ordering::Relaxed);
        let guard = self.started_at.lock().await;
        let elapsed = guard.as_ref().map(|t| t.elapsed().as_secs() as u32).unwrap_or(0);
        offset + elapsed
    }

    /// 获取当前播放进度
    pub async fn get_progress(&self) -> Progress {
        let current_secs = if let Some(p) = *self.paused_at.lock().await {
            p
        } else {
            self.running_secs().await
        };
        Progress {
            current_secs,
            total_secs: self.total_secs.load(Ordering::Relaxed),
        }
    }
}
