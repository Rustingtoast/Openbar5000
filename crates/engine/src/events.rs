use crate::playlist::Track;
use crate::waveform::WaveformChannel;
use std::time::Duration;

/// Sent from the playback thread to the UI thread. The UI drains these via a
/// timer on the Slint event loop (see `crates/app`) and never talks to the
/// engine thread directly otherwise.
#[derive(Debug, Clone)]
pub enum EngineEvent {
    TrackChanged {
        index: usize,
        /// Boxed: a track (with all its tags) is far larger than every
        /// other event, which are sent many times a second.
        track: Box<Track>,
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
    /// Sent periodically while a `LoadPlaylist`/`LoadFolder`/
    /// `RefreshMetadata` reads tags, so the UI can show progress; the load
    /// finishes with `PlaylistUpdated`.
    LoadProgress {
        done: usize,
        total: usize,
    },
    /// The whole track's waveform outline, one entry per channel, computed
    /// on a background thread after `TrackChanged`; drives the waveform
    /// seekbar.
    WaveformReady {
        index: usize,
        channels: Vec<WaveformChannel>,
    },
    /// Live spectrum bars (and, if enabled, their falling peak markers) for
    /// the audio currently being written to the output device, sent at the
    /// configured interval while enabled.
    SpectrumUpdate {
        bars: Vec<f32>,
        peaks: Vec<f32>,
    },
    /// Playback now goes to this device; `fallback` when it's the system
    /// default standing in for a chosen device that isn't connected.
    OutputDeviceChanged { name: String, fallback: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    Stopped,
    Playing,
    Paused,
}
