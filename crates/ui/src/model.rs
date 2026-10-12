use crate::PlaylistRow;
use std::time::Duration;

/// Formats a duration as foobar2000-style `m:ss` (or `h:mm:ss` past an hour).
pub fn format_duration(duration: Duration) -> String {
    let total_secs = duration.as_secs();
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

pub fn playlist_row(
    track_number: &str,
    artist: &str,
    title: &str,
    album: &str,
    duration: Duration,
) -> PlaylistRow {
    PlaylistRow {
        track_number: track_number.into(),
        artist: artist.into(),
        title: title.into(),
        album: album.into(),
        duration: format_duration(duration).into(),
    }
}
