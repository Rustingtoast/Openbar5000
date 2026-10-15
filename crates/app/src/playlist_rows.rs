use engine::Track;
use rand::seq::SliceRandom;
use std::collections::{HashMap, HashSet};
use std::path::Path;
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
    /// With smart grouping (Preferences > Appearance) albums are matched
    /// loosely -- see [`SmartContext`] -- instead of by exact text.
    fn key(self, track: &Track, smart: Option<&SmartContext>) -> String {
        let m = &track.metadata;
        if let Some(context) = smart {
            match self {
                Self::Artist => return owner_key(m),
                Self::ArtistAlbum => return context.album_key(track),
                Self::ArtistAlbumDisc => return context.disc_key(track),
                Self::Path | Self::Date => {}
            }
        }
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

/// Lowercase, single-spaced, trimmed: how smart grouping compares text.
fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// `artist` without a "feat./ft./featuring …" credit (or a stray trailing
/// ";"): the performer an album belongs to, as displayed.
fn primary_artist(artist: &str) -> &str {
    let lower = artist.to_lowercase();
    let cut = [" feat. ", " feat ", " ft. ", " ft ", " featuring ", " (feat", " [feat", " (ft", " [ft"]
        .iter()
        .filter_map(|marker| lower.find(marker))
        .min();
    let artist = match cut {
        // `find` on the lowercased text: the byte offsets match for ASCII
        // markers, and `get` guards the rare case they don't.
        Some(i) => artist.get(..i).unwrap_or(artist),
        None => artist,
    };
    artist.trim().trim_end_matches(';').trim()
}

/// Who an album is filed under, for comparing: the first name credited as
/// album artist (or, untagged, as artist) -- "Lorien Testard & Alice
/// Duport-Percier", "Lorien Testard, Victor Borba;" and "Lorien Testard"
/// all count as one. Applied the same way to every track, so a cut through
/// a name like "Above & Beyond" still matches itself.
fn owner_key(m: &engine::TrackMetadata) -> String {
    let credit = if m.album_artist.trim().is_empty() { &m.artist } else { &m.album_artist };
    let credit = normalize(primary_artist(credit));
    let end = [",", ";", " & ", " and ", " x ", " / ", " vs "]
        .iter()
        .filter_map(|separator| credit.find(separator))
        .min()
        .unwrap_or(credit.len());
    credit[..end].trim().to_string()
}

/// An album name as smart grouping compares it: case, spacing and
/// punctuation ignored ("Clair Obscur: Expedition 33" is "Clair Obscur
/// Expedition 33"), and a disc suffix dropped ("Album (Disc 2)", "Album -
/// CD 1").
fn album_words(album: &str) -> String {
    let cleaned: String = album
        .chars()
        .map(|c| if c.is_alphanumeric() { c.to_lowercase().next().unwrap_or(c) } else { ' ' })
        .collect();
    let mut words: Vec<&str> = cleaned.split_whitespace().collect();
    if let [.., marker, number] = words.as_slice() {
        if ["disc", "disk", "cd"].contains(marker) && number.chars().all(|c| c.is_ascii_digit()) {
            words.truncate(words.len() - 2);
        }
    }
    words.join(" ")
}

/// The folder a file without an album tag is grouped by -- one level up
/// from a disc sub-folder ("CD1", "Disc 2").
fn album_folder(path: &Path) -> String {
    let Some(dir) = path.parent() else {
        return String::new();
    };
    let name = dir.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let disc_folder = ["cd", "disc", "disk"].iter().any(|prefix| {
        name.strip_prefix(prefix)
            .map(str::trim)
            .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
    });
    let dir = if disc_folder { dir.parent().unwrap_or(dir) } else { dir };
    dir.to_string_lossy().into_owned()
}

/// What smart grouping needs to know about the whole playlist: which album
/// names are shared by albums with different (tagged) album artists, and
/// which albums really have several discs.
#[derive(Default)]
pub struct SmartContext {
    split_by_owner: HashSet<String>,
    multi_disc: HashSet<String>,
}

impl SmartContext {
    pub fn new(tracks: &[Track]) -> Self {
        let mut owners: HashMap<String, HashSet<String>> = HashMap::new();
        for track in tracks {
            let m = &track.metadata;
            if !m.album_artist.trim().is_empty() {
                owners.entry(album_words(&m.album)).or_default().insert(owner_key(m));
            }
        }
        let split_by_owner = owners
            .into_iter()
            .filter(|(album, owners)| !album.is_empty() && owners.len() > 1)
            .map(|(album, _)| album)
            .collect();
        let mut context = Self { split_by_owner, multi_disc: HashSet::new() };
        // Only tagged disc numbers count: a track without one belongs to
        // whichever disc its album has.
        let mut discs: HashMap<String, HashSet<u32>> = HashMap::new();
        for track in tracks {
            if let Some(disc) = track.metadata.disc_number {
                discs.entry(context.album_key(track)).or_default().insert(disc);
            }
        }
        context.multi_disc = discs.into_iter().filter(|(_, d)| d.len() > 1).map(|(k, _)| k).collect();
        context
    }

    /// Which album a track belongs to: its album name, compared loosely --
    /// split by album artist only when the playlist has differently
    /// credited albums of that name. Files without an album tag group by
    /// folder.
    fn album_key(&self, track: &Track) -> String {
        let m = &track.metadata;
        let album = album_words(&m.album);
        if album.is_empty() {
            return format!("dir:{}", album_folder(&track.path));
        }
        if self.split_by_owner.contains(&album) {
            format!("{album}\u{1f}{}", owner_key(m))
        } else {
            album
        }
    }

    /// `album_key` plus the disc, for albums that have more than one.
    fn disc_key(&self, track: &Track) -> String {
        let album = self.album_key(track);
        if self.multi_disc.contains(&album) {
            format!("{album}\u{1f}{}", track.metadata.disc_number.unwrap_or(1))
        } else {
            album
        }
    }
}

/// Smart grouping's playlist order: each group's tracks gathered where the
/// group first appears, in disc and track-number order -- so an album whose
/// files were added out of order or mixed with others reads as one.
/// `None` when the playlist already is in that order.
pub fn smart_order(tracks: &[Track], grouping: PlaylistGrouping) -> Option<Vec<usize>> {
    let context = SmartContext::new(tracks);
    let keys: Vec<String> = tracks.iter().map(|t| grouping.key(t, Some(&context))).collect();
    let mut first_seen: HashMap<&str, usize> = HashMap::new();
    for (i, key) in keys.iter().enumerate() {
        first_seen.entry(key.as_str()).or_insert(i);
    }
    let mut order: Vec<usize> = (0..tracks.len()).collect();
    // Inside a group, songs go by disc and track number (untagged ones
    // last, in the order they were).
    order.sort_by_cached_key(|&i| {
        let m = &tracks[i].metadata;
        (
            first_seen[keys[i].as_str()],
            album_words(&m.album),
            m.disc_number.unwrap_or(1),
            m.track_number.unwrap_or(u32::MAX),
            i,
        )
    });
    order.iter().enumerate().any(|(k, &i)| k != i).then_some(order)
}

/// The artist a smart group's header shows: the credit most of its tracks
/// carry (the shorter one on a tie), or "Various Artists" if they name
/// different people.
fn group_artist(group: &[Track]) -> String {
    let first = owner_key(&group[0].metadata);
    if group.iter().any(|t| owner_key(&t.metadata) != first) {
        return "Various Artists".to_string();
    }
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for track in group {
        let m = &track.metadata;
        let credit = if m.album_artist.trim().is_empty() { &m.artist } else { &m.album_artist };
        *counts.entry(primary_artist(credit)).or_insert(0) += 1;
    }
    counts
        .into_iter()
        .max_by(|(a, x), (b, y)| x.cmp(y).then(b.len().cmp(&a.len())))
        .map(|(credit, _)| credit.to_string())
        .unwrap_or_default()
}

/// Group-header cover images, kept across row rebuilds (which happen on
/// every track change, search keystroke, collapse...) so each thumbnail is
/// turned into a `slint::Image` once -- and the renderer sees the same
/// image, rather than a fresh texture, every time. Covers no track uses any
/// more are dropped with [`CoverCache::retain_used`].
#[derive(Default)]
pub struct CoverCache {
    /// By thumbnail: tracks sharing one cover (see `engine::load_metadata`)
    /// share one image. The `Arc` is kept so its address isn't reused.
    by_cover: HashMap<usize, (std::sync::Arc<engine::CoverThumbnail>, slint::Image)>,
    placeholder: Option<slint::Image>,
}

impl CoverCache {
    /// Drops the images of covers no track in `tracks` has any more.
    pub fn retain_used(&mut self, tracks: &[Track]) {
        let used: HashSet<usize> = tracks
            .iter()
            .filter_map(|t| t.metadata.cover_thumbnail.as_ref())
            .map(|cover| std::sync::Arc::as_ptr(cover) as usize)
            .collect();
        self.by_cover.retain(|key, _| used.contains(key));
    }

    fn placeholder(&mut self) -> slint::Image {
        self.placeholder
            .get_or_insert_with(placeholder_cover_image)
            .clone()
    }

    fn cover(&mut self, track: &Track) -> slint::Image {
        let Some(thumb) = track.metadata.cover_thumbnail.as_ref() else {
            return self.placeholder();
        };
        self.by_cover
            .entry(std::sync::Arc::as_ptr(thumb) as usize)
            .or_insert_with(|| (thumb.clone(), cover_image_from_rgba(thumb.width, thumb.height, &thumb.rgba)))
            .1
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

/// The Sorting menu's choice: kept applied, so tracks added later land in
/// order too. `Manual` leaves the playlist as it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortMode {
    #[default]
    Manual,
    Artist,
    Album,
    Title,
    TrackNumber,
    Date,
    FilePath,
}

impl SortMode {
    /// Every mode, in the Sorting menu's order, with its menu title.
    pub const ALL: [(SortMode, &'static str); 7] = [
        (Self::Manual, "Manual (no sorting)"),
        (Self::Artist, "By Artist"),
        (Self::Album, "By Album"),
        (Self::Title, "By Title"),
        (Self::TrackNumber, "By Track Number"),
        (Self::Date, "By Date"),
        (Self::FilePath, "By File Path"),
    ];

    /// Stable id shared by the settings store and the Slint menu.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Artist => "artist",
            Self::Album => "album",
            Self::Title => "title",
            Self::TrackNumber => "track-number",
            Self::Date => "date",
            Self::FilePath => "path",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "artist" => Self::Artist,
            "album" => Self::Album,
            "title" => Self::Title,
            "track-number" => Self::TrackNumber,
            "date" => Self::Date,
            "path" => Self::FilePath,
            _ => Self::Manual,
        }
    }

    /// The sorts applied in turn, least significant first: each is stable,
    /// so e.g. "by artist" keeps every artist's albums together and their
    /// tracks in disc/track order.
    fn keys(self) -> &'static [PlaylistSort] {
        use PlaylistSort::*;
        match self {
            Self::Manual => &[],
            Self::Artist => &[TrackNumber, Album, Artist],
            Self::Album => &[TrackNumber, Album],
            Self::Title => &[Title],
            Self::TrackNumber => &[TrackNumber],
            Self::Date => &[TrackNumber, Album, Date],
            Self::FilePath => &[FilePath],
        }
    }
}

/// The order `mode` puts `tracks` in, as for [`sort_order`]; `None` when
/// they already are (or `mode` is `Manual`), so nothing needs reordering.
pub fn sorted_order(tracks: &[Track], mode: SortMode) -> Option<Vec<usize>> {
    let mut order: Vec<usize> = (0..tracks.len()).collect();
    for &key in mode.keys() {
        let current: Vec<Track> = order.iter().map(|&i| tracks[i].clone()).collect();
        order = sort_order(&current, key).into_iter().map(|k| order[k]).collect();
    }
    (order.iter().enumerate().any(|(k, &i)| k != i)).then_some(order)
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
    smart: bool,
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
    let context = smart.then(|| SmartContext::new(tracks));
    let context = context.as_ref();

    let mut rows = Vec::new();
    let mut group_start = 0;
    while group_start < tracks.len() {
        let key = grouping.key(&tracks[group_start], context);
        let mut group_end = group_start + 1;
        while group_end < tracks.len() && grouping.key(&tracks[group_end], context) == key {
            group_end += 1;
        }

        let matching: Vec<usize> = (group_start..group_end)
            .filter(|&i| matches(&tracks[i]))
            .collect();
        if !matching.is_empty() {
            let first = &tracks[group_start];
            // The group's cover: its first track that has one.
            let cover_track = tracks[group_start..group_end]
                .iter()
                .find(|t| t.metadata.cover_thumbnail.is_some())
                .unwrap_or(first);
            let total_duration: Duration = (group_start..group_end)
                .map(|i| tracks[i].metadata.duration)
                .sum();
            let is_collapsed = query.is_empty() && collapsed.contains(&key);
            let contains_current =
                current_index.is_some_and(|c| (group_start..group_end).contains(&c));
            rows.push(header_row(
                first,
                smart.then(|| &tracks[group_start..group_end]),
                cover_track.metadata.cover_thumbnail.is_some(),
                covers.cover(cover_track),
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

/// List view: every matching track as a plain row, numbered by playlist
/// position, with no album headers.
pub fn build_list_rows(
    tracks: &[Track],
    current_index: Option<usize>,
    query: &str,
    covers: &mut CoverCache,
) -> Vec<PlaylistRow> {
    let query = query.trim().to_lowercase();
    let blank_cover = covers.placeholder();
    tracks
        .iter()
        .enumerate()
        .filter(|(_, track)| {
            query.is_empty()
                || track.metadata.title.to_lowercase().contains(&query)
                || track.metadata.artist.to_lowercase().contains(&query)
                || track.metadata.album.to_lowercase().contains(&query)
        })
        .map(|(i, _)| {
            let mut row = track_row(tracks, i, 0, "", current_index, &blank_cover);
            row.track_number = (i + 1).to_string().into();
            row
        })
        .collect()
}

/// Every group key in the playlist, for "Collapse all".
pub fn all_group_keys(tracks: &[Track], grouping: PlaylistGrouping, smart: bool) -> HashSet<String> {
    let context = smart.then(|| SmartContext::new(tracks));
    tracks.iter().map(|t| grouping.key(t, context.as_ref())).collect()
}

/// The group key `index` belongs to, for "Show now playing" (to expand its
/// group before scrolling to it).
pub fn group_key_of(
    tracks: &[Track],
    index: usize,
    grouping: PlaylistGrouping,
    smart: bool,
) -> Option<String> {
    let context = smart.then(|| SmartContext::new(tracks));
    tracks.get(index).map(|t| grouping.key(t, context.as_ref()))
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
        artist: track.metadata.artist.clone().into(),
        album: track.metadata.album.clone().into(),
        duration: format_duration(track.metadata.duration).into(),
        track_index: i as i32,
        is_current: current_index == Some(i),
    }
}

#[allow(clippy::too_many_arguments)]
fn header_row(
    first: &Track,
    // The whole group, when smart grouping names it from all its tracks.
    smart_group: Option<&[Track]>,
    has_cover: bool,
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
    // Smart groups are named from all their tracks: their album artist or
    // shared main artist, and their folder when there's no album tag.
    let artist = match smart_group {
        Some(group) => group_artist(group),
        None => m.artist.clone(),
    };
    let album = match smart_group {
        Some(_) if m.album.trim().is_empty() => Path::new(&album_folder(&first.path))
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        _ => m.album.clone(),
    };
    // Artist on top, album underneath -- the reference UI's header layout.
    let (title, subtitle) = match grouping {
        PlaylistGrouping::Artist => (or_unknown(&artist, "Unknown Artist"), String::new()),
        PlaylistGrouping::ArtistAlbum => (
            or_unknown(&artist, "Unknown Artist"),
            or_unknown(&album, "Unknown Album"),
        ),
        PlaylistGrouping::ArtistAlbumDisc => {
            let album = or_unknown(&album, "Unknown Album");
            let subtitle = match m.disc_number {
                Some(disc) => format!("{album} (Disc {disc})"),
                None => album,
            };
            (or_unknown(&artist, "Unknown Artist"), subtitle)
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
        has_cover,
        cover_image,
        group_title: title.into(),
        group_subtitle: subtitle.into(),
        group_info: group_info_text(first, track_count, total_duration).into(),
        group_date: m.date.clone().into(),
        group_key: key.into(),
        is_collapsed,
        track_number: String::new().into(),
        title: String::new().into(),
        artist: String::new().into(),
        album: String::new().into(),
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
    fn sort_modes_order_by_artist_then_album_then_track() {
        let tracks = tracks();
        let order = sorted_order(&tracks, SortMode::Artist).unwrap();
        let titles: Vec<_> = order.iter().map(|&i| tracks[i].metadata.title.as_str()).collect();
        assert_eq!(titles, ["Yankee", "xray", "zulu"]);
        // Already sorted, or manual: nothing to do.
        let sorted: Vec<Track> = order.iter().map(|&i| tracks[i].clone()).collect();
        assert_eq!(sorted_order(&sorted, SortMode::Artist), None);
        assert_eq!(sorted_order(&tracks, SortMode::Manual), None);
        assert_eq!(SortMode::from_str(SortMode::TrackNumber.as_str()), SortMode::TrackNumber);
        assert_eq!(SortMode::from_str("nonsense"), SortMode::Manual);
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
            false,
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
            false,
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
            false,
            &collapsed,
            &mut CoverCache::default(),
        );
        assert_eq!(rows.len(), 2);
    }

    fn tagged(path: &str, artist: &str, album_artist: &str, album: &str, disc: Option<u32>, number: u32) -> Track {
        let mut t = track(path, artist, album, &format!("t{number}"), number);
        t.metadata.album_artist = album_artist.into();
        t.metadata.disc_number = disc;
        t
    }

    #[test]
    fn smart_grouping_keeps_an_album_together_despite_messy_tags() {
        let t = vec![
            tagged("/m/abgt/1.flac", "Above & Beyond", "", "Common Ground", None, 1),
            tagged("/m/abgt/2.flac", "Above & Beyond feat. Zoë Johnston", "", "common  ground", None, 2),
            tagged("/m/abgt/3.flac", "Above & Beyond ft. Marty Longstaff", "", "Common Ground (Disc 1)", None, 3),
        ];
        let strict = build_grouped_rows(&t, None, "", PlaylistGrouping::ArtistAlbum, false, &HashSet::new(), &mut CoverCache::default());
        assert_eq!(strict.iter().filter(|r| r.is_header).count(), 3);
        let smart = build_grouped_rows(&t, None, "", PlaylistGrouping::ArtistAlbum, true, &HashSet::new(), &mut CoverCache::default());
        assert_eq!(smart.iter().filter(|r| r.is_header).count(), 1);
        assert_eq!(smart[0].group_title, "Above & Beyond");
    }

    #[test]
    fn smart_grouping_names_compilations_and_groups_untagged_files_by_folder() {
        let compilation = vec![
            tagged("/m/hits/1.flac", "Artist A", "", "Big Hits", None, 1),
            tagged("/m/hits/2.flac", "Artist B", "", "Big Hits", None, 2),
        ];
        let rows = build_grouped_rows(&compilation, None, "", PlaylistGrouping::ArtistAlbum, true, &HashSet::new(), &mut CoverCache::default());
        assert_eq!(rows[0].group_title, "Various Artists");
        assert_eq!(rows.iter().filter(|r| r.is_header).count(), 1);

        let loose = vec![
            tagged("/m/My Album/CD1/a.mp3", "X", "", "", None, 1),
            tagged("/m/My Album/CD2/b.mp3", "Y", "", "", None, 1),
        ];
        let rows = build_grouped_rows(&loose, None, "", PlaylistGrouping::ArtistAlbum, true, &HashSet::new(), &mut CoverCache::default());
        assert_eq!(rows.iter().filter(|r| r.is_header).count(), 1);
        assert_eq!(rows[0].group_subtitle, "My Album");
    }

    fn headers(t: &[Track]) -> Vec<String> {
        build_grouped_rows(t, None, "", PlaylistGrouping::ArtistAlbumDisc, true, &HashSet::new(), &mut CoverCache::default())
            .into_iter()
            .filter(|r| r.is_header)
            .map(|r| format!("{} / {}", r.group_title, r.group_subtitle))
            .collect()
    }

    #[test]
    fn smart_grouping_merges_a_soundtrack_with_inconsistent_credits_discs_and_folders() {
        // Like the Clair Obscur OST: album artist tagged differently per
        // track (or not at all), disc set on some tracks only, files in two
        // folders, and a colon in some copies' album name.
        let t = vec![
            tagged("/m/Music/a.wav", "Lorien Testard", "", "Clair Obscur Expedition 33 Original Soundtrack (Act I)", None, 1),
            tagged("/m/Music/OST (Act I)/b.wav", "Lorien Testard & Alice Duport-Percier", "Lorien Testard & Alice Duport-Percier", "Clair Obscur Expedition 33 Original Soundtrack (Act I)", Some(1), 2),
            tagged("/m/Music/c.wav", "Lorien Testard", "Lorien Testard, Alice Duport-Percier, Victor Borba;", "Clair Obscur: Expedition 33 Original Soundtrack (Act I)", Some(1), 3),
        ];
        assert_eq!(headers(&t), ["Lorien Testard / Clair Obscur Expedition 33 Original Soundtrack (Act I)"]);
    }

    #[test]
    fn smart_grouping_merges_a_compilation_spread_over_artist_folders() {
        // Like "One Night Only, Vol 1": one folder per artist, one track
        // with an album artist.
        let t = vec![
            tagged("/m/Hemlock/One Night Only, Vol 1/a.mp3", "Hemlock", "", "One Night Only, Vol 1", None, 1),
            tagged("/m/Arora/One Night Only, Vol 1/b.mp3", "Arora", "", "One Night Only, Vol 1", None, 2),
            tagged("/m/Deckard Voltaire/One Night Only, Vol 1/c.mp3", "Deckard Voltaire", "Deckard Voltaire", "One Night Only, Vol 1", None, 3),
        ];
        assert_eq!(headers(&t), ["Various Artists / One Night Only, Vol 1"]);
    }

    #[test]
    fn smart_grouping_keeps_same_named_albums_by_different_artists_apart() {
        let t = vec![
            tagged("/m/x/1.mp3", "Queen", "Queen", "Greatest Hits", None, 1),
            tagged("/m/y/1.mp3", "ABBA", "ABBA", "Greatest Hits", None, 1),
        ];
        assert_eq!(headers(&t).len(), 2);
        // A real two-disc album still shows its discs.
        let t = vec![
            tagged("/m/z/1.flac", "A", "", "Double", Some(1), 1),
            tagged("/m/z/2.flac", "A", "", "Double", Some(2), 1),
        ];
        assert_eq!(headers(&t), ["A / Double (Disc 1)", "A / Double (Disc 2)"]);
        // A track without a disc number joins its album's one disc.
        let t = vec![
            tagged("/m/w/1.flac", "A", "", "Act II", Some(2), 1),
            tagged("/m/w/2.mp3", "A", "", "Act II", None, 2),
        ];
        assert_eq!(headers(&t).len(), 1);
    }

    #[test]
    fn smart_order_gathers_scattered_tracks_in_track_order() {
        // The album's scattered tracks gather where it first appears, in
        // track order, ahead of the other album.
        let t = vec![
            tagged("/m/a/2.flac", "A", "", "One", None, 2),
            tagged("/m/b/1.flac", "B", "", "Two", None, 1),
            tagged("/m/a/1.flac", "A", "", "One", None, 1),
        ];
        let order = smart_order(&t, PlaylistGrouping::ArtistAlbum).unwrap();
        assert_eq!(order, [2, 0, 1]);
        // Track number wins over title: "Zebra" (1) before "apple" (2).
        let by_number = vec![
            track("/m/c/2.flac", "C", "Three", "apple", 2),
            track("/m/c/1.flac", "C", "Three", "Zebra", 1),
        ];
        assert_eq!(smart_order(&by_number, PlaylistGrouping::ArtistAlbum), Some(vec![1, 0]));
        let gathered: Vec<Track> = order.iter().map(|&i| t[i].clone()).collect();
        assert_eq!(smart_order(&gathered, PlaylistGrouping::ArtistAlbum), None);
        assert_eq!(album_words("Album - Disc 2"), "album");
        assert_eq!(album_words("Discovery"), "discovery");
        assert_eq!(album_words("Clair Obscur: Expedition 33 (Act I)"), album_words("clair obscur expedition 33 (act i)"));
        assert_eq!(primary_artist("Above & Beyond (feat. Richard Bedford)"), "Above & Beyond");
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
            false,
            &HashSet::new(),
            &mut CoverCache::default(),
        );
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].group_title, "a");
    }
}
