//! Listening statistics: every counted play is logged (`Store::log_play`,
//! with the tags it had then and how long it was listened to), and summed
//! up here for the statistics window and the year in review.

use chrono::{Datelike, TimeZone, Timelike};
use std::collections::HashMap;

/// One logged play.
#[derive(Debug, Clone, PartialEq)]
pub struct Play {
    /// Unix seconds.
    pub played_at: i64,
    pub path: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// As tagged: may name several ("Rock; Pop").
    pub genre: String,
    /// How long it was listened to.
    pub seconds: i64,
}

/// One entry of a top list: what, by whom (for songs and albums), how
/// often, and for how long.
#[derive(Debug, Clone, PartialEq)]
pub struct Ranked {
    pub name: String,
    pub by: String,
    pub plays: u32,
    pub seconds: i64,
}

/// What a stretch of time's plays add up to.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Summary {
    pub plays: u32,
    pub seconds: i64,
    pub songs: usize,
    pub artists: usize,
    pub top_songs: Vec<Ranked>,
    pub top_artists: Vec<Ranked>,
    pub top_albums: Vec<Ranked>,
    /// By genre; a play tagged with several counts for each.
    pub top_genres: Vec<Ranked>,
    /// Plays by hour of the day (local time).
    pub by_hour: [u32; 24],
    /// Plays by month (January first).
    pub by_month: [u32; 12],
    /// The day with the most plays: (year, month, day, plays).
    pub busiest_day: Option<(i32, u32, u32, u32)>,
}

/// How many entries each top list keeps.
const TOP: usize = 10;

/// The genres a tag names: "Rock; Pop/Indie, Jazz" is four. Case and
/// surrounding space don't make a different genre.
pub fn genres(tag: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for genre in tag.split([';', '/', ',', '|']) {
        let genre = genre.trim();
        if genre.is_empty() {
            continue;
        }
        if !found.iter().any(|g| g.eq_ignore_ascii_case(genre)) {
            found.push(title_case(genre));
        }
    }
    found
}

/// "hip hop" -> "Hip Hop", "ROCK" -> "Rock": genres tagged in different
/// cases count as one. A word written in mixed case ("Lo-Fi", "EDM-ish")
/// is kept as it is.
fn title_case(text: &str) -> String {
    text.split_whitespace()
        .map(|word| {
            let lower = word.chars().all(|c| !c.is_uppercase());
            let upper = word.chars().all(|c| !c.is_lowercase());
            if !(lower || upper) {
                return word.to_string();
            }
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars.flat_map(char::to_lowercase)).collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn or_unknown(text: &str, unknown: &str) -> String {
    let text = text.trim();
    if text.is_empty() { unknown.to_string() } else { text.to_string() }
}

/// Counts into `map` under `key`.
fn tally(map: &mut HashMap<(String, String), (u32, i64)>, key: (String, String), seconds: i64) {
    let entry = map.entry(key).or_default();
    entry.0 += 1;
    entry.1 += seconds;
}

/// The `TOP` most played, most first (then the longest listened, then by
/// name, so the order is stable).
fn ranked(map: HashMap<(String, String), (u32, i64)>) -> Vec<Ranked> {
    let mut list: Vec<Ranked> = map
        .into_iter()
        .map(|((name, by), (plays, seconds))| Ranked { name, by, plays, seconds })
        .collect();
    list.sort_by(|a, b| {
        b.plays
            .cmp(&a.plays)
            .then(b.seconds.cmp(&a.seconds))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    list.truncate(TOP);
    list
}

/// Sums `plays` up, telling hours, months and days by `tz`.
pub fn summarize<Tz: TimeZone>(plays: &[Play], tz: &Tz) -> Summary {
    let mut summary = Summary { plays: plays.len() as u32, ..Summary::default() };
    let (mut songs, mut artists, mut albums, mut genre_map) =
        (HashMap::new(), HashMap::new(), HashMap::new(), HashMap::new());
    let mut days: HashMap<(i32, u32, u32), u32> = HashMap::new();
    for play in plays {
        let seconds = play.seconds.max(0);
        summary.seconds += seconds;
        let artist = or_unknown(&play.artist, "Unknown artist");
        let title = or_unknown(&play.title, &file_title(&play.path));
        tally(&mut songs, (title, artist.clone()), seconds);
        tally(&mut artists, (artist.clone(), String::new()), seconds);
        if !play.album.trim().is_empty() {
            tally(&mut albums, (play.album.trim().to_string(), artist), seconds);
        }
        let tagged = genres(&play.genre);
        if tagged.is_empty() {
            tally(&mut genre_map, ("Untagged".to_string(), String::new()), seconds);
        }
        for genre in tagged {
            tally(&mut genre_map, (genre, String::new()), seconds);
        }
        if let Some(when) = tz.timestamp_opt(play.played_at, 0).single() {
            summary.by_hour[when.hour() as usize] += 1;
            summary.by_month[when.month0() as usize] += 1;
            *days.entry((when.year(), when.month(), when.day())).or_default() += 1;
        }
    }
    summary.songs = songs.len();
    summary.artists = artists.len();
    summary.top_songs = ranked(songs);
    summary.top_artists = ranked(artists);
    summary.top_albums = ranked(albums);
    summary.top_genres = ranked(genre_map);
    summary.busiest_day = days
        .into_iter()
        .max_by(|(day_a, a), (day_b, b)| a.cmp(b).then(day_b.cmp(day_a)))
        .map(|((year, month, day), plays)| (year, month, day, plays));
    summary
}

/// A song without a title tag: its file's name.
fn file_title(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Untitled".to_string())
}

/// "3 h 25 min", "12 min", "45 s".
pub fn format_listening(seconds: i64) -> String {
    let seconds = seconds.max(0);
    let (hours, minutes) = (seconds / 3600, seconds / 60 % 60);
    if hours > 0 {
        format!("{hours} h {minutes} min")
    } else if minutes > 0 {
        format!("{minutes} min")
    } else {
        format!("{seconds} s")
    }
}

/// The stretches of time the statistics window offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    AllTime,
    ThisYear,
    Last30Days,
}

impl Period {
    pub const ALL: [Period; 3] = [Period::AllTime, Period::ThisYear, Period::Last30Days];

    pub fn label(self) -> &'static str {
        match self {
            Period::AllTime => "All time",
            Period::ThisYear => "This year",
            Period::Last30Days => "Last 30 days",
        }
    }

    /// From (Unix seconds) up to `now`.
    pub fn start<Tz: TimeZone>(self, now: i64, tz: &Tz) -> i64 {
        match self {
            Period::AllTime => i64::MIN,
            Period::Last30Days => now - 30 * 24 * 3600,
            Period::ThisYear => tz
                .timestamp_opt(now, 0)
                .single()
                .and_then(|today| tz.with_ymd_and_hms(today.year(), 1, 1, 0, 0, 0).single())
                .map_or(now - 365 * 24 * 3600, |start| start.timestamp()),
        }
    }
}

/// The year in review's season: from 20 December to the end of January.
/// `Some(the year reviewed)` in it -- December's own year, or in January
/// the year just gone.
pub fn review_year(year: i32, month: u32, day: u32) -> Option<i32> {
    match (month, day) {
        (12, 20..) => Some(year),
        (1, _) => Some(year - 1),
        _ => None,
    }
}

/// Whether there's a year in review to offer at `now`: in its season, with
/// listening recorded for over a month (`since`) -- so it says something.
pub fn review_offered<Tz: TimeZone>(now: i64, since: Option<i64>, tz: &Tz) -> Option<i32> {
    let today = tz.timestamp_opt(now, 0).single()?;
    let year = review_year(today.year(), today.month(), today.day())?;
    let since = since?;
    (now - since >= 30 * 24 * 3600).then_some(year)
}

/// The year `year`, as Unix seconds from its start to the next's.
pub fn year_range<Tz: TimeZone>(year: i32, tz: &Tz) -> (i64, i64) {
    let start = |year| tz.with_ymd_and_hms(year, 1, 1, 0, 0, 0).single().map_or(0, |t| t.timestamp());
    (start(year), start(year + 1))
}

/// A word for when someone listens most, from their plays by hour.
pub fn listening_mood(by_hour: &[u32; 24]) -> &'static str {
    let (mut best, mut most) = (0, 0);
    // In four-hour stretches, so one late night doesn't decide it.
    for start in (0..24).step_by(4) {
        let plays: u32 = by_hour[start..start + 4].iter().sum();
        if plays > most {
            (best, most) = (start, plays);
        }
    }
    match best {
        0 => "Night owl",
        4 => "Early bird",
        8 => "Morning listener",
        12 => "Afternoon groover",
        16 => "Evening unwinder",
        _ => "Late-night dreamer",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn play(at: i64, title: &str, artist: &str, album: &str, genre: &str, seconds: i64) -> Play {
        Play {
            played_at: at,
            path: format!("/m/{title}.flac"),
            title: title.into(),
            artist: artist.into(),
            album: album.into(),
            genre: genre.into(),
            seconds,
        }
    }

    #[test]
    fn genres_split_and_merge_regardless_of_case() {
        assert_eq!(genres("Rock; pop/Indie,  jazz | ROCK"), ["Rock", "Pop", "Indie", "Jazz"]);
        assert_eq!(genres("hip hop"), ["Hip Hop"]);
        assert_eq!(genres("Lo-Fi; lo-fi"), ["Lo-Fi"]);
        assert!(genres(" ; / ").is_empty());
    }

    #[test]
    fn a_summary_counts_ranks_and_buckets() {
        // 2026-03-01 10:00 UTC, then an hour apart.
        let t = 1_772_359_200;
        let plays = [
            play(t, "One", "Ana", "First", "Rock", 200),
            play(t + 3600, "Two", "Ben", "Second", "Pop; rock", 180),
            play(t + 7200, "One", "Ana", "First", "Rock", 210),
            Play { path: "/m/Secret Track.flac".into(), ..play(t + 86_400, "", "", "", "", 30) },
        ];
        let summary = summarize(&plays, &Utc);
        assert_eq!(summary.plays, 4);
        assert_eq!(summary.seconds, 620);
        assert_eq!(summary.songs, 3);
        assert_eq!(summary.artists, 3);
        assert_eq!(summary.top_songs[0].name, "One");
        assert_eq!(summary.top_songs[0].plays, 2);
        assert_eq!(summary.top_artists[0].name, "Ana");
        assert_eq!(summary.top_genres[0].name, "Rock");
        assert_eq!(summary.top_genres[0].plays, 3);
        assert!(summary.top_genres.iter().any(|g| g.name == "Untagged"));
        // An untitled song is known by its file, an unknown artist as such.
        assert!(summary.top_songs.iter().any(|s| s.name == "Secret Track" && s.by == "Unknown artist"));
        // The last, a day later, is at 10:00 too.
        assert_eq!(summary.by_hour[10], 2);
        assert_eq!(summary.by_hour[11], 1);
        assert_eq!(summary.by_month[2], 4);
        assert_eq!(summary.busiest_day, Some((2026, 3, 1, 3)));
    }

    #[test]
    fn nothing_played_sums_to_nothing() {
        let summary = summarize(&[], &Utc);
        assert_eq!(summary, Summary::default());
    }

    #[test]
    fn the_review_season_runs_from_late_december_through_january() {
        assert_eq!(review_year(2026, 12, 19), None);
        assert_eq!(review_year(2026, 12, 20), Some(2026));
        assert_eq!(review_year(2026, 12, 31), Some(2026));
        assert_eq!(review_year(2027, 1, 31), Some(2026));
        assert_eq!(review_year(2027, 2, 1), None);
    }

    #[test]
    fn the_review_needs_over_a_month_of_listening() {
        let dec_21 = Utc.with_ymd_and_hms(2026, 12, 21, 12, 0, 0).unwrap().timestamp();
        assert_eq!(review_offered(dec_21, None, &Utc), None);
        assert_eq!(review_offered(dec_21, Some(dec_21 - 10 * 86_400), &Utc), None);
        assert_eq!(review_offered(dec_21, Some(dec_21 - 40 * 86_400), &Utc), Some(2026));
        let july = Utc.with_ymd_and_hms(2026, 7, 1, 12, 0, 0).unwrap().timestamp();
        assert_eq!(review_offered(july, Some(0), &Utc), None);
    }

    #[test]
    fn periods_and_years_are_where_they_should_be() {
        let now = Utc.with_ymd_and_hms(2026, 6, 15, 12, 0, 0).unwrap().timestamp();
        assert_eq!(Period::ThisYear.start(now, &Utc), Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap().timestamp());
        assert_eq!(Period::Last30Days.start(now, &Utc), now - 30 * 86_400);
        let (from, to) = year_range(2026, &Utc);
        assert_eq!(to - from, 365 * 86_400);
    }

    #[test]
    fn listening_is_told_in_hours_minutes_or_seconds() {
        assert_eq!(format_listening(12_300), "3 h 25 min");
        assert_eq!(format_listening(720), "12 min");
        assert_eq!(format_listening(45), "45 s");
        assert_eq!(format_listening(-5), "0 s");
    }

    #[test]
    fn moods_follow_the_busiest_hours() {
        let mut hours = [0; 24];
        hours[1] = 5;
        hours[2] = 3;
        hours[23] = 6;
        assert_eq!(listening_mood(&hours), "Night owl");
        hours[18] = 20;
        assert_eq!(listening_mood(&hours), "Evening unwinder");
    }
}
