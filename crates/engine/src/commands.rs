use crate::dsp::EqualizerSettings;
use crate::spectrum::SpectrumConfig;
use std::time::Duration;

/// How the playlist advances once a track ends (naturally, or via
/// Next/Previous) -- toggled by the shuffle/repeat buttons in the
/// transport bar, and mutually exclusive by construction (there's exactly
/// one active mode, not two independent flags).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlaybackMode {
    #[default]
    Normal,
    /// Wraps back to the first track once the last one ends (naturally, or
    /// via Next), and symmetrically wraps to the last track if Previous is
    /// pressed on the first one -- the queue repeats as a whole.
    RepeatAll,
    /// Replays the current track in place when it reaches its end.
    /// Explicit Next/Previous still navigate normally (no wraparound) --
    /// this only governs what happens when a track finishes on its own.
    RepeatOne,
    /// Advances through a randomized order that visits every playlist
    /// track once before any repeat.
    Shuffle,
}

/// Sent from the UI thread to the playback thread. See
/// [`crate::PlaybackEngine`] for how these are consumed.
#[derive(Debug, Clone)]
pub enum EngineCommand {
    Play,
    Pause,
    Stop,
    /// Seek within the current track.
    Seek(Duration),
    Next,
    Previous,
    /// 0.0 (silent) to 1.0 (unity gain).
    SetVolume(f32),
    LoadPlaylist(Vec<std::path::PathBuf>),
    /// Adds tracks to the end of the playlist, keeping what's playing.
    AppendToPlaylist(Vec<std::path::PathBuf>),
    /// Recursively scans a folder for audio files and loads them as the
    /// playlist (done on the engine thread so a big or slow folder never
    /// blocks the UI). Sends `EngineEvent::Error` instead, leaving the
    /// playlist untouched, if nothing playable is found.
    LoadFolder(std::path::PathBuf),
    /// Jump to a specific index in the current playlist and play it.
    PlayIndex(usize),
    /// Removes a single track from the playlist by index.
    RemoveIndex(usize),
    ClearPlaylist,
    /// Reorders the playlist so position `k` holds the track currently at
    /// `order[k]` (the playlist's Sort submenu). Ignored unless `order` is a
    /// permutation of the playlist's current indices.
    ReorderPlaylist(Vec<usize>),
    /// Re-reads tag metadata for every playlist entry from disk, keeping
    /// order and what's playing.
    RefreshMetadata,
    /// Appends a track to the playback queue: queued tracks play next, in
    /// order, ahead of whatever the playback mode would otherwise pick.
    EnqueueIndex(usize),
    /// Gates the (otherwise per-chunk) live FFT work in the pipeline --
    /// disabled by default so playback never pays for it unless the
    /// spectrum view is actually visible.
    SetSpectrumEnabled(bool),
    /// Gates whether loading a track spawns a background waveform-peak
    /// computation.
    SetWaveformEnabled(bool),
    /// Channels/amplitude-range/FFT-size/peak-physics settings for the
    /// spectrum analyzer.
    SetSpectrumConfig(SpectrumConfig),
    SetPlaybackMode(PlaybackMode),
    /// The graphic equalizer's bands and on/off switch (issue #7); takes
    /// effect from the next decoded chunk.
    SetEqualizer(EqualizerSettings),
    /// Plays through the output device with this id (see
    /// [`crate::output_devices`]), or `None` to follow the system default
    /// -- including when it changes, e.g. headphones plugged in. A chosen
    /// device that's unplugged falls back to the default until it returns.
    SetOutputDevice(Option<String>),
}
