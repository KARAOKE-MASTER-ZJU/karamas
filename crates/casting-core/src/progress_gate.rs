use std::time::{Duration, Instant};

const OLD_PROGRESS_WINDOW: Duration = Duration::from_secs(3);

#[derive(Default)]
enum Phase {
    #[default]
    Ready,
    AwaitingCommand,
    ProtectingUntil(Instant),
}

/// A successful device play response starts a bounded DLNA protection window.
/// Simulated progress can be trusted immediately after that response.
#[derive(Default)]
pub(crate) struct ProgressGate {
    song: Option<(Option<String>, String)>,
    last_current: Option<i32>,
    baseline: Option<i32>,
    phase: Phase,
    revision: u64,
    pub(crate) auto_next: crate::auto_next::AutoNextSong,
}

impl ProgressGate {
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }
    pub(crate) fn song(&self) -> Option<(Option<String>, String)> {
        self.song.clone()
    }

    pub(crate) fn update_song(&mut self, id: Option<String>, url: Option<String>) {
        let song = url.map(|url| (id, url));
        if song != self.song {
            // Only a confirmed song change starts waiting for device playback.
            if !matches!(self.phase, Phase::AwaitingCommand) {
                self.baseline = self.last_current;
            }
            self.phase = Phase::AwaitingCommand;
            self.song = song;
            self.revision += 1;
        }
    }

    pub(crate) fn play_token(&self, url: &str) -> Option<u64> {
        self.song
            .as_ref()
            .filter(|(_, current)| current == url)
            .map(|_| self.revision)
    }

    pub(crate) fn begin_recast(&mut self) -> Option<(String, u64)> {
        let (_, url) = self.song.as_ref()?;
        let url = url.clone();
        if !matches!(self.phase, Phase::AwaitingCommand) {
            self.baseline = self.last_current;
        }
        self.phase = Phase::AwaitingCommand;
        self.revision += 1;
        self.auto_next.cancel_pending_request();
        Some((url, self.revision))
    }

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
        self.auto_next.reset_for_playback();
        self.revision += 1;
        true
    }

    pub(crate) fn filter(&mut self, revision: u64, current: i32, total: i32) -> (i32, i32) {
        self.filter_at(revision, current, total, Instant::now())
    }

    fn filter_at(&mut self, revision: u64, current: i32, total: i32, now: Instant) -> (i32, i32) {
        if revision != self.revision || self.song.is_none() || current < 0 || total <= 0 {
            return (-1, -1);
        }
        match self.phase {
            Phase::AwaitingCommand => return (-1, -1),
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
#[path = "../test/progress_gate.rs"]
mod tests;
