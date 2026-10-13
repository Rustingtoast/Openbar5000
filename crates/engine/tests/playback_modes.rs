//! Exercises RepeatOne, RepeatAll, and Shuffle end-to-end through the real
//! playback thread (real decode + real cpal output), not just the pure
//! `ShuffleOrder` unit tests -- this is what actually proves
//! `SetPlaybackMode` and the natural-end-of-track path wire together
//! correctly.

use engine::{EngineCommand, EngineEvent, PlaybackEngine, PlaybackMode};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/test_tone.wav")
}

/// Polls for the next `TrackChanged` event (its `index`), failing the test
/// if none arrives within `timeout`.
fn next_track_changed(engine: &PlaybackEngine, timeout: Duration) -> usize {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(EngineEvent::TrackChanged { index, .. }) = engine.try_recv_event() {
            return index;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for TrackChanged");
}

fn drain_until_playlist_updated(engine: &PlaybackEngine) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Some(EngineEvent::PlaylistUpdated { .. }) = engine.try_recv_event() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for PlaylistUpdated");
}

/// The fixture is ~2.1s long -- natural-end tests need a deadline longer
/// than that, but short enough not to make the suite painfully slow.
const TRACK_END_TIMEOUT: Duration = Duration::from_secs(6);

#[test]
fn repeat_one_replays_the_same_track_when_it_ends_naturally() {
    let engine = PlaybackEngine::spawn();

    engine.send(EngineCommand::LoadPlaylist(vec![fixture()]));
    drain_until_playlist_updated(&engine);

    engine.send(EngineCommand::SetPlaybackMode(PlaybackMode::RepeatOne));
    engine.send(EngineCommand::Play);

    assert_eq!(next_track_changed(&engine, Duration::from_secs(2)), 0);
    // The track finishes on its own (~2.1s in) and, under RepeatOne,
    // restarts itself in place rather than stopping or advancing -- there's
    // only one track, so a *second* `TrackChanged` for index 0 only
    // happens if the repeat actually fired.
    assert_eq!(next_track_changed(&engine, TRACK_END_TIMEOUT), 0);
}

#[test]
fn repeat_all_wraps_back_to_the_first_track_after_the_last_one_ends() {
    let engine = PlaybackEngine::spawn();

    engine.send(EngineCommand::LoadPlaylist(vec![fixture(), fixture()]));
    drain_until_playlist_updated(&engine);

    engine.send(EngineCommand::SetPlaybackMode(PlaybackMode::RepeatAll));
    engine.send(EngineCommand::Play);

    assert_eq!(next_track_changed(&engine, Duration::from_secs(2)), 0);
    // Track 0 ends naturally -> advances to track 1 (normal behavior).
    assert_eq!(next_track_changed(&engine, TRACK_END_TIMEOUT), 1);
    // Track 1 (the last one) ends naturally -> RepeatAll wraps to track 0
    // instead of stopping.
    assert_eq!(next_track_changed(&engine, TRACK_END_TIMEOUT), 0);
}

#[test]
fn shuffle_mode_visits_every_track_once_before_repeating() {
    let engine = PlaybackEngine::spawn();

    const TRACK_COUNT: usize = 4;
    engine.send(EngineCommand::LoadPlaylist(vec![fixture(); TRACK_COUNT]));
    drain_until_playlist_updated(&engine);

    engine.send(EngineCommand::SetPlaybackMode(PlaybackMode::Shuffle));
    engine.send(EngineCommand::Play);

    let mut visited = HashSet::new();
    visited.insert(next_track_changed(&engine, Duration::from_secs(2)));
    for _ in 0..(TRACK_COUNT - 1) {
        engine.send(EngineCommand::Next);
        visited.insert(next_track_changed(&engine, Duration::from_secs(2)));
    }

    assert_eq!(visited, (0..TRACK_COUNT).collect());
}
