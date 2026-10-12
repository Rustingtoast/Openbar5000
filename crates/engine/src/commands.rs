use crate::spectrum::SpectrumConfig;
use std::time::Duration;

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
    /// Jump to a specific index in the current playlist and play it.
    PlayIndex(usize),
    /// Removes a single track from the playlist by index.
    RemoveIndex(usize),
    ClearPlaylist,
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
}
