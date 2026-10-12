pub mod decoders;
pub mod dsp;
pub mod output;

mod commands;
mod events;
mod library;
mod metadata;
mod pipeline;
mod playback_engine;
mod playlist;
mod spectrum;
mod waveform;

pub use commands::EngineCommand;
pub use events::{EngineEvent, PlaybackState};
pub use library::{scan_audio_files, AUDIO_EXTENSIONS};
pub use metadata::load_metadata;
pub use playback_engine::PlaybackEngine;
pub use playlist::{Playlist, Track, TrackMetadata};
pub use waveform::compute_peaks;

pub use plugin_api::{AudioInfo, DecodeError, Decoder, Dsp, OutputError, OutputSink};

/// Probes `path` for its technical audio format (sample rate, channels, bit
/// depth, duration) without starting playback. Used by the UI's properties
/// view, which needs this for an arbitrary playlist entry, not just the
/// currently loaded track.
pub fn probe_audio_info(path: &std::path::Path) -> Result<AudioInfo, DecodeError> {
    decoders::SymphoniaDecoder::new().open(path)
}
