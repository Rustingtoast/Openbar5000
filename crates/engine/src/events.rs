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
    /// Sent on a fresh load and again whenever the playlist's contents
    /// change (track removed, playlist cleared). `current_index` reflects
    /// what's still playing/loaded after the change, if anything.
    PlaylistUpdated {
        tracks: Vec<Track>,
        current_index: Option<usize>,
    },
    /// Peak amplitudes for the whole track, computed on a background thread
    /// after `TrackChanged`; drives the waveform seekbar.
    WaveformReady {
        index: usize,
        peaks: Vec<f32>,
    },
    /// Live spectrum bars (and, if enabled, their falling peak markers) for
    /// the audio currently being written to the output device, sent at the
    /// configured interval while enabled.
    SpectrumUpdate {
        bars: Vec<f32>,
        peaks: Vec<f32>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    Stopped,
    Playing,
    Paused,
}
