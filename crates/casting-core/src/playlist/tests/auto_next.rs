use super::*;

#[test]
fn old_end_progress_only_fires_once_even_with_a_new_duration() {
    let mut trigger = AutoNextSong::default();
    assert!(!trigger.observe(290, 300));
    assert!(trigger.observe(298, 300));
    for (current, total) in [(299, 300), (300, 300), (301, 180), (301, 600), (302, 180)] {
        assert!(!trigger.observe(current, total));
    }
}

#[test]
fn rewind_alone_does_not_rearm_the_end_event() {
    let mut trigger = AutoNextSong::default();
    assert!(trigger.observe(298, 300));
    assert!(!trigger.observe(1, 180));
    assert!(!trigger.observe(2, 180));
    assert!(!trigger.observe(178, 180));
    assert!(!trigger.observe(180, 180));
}

#[test]
fn errors_loading_and_small_jitter_do_not_rearm() {
    let mut trigger = AutoNextSong::default();
    assert!(trigger.observe(298, 300));
    for (current, total) in [(-1, -1), (0, 0), (0, 300), (1, 0), (297, 300), (298, 300)] {
        assert!(!trigger.observe(current, total));
    }
}

#[test]
fn rewind_still_inside_end_zone_does_not_rearm() {
    let mut trigger = AutoNextSong::default();
    assert!(trigger.observe(305, 300));
    assert!(!trigger.observe(299, 300));
    assert!(!trigger.observe(300, 300));
}

#[test]
fn polling_past_exact_end_still_triggers_once() {
    let mut trigger = AutoNextSong::default();
    assert!(!trigger.observe(175, 180));
    assert!(trigger.observe(181, 180));
    assert!(!trigger.observe(182, 180));
}

#[test]
fn play_success_rearms_once_even_without_a_progress_rewind() {
    let mut trigger = AutoNextSong::default();
    trigger.reset_for_playback();
    assert!(trigger.observe(298, 300));
    assert!(!trigger.observe(298, 300));
    trigger.reset_for_playback();
    // When the DLNA guard expires, the current sample is trusted as requested.
    assert!(trigger.observe(298, 300));
    assert!(!trigger.observe(299, 300));
}
