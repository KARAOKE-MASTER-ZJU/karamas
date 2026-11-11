use crate::cast::{PlaybackSnapshot, Progress};
use tokio::time::Instant;

/// 当前播放会话的计时基准；回传校正位置，消息间隔内继续本地计时。
#[derive(Clone)]
pub(crate) struct PlaybackTimeline {
    position: Option<u32>,
    total: Option<u32>,
    playing: Option<bool>,
    measured_at: Instant,
    confirmed: bool,
    last_reported: Option<u32>,
    frozen: bool,
}

impl Default for PlaybackTimeline {
    fn default() -> Self {
        Self {
            position: None,
            total: None,
            playing: None,
            measured_at: Instant::now(),
            confirmed: false,
            last_reported: None,
            frozen: false,
        }
    }
}

impl PlaybackTimeline {
    fn current_secs(&self) -> u32 {
        let elapsed = if self.position.is_some() && self.playing != Some(false) && !self.frozen {
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
        self.frozen = false;
    }

    /// 切歌等待期间停止本地累加，保留设备回传校正和播放状态。
    pub fn freeze(&mut self) {
        self.update(None, None);
        self.frozen = true;
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

    /// 接收设备校正；重复的位置表示没有新进度，继续沿用本地计时。
    pub fn report(&mut self, position: Option<u32>, total: Option<u32>, playing: Option<bool>) {
        if let Some(total) = total.filter(|total| *total > 0) {
            self.total = Some(total);
        }
        let changed_position = position.filter(|position| Some(*position) != self.last_reported);
        if position.is_some() {
            self.last_reported = position;
        }
        if changed_position.is_some()
            || playing.is_some_and(|playing| Some(playing) != self.playing)
        {
            self.update(changed_position, playing);
        }
    }
}
