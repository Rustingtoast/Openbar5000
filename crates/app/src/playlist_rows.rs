use engine::Track;
use std::time::Duration;
use ui::{decode_cover_image, format_duration, PlaylistRow};

/// Builds the flat, display-ready row list the Slint playlist view iterates:
/// a header row for each contiguous run of tracks sharing the same
/// artist/album (mirroring the reference UI's album-grouped playlist),
/// followed by that group's track rows. `query` filters by substring match
/// on title/artist/album; a group is kept (header + only its matching
/// tracks) if at least one of its tracks matches.
pub fn build_grouped_rows(
    tracks: &[Track],
    current_index: Option<usize>,
    query: &str,
) -> Vec<PlaylistRow> {
    let query = query.trim().to_lowercase();
    let matches = |track: &Track| {
        query.is_empty()
            || track.metadata.title.to_lowercase().contains(&query)
            || track.metadata.artist.to_lowercase().contains(&query)
            || track.metadata.album.to_lowercase().contains(&query)
    };
    let blank_cover = decode_cover_image(None);

    let mut rows = Vec::new();
    let mut group_start = 0;
    while group_start < tracks.len() {
        let artist = &tracks[group_start].metadata.artist;
        let album = &tracks[group_start].metadata.album;
        let mut group_end = group_start + 1;
        while group_end < tracks.len()
            && tracks[group_end].metadata.artist == *artist
            && tracks[group_end].metadata.album == *album
        {
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
            rows.push(header_row(first, group_end - group_start, total_duration));

            for i in matching {
                let track = &tracks[i];
                let track_number = track
                    .metadata
                    .track_number
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| (i - group_start + 1).to_string());
                rows.push(PlaylistRow {
                    is_header: false,
                    cover_image: blank_cover.clone(),
                    group_title: String::new().into(),
                    group_subtitle: String::new().into(),
                    group_info: String::new().into(),
                    track_number: track_number.into(),
                    title: track.metadata.title.clone().into(),
                    duration: format_duration(track.metadata.duration).into(),
                    track_index: i as i32,
                    is_current: current_index == Some(i),
                });
            }
        }

        group_start = group_end;
    }
    rows
}

fn header_row(first: &Track, track_count: usize, total_duration: Duration) -> PlaylistRow {
    let title = if first.metadata.album.is_empty() {
        "Unknown Album".to_string()
    } else {
        first.metadata.album.clone()
    };
    let subtitle = if first.metadata.artist.is_empty() {
        "Unknown Artist".to_string()
    } else {
        first.metadata.artist.clone()
    };

    PlaylistRow {
        is_header: true,
        cover_image: decode_cover_image(first.metadata.cover_art.as_deref()),
        group_title: title.into(),
        group_subtitle: subtitle.into(),
        group_info: group_info_text(first, track_count, total_duration).into(),
        track_number: String::new().into(),
        title: String::new().into(),
        duration: String::new().into(),
        track_index: 0,
        is_current: false,
    }
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
