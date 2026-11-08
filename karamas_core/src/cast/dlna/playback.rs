use crate::cast::{PlaybackSnapshot, timeline::PlaybackTimeline};
use tokio::time::{Duration, Instant};

const OLD_PROGRESS_WINDOW: Duration = Duration::from_secs(3);

/// DLNA 查询结果按播放版本校验，再交给共用计时器校正。
#[derive(Default)]
pub(super) struct DlnaPlayback {
    timeline: PlaybackTimeline,
    revision: u64,
    previous_position: Option<u32>,
    protect_until: Option<Instant>,
}

impl DlnaPlayback {
    pub fn begin_song(&mut self) {
        self.previous_position = self.snapshot().map(|state| state.progress.current_secs);
        self.timeline = PlaybackTimeline::default();
        self.protect_until = None;
        self.revision += 1;
    }

    pub fn start(&mut self, position: u32) {
        if let Some(state) = self.snapshot() {
            self.previous_position = Some(state.progress.current_secs);
        }
        self.timeline.update(Some(position), Some(true));
        self.timeline.confirm_play(0);
        self.protect_until = Some(Instant::now() + OLD_PROGRESS_WINDOW);
        self.revision += 1;
    }

    pub fn update(&mut self, position: Option<u32>, playing: Option<bool>) {
        self.timeline.update(position, playing);
        self.revision += 1;
    }

    pub fn snapshot(&self) -> Option<PlaybackSnapshot> {
        self.timeline.snapshot()
    }

    pub fn query_revision(&self) -> Option<u64> {
        self.snapshot().map(|_| self.revision)
    }

    pub fn report(&mut self, revision: u64, current: Option<u32>, total: u32) {
        if self.revision != revision {
            return;
        }
        if let Some(deadline) = self.protect_until {
            let rewound = current.is_some_and(|current| {
                current == 0 || self.previous_position.is_some_and(|old| current < old)
            });
            if Instant::now() < deadline && !rewound {
                return;
            }
            self.protect_until = None;
        }
        self.timeline.report(current, Some(total), None);
    }
}
