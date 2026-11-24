//! Exercises `RemoveIndex`/`ClearPlaylist` end-to-end through the real
//! playback thread, confirming the emitted `PlaylistUpdated` events reflect
//! the mutation (added alongside the "Open Folder"/"Properties" UI refinement,
//! which depends on this event always describing the playlist accurately).

use engine::{EngineCommand, EngineEvent, PlaybackEngine, PlaybackMode, PlaybackState};
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
            ..
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

/// The next `PlaylistUpdated`'s track count, current index and playlist id.
fn next_update_with_id(engine: &PlaybackEngine) -> (usize, Option<usize>, i64) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Some(EngineEvent::PlaylistUpdated { tracks, current_index, playlist_id }) =
            engine.try_recv_event()
        {
            return (tracks.len(), current_index, playlist_id);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for PlaylistUpdated");
}

#[test]
fn several_tracks_go_in_one_removal() {
    let engine = PlaybackEngine::spawn();
    engine.send(EngineCommand::LoadPlaylist(vec![fixture(), fixture(), fixture(), fixture()]));
    next_playlist_update(&engine);
    engine.send(EngineCommand::RemoveIndices(vec![0, 2, 2, 3]));
    let (count, _) = next_playlist_update(&engine);
    assert_eq!(count, 1);
}

#[test]
fn switching_playlists_keeps_the_song_playing_and_names_the_playlist() {
    // A second file, so one playlist can lack the song that's playing.
    let other = std::env::temp_dir().join(format!("openbar5000-switch-{}.wav", std::process::id()));
    std::fs::copy(fixture(), &other).unwrap();

    let engine = PlaybackEngine::spawn();
    engine.send(EngineCommand::SwitchPlaylist { id: 7, paths: vec![fixture(), other.clone()] });
    assert_eq!(next_update_with_id(&engine), (2, None, 7));
    engine.send(EngineCommand::PlayIndex(0));
    assert_eq!(next_track_changed(&engine), 0);

    // The playing song is in the new playlist too: it's the current one.
    engine.send(EngineCommand::SwitchPlaylist { id: 8, paths: vec![other.clone(), fixture()] });
    assert_eq!(next_update_with_id(&engine), (2, Some(1), 8));

    // It isn't: nothing is current, but playback isn't stopped.
    engine.send(EngineCommand::SwitchPlaylist { id: 9, paths: vec![other.clone()] });
    assert_eq!(next_update_with_id(&engine), (1, None, 9));
    let stopped = std::iter::from_fn(|| engine.try_recv_event())
        .any(|event| matches!(event, EngineEvent::PlaybackStateChanged(engine::PlaybackState::Stopped)));
    assert!(!stopped, "switching playlists must not stop playback");

    std::fs::remove_file(&other).unwrap();
}

#[test]
fn play_library_reports_the_throwaway_playlist_id_and_plays_the_picked_track() {
    let engine = PlaybackEngine::spawn();

    // The media library plays a whole album (here three copies of the
    // fixture) starting at the double-clicked track, and must report
    // `TEMP_PLAYLIST_ID` so the app knows not to save it over a named
    // playlist (issue #37).
    engine.send(EngineCommand::PlayLibrary {
        paths: vec![fixture(), fixture(), fixture()],
        start: 1,
    });
    let (count, current, id) = next_update_with_id(&engine);
    assert_eq!(count, 3);
    assert_eq!(current, None);
    assert_eq!(id, engine::TEMP_PLAYLIST_ID);

    assert_eq!(next_track_changed(&engine), 1);
}

/// Every event until one matches `done` (included), failing the test if
/// none does within a few seconds.
fn events_until(engine: &PlaybackEngine, done: impl Fn(&EngineEvent) -> bool) -> Vec<EngineEvent> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut events = Vec::new();
    while Instant::now() < deadline {
        match engine.try_recv_event() {
            Some(event) => {
                let finished = done(&event);
                events.push(event);
                if finished {
                    return events;
                }
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    panic!("timed out; got {events:?}");
}

/// Every event arriving within `wait`.
fn events_for(engine: &PlaybackEngine, wait: Duration) -> Vec<EngineEvent> {
    std::thread::sleep(wait);
    std::iter::from_fn(|| engine.try_recv_event()).collect()
}

fn is_state(event: &EngineEvent, state: PlaybackState) -> bool {
    matches!(event, EngineEvent::PlaybackStateChanged(s) if *s == state)
}

/// A muted engine holding `count` copies of the fixture.
fn engine_with_tracks(count: usize) -> PlaybackEngine {
    let engine = PlaybackEngine::spawn();
    engine.send(EngineCommand::SetVolume(0.0));
    engine.send(EngineCommand::LoadPlaylist(vec![fixture(); count]));
    next_playlist_update(&engine);
    engine
}

const RANDOM: EngineCommand = EngineCommand::PlayRandom { among: None };

#[test]
fn play_random_starts_another_track_from_the_playlist() {
    let engine = PlaybackEngine::spawn();
    engine.send(EngineCommand::SetVolume(0.0));

    // Nothing to pick from: nothing happens.
    engine.send(RANDOM);
    let reacted = events_for(&engine, Duration::from_millis(200)).iter().any(|event| {
        matches!(event, EngineEvent::TrackChanged { .. } | EngineEvent::PlaybackStateChanged(_))
    });
    assert!(!reacted, "Random on an empty playlist must do nothing");

    engine.send(EngineCommand::LoadPlaylist(vec![fixture(); 3]));
    next_playlist_update(&engine);

    // From a stop, it plays.
    engine.send(RANDOM);
    let events = events_until(&engine, |e| is_state(e, PlaybackState::Playing));
    let mut current = events
        .iter()
        .find_map(|e| match e {
            EngineEvent::TrackChanged { index, .. } => Some(*index),
            _ => None,
        })
        .expect("Random from a stop loads a track");
    assert!(current < 3);

    // Once one plays, Random always moves to a different one.
    for _ in 0..10 {
        engine.send(RANDOM);
        let next = next_track_changed(&engine);
        assert!(next < 3);
        assert_ne!(next, current, "Random replayed the track that was playing");
        current = next;
    }
}

#[test]
fn play_random_only_picks_among_the_tracks_given() {
    let engine = engine_with_tracks(5);
    engine.send(EngineCommand::PlayIndex(0));
    assert_eq!(next_track_changed(&engine), 0);

    // Track 0 plays but isn't among them (a search hides it): any of them.
    // Then, never the one playing: they alternate. 9 is stale, ignored.
    let among = || EngineCommand::PlayRandom { among: Some(vec![1, 3, 9]) };
    engine.send(among());
    let mut current = next_track_changed(&engine);
    assert!(current == 1 || current == 3);
    for _ in 0..6 {
        engine.send(among());
        let next = next_track_changed(&engine);
        assert_eq!(next, 4 - current);
        current = next;
    }

    // Nothing to pick from: nothing happens.
    engine.send(EngineCommand::PlayRandom { among: Some(vec![]) });
    let reacted = events_for(&engine, Duration::from_millis(200))
        .iter()
        .any(|e| matches!(e, EngineEvent::TrackChanged { .. }));
    assert!(!reacted, "Random with nothing showing must do nothing");
}

#[test]
fn play_random_while_paused_changes_the_track_but_stays_paused() {
    let engine = engine_with_tracks(3);
    engine.send(EngineCommand::PlayIndex(0));
    assert_eq!(next_track_changed(&engine), 0);
    engine.send(EngineCommand::Pause);
    events_until(&engine, |e| is_state(e, PlaybackState::Paused));

    engine.send(RANDOM);
    assert_ne!(next_track_changed(&engine), 0);
    let started = events_for(&engine, Duration::from_millis(300))
        .iter()
        .any(|e| is_state(e, PlaybackState::Playing));
    assert!(!started, "Random started a paused player, unlike Next");
}

#[test]
fn play_random_after_a_stop_can_pick_the_last_track() {
    let engine = engine_with_tracks(2);
    let mut last = None;
    let mut picked_last = false;
    // Two tracks: from a stop each comes up half the time, so the last
    // one turns up within 30 tries but for a 1 in 2^30 chance.
    for _ in 0..30 {
        engine.send(EngineCommand::Stop);
        engine.send(RANDOM);
        let pick = next_track_changed(&engine);
        picked_last |= Some(pick) == last;
        last = Some(pick);
    }
    assert!(picked_last, "after a stop, Random never picked the track stopped on");
}

#[test]
fn play_random_in_shuffle_mode_carries_the_shuffle_on() {
    let engine = engine_with_tracks(4);
    engine.send(EngineCommand::SetPlaybackMode(PlaybackMode::Shuffle));
    engine.send(EngineCommand::PlayIndex(2));
    assert_eq!(next_track_changed(&engine), 2);

    // Random never brings back a track this cycle already played: the
    // starting track and three Randoms are all four.
    let mut heard = vec![2];
    for _ in 0..3 {
        engine.send(RANDOM);
        heard.push(next_track_changed(&engine));
    }
    let mut sorted = heard.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, vec![0, 1, 2, 3], "a track came back within the cycle: {heard:?}");

    // And Previous still goes back through them.
    engine.send(EngineCommand::Previous);
    assert_eq!(next_track_changed(&engine), heard[2]);
}

/// A muted engine playing track 0 of [fixture, missing file, fixture].
fn engine_with_a_missing_track(name: &str) -> PlaybackEngine {
    let missing = std::env::temp_dir().join(format!("openbar5000-{name}-{}.wav", std::process::id()));
    let engine = PlaybackEngine::spawn();
    engine.send(EngineCommand::SetVolume(0.0));
    engine.send(EngineCommand::LoadPlaylist(vec![fixture(), missing, fixture()]));
    next_playlist_update(&engine);
    engine.send(EngineCommand::PlayIndex(0));
    assert_eq!(next_track_changed(&engine), 0);
    events_until(&engine, |e| is_state(e, PlaybackState::Playing));
    engine
}

#[test]
fn a_track_that_cant_be_opened_leaves_the_one_playing_alone() {
    let engine = engine_with_a_missing_track("missing-pick");
    engine.send(EngineCommand::PlayIndex(1));
    let events = events_until(&engine, |e| matches!(e, EngineEvent::Error(_)));
    let later = events_for(&engine, Duration::from_millis(300));
    let changed = events.iter().chain(&later).any(|e| {
        matches!(e, EngineEvent::TrackChanged { .. } | EngineEvent::PlaybackStateChanged(_))
    });
    assert!(!changed, "a failed pick changed playback: {events:?} {later:?}");
}

#[test]
fn next_skips_a_track_that_cant_be_opened() {
    let engine = engine_with_a_missing_track("missing-next");
    engine.send(EngineCommand::Next);
    // The track change, then the error for the skipped file (so the
    // status bar ends on it), and no stop in between.
    let events = events_until(&engine, |e| matches!(e, EngineEvent::Error(_)));
    assert!(
        matches!(events.as_slice(), [EngineEvent::TrackChanged { index: 2, .. }, EngineEvent::Error(_)]),
        "{events:?}"
    );
}

#[test]
fn next_skipping_a_track_while_paused_stays_paused() {
    let engine = engine_with_a_missing_track("missing-paused");
    engine.send(EngineCommand::Pause);
    events_until(&engine, |e| is_state(e, PlaybackState::Paused));
    engine.send(EngineCommand::Next);
    let events = events_until(&engine, |e| matches!(e, EngineEvent::Error(_)));
    assert!(matches!(events.first(), Some(EngineEvent::TrackChanged { index: 2, .. })), "{events:?}");
    let later = events_for(&engine, Duration::from_millis(300));
    assert!(
        !events.iter().chain(&later).any(|e| matches!(e, EngineEvent::PlaybackStateChanged(_))),
        "skipping while paused changed the state: {events:?} {later:?}"
    );
}
