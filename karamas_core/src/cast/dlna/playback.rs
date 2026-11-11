use crate::cast::{PlaybackSnapshot, timeline::PlaybackTimeline};
use tokio::time::{Duration, Instant};

const OLD_PROGRESS_WINDOW: Duration = Duration::from_secs(3);
const PAUSE_DETECTION_WINDOW: Duration = Duration::from_secs(3);

/// DLNA 查询结果按播放版本校验，再交给共用计时器校正。
#[derive(Default)]
pub(super) struct DlnaPlayback {
    timeline: PlaybackTimeline,
    revision: u64,
    previous_position: Option<u32>,
    protect_until: Option<Instant>,
    observed_position: Option<(u32, Instant)>,
}

impl DlnaPlayback {
    pub fn freeze(&mut self) {
        self.timeline.freeze();
    }

    pub fn begin_song(&mut self) {
        self.previous_position = self.snapshot().map(|state| state.progress.current_secs);
        self.timeline = PlaybackTimeline::default();
        self.protect_until = None;
        self.observed_position = None;
        self.revision += 1;
    }

    pub fn start(&mut self, position: u32) {
        if let Some(state) = self.snapshot() {
            self.previous_position = Some(state.progress.current_secs);
        }
        self.timeline.update(Some(position), Some(true));
        self.timeline.confirm_play(0);
        self.protect_until = Some(Instant::now() + OLD_PROGRESS_WINDOW);
        self.observed_position = None;
        self.revision += 1;
    }

    pub fn update(&mut self, position: Option<u32>, playing: Option<bool>) {
        self.timeline.update(position, playing);
        self.observed_position = None;
        self.revision += 1;
    }

    pub fn snapshot(&self) -> Option<PlaybackSnapshot> {
        self.timeline.snapshot()
    }

    pub fn query_revision(&self) -> Option<u64> {
        self.snapshot().map(|_| self.revision)
    }

    pub fn query_failed(&mut self, revision: u64) {
        if self.revision == revision {
            self.observed_position = None;
        }
    }

    /// 返回播放状态是否变化，供调用方触发状态上报。
    pub fn report(&mut self, revision: u64, current: Option<u32>, total: u32) -> bool {
        if self.revision != revision {
            return false;
        }
        if let Some(deadline) = self.protect_until {
            let rewound = current.is_some_and(|current| {
                current == 0 || self.previous_position.is_some_and(|old| current < old)
            });
            if Instant::now() < deadline && !rewound {
                return false;
            }
            self.protect_until = None;
        }
        let was_playing = self.snapshot().map(|state| state.is_playing);
        if let Some(current) = current {
            match self.observed_position {
                Some((previous, since)) if previous == current => {
                    if since.elapsed() >= PAUSE_DETECTION_WINDOW {
                        self.timeline.update(Some(current), Some(false));
                    }
                }
                previous => {
                    self.observed_position = Some((current, Instant::now()));
                    if previous.is_some() {
                        self.timeline.update(Some(current), Some(true));
                    }
                }
            }
        } else {
            self.observed_position = None;
        }
        self.timeline.report(current, Some(total), None);
        self.snapshot().map(|state| state.is_playing) != was_playing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn switching_freezes_local_time_until_new_playback() {
        let mut playback = DlnaPlayback::default();
        playback.start(0);
        let revision = playback.query_revision().unwrap();
        playback.report(revision, Some(0), 180);
        playback.report(revision, Some(178), 180);
        playback.freeze();
        tokio::time::advance(Duration::from_secs(10)).await;
        let state = playback.snapshot().unwrap();
        assert_eq!(state.progress.current_secs, 178);
        assert!(state.is_playing);

        // 设备回包仍能校准位置，但不能重新启动本地累加。
        playback.report(revision, Some(179), 180);
        tokio::time::advance(Duration::from_secs(1)).await;
        assert_eq!(playback.snapshot().unwrap().progress.current_secs, 179);

        playback.begin_song();
        playback.start(0);
        let revision = playback.query_revision().unwrap();
        tokio::time::advance(Duration::from_secs(1)).await;
        playback.report(revision, Some(179), 180);
        assert_eq!(playback.snapshot().unwrap().progress.current_secs, 1);
        playback.report(revision, Some(0), 240);
        tokio::time::advance(Duration::from_secs(2)).await;
        let state = playback.snapshot().unwrap();
        assert_eq!(state.progress.current_secs, 2);
        assert!(state.is_playing);
    }
}
