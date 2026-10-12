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
}
