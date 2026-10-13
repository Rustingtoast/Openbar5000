//! Exercises `RemoveIndex`/`ClearPlaylist` end-to-end through the real
//! playback thread, confirming the emitted `PlaylistUpdated` events reflect
//! the mutation (added alongside the "Open Folder"/"Properties" UI refinement,
//! which depends on this event always describing the playlist accurately).

use engine::{EngineCommand, EngineEvent, PlaybackEngine};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/test_tone.wav")
}

/// Polls for the next `PlaylistUpdated` event, failing the test if none
/// arrives within a few seconds.
fn next_playlist_update(engine: &PlaybackEngine) -> (usize, Option<usize>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Some(EngineEvent::PlaylistUpdated {
            tracks,
            current_index,
        }) = engine.try_recv_event()
        {
            return (tracks.len(), current_index);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for PlaylistUpdated");
}

#[test]
fn remove_and_clear_update_the_playlist() {
    let engine = PlaybackEngine::spawn();

    engine.send(EngineCommand::LoadPlaylist(vec![fixture(), fixture()]));
    let (count, current) = next_playlist_update(&engine);
    assert_eq!(count, 2);
    assert_eq!(current, None);

    engine.send(EngineCommand::RemoveIndex(0));
    let (count, current) = next_playlist_update(&engine);
    assert_eq!(count, 1);
    assert_eq!(current, None);

    engine.send(EngineCommand::ClearPlaylist);
    let (count, current) = next_playlist_update(&engine);
    assert_eq!(count, 0);
    assert_eq!(current, None);
}

/// Polls for the next `TrackChanged` event (its `index`).
fn next_track_changed(engine: &PlaybackEngine) -> usize {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Some(EngineEvent::TrackChanged { index, .. }) = engine.try_recv_event() {
            return index;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for TrackChanged");
}

#[test]
fn reorder_keeps_the_current_track_and_rejects_stale_orders() {
    let engine = PlaybackEngine::spawn();

    engine.send(EngineCommand::LoadPlaylist(vec![
        fixture(),
        fixture(),
        fixture(),
    ]));
    next_playlist_update(&engine);
    engine.send(EngineCommand::PlayIndex(0));
    assert_eq!(next_track_changed(&engine), 0);

    engine.send(EngineCommand::ReorderPlaylist(vec![2, 0, 1]));
    let (count, current) = next_playlist_update(&engine);
    assert_eq!(count, 3);
    assert_eq!(current, Some(1));

    // Wrong length (computed against an older playlist): ignored, so the
    // next update seen is the one from the clear below.
    engine.send(EngineCommand::ReorderPlaylist(vec![1, 0]));
    engine.send(EngineCommand::ClearPlaylist);
    assert_eq!(next_playlist_update(&engine), (0, None));
}

#[test]
fn queued_tracks_play_before_the_normal_order() {
    let engine = PlaybackEngine::spawn();

    engine.send(EngineCommand::LoadPlaylist(vec![
        fixture(),
        fixture(),
        fixture(),
    ]));
    next_playlist_update(&engine);
    engine.send(EngineCommand::EnqueueIndex(2));
    engine.send(EngineCommand::PlayIndex(0));
    assert_eq!(next_track_changed(&engine), 0);

    engine.send(EngineCommand::Next);
    assert_eq!(next_track_changed(&engine), 2);
    // Queue consumed -- back to the normal order from where we are now.
    engine.send(EngineCommand::Previous);
    assert_eq!(next_track_changed(&engine), 1);
}
