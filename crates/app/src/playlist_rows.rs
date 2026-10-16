use engine::Track;
use rand::seq::SliceRandom;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;
use ui::{cover_image_from_rgba, format_duration, placeholder_cover_image, PlaylistRow};

/// How contiguous runs of tracks are bundled under a header row -- the
/// playlist context menu's Grouping submenu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlaylistGrouping {
    Artist,
    ArtistAlbum,
    #[default]
    ArtistAlbumDisc,
    Path,
    Date,
}

impl PlaylistGrouping {
    /// Stable id shared by the settings store and the Slint menu.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Artist => "artist",
            Self::ArtistAlbum => "artist-album",
            Self::ArtistAlbumDisc => "artist-album-disc",
            Self::Path => "path",
            Self::Date => "date",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "artist" => Self::Artist,
            "artist-album" => Self::ArtistAlbum,
            "path" => Self::Path,
            "date" => Self::Date,
            _ => Self::ArtistAlbumDisc,
        }
    }

    /// Tracks with equal keys (and adjacent in the playlist) share a header.
    fn key(self, track: &Track) -> String {
        let m = &track.metadata;
        match self {
            Self::Artist => m.artist.clone(),
            Self::ArtistAlbum => format!("{}\u{1f}{}", m.artist, m.album),
            Self::ArtistAlbumDisc => format!(
                "{}\u{1f}{}\u{1f}{}",
                m.artist,
                m.album,
                m.disc_number.unwrap_or(0)
            ),
            Self::Path => parent_dir(&track.path),
            Self::Date => m.date.clone(),
        }
    }
}

/// Group-header cover images, kept across row rebuilds (which happen on
/// every track change, search keystroke, collapse...) so each thumbnail is
/// turned into a `slint::Image` once -- and the renderer sees the same
/// image, rather than a fresh texture, every time. Clear it whenever the
/// tracks' metadata may have changed.
#[derive(Default)]
pub struct CoverCache {
    by_path: HashMap<PathBuf, slint::Image>,
    placeholder: Option<slint::Image>,
}

impl CoverCache {
    pub fn clear(&mut self) {
        self.by_path.clear();
    }

    fn placeholder(&mut self) -> slint::Image {
        self.placeholder
            .get_or_insert_with(placeholder_cover_image)
            .clone()
    }

    fn cover(&mut self, track: &Track) -> slint::Image {
        let Some(thumb) = track.metadata.cover_thumbnail.as_deref() else {
            return self.placeholder();
        };
        self.by_path
            .entry(track.path.clone())
            .or_insert_with(|| cover_image_from_rgba(thumb.width, thumb.height, &thumb.rgba))
            .clone()
    }
}

/// The context menu's Sort submenu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaylistSort {
    Album,
    Artist,
    FilePath,
    Title,
    TrackNumber,
    Date,
    Randomize,
    Reverse,
}

impl PlaylistSort {
    pub fn from_str(value: &str) -> Option<Self> {
        Some(match value {
            "album" => Self::Album,
            "artist" => Self::Artist,
            "path" => Self::FilePath,
            "title" => Self::Title,
            "track-number" => Self::TrackNumber,
            "date" => Self::Date,
            "randomize" => Self::Randomize,
            "reverse" => Self::Reverse,
            _ => return None,
        })
    }
}

/// The new playlist order for `sort` as old indices (position `k` holds the
/// track currently at `order[k]`) -- what `EngineCommand::ReorderPlaylist`
/// takes. Key sorts are stable, so re-sorting by a second key keeps ties in
/// the previous key's order, same as foobar2000.
pub fn sort_order(tracks: &[Track], sort: PlaylistSort) -> Vec<usize> {
    let mut order: Vec<usize> = (0..tracks.len()).collect();
    let text = |s: &str| s.to_lowercase();
    match sort {
        PlaylistSort::Album => order.sort_by_cached_key(|&i| text(&tracks[i].metadata.album)),
        PlaylistSort::Artist => order.sort_by_cached_key(|&i| text(&tracks[i].metadata.artist)),
        PlaylistSort::FilePath => order.sort_by_cached_key(|&i| tracks[i].path.clone()),
        PlaylistSort::Title => order.sort_by_cached_key(|&i| text(&tracks[i].metadata.title)),
        PlaylistSort::TrackNumber => order.sort_by_key(|&i| {
            let m = &tracks[i].metadata;
            (
                m.disc_number.unwrap_or(0),
                m.track_number.unwrap_or(u32::MAX),
            )
        }),
        PlaylistSort::Date => order.sort_by_cached_key(|&i| tracks[i].metadata.date.clone()),
        PlaylistSort::Randomize => order.shuffle(&mut rand::rng()),
        PlaylistSort::Reverse => order.reverse(),
    }
    order
}

/// Builds the flat, display-ready row list the Slint playlist view iterates:
/// a header row for each contiguous run of tracks sharing the same
/// `grouping` key (mirroring the reference UI's grouped playlist), followed
/// by that group's track rows unless its key is in `collapsed`. `query`
/// filters by substring match on title/artist/album; a group is kept
/// (header + only its matching tracks) if at least one of its tracks
/// matches, and is always shown expanded while searching.
pub fn build_grouped_rows(
    tracks: &[Track],
    current_index: Option<usize>,
    query: &str,
    grouping: PlaylistGrouping,
    collapsed: &HashSet<String>,
    covers: &mut CoverCache,
) -> Vec<PlaylistRow> {
    let query = query.trim().to_lowercase();
    let matches = |track: &Track| {
        query.is_empty()
            || track.metadata.title.to_lowercase().contains(&query)
            || track.metadata.artist.to_lowercase().contains(&query)
            || track.metadata.album.to_lowercase().contains(&query)
    };
    let blank_cover = covers.placeholder();

    let mut rows = Vec::new();
    let mut group_start = 0;
    while group_start < tracks.len() {
        let key = grouping.key(&tracks[group_start]);
        let mut group_end = group_start + 1;
        while group_end < tracks.len() && grouping.key(&tracks[group_end]) == key {
            group_end += 1;
        }

        let matching: Vec<usize> = (group_start..group_end)
            .filter(|&i| matches(&tracks[i]))
            .collect();
        if !matching.is_empty() {
            let first = &tracks[group_start];
            let total_duration: Duration = (group_start..group_end)
                .map(|i| tracks[i].metadata.duration)
                .sum();
            let is_collapsed = query.is_empty() && collapsed.contains(&key);
            let contains_current =
                current_index.is_some_and(|c| (group_start..group_end).contains(&c));
            rows.push(header_row(
                first,
                covers.cover(first),
                grouping,
                &key,
                group_start,
                group_end - group_start,
                total_duration,
                is_collapsed,
                is_collapsed && contains_current,
            ));

            if !is_collapsed {
                for i in matching {
                    rows.push(track_row(
                        tracks,
                        i,
                        group_start,
                        &key,
                        current_index,
                        &blank_cover,
                    ));
                }
            }
        }

        group_start = group_end;
    }
    rows
}

/// Every group key in the playlist, for "Collapse all".
pub fn all_group_keys(tracks: &[Track], grouping: PlaylistGrouping) -> HashSet<String> {
    tracks.iter().map(|t| grouping.key(t)).collect()
}

/// The group key `index` belongs to, for "Show now playing" (to expand its
/// group before scrolling to it).
pub fn group_key_of(tracks: &[Track], index: usize, grouping: PlaylistGrouping) -> Option<String> {
    tracks.get(index).map(|t| grouping.key(t))
}

fn track_row(
    tracks: &[Track],
    i: usize,
    group_start: usize,
    key: &str,
    current_index: Option<usize>,
    blank_cover: &slint::Image,
) -> PlaylistRow {
    let track = &tracks[i];
    let track_number = track
        .metadata
        .track_number
        .map(|n| n.to_string())
        .unwrap_or_else(|| (i - group_start + 1).to_string());
    PlaylistRow {
        is_header: false,
        cover_image: blank_cover.clone(),
        has_cover: false,
        group_title: String::new().into(),
        group_subtitle: String::new().into(),
        group_info: String::new().into(),
        group_date: String::new().into(),
        group_key: key.into(),
        is_collapsed: false,
        track_number: track_number.into(),
        title: track.metadata.title.clone().into(),
        duration: format_duration(track.metadata.duration).into(),
        track_index: i as i32,
        is_current: current_index == Some(i),
    }
}

#[allow(clippy::too_many_arguments)]
fn header_row(
    first: &Track,
    cover_image: slint::Image,
    grouping: PlaylistGrouping,
    key: &str,
    group_start: usize,
    track_count: usize,
    total_duration: Duration,
    is_collapsed: bool,
    is_current: bool,
) -> PlaylistRow {
    let m = &first.metadata;
    let or_unknown = |value: &str, unknown: &str| {
        if value.is_empty() {
            unknown.to_string()
        } else {
            value.to_string()
        }
    };
    // Artist on top, album underneath -- the reference UI's header layout.
    let (title, subtitle) = match grouping {
        PlaylistGrouping::Artist => (or_unknown(&m.artist, "Unknown Artist"), String::new()),
        PlaylistGrouping::ArtistAlbum => (
            or_unknown(&m.artist, "Unknown Artist"),
            or_unknown(&m.album, "Unknown Album"),
        ),
        PlaylistGrouping::ArtistAlbumDisc => {
            let album = or_unknown(&m.album, "Unknown Album");
            let subtitle = match m.disc_number {
                Some(disc) => format!("{album} (Disc {disc})"),
                None => album,
            };
            (or_unknown(&m.artist, "Unknown Artist"), subtitle)
        }
        PlaylistGrouping::Path => {
            let dir = Path::new(key);
            let name = dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| key.to_string());
            (name, key.to_string())
        }
        PlaylistGrouping::Date => (or_unknown(&m.date, "Unknown Date"), String::new()),
    };

    PlaylistRow {
        is_header: true,
        has_cover: first.metadata.cover_thumbnail.is_some(),
        cover_image,
        group_title: title.into(),
        group_subtitle: subtitle.into(),
        group_info: group_info_text(first, track_count, total_duration).into(),
        group_date: m.date.clone().into(),
        group_key: key.into(),
        is_collapsed,
        track_number: String::new().into(),
        title: String::new().into(),
        duration: String::new().into(),
        // The group's first track, so header-row actions have a sensible
        // target (e.g. Play).
        track_index: group_start as i32,
        is_current,
    }
}

fn parent_dir(path: &Path) -> String {
    path.parent()
        .map(|p| p.display().to_string())
        .unwrap_or_default()
}

fn group_info_text(track: &Track, track_count: usize, total_duration: Duration) -> String {
    let quality = match (track.metadata.bit_depth, track.metadata.sample_rate) {
        (Some(depth), Some(rate)) => format!("{depth}bit/{:.1}kHz", rate as f64 / 1000.0),
        _ => track
            .metadata
            .bitrate_kbps
            .map(|kbps| format!("{kbps}kbps"))
            .unwrap_or_default(),
    };
    let track_word = if track_count == 1 { "Track" } else { "Tracks" };

    [
        track.metadata.format_label.clone(),
        quality,
        format!("{track_count} {track_word}"),
        format!("Time: {}", format_duration(total_duration)),
    ]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join(" | ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::TrackMetadata;
    use std::path::PathBuf;

    fn track(path: &str, artist: &str, album: &str, title: &str, number: u32) -> Track {
        Track {
            path: PathBuf::from(path),
            metadata: TrackMetadata {
                artist: artist.into(),
                album: album.into(),
                title: title.into(),
                track_number: Some(number),
                ..Default::default()
            },
        }
    }

    fn tracks() -> Vec<Track> {
        vec![
            track("/m/b/2.flac", "Beta", "Two", "zulu", 2),
            track("/m/a/1.flac", "Alpha", "One", "Yankee", 1),
            track("/m/b/1.flac", "Beta", "Two", "xray", 1),
        ]
    }

    #[test]
    fn sorts_are_stable_and_case_insensitive() {
        let t = tracks();
        assert_eq!(sort_order(&t, PlaylistSort::Artist), [1, 0, 2]);
        assert_eq!(sort_order(&t, PlaylistSort::Title), [2, 1, 0]);
        assert_eq!(sort_order(&t, PlaylistSort::TrackNumber), [1, 2, 0]);
        assert_eq!(sort_order(&t, PlaylistSort::FilePath), [1, 2, 0]);
        assert_eq!(sort_order(&t, PlaylistSort::Reverse), [2, 1, 0]);
        let mut shuffled = sort_order(&t, PlaylistSort::Randomize);
        shuffled.sort();
        assert_eq!(shuffled, [0, 1, 2]);
    }

    #[test]
    fn collapsed_groups_keep_only_their_header() {
        let t = tracks();
        let grouping = PlaylistGrouping::Artist;
        let expanded = build_grouped_rows(
            &t,
            None,
            "",
            grouping,
            &HashSet::new(),
            &mut CoverCache::default(),
        );
        // Beta | zulu | Alpha | Yankee | Beta | xray
        assert_eq!(expanded.len(), 6);
        // No embedded art in these tracks: headers show the disc icon.
        assert!(!expanded[0].has_cover);

        let collapsed = HashSet::from(["Beta".to_string()]);
        let rows = build_grouped_rows(
            &t,
            Some(0),
            "",
            grouping,
            &collapsed,
            &mut CoverCache::default(),
        );
        assert_eq!(rows.len(), 4);
        assert!(rows[0].is_header && rows[0].is_collapsed && rows[0].is_current);

        // Searching always shows the matches, collapsed or not.
        let rows = build_grouped_rows(
            &t,
            None,
            "zulu",
            grouping,
            &collapsed,
            &mut CoverCache::default(),
        );
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn path_grouping_uses_the_folder() {
        let t = vec![
            track("/m/a/1.flac", "X", "Y", "a", 1),
            track("/m/a/2.flac", "Z", "W", "b", 2),
        ];
        let rows = build_grouped_rows(
            &t,
            None,
            "",
            PlaylistGrouping::Path,
            &HashSet::new(),
            &mut CoverCache::default(),
        );
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].group_title, "a");
    }
}
