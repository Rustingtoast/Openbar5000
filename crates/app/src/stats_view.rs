//! Shows the listening statistics (see `stats`): the statistics window, and
//! the year in review's figures.

use crate::stats::{self, Period, Ranked, Summary};
use crate::store::Store;
use chrono::{Local, TimeZone};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::rc::Rc;
use ui::{StatBar, StatRow, StatsWindow, YearReview};

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const MONTH_NAMES: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November",
    "December",
];

/// Now, Unix seconds -- or noon of `OPENBAR5000_DATE` (YYYY-MM-DD), to try
/// the year in review out of season (as for the holiday logos).
pub fn now() -> i64 {
    if let Some(date) = std::env::var("OPENBAR5000_DATE")
        .ok()
        .and_then(|value| chrono::NaiveDate::parse_from_str(&value, "%Y-%m-%d").ok())
    {
        if let Some(noon) = date.and_hms_opt(12, 0, 0).and_then(|t| Local.from_local_datetime(&t).single()) {
            return noon.timestamp();
        }
    }
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// The year in review on offer now, if any (see `stats::review_offered`).
pub fn review_offered(store: &Store) -> Option<i32> {
    stats::review_offered(now(), store.listening_since(), &Local)
}

fn model<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::from(Rc::new(VecModel::from(items)))
}

fn plays_text(plays: u32) -> String {
    if plays == 1 { "1 play".to_string() } else { format!("{plays} plays") }
}

/// A top list's rows, each bar its share of the first's plays.
fn rows(list: &[Ranked], take: usize) -> Vec<StatRow> {
    let most = list.first().map_or(1, |first| first.plays.max(1)) as f32;
    list.iter()
        .take(take)
        .map(|entry| StatRow {
            name: entry.name.clone().into(),
            by: entry.by.clone().into(),
            value: plays_text(entry.plays).into(),
            share: entry.plays as f32 / most,
        })
        .collect()
}

fn bars(counts: &[u32], labels: impl Fn(usize) -> String) -> Vec<StatBar> {
    let most = counts.iter().copied().max().unwrap_or(0).max(1) as f32;
    counts
        .iter()
        .enumerate()
        .map(|(i, &count)| StatBar {
            label: labels(i).into(),
            value: count as f32 / most,
            text: plays_text(count).into(),
        })
        .collect()
}

/// "3 March 2026".
fn date(year: i32, month: u32, day: u32) -> String {
    format!("{day} {} {year}", MONTH_NAMES[(month.clamp(1, 12) - 1) as usize])
}

fn since_text(store: &Store) -> String {
    match store.listening_since().and_then(|since| Local.timestamp_opt(since, 0).single()) {
        Some(since) => {
            use chrono::Datelike;
            format!("Counting since {}. A song counts once you've listened to half of it.", date(since.year(), since.month(), since.day()))
        }
        None => "Nothing counted yet: a song counts once you've listened to half of it (at most four minutes).".to_string(),
    }
}

/// Fills `window` with `period`'s statistics.
pub fn fill(window: &StatsWindow, store: &Store, period: Period) {
    let now = now();
    let plays = store.plays_between(period.start(now, &Local), now + 1);
    let summary = stats::summarize(&plays, &Local);
    window.set_periods(model(Period::ALL.iter().map(|p| SharedString::from(p.label())).collect()));
    window.set_plays(summary.plays.to_string().into());
    window.set_listening(stats::format_listening(summary.seconds).into());
    window.set_songs(summary.songs.to_string().into());
    window.set_artists(summary.artists.to_string().into());
    window.set_top_songs(model(rows(&summary.top_songs, 10)));
    window.set_top_artists(model(rows(&summary.top_artists, 10)));
    window.set_top_genres(model(rows(&summary.top_genres, 10)));
    window.set_top_albums(model(rows(&summary.top_albums, 10)));
    window.set_by_hour(model(bars(&summary.by_hour, |hour| format!("{hour}h"))));
    window.set_by_month(model(bars(&summary.by_month, |month| MONTHS[month].to_string())));
    window.set_facts(facts(&summary).into());
    window.set_since(since_text(store).into());
}

/// A line or two of what stands out.
fn facts(summary: &Summary) -> String {
    if summary.plays == 0 {
        return String::new();
    }
    let mut facts = vec![format!("You're a {}.", stats::listening_mood(&summary.by_hour).to_lowercase())];
    if let Some((year, month, day, plays)) = summary.busiest_day {
        facts.push(format!("Busiest day: {}, {}.", date(year, month, day), plays_text(plays)));
    }
    facts.join(" ")
}

/// Opens (or refreshes) the statistics window.
pub fn show(window: &StatsWindow, store: &Store) {
    let period = Period::ALL.get(window.get_period_index().max(0) as usize).copied().unwrap_or(Period::AllTime);
    fill(window, store, period);
    crate::platform::show_in_front(window.window());
    window.invoke_focus_keys();
}

/// The year in review's figures for `year`.
pub fn year_review(store: &Store, year: i32) -> YearReview {
    let (from, to) = stats::year_range(year, &Local);
    let plays = store.plays_between(from, to);
    let summary = stats::summarize(&plays, &Local);
    let minutes = (summary.seconds / 60).max(if summary.seconds > 0 { 1 } else { 0 });
    let top_song = summary.top_songs.first();
    let top_artist = summary.top_artists.first();
    let busiest_month = summary
        .by_month
        .iter()
        .enumerate()
        .max_by_key(|(i, &plays)| (plays, std::cmp::Reverse(*i)))
        .filter(|(_, &plays)| plays > 0)
        .map(|(month, &plays)| format!("{} was your month: {}.", MONTH_NAMES[month], plays_text(plays)))
        .unwrap_or_default();
    let busiest_day = summary
        .busiest_day
        .map(|(year, month, day, plays)| format!("And on {}, {} -- your biggest day.", date(year, month, day), plays_text(plays)))
        .unwrap_or_default();
    // Genres for the review: the tagged ones only, top five.
    let genres: Vec<Ranked> = summary.top_genres.iter().filter(|g| g.name != "Untagged").cloned().collect();
    YearReview {
        year,
        minutes: group_thousands(minutes).into(),
        plays: group_thousands(i64::from(summary.plays)).into(),
        songs: summary.songs.to_string().into(),
        artists: summary.artists.to_string().into(),
        top_song: top_song.map(|s| s.name.clone()).unwrap_or_else(|| "Nothing yet".to_string()).into(),
        top_song_by: top_song.map(|s| s.by.clone()).unwrap_or_default().into(),
        top_song_plays: top_song
            .map(|s| format!("{} -- {} of it.", plays_text(s.plays), stats::format_listening(s.seconds)))
            .unwrap_or_default()
            .into(),
        top_artist: top_artist.map(|a| a.name.clone()).unwrap_or_else(|| "Nobody yet".to_string()).into(),
        top_artist_plays: top_artist
            .map(|a| format!("{}, {}.", plays_text(a.plays), stats::format_listening(a.seconds)))
            .unwrap_or_default()
            .into(),
        genres: model(rows(&genres, 5)),
        top_songs: model(rows(&summary.top_songs, 5)),
        mood: stats::listening_mood(&summary.by_hour).into(),
        busiest_day: busiest_day.into(),
        busiest_month: busiest_month.into(),
    }
}

/// 12345 -> "12,345".
fn group_thousands(n: i64) -> String {
    let digits = n.abs().to_string();
    let mut out = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    if n < 0 { format!("-{out}") } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thousands_are_grouped() {
        assert_eq!(group_thousands(0), "0");
        assert_eq!(group_thousands(999), "999");
        assert_eq!(group_thousands(1000), "1,000");
        assert_eq!(group_thousands(1_234_567), "1,234,567");
    }

    #[test]
    fn a_year_with_nothing_played_still_reviews() {
        let store = Store::open_in_memory().unwrap();
        let review = year_review(&store, 2026);
        assert_eq!(review.minutes, "0");
        assert_eq!(review.top_song, "Nothing yet");
    }

    #[test]
    fn a_year_of_plays_reviews_its_top_song() {
        let store = Store::open_in_memory().unwrap();
        let at = Local.with_ymd_and_hms(2026, 5, 4, 20, 0, 0).unwrap().timestamp();
        for (title, seconds) in [("A", 200), ("B", 100), ("A", 220)] {
            store.log_play(&stats::Play {
                played_at: at,
                path: format!("/m/{title}.flac"),
                title: title.into(),
                artist: "Ana".into(),
                album: "First".into(),
                genre: "Rock".into(),
                seconds,
            });
        }
        let review = year_review(&store, 2026);
        assert_eq!(review.top_song, "A");
        assert_eq!(review.minutes, "8");
        assert_eq!(review.top_artist, "Ana");
        assert_eq!(review.busiest_month, "May was your month: 3 plays.");
        // Another year: nothing.
        assert_eq!(year_review(&store, 2025).top_song, "Nothing yet");
    }
}
