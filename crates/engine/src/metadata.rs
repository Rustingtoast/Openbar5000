use crate::playlist::{CoverThumbnail, TrackMetadata};
use lofty::file::{FileType, TaggedFileExt};
use lofty::prelude::{Accessor, AudioFile};
use std::collections::HashMap;
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

/// Thumbnails already made, by a hash of the embedded image's bytes: an
/// album's tracks usually all embed the same cover, and decoding a large
/// JPEG/PNG is by far the slowest part of reading a track (~97% of loading
/// a playlist of tagged albums), so each distinct cover is decoded once.
/// Each entry is a `OnceLock` so loader threads that meet the same new
/// cover at once wait for one decode instead of all doing it.
type CoverSlot = Arc<std::sync::OnceLock<Option<Arc<CoverThumbnail>>>>;
static COVER_CACHE: std::sync::Mutex<Option<HashMap<u64, CoverSlot>>> = std::sync::Mutex::new(None);
/// Distinct covers kept (at ~37 KB each) before the cache starts over.
const COVER_CACHE_LIMIT: usize = 4096;

fn shared_cover_thumbnail(bytes: &[u8]) -> Option<Arc<CoverThumbnail>> {
    let key = {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        bytes.hash(&mut hasher);
        hasher.finish()
    };
    let slot = {
        let mut cache = COVER_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        let cache = cache.get_or_insert_with(HashMap::new);
        if cache.len() >= COVER_CACHE_LIMIT && !cache.contains_key(&key) {
            cache.clear();
        }
        cache.entry(key).or_default().clone()
    };
    slot.get_or_init(|| cover_thumbnail(bytes).map(Arc::new)).clone()
}

/// Reads `path`'s embedded cover art at up to `max_size` px on its longest
/// edge -- for the album-art panel, which shows one cover far larger than
/// the playlist thumbnails kept in `TrackMetadata`. Decodes the file's
/// art, so call it off the UI thread. `None` if there's no usable art.
pub fn load_cover_art(path: &Path, max_size: u32) -> Option<CoverThumbnail> {
    let tagged_file = lofty::read_from_path(path).ok()?;
    scaled_cover(first_picture(&tagged_file)?, max_size)
}

/// Field names lyrics are stored under, compared with [`lyrics_key`]
/// (case, spaces, `_` and `-` ignored): `LYRICS`, `UNSYNCEDLYRICS`,
/// foobar2000/Qobuz-style `UNSYNCED LYRICS`, `UNSYNC_LYRICS`, ...
const LYRICS_FIELDS: [&str; 4] = ["LYRICS", "UNSYNCEDLYRICS", "UNSYNCLYRICS", "SYNCEDLYRICS"];

fn lyrics_key(name: &str) -> bool {
    let name: String = name
        .chars()
        .filter(|c| !matches!(c, ' ' | '_' | '-'))
        .collect::<String>()
        .to_uppercase();
    LYRICS_FIELDS.contains(&name.as_str())
}

/// The audio container of `path`, sniffed from its content.
fn file_type(path: &Path) -> Option<lofty::file::FileType> {
    lofty::probe::Probe::open(path).ok()?.guess_file_type().ok()?.file_type()
}

fn open_for_tags(path: &Path) -> Result<std::fs::File, String> {
    std::fs::File::open(path).map_err(|e| format!("couldn't open the file: {e}"))
}

/// Lyrics in an ID3v2 tag (MP3, WAV, AIFF, and stray ones in FLAC): a
/// USLT frame in any language, else a TXXX frame named like lyrics.
fn id3v2_lyrics(tag: &lofty::id3::v2::Id3v2Tag) -> Option<String> {
    use lofty::id3::v2::Frame;
    tag.iter()
        .find_map(|frame| match frame {
            Frame::UnsynchronizedText(f) if !f.content.trim().is_empty() => Some(f.content.to_string()),
            _ => None,
        })
        .or_else(|| {
            tag.iter().find_map(|frame| match frame {
                Frame::UserText(f) if lyrics_key(&f.description) && !f.content.trim().is_empty() => {
                    Some(f.content.to_string())
                }
                _ => None,
            })
        })
}

/// Lyrics in Vorbis comments (FLAC, Ogg), under any common field name.
fn vorbis_lyrics(tag: &lofty::ogg::tag::VorbisComments) -> Option<String> {
    tag.items()
        .find(|(key, value)| lyrics_key(key) && !value.trim().is_empty())
        .map(|(_, value)| value.to_string())
}

/// The lyrics stored in `path`'s metadata, if any (plain or LRC-timed
/// text).
///
/// Read with each format's own tag API: lofty's format-neutral view drops
/// fields it has no name for, such as the `UNSYNCED LYRICS` Vorbis comment
/// foobar2000 and Qobuz write.
pub fn read_lyrics(path: &Path) -> Option<String> {
    use lofty::config::ParseOptions;
    use lofty::file::{AudioFile, FileType};

    let found = match file_type(path)? {
        FileType::Flac => {
            let flac = lofty::flac::FlacFile::read_from(&mut open_for_tags(path).ok()?, ParseOptions::new()).ok()?;
            flac.vorbis_comments()
                .and_then(vorbis_lyrics)
                .or_else(|| flac.id3v2().and_then(id3v2_lyrics))
        }
        FileType::Vorbis => {
            let ogg = lofty::ogg::VorbisFile::read_from(&mut open_for_tags(path).ok()?, ParseOptions::new()).ok()?;
            vorbis_lyrics(ogg.vorbis_comments())
        }
        FileType::Opus => {
            let ogg = lofty::ogg::OpusFile::read_from(&mut open_for_tags(path).ok()?, ParseOptions::new()).ok()?;
            vorbis_lyrics(ogg.vorbis_comments())
        }
        FileType::Mpeg => {
            let mp3 = lofty::mpeg::MpegFile::read_from(&mut open_for_tags(path).ok()?, ParseOptions::new()).ok()?;
            mp3.id3v2().and_then(id3v2_lyrics)
        }
        FileType::Wav => {
            let wav = lofty::iff::wav::WavFile::read_from(&mut open_for_tags(path).ok()?, ParseOptions::new()).ok()?;
            wav.id3v2().and_then(id3v2_lyrics)
        }
        FileType::Aiff => {
            let aiff = lofty::iff::aiff::AiffFile::read_from(&mut open_for_tags(path).ok()?, ParseOptions::new()).ok()?;
            aiff.id3v2().and_then(id3v2_lyrics)
        }
        _ => {
            use lofty::tag::ItemKey;
            let tagged_file = lofty::read_from_path(path).ok()?;
            let primary = tagged_file.primary_tag().into_iter();
            primary
                .chain(tagged_file.tags())
                .find_map(|tag| {
                    tag.get_string(ItemKey::Lyrics)
                        .or_else(|| tag.get_string(ItemKey::UnsyncLyrics))
                })
                .map(str::to_string)
        }
    };
    found.filter(|text| !text.trim().is_empty())
}

/// Replaces the lyrics in an ID3v2 tag: every USLT frame and lyrics-named
/// TXXX goes, and `lyrics` (if any) becomes one USLT frame in English --
/// what Windows, iTunes and taggers look for (lofty's generic path writes
/// the "unknown" language `XXX`, which some ignore).
fn set_id3v2_lyrics(tag: &mut lofty::id3::v2::Id3v2Tag, lyrics: &str) {
    use lofty::id3::v2::{Frame, UnsynchronizedTextFrame};
    use lofty::TextEncoding;
    tag.retain(|frame| match frame {
        Frame::UnsynchronizedText(_) => false,
        Frame::UserText(f) => !lyrics_key(&f.description),
        _ => true,
    });
    if !lyrics.is_empty() {
        // UTF-16 is valid in both ID3v2.3 and v2.4.
        tag.insert(Frame::UnsynchronizedText(UnsynchronizedTextFrame::new(
            TextEncoding::UTF16,
            *b"eng",
            String::new(),
            lyrics.to_string(),
        )));
    }
}

/// Replaces the lyrics in Vorbis comments: every lyrics-named field goes
/// (whatever its spelling), `lyrics` (if any) is stored as `LYRICS`.
fn set_vorbis_lyrics(tag: &mut lofty::ogg::tag::VorbisComments, lyrics: &str) {
    let keys: Vec<String> = tag
        .items()
        .filter(|(key, _)| lyrics_key(key))
        .map(|(key, _)| key.to_string())
        .collect();
    for key in keys {
        tag.remove(&key).for_each(drop);
    }
    if !lyrics.is_empty() {
        tag.insert("LYRICS".to_string(), lyrics.to_string());
    }
}

/// Saves `lyrics` into `path`'s metadata, or removes them when `lyrics`
/// is blank. Only the lyrics change: each format is edited through its own
/// tag API, so every other field -- including ones lofty's format-neutral
/// view doesn't know -- stays as it was, and an ID3v2.3 tag stays v2.3.
///
/// Written to a copy that then replaces the original in one atomic
/// rename: a failure can't leave a half-written file, and a track that's
/// playing keeps reading its old, unchanged data rather than a file being
/// rewritten underneath it.
pub fn write_lyrics(path: &Path, lyrics: &str) -> Result<(), String> {
    use lofty::config::{ParseOptions, WriteOptions};
    use lofty::file::{AudioFile, FileType};
    use lofty::id3::v2::{Id3v2Tag, Id3v2Version};

    let kind = file_type(path).ok_or("not a recognised audio file")?;
    if !matches!(kind, FileType::Flac | FileType::Mpeg | FileType::Wav | FileType::Aiff | FileType::Mp4) {
        return Err(format!("saving lyrics into {kind:?} files isn't supported yet"));
    }
    // The copy keeps the real extension: lofty picks the format by it.
    let stem = path.file_stem().ok_or("not a file")?.to_string_lossy();
    let temp = match path.extension() {
        Some(ext) => path.with_file_name(format!(".{stem}.openbar5000-tmp.{}", ext.to_string_lossy())),
        None => path.with_file_name(format!(".{stem}.openbar5000-tmp")),
    };
    std::fs::copy(path, &temp).map_err(|e| format!("couldn't copy the file: {e}"))?;
    let lyrics = lyrics.trim_end();
    let result = (|| -> Result<(), String> {
        fn read_error(e: impl std::fmt::Display) -> String {
            format!("couldn't read the tags: {e}")
        }
        fn write_error(e: impl std::fmt::Display) -> String {
            format!("couldn't write the tags: {e}")
        }
        // Keeps an existing ID3v2.3 tag v2.3 (lofty writes v2.4 otherwise).
        let id3_options = |tag: Option<&Id3v2Tag>| {
            WriteOptions::default().use_id3v23(tag.is_some_and(|t| t.original_version() == Id3v2Version::V3))
        };
        let mut file = open_for_tags(&temp)?;
        match kind {
            FileType::Flac => {
                let mut flac = lofty::flac::FlacFile::read_from(&mut file, ParseOptions::new()).map_err(read_error)?;
                if flac.vorbis_comments().is_none() {
                    flac.set_vorbis_comments(lofty::ogg::tag::VorbisComments::default());
                }
                set_vorbis_lyrics(flac.vorbis_comments_mut().expect("set above"), lyrics);
                // A stray ID3v2 block (issue #20) mustn't keep old lyrics.
                if let Some(id3) = flac.id3v2_mut() {
                    set_id3v2_lyrics(id3, "");
                }
                flac.save_to_path(&temp, WriteOptions::default()).map_err(write_error)?;
            }
            FileType::Mpeg => {
                let mut mp3 = lofty::mpeg::MpegFile::read_from(&mut file, ParseOptions::new()).map_err(read_error)?;
                let options = id3_options(mp3.id3v2());
                if mp3.id3v2().is_none() {
                    mp3.set_id3v2(Id3v2Tag::default());
                }
                set_id3v2_lyrics(mp3.id3v2_mut().expect("set above"), lyrics);
                mp3.save_to_path(&temp, options).map_err(write_error)?;
            }
            FileType::Wav => {
                let mut wav = lofty::iff::wav::WavFile::read_from(&mut file, ParseOptions::new()).map_err(read_error)?;
                let options = id3_options(wav.id3v2());
                if wav.id3v2().is_none() {
                    wav.set_id3v2(Id3v2Tag::default());
                }
                set_id3v2_lyrics(wav.id3v2_mut().expect("set above"), lyrics);
                wav.save_to_path(&temp, options).map_err(write_error)?;
            }
            FileType::Aiff => {
                let mut aiff = lofty::iff::aiff::AiffFile::read_from(&mut file, ParseOptions::new()).map_err(read_error)?;
                let options = id3_options(aiff.id3v2());
                if aiff.id3v2().is_none() {
                    aiff.set_id3v2(Id3v2Tag::default());
                }
                set_id3v2_lyrics(aiff.id3v2_mut().expect("set above"), lyrics);
                aiff.save_to_path(&temp, options).map_err(write_error)?;
            }
            FileType::Mp4 => {
                use lofty::mp4::{Atom, AtomData, AtomIdent, Ilst};
                let mut mp4 = lofty::mp4::Mp4File::read_from(&mut file, ParseOptions::new()).map_err(read_error)?;
                if mp4.ilst().is_none() {
                    mp4.set_ilst(Ilst::default());
                }
                let ilst = mp4.ilst_mut().expect("set above");
                let lyrics_atom = AtomIdent::Fourcc(*b"\xa9lyr");
                ilst.remove(&lyrics_atom).for_each(drop);
                if !lyrics.is_empty() {
                    ilst.insert(Atom::new(lyrics_atom, AtomData::UTF8(lyrics.to_string())));
                }
                mp4.save_to_path(&temp, WriteOptions::default()).map_err(write_error)?;
            }
            _ => unreachable!("checked above"),
        }
        drop(file);
        std::fs::rename(&temp, path).map_err(|e| format!("couldn't replace the file: {e}"))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
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

/// The tags the media library indexes -- a lighter read than
/// [`load_metadata`]: no cover art is loaded, let alone decoded.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LibraryTags {
    pub title: String,
    pub artist: String,
    pub album_artist: String,
    pub album: String,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub date: String,
    pub genre: String,
    pub duration: Duration,
}

/// Reads [`LibraryTags`]; like `load_metadata`, an unreadable file still
/// gets an entry, titled after its file name.
pub fn read_library_tags(path: &Path) -> LibraryTags {
    let fallback_title = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tagged_file = lofty::probe::Probe::open(path)
        .map(|probe| probe.options(lofty::config::ParseOptions::new().read_cover_art(false)))
        .and_then(|probe| probe.read());
    let Ok(tagged_file) = tagged_file else {
        return LibraryTags {
            title: fallback_title,
            ..Default::default()
        };
    };
    let duration = Duration::from_secs_f64(tagged_file.properties().duration().as_secs_f64());
    let Some(tag) = best_tag(&tagged_file) else {
        return LibraryTags {
            title: fallback_title,
            duration,
            ..Default::default()
        };
    };
    let text = |value: Option<std::borrow::Cow<'_, str>>| {
        value.map(|s| s.trim().to_string()).unwrap_or_default()
    };
    LibraryTags {
        title: Some(text(tag.title()))
            .filter(|title| !title.is_empty())
            .unwrap_or(fallback_title),
        artist: text(tag.artist()),
        album_artist: tag
            .get_string(lofty::tag::ItemKey::AlbumArtist)
            .map(|s| s.trim().to_string())
            .unwrap_or_default(),
        album: text(tag.album()),
        track_number: tag.track(),
        disc_number: tag.disk(),
        date: tag.date().map(|d| d.to_string()).unwrap_or_default(),
        genre: text(tag.genre()),
        duration,
    }
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

    let text = |key: lofty::tag::ItemKey| {
        tag.and_then(|tag| tag.get_string(key))
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    };
    let genre = tag.and_then(|tag| tag.genre()).map(|g| g.trim().to_string()).unwrap_or_default();
    let composer = text(lofty::tag::ItemKey::Composer);
    let album_artist = text(lofty::tag::ItemKey::AlbumArtist);
    let bpm = [lofty::tag::ItemKey::IntegerBpm, lofty::tag::ItemKey::Bpm]
        .into_iter()
        .find_map(|key| tag?.get_string(key)?.trim().parse::<f32>().ok())
        .filter(|bpm| *bpm > 0.0)
        .map(|bpm| bpm.round() as u32);
    let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

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
            first_picture(&tagged_file).and_then(shared_cover_thumbnail),
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
        album_artist,
        album,
        track_number,
        disc_number,
        date,
        genre,
        composer,
        bpm,
        file_size,
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

    /// A minimal FLAC: "fLaC", STREAMINFO, then an empty (last)
    /// VORBIS_COMMENT block. Tag reading/writing needs no audio frames.
    fn minimal_flac() -> Vec<u8> {
        flac_with_comments(&[])
    }

    /// `minimal_flac` with these Vorbis comments (`NAME=value` fields).
    fn flac_with_comments(comments: &[&str]) -> Vec<u8> {
        let mut bytes = b"fLaC".to_vec();
        bytes.extend_from_slice(&[0x00, 0x00, 0x00, 34]);
        bytes.extend_from_slice(&[0x10, 0x00, 0x10, 0x00, 0, 0, 0, 0, 0, 0]);
        let packed: u64 = (44_100 << 44) | (1 << 41) | (15 << 36);
        bytes.extend_from_slice(&packed.to_be_bytes());
        bytes.extend_from_slice(&[0; 16]);
        let vendor = b"test";
        let mut block = Vec::new();
        block.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
        block.extend_from_slice(vendor);
        block.extend_from_slice(&(comments.len() as u32).to_le_bytes());
        for comment in comments {
            block.extend_from_slice(&(comment.len() as u32).to_le_bytes());
            block.extend_from_slice(comment.as_bytes());
        }
        let len = block.len() as u32;
        bytes.extend_from_slice(&[0x84, (len >> 16) as u8, (len >> 8) as u8, len as u8]);
        bytes.extend_from_slice(&block);
        bytes
    }

    /// A minimal MP3 with an ID3v2.3 tag holding `frames` (id, body):
    /// the tag, then a few silent MPEG-1 Layer III frames.
    fn mp3_with_id3v23(frames: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut tag = Vec::new();
        for (id, body) in frames {
            tag.extend_from_slice(id.as_bytes());
            tag.extend_from_slice(&(body.len() as u32).to_be_bytes());
            tag.extend_from_slice(&[0, 0]);
            tag.extend_from_slice(body);
        }
        let size = tag.len() as u32;
        let mut bytes = b"ID3\x03\x00\x00".to_vec();
        bytes.extend_from_slice(&[(size >> 21) as u8 & 0x7f, (size >> 14) as u8 & 0x7f, (size >> 7) as u8 & 0x7f, size as u8 & 0x7f]);
        bytes.extend_from_slice(&tag);
        for _ in 0..8 {
            // 128 kbps, 44.1 kHz, joint stereo: 417-byte frames.
            bytes.extend_from_slice(&[0xFF, 0xFB, 0x90, 0x64]);
            bytes.extend_from_slice(&[0; 413]);
        }
        bytes
    }

    fn temp_file(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("openbar5000-{}-{}", name, std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    /// The real-world case behind "the lyrics should be in the metadata": a
    /// FLAC tagged by foobar2000/Qobuz keeps them in `UNSYNCED LYRICS`,
    /// which lofty's format-neutral view drops.
    #[test]
    fn flac_unsynced_lyrics_are_read_and_other_fields_survive_a_write() {
        let path = temp_file(
            "qobuz.flac",
            &flac_with_comments(&[
                "TITLE=Gold",
                "UPC=5039061126799",
                "MY CUSTOM FIELD=keep me",
                "UNSYNCED LYRICS=[Verse 1]\r\nI've stayed up late",
            ]),
        );
        assert_eq!(read_lyrics(&path).as_deref(), Some("[Verse 1]\r\nI've stayed up late"));

        write_lyrics(&path, "[00:01.00]New").unwrap();
        assert_eq!(read_lyrics(&path).as_deref(), Some("[00:01.00]New"));
        let flac = lofty::flac::FlacFile::read_from(
            &mut std::fs::File::open(&path).unwrap(),
            lofty::config::ParseOptions::new(),
        )
        .unwrap();
        use lofty::file::AudioFile;
        let comments = flac.vorbis_comments().unwrap();
        assert_eq!(comments.get("LYRICS"), Some("[00:01.00]New"));
        assert_eq!(comments.get("UNSYNCED LYRICS"), None, "old copy removed, not duplicated");
        assert_eq!(comments.get("TITLE"), Some("Gold"));
        assert_eq!(comments.get("UPC"), Some("5039061126799"));
        assert_eq!(comments.get("MY CUSTOM FIELD"), Some("keep me"));
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn mp3_lyrics_become_an_english_uslt_frame_and_id3v23_stays_v23() {
        let text = |s: &str| [&[0u8][..], s.as_bytes()].concat();
        let path = temp_file(
            "song.mp3",
            &mp3_with_id3v23(&[
                ("TPE1", text("Arora")),
                ("TXXX", [&[0u8][..], b"UNSYNCED LYRICS\0", b"Old lyrics"].concat()),
            ]),
        );
        assert_eq!(read_lyrics(&path).as_deref(), Some("Old lyrics"));

        write_lyrics(&path, "[00:02.00]Hello").unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..4], b"ID3\x03", "still ID3v2.3");
        assert_eq!(read_lyrics(&path).as_deref(), Some("[00:02.00]Hello"));
        let mp3 = lofty::mpeg::MpegFile::read_from(
            &mut std::fs::File::open(&path).unwrap(),
            lofty::config::ParseOptions::new(),
        )
        .unwrap();
        let id3 = mp3.id3v2().unwrap();
        let uslt: Vec<_> = id3.unsync_text().collect();
        assert_eq!(uslt.len(), 1);
        assert_eq!(&uslt[0].language, b"eng");
        assert_eq!(id3.get_user_text("UNSYNCED LYRICS"), None, "old TXXX copy removed");
        assert_eq!(id3.get_text(&lofty::id3::v2::FrameId::Valid("TPE1".into())), Some("Arora"));
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn flac_lyrics_go_into_the_vorbis_comments() {
        let dir = std::env::temp_dir().join(format!("openbar5000-flac-lyrics-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("song.flac");
        std::fs::write(&path, minimal_flac()).unwrap();
        write_lyrics(&path, "[00:05.00] Hello").unwrap();
        assert_eq!(read_lyrics(&path).as_deref(), Some("[00:05.00] Hello"));
        let tagged = lofty::read_from_path(&path).unwrap();
        assert_eq!(
            tagged.primary_tag().unwrap().get_string(lofty::tag::ItemKey::Lyrics),
            Some("[00:05.00] Hello")
        );
        std::fs::remove_dir_all(&dir).ok();
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

        bytes.extend_from_slice(&minimal_flac());

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

    /// A scratch copy of `fixture` (from tests/fixtures) to modify.
    fn scratch_copy(fixture: &str, name: &str) -> std::path::PathBuf {
        let source = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(fixture);
        let dir = std::env::temp_dir().join(format!("openbar5000-lyrics-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(fixture);
        std::fs::copy(source, &path).unwrap();
        path
    }

    #[test]
    fn lyrics_round_trip_through_the_file_and_can_be_removed() {
        let path = scratch_copy("test_tone.wav", "wav");
        assert_eq!(read_lyrics(&path), None);
        let lrc = "[00:01.00] First line\n[00:02.50] Second line";
        write_lyrics(&path, lrc).unwrap();
        assert_eq!(read_lyrics(&path).as_deref(), Some(lrc));
        // Audio and other tags survive, and no temporary file is left.
        let metadata = load_metadata(&path);
        assert!(metadata.duration.as_secs_f32() > 1.0);
        assert!(metadata.file_size > 0);
        let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("openbar5000-tmp"))
            .collect();
        assert!(leftovers.is_empty());
        // Blank lyrics remove the field.
        write_lyrics(&path, "  \n").unwrap();
        assert_eq!(read_lyrics(&path), None);
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn a_failed_write_leaves_the_original_untouched() {
        let missing = std::env::temp_dir().join("openbar5000-does-not-exist.flac");
        assert!(write_lyrics(&missing, "x").is_err());
        let path = scratch_copy("test_tone.wav", "fail");
        let before = std::fs::read(&path).unwrap();
        // Not an audio file at all: lofty refuses, original stays intact.
        let junk = path.with_file_name("junk.mp3");
        std::fs::write(&junk, b"not audio").unwrap();
        assert!(write_lyrics(&junk, "x").is_err());
        assert_eq!(std::fs::read(&junk).unwrap(), b"not audio");
        assert_eq!(std::fs::read(&path).unwrap(), before);
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
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
