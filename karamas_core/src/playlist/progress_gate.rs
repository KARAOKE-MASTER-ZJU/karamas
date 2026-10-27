use std::time::{Duration, Instant};

const OLD_PROGRESS_WINDOW: Duration = Duration::from_secs(3);

#[derive(Default)]
enum Phase {
    #[default]
    Ready,
    AwaitingCommand,
    FailedCommand,
    ProtectingUntil(Instant),
}

/// A successful device play response starts a bounded DLNA protection window.
/// Simulated progress can be trusted immediately after that response.
#[derive(Default)]
pub(crate) struct ProgressGate {
    song: Option<(Option<String>, String)>,
    last_current: Option<i32>,
    progress: Option<(i32, i32)>,
    baseline: Option<i32>,
    phase: Phase,
    revision: u64,
    pub(crate) auto_next: crate::playlist::auto_next::AutoNextSong,
}

impl ProgressGate {
    /// 读取当前进度状态的版本号。
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    /// 查询失败时保留上次有效进度，直到歌曲或播放实例改变。
    pub(crate) fn cached_progress(&self) -> (i32, i32) {
        self.progress.unwrap_or((-1, -1))
    }

    /// 定位成功后更新显示进度，不改变播放确认和自动切歌状态。
    pub(crate) fn seek_succeeded(&mut self, revision: u64, seconds: u32) {
        if revision != self.revision {
            return;
        }
        if let Some((_, total)) = self.progress {
            let current = seconds.min(i32::MAX as u32) as i32;
            self.progress = Some((current, total));
            self.last_current = Some(current);
        }
    }

    /// 播放首次失败后，恢复播放的确认可以解除等待状态。
    pub(crate) fn confirm_local_resume(&mut self) {
        if matches!(self.phase, Phase::FailedCommand) {
            self.play_succeeded(self.revision, false, Instant::now());
        }
    }

    /// 只有已失败的本地播放可由原生控件恢复，等待新歌时仍屏蔽旧播放器事件。
    pub(crate) fn local_play_failed(&mut self, token: u64) {
        if self.revision == token && self.song.is_some() {
            self.phase = Phase::FailedCommand;
        }
    }

    /// 读取当前歌曲的 ID 和地址。
    pub(crate) fn song(&self) -> Option<(Option<String>, String)> {
        self.song.clone()
    }

    /// 更新当前歌曲，并在歌曲变化时屏蔽旧播放进度。
    pub(crate) fn update_song(&mut self, id: Option<String>, url: Option<String>) {
        let song = url.map(|url| (id, url));
        if song != self.song {
            // Only a confirmed song change starts waiting for device playback.
            if !matches!(self.phase, Phase::AwaitingCommand) {
                self.baseline = self.last_current;
            }
            self.phase = Phase::AwaitingCommand;
            self.song = song;
            self.progress = None;
            self.revision += 1;
        }
    }

    /// 获取匹配当前歌曲的播放确认标识。
    pub(crate) fn play_token(&self, url: &str) -> Option<u64> {
        self.song
            .as_ref()
            .filter(|(_, current)| current == url)
            .map(|_| self.revision)
    }

    /// 开始重新投屏，返回歌曲地址和新的播放确认标识。
    pub(crate) fn begin_recast(&mut self) -> Option<(String, u64)> {
        let (_, url) = self.song.as_ref()?;
        let url = url.clone();
        if !matches!(self.phase, Phase::AwaitingCommand) {
            self.baseline = self.last_current;
        }
        self.phase = Phase::AwaitingCommand;
        self.progress = None;
        self.revision += 1;
        self.auto_next.cancel_pending_request();
        Some((url, self.revision))
    }

    /// 确认当前版本播放成功，并重置进度保护和自动切歌状态。
    pub(crate) fn play_succeeded(
        &mut self,
        token: u64,
        hardware_progress: bool,
        now: Instant,
    ) -> bool {
        if token != self.revision || self.song.is_none() {
            return false;
        }
        self.phase = if hardware_progress {
            Phase::ProtectingUntil(now + OLD_PROGRESS_WINDOW)
        } else {
            Phase::Ready
        };
        self.last_current = Some(0);
        self.progress = None;
        self.auto_next.reset_for_playback();
        self.revision += 1;
        true
    }

    /// 过滤过期或切歌期间不可信的播放进度。
    pub(crate) fn filter(&mut self, revision: u64, current: i32, total: i32) -> (i32, i32) {
        let progress = self.filter_at(revision, current, total, Instant::now());
        if progress.0 >= 0 {
            self.progress = Some(progress);
        }
        progress
    }

    /// 按指定时间判断进度保护是否结束，并过滤播放进度。
    fn filter_at(&mut self, revision: u64, current: i32, total: i32, now: Instant) -> (i32, i32) {
        if revision != self.revision || self.song.is_none() || current < 0 || total <= 0 {
            return (-1, -1);
        }
        match self.phase {
            Phase::AwaitingCommand | Phase::FailedCommand => return (-1, -1),
            Phase::ProtectingUntil(deadline) => {
                // Zero is valid, including when the preceding song was also at zero.
                let rewound = current == 0 || self.baseline.is_some_and(|old| current < old);
                if now < deadline && !rewound {
                    return (0, total);
                }
                self.phase = Phase::Ready;
                // Invalidate other queries sent before the first trusted sample.
                self.revision += 1;
            }
            Phase::Ready => {}
        }
        self.last_current = Some(current);
        (current, total)
    }
}

#[cfg(test)]
#[path = "tests/progress_gate.rs"]
mod tests;
