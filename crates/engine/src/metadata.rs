use crate::playlist::TrackMetadata;
use lofty::file::TaggedFileExt;
use lofty::prelude::{Accessor, AudioFile};
use std::path::Path;
use std::time::Duration;

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

    let duration = tagged_file.properties().duration();
    let tag = tagged_file
        .primary_tag()
        .or_else(|| tagged_file.first_tag());

    let (title, artist, album, track_number) = match tag {
        Some(tag) => (
            tag.title()
                .map(|s| s.into_owned())
                .unwrap_or(fallback_title),
            tag.artist().map(|s| s.into_owned()).unwrap_or_default(),
            tag.album().map(|s| s.into_owned()).unwrap_or_default(),
            tag.track(),
        ),
        None => (fallback_title, String::new(), String::new(), None),
    };

    TrackMetadata {
        title,
        artist,
        album,
        track_number,
        duration: Duration::from_secs_f64(duration.as_secs_f64()),
    }
}
