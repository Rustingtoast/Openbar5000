pub mod decoders;
pub mod dsp;
pub mod output;

mod background;
mod clock;
mod commands;
mod events;
mod library;
mod metadata;
mod pipeline;
mod playback_engine;
mod playlist;
mod shuffle;
mod spectrum;
mod tag_editor;
mod waveform;

pub use background::lower_thread_priority;
pub use clock::PlaybackClock;
pub use commands::{EngineCommand, PlaybackMode, TEMP_PLAYLIST_ID};
pub use dsp::{EqualizerSettings, EQ_BANDS_HZ, EQ_BAND_COUNT, EQ_MAX_DB};
pub use output::{default_output_device, output_devices, OutputDeviceInfo};
pub use events::{EngineEvent, PlaybackState};
pub use library::{scan_audio_files, AUDIO_EXTENSIONS};
pub use metadata::{
    load_cover_art, load_metadata, read_library_tags, read_lyrics, write_lyrics, LibraryTags,
    COVER_THUMBNAIL_SIZE,
};
pub use playback_engine::PlaybackEngine;
pub use playlist::{CoverThumbnail, Playlist, Track, TrackMetadata};
pub use spectrum::{SpectrumChannelMode, SpectrumConfig};
pub use tag_editor::{
    edit_tags, read_details, read_pictures, read_tag_fields, FileDetails, PictureInfo, PictureKind, TagEdit,
    TagField,
};
pub use waveform::{compute_peaks, WaveformChannel};

pub use plugin_api::{AudioInfo, DecodeError, Decoder, Dsp, OutputError, OutputSink};

/// Probes `path` for its technical audio format (sample rate, channels, bit
/// depth, duration) without starting playback. Used by the UI's properties
/// view, which needs this for an arbitrary playlist entry, not just the
/// currently loaded track.
pub fn probe_audio_info(path: &std::path::Path) -> Result<AudioInfo, DecodeError> {
    decoders::SymphoniaDecoder::new().open(path)
}
