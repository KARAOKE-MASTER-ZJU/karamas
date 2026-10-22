use super::*;
use crate::cast::progress::LocalProgressTracker;
use crate::cast::{Capabilities, CastError, Caster, Progress, SongRef};

struct TestCaster {
    progress: Arc<LocalProgressTracker>,
    hardware: bool,
    fail: bool,
    started: Notify,
    reply: Notify,
    songs: Mutex<Vec<String>>,
}

impl TestCaster {
    fn new(hardware: bool, fail: bool) -> Self {
        Self {
            progress: LocalProgressTracker::new(),
            hardware,
            fail,
            started: Notify::new(),
            reply: Notify::new(),
            songs: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl Caster for TestCaster {
    async fn play_song(&self, song: &SongRef) -> Result<(), CastError> {
        self.songs.lock().await.push(song.0.clone());
        self.started.notify_one();
        self.reply.notified().await;
        if self.fail {
            return Err(CastError::Device("play failed".into()));
        }
        self.progress.start(180).await;
        Ok(())
    }
    async fn get_progress(&self) -> Result<Progress, CastError> {
        Ok(self.progress.get_progress().await)
    }
    async fn resume(&self) -> Result<(), CastError> {
        Ok(())
    }
    async fn pause(&self) -> Result<(), CastError> {
        Ok(())
    }
    async fn stop(&self) -> Result<(), CastError> {
        Ok(())
    }
    async fn seek(&self, _: u32) -> Result<(), CastError> {
        Ok(())
    }
    async fn set_volume(&self, _: u32) -> Result<(), CastError> {
        Ok(())
    }
    async fn get_volume(&self) -> Result<Option<u32>, CastError> {
        Ok(None)
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            hardware_progress: self.hardware,
            seek: true,
            absolute_volume: false,
        }
    }
}

#[tokio::test]
async fn failed_initial_cast_can_retry_the_same_song_without_reconnecting() {
    let mut caster = TestCaster::new(false, true);
    let gate = setup(240).await;
    let token = gate.lock().await.play_token("B").unwrap();
    caster.reply.notify_one();
    assert!(!play_and_confirm(&caster, "B", token, &gate).await);
    caster.fail = false;
    caster.reply.notify_one();
    let result = recast_with_caster(
        &caster, &gate, &Mutex::new(Default::default()), &Mutex::new(()),
    ).await;
    assert_eq!(result, 1);
    assert_eq!(*caster.songs.lock().await, ["B", "B"]);
    let mut state = gate.lock().await;
    assert_eq!(state.song(), Some((Some("B".into()), "B".into())));
    let revision = state.revision();
    assert_eq!(state.filter(revision, 0, 180), (0, 180));
    assert!(state.auto_next.observe(178, 180));
}

#[tokio::test]
async fn recast_rejects_duplicates_and_masks_old_end_progress() {
    let caster = TestCaster::new(false, false);
    let gate = setup(240).await;
    let cache = Mutex::new(Default::default());
    let playback = Mutex::new(());
    let (result, ()) = tokio::join!(
        recast_with_caster(&caster, &gate, &cache, &playback),
        async {
            caster.started.notified().await;
            assert_eq!(recast_with_caster(&caster, &gate, &cache, &playback).await, -2);
            let mut state = gate.lock().await;
            let revision = state.revision();
            let (current, total) = state.filter(revision, 298, 300);
            assert_eq!((current, total), (-1, -1));
            assert!(!state.auto_next.observe(current, total));
            drop(state);
            caster.reply.notify_one();
        },
    );
    assert_eq!(result, 1);
    assert_eq!(*caster.songs.lock().await, ["B"]);
}

#[tokio::test]
async fn recast_failure_releases_busy_state_for_another_attempt() {
    let mut caster = TestCaster::new(true, true);
    let gate = setup(240).await;
    let cache = Mutex::new(Default::default());
    let playback = Mutex::new(());
    caster.reply.notify_one();
    assert_eq!(recast_with_caster(&caster, &gate, &cache, &playback).await, -1);
    caster.fail = false;
    caster.reply.notify_one();
    assert_eq!(recast_with_caster(&caster, &gate, &cache, &playback).await, 1);
    let mut state = gate.lock().await;
    let revision = state.revision();
    assert_eq!(state.filter(revision, 298, 180), (0, 180));
    let revision = state.revision();
    assert_eq!(state.filter(revision, 0, 180), (0, 180));
}

#[tokio::test]
async fn missing_current_song_does_not_send_a_play_command() {
    let caster = TestCaster::new(false, false);
    assert_eq!(recast_with_caster(
        &caster, &Mutex::new(Default::default()),
        &Mutex::new(Default::default()), &Mutex::new(()),
    ).await, 0);
    assert!(caster.songs.lock().await.is_empty());
}

#[tokio::test(start_paused = true)]
async fn recast_timeout_releases_busy_state_for_another_attempt() {
    let caster = TestCaster::new(false, false);
    let gate = setup(240).await;
    let cache = Mutex::new(Default::default());
    let playback = Mutex::new(());
    assert_eq!(recast_with_caster(&caster, &gate, &cache, &playback).await, -1);
    caster.reply.notify_one();
    assert_eq!(recast_with_caster(&caster, &gate, &cache, &playback).await, 1);
}

#[tokio::test(start_paused = true)]
async fn recast_allows_cold_dash_preparation_beyond_thirty_seconds() {
    let caster = TestCaster::new(true, false);
    let gate = setup(240).await;
    let cache = Mutex::new(Default::default());
    let playback = Mutex::new(());
    let (result, ()) = tokio::join!(
        recast_with_caster(&caster, &gate, &cache, &playback),
        async {
            caster.started.notified().await;
            tokio::time::sleep(Duration::from_secs(90)).await;
            caster.reply.notify_one();
        },
    );
    assert_eq!(result, 1);
}

async fn setup(current: i32) -> Mutex<progress_gate::ProgressGate> {
    let mut gate = progress_gate::ProgressGate::default();
    gate.update_song(Some("A".into()), Some("A".into()));
    let token = gate.revision();
    assert!(gate.play_succeeded(token, false, std::time::Instant::now()));
    let rev = gate.revision();
    assert_eq!(gate.filter(rev, current, 300), (current, 300));
    assert!(gate.auto_next.observe(298, 300));
    gate.update_song(Some("B".into()), Some("B".into()));
    Mutex::new(gate)
}

#[tokio::test]
async fn cloud_success_releases_progress_even_when_switching_at_zero() {
    let caster = TestCaster::new(false, false);
    let gate = setup(0).await;
    let token = gate.lock().await.play_token("B").unwrap();
    tokio::join!(play_and_confirm(&caster, "B", token, &gate), async {
        caster.started.notified().await;
        let mut state = gate.lock().await;
        let rev = state.revision();
        assert_eq!(state.filter(rev, 0, 300), (-1, -1));
        drop(state);
        caster.reply.notify_one();
    });
    let actual = caster.get_progress().await.unwrap();
    assert_eq!(actual.current_secs, 0);
    assert_eq!(actual.total_secs, 180);
    let mut state = gate.lock().await;
    let rev = state.revision();
    assert_eq!(state.filter(rev, 0, 180), (0, 180));
    assert!(state.auto_next.observe(178, 180));
}

#[tokio::test]
async fn failed_play_does_not_start_a_new_clock_or_confirm_playback() {
    let caster = TestCaster::new(false, true);
    caster.progress.start(300).await;
    caster.progress.seek(240).await;
    let gate = setup(240).await;
    let token = gate.lock().await.play_token("B").unwrap();
    caster.reply.notify_one();
    play_and_confirm(&caster, "B", token, &gate).await;
    let actual = caster.get_progress().await.unwrap();
    assert_eq!(actual.total_secs, 300);
    assert!(actual.current_secs >= 240);
    let mut state = gate.lock().await;
    let rev = state.revision();
    assert!(!state.auto_next.observe(298, 300));
    assert_eq!(state.filter(rev, 0, 180), (-1, -1));
}

#[tokio::test]
async fn late_success_cannot_release_a_newer_switch() {
    let caster = TestCaster::new(true, false);
    let gate = setup(2).await;
    let token = gate.lock().await.play_token("B").unwrap();
    tokio::join!(play_and_confirm(&caster, "B", token, &gate), async {
        caster.started.notified().await;
        gate.lock()
            .await
            .update_song(Some("C".into()), Some("C".into()));
        caster.reply.notify_one();
    });
    let mut state = gate.lock().await;
    let rev = state.revision();
    assert!(!state.auto_next.observe(298, 300));
    assert_eq!(state.filter(rev, 0, 180), (-1, -1));
}
