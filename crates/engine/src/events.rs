use crate::playlist::Track;
use std::time::Duration;

/// Sent from the playback thread to the UI thread. The UI drains these via a
/// timer on the Slint event loop (see `crates/app`) and never talks to the
/// engine thread directly otherwise.
#[derive(Debug, Clone)]
pub enum EngineEvent {
    TrackChanged {
        index: usize,
        track: Track,
    },
    PositionUpdate {
        position: Duration,
        duration: Duration,
    },
    PlaybackStateChanged(PlaybackState),
    Error(String),
    Underrun,
    PlaylistLoaded {
        tracks: Vec<Track>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    Stopped,
    Playing,
    Paused,
}
