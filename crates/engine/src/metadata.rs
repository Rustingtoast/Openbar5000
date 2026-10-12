use crate::playlist::TrackMetadata;
use lofty::file::{FileType, TaggedFileExt};
use lofty::prelude::{Accessor, AudioFile};
use std::path::Path;
use std::time::Duration;

fn format_label(file_type: FileType) -> String {
    match file_type {
        FileType::Aac => "AAC",
        FileType::Aiff => "AIFF",
        FileType::Ape => "APE",
        FileType::Flac => "FLAC",
        FileType::Mpeg => "MP3",
        FileType::Mp4 => "MP4",
        FileType::Mpc => "MPC",
        FileType::Opus => "Opus",
        FileType::Vorbis => "Vorbis",
        FileType::Speex => "Speex",
        FileType::Wav => "WAV",
        FileType::WavPack => "WavPack",
        FileType::Custom(name) => name,
        _ => "",
    }
    .to_string()
}

/// Reads tag + duration metadata for the playlist grid. Best-effort: a file
/// lofty can't parse still gets a `TrackMetadata` falling back to its file
/// name, rather than failing the whole playlist load.
pub fn load_metadata(path: &Path) -> TrackMetadata {
    let fallback_title = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();

    let Ok(tagged_file) = lofty::read_from_path(path) else {
        return TrackMetadata {
            title: fallback_title,
            ..Default::default()
        };
    };

    let properties = tagged_file.properties();
    let duration = Duration::from_secs_f64(properties.duration().as_secs_f64());
    let bitrate_kbps = properties.audio_bitrate();
    let sample_rate = properties.sample_rate();
    let bit_depth = properties.bit_depth();
    let format_label = format_label(tagged_file.file_type());

    let tag = tagged_file
        .primary_tag()
        .or_else(|| tagged_file.first_tag());

    let (title, artist, album, track_number, cover_art) = match tag {
        Some(tag) => (
            tag.title()
                .map(|s| s.into_owned())
                .unwrap_or(fallback_title),
            tag.artist().map(|s| s.into_owned()).unwrap_or_default(),
            tag.album().map(|s| s.into_owned()).unwrap_or_default(),
            tag.track(),
            tag.pictures().first().map(|pic| pic.data().to_vec()),
        ),
        None => (fallback_title, String::new(), String::new(), None, None),
    };

    TrackMetadata {
        title,
        artist,
        album,
        track_number,
        duration,
        cover_art,
        format_label,
        bitrate_kbps,
        sample_rate,
        bit_depth,
    }
}
