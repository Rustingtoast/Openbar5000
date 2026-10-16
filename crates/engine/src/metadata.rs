use crate::playlist::{CoverThumbnail, TrackMetadata};
use lofty::file::{FileType, TaggedFileExt};
use lofty::prelude::{Accessor, AudioFile};
use std::path::Path;
use std::sync::Arc;
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

/// Longest edge of a playlist cover thumbnail: the slot is 48px, doubled
/// for HiDPI displays.
pub const COVER_THUMBNAIL_SIZE: u32 = 96;

/// Decodes embedded JPEG/PNG art and shrinks it (keeping aspect ratio) to
/// fit `COVER_THUMBNAIL_SIZE`. `None` if it doesn't decode.
fn cover_thumbnail(bytes: &[u8]) -> Option<CoverThumbnail> {
    scaled_cover(bytes, COVER_THUMBNAIL_SIZE)
}

/// Reads `path`'s embedded cover art at up to `max_size` px on its longest
/// edge -- for the album-art panel, which shows one cover far larger than
/// the playlist thumbnails kept in `TrackMetadata`. Decodes the file's
/// art, so call it off the UI thread. `None` if there's no usable art.
pub fn load_cover_art(path: &Path, max_size: u32) -> Option<CoverThumbnail> {
    let tagged_file = lofty::read_from_path(path).ok()?;
    scaled_cover(first_picture(&tagged_file)?, max_size)
}

/// The tag to read text fields from. Normally the format's primary tag,
/// but files exist with an empty primary tag and the real one elsewhere --
/// e.g. a FLAC whose Vorbis comments are blank while a stray ID3v2 block
/// (which lofty reads, warning that it couldn't *write* it back) holds the
/// title and artist (issue #20).
fn best_tag(tagged_file: &lofty::file::TaggedFile) -> Option<&lofty::tag::Tag> {
    let has_text = |tag: &&lofty::tag::Tag| tag.title().is_some() || tag.artist().is_some();
    tagged_file
        .primary_tag()
        .filter(has_text)
        .or_else(|| tagged_file.tags().iter().find(has_text))
        .or_else(|| tagged_file.primary_tag())
        .or_else(|| tagged_file.first_tag())
}

/// Embedded cover art from whichever tag carries one (see `best_tag`).
fn first_picture(tagged_file: &lofty::file::TaggedFile) -> Option<&[u8]> {
    let primary = tagged_file.primary_tag().into_iter();
    primary
        .chain(tagged_file.tags())
        .find_map(|tag| tag.pictures().first())
        .map(|picture| picture.data())
}

fn scaled_cover(bytes: &[u8], max_size: u32) -> Option<CoverThumbnail> {
    let decoded = image::load_from_memory(bytes).ok()?;
    // `thumbnail` only ever shrinks; small art stays as it is.
    let scaled = if decoded.width().max(decoded.height()) > max_size {
        decoded.thumbnail(max_size, max_size)
    } else {
        decoded
    };
    let thumbnail = scaled.into_rgba8();
    let (width, height) = thumbnail.dimensions();
    Some(CoverThumbnail {
        width,
        height,
        rgba: thumbnail.into_raw(),
    })
}

/// Loads metadata for every path, spreading the (disk- and decode-bound)
/// work over all CPU cores. `on_progress(done, total)` is called from this
/// thread as tracks finish, for a loading indicator. Output order matches
/// `paths`.
pub fn load_metadata_parallel(
    paths: &[std::path::PathBuf],
    mut on_progress: impl FnMut(usize, usize),
) -> Vec<TrackMetadata> {
    let total = paths.len();
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(total.max(1));
    let next = std::sync::atomic::AtomicUsize::new(0);
    let (done_tx, done_rx) = crossbeam_channel::unbounded();

    let mut results: Vec<Option<TrackMetadata>> = vec![None; total];
    std::thread::scope(|scope| {
        for _ in 0..workers {
            let done_tx = done_tx.clone();
            let next = &next;
            scope.spawn(move || loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(path) = paths.get(i) else { break };
                if done_tx.send((i, load_metadata(path))).is_err() {
                    break;
                }
            });
        }
        drop(done_tx);
        for (done, (i, metadata)) in done_rx.iter().enumerate() {
            results[i] = Some(metadata);
            on_progress(done + 1, total);
        }
    });
    results
        .into_iter()
        .map(|m| m.expect("every index is loaded exactly once"))
        .collect()
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

    let tag = best_tag(&tagged_file);

    let (title, artist, album, track_number, disc_number, date, cover_thumbnail) = match tag {
        Some(tag) => (
            tag.title()
                .map(|s| s.into_owned())
                .unwrap_or(fallback_title),
            tag.artist().map(|s| s.into_owned()).unwrap_or_default(),
            tag.album().map(|s| s.into_owned()).unwrap_or_default(),
            tag.track(),
            tag.disk(),
            tag.date().map(|d| d.to_string()).unwrap_or_default(),
            first_picture(&tagged_file)
                .and_then(cover_thumbnail)
                .map(Arc::new),
        ),
        None => (
            fallback_title,
            String::new(),
            String::new(),
            None,
            None,
            String::new(),
            None,
        ),
    };

    TrackMetadata {
        title,
        artist,
        album,
        track_number,
        disc_number,
        date,
        duration,
        cover_thumbnail,
        format_label,
        bitrate_kbps,
        sample_rate,
        bit_depth,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::DynamicImage::new_rgba8(width, height)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        bytes
    }

    #[test]
    fn cover_thumbnails_shrink_to_fit_and_keep_aspect_ratio() {
        let thumb = cover_thumbnail(&png(1000, 500)).unwrap();
        assert_eq!(
            (thumb.width, thumb.height),
            (COVER_THUMBNAIL_SIZE, COVER_THUMBNAIL_SIZE / 2)
        );
        assert_eq!(thumb.rgba.len(), (thumb.width * thumb.height * 4) as usize);
        assert!(cover_thumbnail(b"not an image").is_none());

        // Larger sizes shrink to the requested bound; small art isn't
        // blown up.
        let big = scaled_cover(&png(1200, 600), 600).unwrap();
        assert_eq!((big.width, big.height), (600, 300));
        let small = scaled_cover(&png(200, 100), 600).unwrap();
        assert_eq!((small.width, small.height), (200, 100));
    }

    /// Issue #20: a FLAC with blank Vorbis comments and the real tags in a
    /// stray ID3v2 block in front of it still gets its title and artist.
    #[test]
    fn flac_with_stray_id3v2_tag_reads_its_title() {
        use lofty::config::WriteOptions;
        use lofty::prelude::TagExt;
        use lofty::tag::{Tag, TagType};

        let mut id3 = Tag::new(TagType::Id3v2);
        id3.set_title("Helpless".into());
        id3.set_artist("Alexander Popov".into());
        let mut bytes = Vec::new();
        id3.dump_to(&mut bytes, WriteOptions::default()).unwrap();

        // A minimal FLAC: "fLaC", STREAMINFO, then an empty (last)
        // VORBIS_COMMENT block. Tag reading needs no audio frames.
        bytes.extend_from_slice(b"fLaC");
        bytes.extend_from_slice(&[0x00, 0x00, 0x00, 34]);
        bytes.extend_from_slice(&[0x10, 0x00, 0x10, 0x00, 0, 0, 0, 0, 0, 0]);
        let packed: u64 = (44_100 << 44) | (1 << 41) | (15 << 36);
        bytes.extend_from_slice(&packed.to_be_bytes());
        bytes.extend_from_slice(&[0; 16]);
        let vendor = b"test";
        let comment_len = 4 + vendor.len() + 4;
        bytes.extend_from_slice(&[0x84, 0, 0, comment_len as u8]);
        bytes.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
        bytes.extend_from_slice(vendor);
        bytes.extend_from_slice(&0u32.to_le_bytes());

        let path = std::env::temp_dir().join(format!("openbar5000-id3-in-flac-{}.flac", std::process::id()));
        std::fs::write(&path, &bytes).unwrap();
        // Precondition: the format's primary tag is the blank Vorbis one,
        // so just taking the primary tag (the old behaviour) would miss
        // the title.
        let tagged = lofty::read_from_path(&path).unwrap();
        assert!(tagged.primary_tag().is_some_and(|tag| tag.title().is_none()));
        let metadata = load_metadata(&path);
        std::fs::remove_file(&path).ok();
        assert_eq!(metadata.title, "Helpless");
        assert_eq!(metadata.artist, "Alexander Popov");
        assert_eq!(metadata.format_label, "FLAC");
    }

    #[test]
    fn parallel_load_keeps_input_order_and_reports_progress() {
        let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/test_tone.wav");
        let paths = vec![
            fixture.clone(),
            std::path::PathBuf::from("/nonexistent/b.flac"),
            fixture,
        ];
        let mut last = (0, 0);
        let metadata = load_metadata_parallel(&paths, |done, total| last = (done, total));
        assert_eq!(last, (3, 3));
        // The unreadable file falls back to its file name, in its own slot.
        assert_eq!(metadata[1].title, "b");
        assert_ne!(metadata[0].title, "b");
    }
}
