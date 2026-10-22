use super::*;

fn sample(gate: &mut ProgressGate, current: i32, total: i32, now: Instant) -> (i32, i32) {
    gate.filter_at(gate.revision(), current, total, now)
}

fn playing(current: i32, now: Instant) -> ProgressGate {
    let mut gate = ProgressGate::default();
    gate.update_song(Some("A".into()), Some("video-A".into()));
    assert!(gate.play_succeeded(gate.revision(), false, now));
    assert_eq!(sample(&mut gate, current, 300, now), (current, 300));
    gate
}

fn change_to_b(gate: &mut ProgressGate) {
    gate.update_song(Some("B".into()), Some("video-B".into()));
}

#[test]
fn dlna_window_starts_at_play_success_not_playlist_update() {
    let now = Instant::now();
    let mut gate = playing(298, now);
    change_to_b(&mut gate);
    assert_eq!(
        sample(&mut gate, 298, 180, now + Duration::from_secs(10)),
        (-1, -1)
    );
    let success = now + Duration::from_secs(10);
    assert!(gate.play_succeeded(gate.revision(), true, success));
    assert_eq!(
        sample(&mut gate, 298, 180, success + Duration::from_millis(2999)),
        (0, 180)
    );
    assert_eq!(
        sample(&mut gate, 298, 180, success + Duration::from_secs(3)),
        (298, 180)
    );
}

#[test]
fn zero_and_small_rewinds_release_dlna_protection_immediately() {
    for (old, new) in [(0, 0), (1, 0), (2, 1), (298, 1)] {
        let now = Instant::now();
        let mut gate = playing(old, now);
        change_to_b(&mut gate);
        assert!(gate.play_succeeded(gate.revision(), true, now));
        assert_eq!(sample(&mut gate, new, 180, now), (new, 180));
    }
}

#[test]
fn simulated_progress_never_waits_for_rewind() {
    let now = Instant::now();
    let mut gate = playing(0, now);
    change_to_b(&mut gate);
    assert!(gate.play_succeeded(gate.revision(), false, now));
    assert_eq!(sample(&mut gate, 1, 180, now), (1, 180));
}

#[test]
fn failed_device_play_keeps_progress_masked_but_next_song_can_play() {
    let now = Instant::now();
    let mut gate = playing(240, now);
    change_to_b(&mut gate);
    assert_eq!(
        sample(&mut gate, 0, 180, now + Duration::from_secs(30)),
        (-1, -1)
    );
    gate.update_song(Some("C".into()), Some("video-C".into()));
    assert!(gate.play_succeeded(gate.revision(), false, now));
    assert_eq!(sample(&mut gate, 1, 180, now), (1, 180));
}

#[test]
fn consecutive_switches_ignore_old_command_and_query_completions() {
    let now = Instant::now();
    let mut gate = playing(1, now);
    change_to_b(&mut gate);
    let old = gate.revision();
    gate.update_song(Some("C".into()), Some("video-C".into()));
    let latest = gate.revision();
    assert!(!gate.play_succeeded(old, true, now));
    assert_eq!(gate.filter_at(old, 0, 180, now), (-1, -1));
    assert!(gate.play_succeeded(latest, true, now));
    assert_eq!(gate.filter_at(latest, 298, 300, now), (-1, -1));
    assert_eq!(sample(&mut gate, 0, 180, now), (0, 180));
}

#[test]
fn queue_only_updates_do_not_restart_window_or_rearm_once() {
    let now = Instant::now();
    let mut gate = playing(240, now);
    change_to_b(&mut gate);
    assert!(gate.play_succeeded(gate.revision(), true, now));
    assert!(gate.auto_next.observe(178, 180));
    let revision = gate.revision();
    change_to_b(&mut gate);
    assert_eq!(gate.revision(), revision);
    assert!(!gate.auto_next.observe(178, 180));
    assert_eq!(
        sample(&mut gate, 240, 300, now + Duration::from_secs(3)),
        (240, 300)
    );
}

#[test]
fn missing_baseline_expires_and_invalid_samples_are_not_zero() {
    let now = Instant::now();
    let mut gate = ProgressGate::default();
    gate.update_song(Some("A".into()), Some("video-A".into()));
    assert!(gate.play_succeeded(gate.revision(), true, now));
    assert_eq!(sample(&mut gate, -1, -1, now), (-1, -1));
    assert_eq!(sample(&mut gate, 0, 0, now), (-1, -1));
    assert_eq!(sample(&mut gate, 100, 300, now), (0, 300));
    assert_eq!(
        sample(&mut gate, 100, 300, now + Duration::from_secs(3)),
        (100, 300)
    );
}

#[test]
fn late_query_from_before_rewind_is_discarded() {
    let now = Instant::now();
    let mut gate = playing(240, now);
    change_to_b(&mut gate);
    assert!(gate.play_succeeded(gate.revision(), true, now));
    let revision = gate.revision();
    assert_eq!(sample(&mut gate, 0, 180, now), (0, 180));
    assert_eq!(gate.filter_at(revision, 298, 180, now), (-1, -1));
}

#[test]
fn old_end_sample_after_new_playback_cannot_consume_once_or_mask_progress() {
    let now = Instant::now();
    let mut gate = playing(298, now);
    let old_query = gate.revision();
    change_to_b(&mut gate);
    assert!(gate.play_succeeded(gate.revision(), false, now));
    let (current, total) = gate.filter_at(old_query, 298, 300, now);
    assert!(!gate.auto_next.observe(current, total));
    assert_eq!(sample(&mut gate, 1, 180, now), (1, 180));
    assert!(gate.auto_next.observe(178, 180));
}
