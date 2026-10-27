use super::DeviceEvent;
use crate::cast::{PlaybackSnapshot, Progress};
use tokio::time::Instant;

/// 当前播放会话的计时基准；回传校正位置，消息间隔内继续本地计时。
#[derive(Clone)]
pub(super) struct PlaybackTimeline {
    position: Option<u32>,
    total: Option<u32>,
    playing: Option<bool>,
    measured_at: Instant,
    confirmed: bool,
}

impl Default for PlaybackTimeline {
    fn default() -> Self {
        Self {
            position: None,
            total: None,
            playing: None,
            measured_at: Instant::now(),
            confirmed: false,
        }
    }
}

impl PlaybackTimeline {
    fn current_secs(&self) -> u32 {
        let elapsed = if self.position.is_some() && self.playing != Some(false) {
            self.measured_at.elapsed().as_secs().min(u32::MAX as u64) as u32
        } else {
            0
        };
        self.position.unwrap_or(0).saturating_add(elapsed)
    }

    pub fn confirm_play(&mut self, duration: u32) {
        // 回传可能先于 HTTP 响应到达，成功响应只补齐尚未收到的字段。
        if self.position.is_none() {
            self.position = Some(0);
            self.measured_at = Instant::now();
        }
        self.total.get_or_insert(duration);
        self.playing.get_or_insert(true);
        self.confirmed = true;
    }

    pub fn snapshot(&self) -> Option<PlaybackSnapshot> {
        self.confirmed.then(|| PlaybackSnapshot {
            progress: Progress {
                current_secs: self.current_secs(),
                total_secs: self.total.unwrap_or(0),
            },
            is_playing: self.playing.unwrap_or(true),
            ended: None,
        })
    }

    pub fn update(&mut self, position: Option<u32>, playing: Option<bool>) {
        self.position = Some(position.unwrap_or_else(|| self.current_secs()));
        self.measured_at = Instant::now();
        if let Some(playing) = playing {
            self.playing = Some(playing);
        }
    }

    pub fn apply(&mut self, event: &DeviceEvent) {
        if let Some(total) = event
            .duration_ms
            .and_then(|ms| u32::try_from(ms / 1000).ok())
            .filter(|total| *total > 0)
        {
            self.total = Some(total);
        }
        let position = event
            .position_secs
            .and_then(|secs| u32::try_from(secs).ok());
        let playing = match event.command {
            5 | 7 => Some(false),
            6 => Some(true),
            _ => None,
        };
        if position.is_some() || playing.is_some() {
            self.update(position, playing);
        }
    }
}
