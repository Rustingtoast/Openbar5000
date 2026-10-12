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
