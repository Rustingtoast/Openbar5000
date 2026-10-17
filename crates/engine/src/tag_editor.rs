//! Reading and writing what the Properties window shows (issue #34): the
//! editable tag fields, the embedded pictures, and the file's technical
//! details.
//!
//! Edits go through lofty's split/merge: a format's own tag is split into
//! the generic fields (edited here) and whatever lofty can't represent
//! generically (synced lyrics, chapters, private frames, ...), and merged
//! back -- so editing a title never loses the rest. Like `write_lyrics`,
//! the file is written as a copy that then replaces the original.

use crate::metadata::{best_tag, file_type, format_label, open_for_tags, scaled_cover};
use crate::playlist::CoverThumbnail;
use lofty::config::{ParseOptions, WriteOptions};
use lofty::file::{AudioFile, FileType, TaggedFileExt};
use lofty::picture::{Picture, PictureType};
use lofty::tag::{ItemKey, MergeTag, SplitTag, Tag};
use std::path::Path;
use std::time::{Duration, SystemTime};

/// The tag fields the Properties window edits, in its order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TagField {
    Artist,
    Title,
    Album,
    Date,
    Genre,
    Composer,
    Performer,
    AlbumArtist,
    TrackNumber,
    TrackTotal,
    DiscNumber,
    DiscTotal,
    Comment,
    Year,
}

impl TagField {
    pub const ALL: [TagField; 14] = [
        Self::Artist,
        Self::Title,
        Self::Album,
        Self::Date,
        Self::Genre,
        Self::Composer,
        Self::Performer,
        Self::AlbumArtist,
        Self::TrackNumber,
        Self::TrackTotal,
        Self::DiscNumber,
        Self::DiscTotal,
        Self::Comment,
        Self::Year,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Artist => "Artist Name",
            Self::Title => "Track Title",
            Self::Album => "Album Title",
            Self::Date => "Date",
            Self::Genre => "Genre",
            Self::Composer => "Composer",
            Self::Performer => "Performer",
            Self::AlbumArtist => "Album Artist",
            Self::TrackNumber => "Track Number",
            Self::TrackTotal => "Total Tracks",
            Self::DiscNumber => "Disc Number",
            Self::DiscTotal => "Total Discs",
            Self::Comment => "Comment",
            Self::Year => "Year",
        }
    }

    fn key(self) -> ItemKey {
        match self {
            Self::Artist => ItemKey::TrackArtist,
            Self::Title => ItemKey::TrackTitle,
            Self::Album => ItemKey::AlbumTitle,
            Self::Date => ItemKey::RecordingDate,
            Self::Genre => ItemKey::Genre,
            Self::Composer => ItemKey::Composer,
            Self::Performer => ItemKey::Performer,
            Self::AlbumArtist => ItemKey::AlbumArtist,
            Self::TrackNumber => ItemKey::TrackNumber,
            Self::TrackTotal => ItemKey::TrackTotal,
            Self::DiscNumber => ItemKey::DiscNumber,
            Self::DiscTotal => ItemKey::DiscTotal,
            Self::Comment => ItemKey::Comment,
            Self::Year => ItemKey::Year,
        }
    }
}

/// The pictures the Artwork tab offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PictureKind {
    Front,
    Back,
    Artist,
    Disc,
    Icon,
}

impl PictureKind {
    pub const ALL: [PictureKind; 5] = [Self::Front, Self::Back, Self::Artist, Self::Disc, Self::Icon];

    pub fn label(self) -> &'static str {
        match self {
            Self::Front => "Front Cover",
            Self::Back => "Back Cover",
            Self::Artist => "Artist",
            Self::Disc => "Disc",
            Self::Icon => "Icon",
        }
    }

    fn picture_type(self) -> PictureType {
        match self {
            Self::Front => PictureType::CoverFront,
            Self::Back => PictureType::CoverBack,
            Self::Artist => PictureType::Artist,
            Self::Disc => PictureType::Media,
            Self::Icon => PictureType::Icon,
        }
    }

    /// Picture types read as this kind (a lead-artist picture shows as the
    /// artist picture; an untyped "Other" picture as the front cover).
    fn matches(self, picture_type: PictureType) -> bool {
        match self {
            Self::Front => matches!(picture_type, PictureType::CoverFront | PictureType::Other),
            Self::Artist => matches!(picture_type, PictureType::Artist | PictureType::LeadArtist),
            Self::Icon => matches!(picture_type, PictureType::Icon | PictureType::OtherIcon),
            _ => picture_type == self.picture_type(),
        }
    }
}

/// An embedded picture, for the Artwork tab.
#[derive(Debug, Clone)]
pub struct PictureInfo {
    pub kind: PictureKind,
    pub bytes: usize,
    pub width: u32,
    pub height: u32,
    /// "JPEG", "PNG", ...
    pub format: String,
    pub thumbnail: Option<CoverThumbnail>,
}

/// What to change in a file: fields to set (empty: remove) and pictures to
/// set or remove. Fields not listed are left alone.
#[derive(Debug, Clone, Default)]
pub struct TagEdit {
    pub fields: Vec<(TagField, String)>,
    pub pictures: Vec<(PictureKind, Option<Vec<u8>>)>,
}

impl TagEdit {
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty() && self.pictures.is_empty()
    }
}

/// The file's technical details, for the Details tab.
#[derive(Debug, Clone, Default)]
pub struct FileDetails {
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub duration: Duration,
    pub sample_rate: Option<u32>,
    pub channels: Option<u8>,
    pub bits_per_sample: Option<u8>,
    pub bitrate_kbps: Option<u32>,
    pub codec: String,
    pub lossless: bool,
    /// The encoder that made the file, when it says.
    pub tool: String,
}

impl FileDetails {
    /// Duration in samples, when the sample rate is known.
    pub fn samples(&self) -> Option<u64> {
        self.sample_rate.map(|rate| (self.duration.as_secs_f64() * rate as f64).round() as u64)
    }
}

/// Every field's current value ("" when unset).
pub fn read_tag_fields(path: &Path) -> Result<Vec<(TagField, String)>, String> {
    let tagged = lofty::read_from_path(path).map_err(|e| format!("couldn't read the tags: {e}"))?;
    let tag = best_tag(&tagged);
    Ok(TagField::ALL
        .iter()
        .map(|&field| {
            let value = tag
                .and_then(|tag| tag.get_string(field.key()))
                .map(|value| value.trim().to_string())
                .unwrap_or_default();
            (field, value)
        })
        .collect())
}

/// The embedded pictures, one per kind (the first of each).
pub fn read_pictures(path: &Path) -> Vec<PictureInfo> {
    let Ok(tagged) = lofty::read_from_path(path) else {
        return Vec::new();
    };
    let pictures: Vec<&Picture> = tagged.tags().iter().flat_map(|tag| tag.pictures()).collect();
    PictureKind::ALL
        .iter()
        .filter_map(|&kind| {
            let picture = pictures.iter().find(|p| kind.matches(p.pic_type()))?;
            let data = picture.data();
            let (width, height) = image::ImageReader::new(std::io::Cursor::new(data))
                .with_guessed_format()
                .ok()
                .and_then(|reader| reader.into_dimensions().ok())
                .unwrap_or((0, 0));
            Some(PictureInfo {
                kind,
                bytes: data.len(),
                width,
                height,
                format: picture
                    .mime_type()
                    .map(|mime| mime.as_str().trim_start_matches("image/").to_uppercase())
                    .unwrap_or_default(),
                thumbnail: scaled_cover(data, 160),
            })
        })
        .collect()
}

pub fn read_details(path: &Path) -> FileDetails {
    let metadata = std::fs::metadata(path).ok();
    let mut details = FileDetails {
        size: metadata.as_ref().map_or(0, |m| m.len()),
        modified: metadata.and_then(|m| m.modified().ok()),
        ..Default::default()
    };
    let Ok(tagged) = lofty::read_from_path(path) else {
        return details;
    };
    let properties = tagged.properties();
    details.duration = properties.duration();
    details.sample_rate = properties.sample_rate();
    details.channels = properties.channels();
    details.bits_per_sample = properties.bit_depth();
    details.bitrate_kbps = properties.audio_bitrate();
    details.codec = format_label(tagged.file_type());
    details.lossless = matches!(
        tagged.file_type(),
        FileType::Flac | FileType::Wav | FileType::Aiff | FileType::Ape | FileType::WavPack
    );
    details.tool = best_tag(&tagged)
        .and_then(|tag| {
            [ItemKey::EncoderSoftware, ItemKey::EncodedBy, ItemKey::EncoderSettings]
                .into_iter()
                .find_map(|key| tag.get_string(key).map(str::to_string))
        })
        .unwrap_or_default();
    // FLAC names its encoder in the Vorbis comment block's vendor string.
    if details.tool.is_empty() && tagged.file_type() == FileType::Flac {
        if let Ok(mut file) = open_for_tags(path) {
            if let Ok(flac) = lofty::flac::FlacFile::read_from(&mut file, ParseOptions::new()) {
                details.tool = flac.vorbis_comments().map(|vc| vc.vendor().to_string()).unwrap_or_default();
            }
        }
    }
    details
}

/// The picture to store for `kind`, from an image file's bytes.
fn picture(kind: PictureKind, data: &[u8]) -> Result<Picture, String> {
    let mut picture = Picture::from_reader(&mut &data[..]).map_err(|e| format!("not a usable image: {e}"))?;
    picture.set_pic_type(kind.picture_type());
    Ok(picture)
}

/// Applies the field and picture edits to a generic tag.
fn apply(tag: &mut Tag, edit: &TagEdit, with_pictures: bool) -> Result<(), String> {
    for (field, value) in &edit.fields {
        let value = value.trim();
        tag.remove_key(field.key());
        if !value.is_empty() {
            // `false` if the format has no such field (e.g. Total Discs
            // somewhere): skipped rather than failing the whole save.
            tag.insert_text(field.key(), value.to_string());
        }
    }
    if with_pictures {
        for (kind, data) in &edit.pictures {
            for picture_type in [kind.picture_type()]
                .into_iter()
                .chain((*kind == PictureKind::Front).then_some(PictureType::Other))
            {
                tag.remove_picture_type(picture_type);
            }
            if let Some(data) = data {
                tag.push_picture(picture(*kind, data)?);
            }
        }
    }
    Ok(())
}

/// Writes `edit` into `path`. Supported: FLAC, MP3, WAV, AIFF, MP4/M4A.
pub fn edit_tags(path: &Path, edit: &TagEdit) -> Result<(), String> {
    use lofty::id3::v2::{Id3v2Tag, Id3v2Version};

    if edit.is_empty() {
        return Ok(());
    }
    let kind = file_type(path).ok_or("not a recognised audio file")?;
    if !matches!(kind, FileType::Flac | FileType::Mpeg | FileType::Wav | FileType::Aiff | FileType::Mp4) {
        return Err(format!("editing tags in {} files isn't supported yet", format_label(kind)));
    }
    let stem = path.file_stem().ok_or("not a file")?.to_string_lossy();
    let temp = match path.extension() {
        Some(ext) => path.with_file_name(format!(".{stem}.openbar5000-tmp.{}", ext.to_string_lossy())),
        None => path.with_file_name(format!(".{stem}.openbar5000-tmp")),
    };
    std::fs::copy(path, &temp).map_err(|e| format!("couldn't copy the file: {e}"))?;
    let result = (|| -> Result<(), String> {
        fn read_error(e: impl std::fmt::Display) -> String {
            format!("couldn't read the tags: {e}")
        }
        fn write_error(e: impl std::fmt::Display) -> String {
            format!("couldn't write the tags: {e}")
        }
        // Keeps an existing ID3v2.3 tag v2.3 (lofty writes v2.4 otherwise).
        let id3_options = |tag: &Id3v2Tag| {
            WriteOptions::default().use_id3v23(tag.original_version() == Id3v2Version::V3)
        };
        let edit_id3 = |tag: Option<Id3v2Tag>| -> Result<(Id3v2Tag, WriteOptions), String> {
            let tag = tag.unwrap_or_default();
            let options = id3_options(&tag);
            let (remainder, mut generic) = tag.split_tag();
            apply(&mut generic, edit, true)?;
            Ok((remainder.merge_tag(generic), options))
        };
        let mut file = open_for_tags(&temp)?;
        match kind {
            FileType::Flac => {
                use lofty::ogg::OggPictureStorage;
                let mut flac = lofty::flac::FlacFile::read_from(&mut file, ParseOptions::new()).map_err(read_error)?;
                let comments = flac.remove_vorbis_comments().unwrap_or_default();
                let (remainder, mut generic) = comments.split_tag();
                // FLAC keeps pictures in blocks of their own, below.
                apply(&mut generic, edit, false)?;
                flac.set_vorbis_comments(remainder.merge_tag(generic));
                for (kind, data) in &edit.pictures {
                    flac.remove_picture_type(kind.picture_type());
                    if *kind == PictureKind::Front {
                        flac.remove_picture_type(PictureType::Other);
                    }
                    if let Some(data) = data {
                        flac.insert_picture(picture(*kind, data)?, None).map_err(write_error)?;
                    }
                }
                flac.save_to_path(&temp, WriteOptions::default()).map_err(write_error)?;
            }
            FileType::Mpeg => {
                let mut mp3 = lofty::mpeg::MpegFile::read_from(&mut file, ParseOptions::new()).map_err(read_error)?;
                let (tag, options) = edit_id3(mp3.remove_id3v2())?;
                mp3.set_id3v2(tag);
                mp3.save_to_path(&temp, options).map_err(write_error)?;
            }
            FileType::Wav => {
                let mut wav = lofty::iff::wav::WavFile::read_from(&mut file, ParseOptions::new()).map_err(read_error)?;
                let (tag, options) = edit_id3(wav.remove_id3v2())?;
                wav.set_id3v2(tag);
                wav.save_to_path(&temp, options).map_err(write_error)?;
            }
            FileType::Aiff => {
                let mut aiff = lofty::iff::aiff::AiffFile::read_from(&mut file, ParseOptions::new()).map_err(read_error)?;
                let (tag, options) = edit_id3(aiff.remove_id3v2())?;
                aiff.set_id3v2(tag);
                aiff.save_to_path(&temp, options).map_err(write_error)?;
            }
            FileType::Mp4 => {
                let mut mp4 = lofty::mp4::Mp4File::read_from(&mut file, ParseOptions::new()).map_err(read_error)?;
                let ilst = mp4.remove_ilst().unwrap_or_default();
                let (remainder, mut generic) = ilst.split_tag();
                apply(&mut generic, edit, true)?;
                mp4.set_ilst(remainder.merge_tag(generic));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_copy(name: &str) -> std::path::PathBuf {
        let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/test_tone.wav");
        let dir = std::env::temp_dir().join(format!("openbar5000-tag-editor-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let copy = dir.join("tone.wav");
        std::fs::copy(fixture, &copy).unwrap();
        copy
    }

    fn png() -> Vec<u8> {
        let mut bytes = Vec::new();
        image::DynamicImage::new_rgba8(40, 30)
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    fn field(fields: &[(TagField, String)], wanted: TagField) -> &str {
        &fields.iter().find(|(f, _)| *f == wanted).unwrap().1
    }

    #[test]
    fn fields_are_written_read_back_and_removed() {
        let path = fixture_copy("fields");
        let edit = TagEdit {
            fields: vec![
                (TagField::Artist, "Above & Beyond".into()),
                (TagField::Title, "Sun & Moon".into()),
                (TagField::TrackNumber, "3".into()),
                (TagField::TrackTotal, "12".into()),
                (TagField::Comment, "great".into()),
            ],
            pictures: Vec::new(),
        };
        edit_tags(&path, &edit).unwrap();
        let fields = read_tag_fields(&path).unwrap();
        assert_eq!(field(&fields, TagField::Artist), "Above & Beyond");
        assert_eq!(field(&fields, TagField::Title), "Sun & Moon");
        assert_eq!(field(&fields, TagField::TrackNumber), "3");
        assert_eq!(field(&fields, TagField::TrackTotal), "12");

        // Clearing a field removes it; others stay.
        edit_tags(&path, &TagEdit { fields: vec![(TagField::Comment, String::new())], pictures: Vec::new() }).unwrap();
        let fields = read_tag_fields(&path).unwrap();
        assert_eq!(field(&fields, TagField::Comment), "");
        assert_eq!(field(&fields, TagField::Artist), "Above & Beyond");
        // Still playable audio.
        assert!(read_details(&path).sample_rate.is_some());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn pictures_are_added_by_kind_and_removed() {
        let path = fixture_copy("pictures");
        let edit = TagEdit {
            fields: Vec::new(),
            pictures: vec![(PictureKind::Front, Some(png())), (PictureKind::Back, Some(png()))],
        };
        edit_tags(&path, &edit).unwrap();
        let pictures = read_pictures(&path);
        let kinds: Vec<_> = pictures.iter().map(|p| p.kind).collect();
        assert_eq!(kinds, [PictureKind::Front, PictureKind::Back]);
        assert_eq!((pictures[0].width, pictures[0].height, pictures[0].format.as_str()), (40, 30, "PNG"));

        edit_tags(&path, &TagEdit { fields: Vec::new(), pictures: vec![(PictureKind::Front, None)] }).unwrap();
        let kinds: Vec<_> = read_pictures(&path).iter().map(|p| p.kind).collect();
        assert_eq!(kinds, [PictureKind::Back]);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn details_describe_the_file() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/test_tone.wav");
        let details = read_details(&path);
        assert_eq!(details.codec, "WAV");
        assert!(details.lossless);
        assert!(details.size > 0 && details.modified.is_some());
        assert!(details.samples().is_some_and(|s| s > 0));
    }
}
