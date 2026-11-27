//! Lyrics model for the Lyrics panel: synced (LRC-style, one timestamp per
//! line) or plain text. Nothing *finds* lyrics yet -- a source (embedded
//! tags, a `.lrc` file next to the track, an online service) only has to
//! produce a [`Lyrics`] and hand it to the panel; display, highlighting
//! and scrolling with playback already work from that.

use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub struct LyricLine {
    /// When the line starts; `None` for unsynced lyrics.
    pub time: Option<Duration>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Lyrics {
    /// In display order (sorted by time when synced).
    pub lines: Vec<LyricLine>,
}

impl Lyrics {
    pub fn is_synced(&self) -> bool {
        self.lines.iter().any(|line| line.time.is_some())
    }

    /// Parses LRC (`[mm:ss.xx] text`, several timestamps per line allowed,
    /// `[ar:...]`-style tags ignored) or, if there are no timestamps at all,
    /// plain text, one line per line.
    pub fn parse(text: &str) -> Self {
        let mut lines = Vec::new();
        let mut any_timestamp = false;
        for raw in text.lines() {
            let mut rest = raw.trim();
            let mut times = Vec::new();
            let mut is_tag = false;
            while let Some(after_bracket) = rest.strip_prefix('[') {
                let Some(close) = after_bracket.find(']') else {
                    break;
                };
                let inside = &after_bracket[..close];
                match parse_timestamp(inside) {
                    Some(time) => times.push(time),
                    // `[ar:Artist]`, `[offset:+100]`, ...: LRC metadata, not
                    // lyrics.
                    None if is_lrc_tag(inside) => is_tag = true,
                    // Anything else -- `[Verse 1]`, `[Chorus]` -- is part of
                    // the text.
                    None => break,
                }
                rest = after_bracket[close + 1..].trim_start();
            }
            if !times.is_empty() {
                any_timestamp = true;
                lines.extend(times.into_iter().map(|time| LyricLine {
                    time: Some(time),
                    text: rest.to_string(),
                }));
            } else if !is_tag {
                lines.push(LyricLine {
                    time: None,
                    text: raw.trim().to_string(),
                });
            }
        }
        if any_timestamp {
            // Unstamped lines in an LRC file are stray; keep the synced ones.
            lines.retain(|line| line.time.is_some());
            lines.sort_by_key(|line| line.time);
        }
        Self { lines }
    }

    /// The line being sung at `position`: the last one starting at or
    /// before it. `None` for unsynced lyrics or before the first line.
    pub fn active_line(&self, position: Duration) -> Option<usize> {
        if !self.is_synced() {
            return None;
        }
        self.lines
            .iter()
            .rposition(|line| line.time.is_some_and(|time| time <= position))
    }

    /// How far through line `index` `position` is (0 to 1): from its start
    /// to the next line's, or to the song's end (`duration`) for the last.
    pub fn line_progress(&self, index: usize, position: Duration, duration: Duration) -> f32 {
        let Some(start) = self.lines.get(index).and_then(|line| line.time) else {
            return 0.0;
        };
        let end = self.lines[index + 1..]
            .iter()
            .find_map(|line| line.time.filter(|&time| time > start))
            .unwrap_or(duration);
        if end <= start {
            return 1.0;
        }
        (position.saturating_sub(start).as_secs_f32() / (end - start).as_secs_f32()).clamp(0.0, 1.0)
    }
}

/// An LRC ID tag's inside, like `ar:Artist` or `offset:+100`: letters,
/// then a colon.
fn is_lrc_tag(inside: &str) -> bool {
    inside
        .split_once(':')
        .is_some_and(|(name, _)| !name.is_empty() && name.chars().all(|c| c.is_ascii_alphabetic()))
}

/// `[mm:ss.xx]`, the LRC timestamp for `position`.
pub fn format_timestamp(position: Duration) -> String {
    let centis = position.as_millis() / 10;
    format!("[{:02}:{:02}.{:02}]", centis / 6000, centis / 100 % 60, centis % 100)
}

/// The byte range of the line containing byte offset `cursor` (without
/// its line break).
fn line_at(text: &str, cursor: usize) -> std::ops::Range<usize> {
    let cursor = cursor.min(text.len());
    let start = text[..cursor].rfind('\n').map_or(0, |i| i + 1);
    let end = text[start..].find('\n').map_or(text.len(), |i| start + i);
    start..end
}

/// `line` without any leading `[mm:ss.xx]` timestamps.
fn strip_timestamps(line: &str) -> &str {
    let mut rest = line;
    while let Some(after) = rest.strip_prefix('[') {
        match after.find(']') {
            Some(close) if parse_timestamp(&after[..close]).is_some() => rest = &after[close + 1..],
            _ => break,
        }
    }
    rest
}

/// The lyric editor's "Synchronise selected line": stamps the line with
/// the cursor (at byte offset `cursor`) with `position`, replacing any
/// timestamp it had, and returns the new text plus the cursor moved to the
/// start of the next line -- so syncing a song is one click per line.
pub fn sync_line(text: &str, cursor: usize, position: Duration) -> (String, usize) {
    let line = line_at(text, cursor);
    let stamped = format!("{}{}", format_timestamp(position), strip_timestamps(&text[line.clone()]));
    let new_text = format!("{}{}{}", &text[..line.start], stamped, &text[line.end..]);
    let line_end = line.start + stamped.len();
    let next = if line_end < new_text.len() { line_end + 1 } else { line_end };
    (new_text, next)
}

/// Every timestamp in `text` moved by `delta_ms` (earlier when negative,
/// never before 0:00) -- for lyrics synced a touch early or late all
/// through. Lines without one, and `[ar:...]`-style tags, stay as they are.
pub fn shift_timestamps(text: &str, delta_ms: i64) -> String {
    text.split('\n')
        .map(|line| {
            let mut out = String::with_capacity(line.len());
            let mut rest = line;
            while let Some(after) = rest.strip_prefix('[') {
                let Some(close) = after.find(']') else { break };
                let Some(time) = parse_timestamp(&after[..close]) else { break };
                let shifted = (time.as_millis() as i64 + delta_ms).max(0) as u64;
                out.push_str(&format_timestamp(Duration::from_millis(shifted)));
                rest = &after[close + 1..];
            }
            out.push_str(rest);
            out
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Removes the timestamps from the line with the cursor; the cursor ends
/// up at that line's start.
pub fn remove_timestamp(text: &str, cursor: usize) -> (String, usize) {
    let line = line_at(text, cursor);
    let bare = strip_timestamps(&text[line.clone()]);
    (format!("{}{}{}", &text[..line.start], bare, &text[line.end..]), line.start)
}

/// `mm:ss`, `mm:ss.xx` or `mm:ss.xxx` (also `:` before the fraction, as
/// some editors write it).
fn parse_timestamp(stamp: &str) -> Option<Duration> {
    let (minutes, seconds) = stamp.split_once(':')?;
    let minutes: u64 = minutes.trim().parse().ok()?;
    let seconds = seconds.trim().replacen(':', ".", 1);
    let seconds: f64 = seconds.parse().ok()?;
    if !(0.0..60.0).contains(&seconds) {
        return None;
    }
    Some(Duration::from_secs(minutes * 60) + Duration::from_secs_f64(seconds))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LRC: &str = "[ar:Alexander Popov]\n[ti:Helpless]\n\
        [00:12.50]I don't know where to turn\n\
        [00:15.00][01:15.00]I feel the empty space\n\
        [00:18.2]\n";

    #[test]
    fn parses_synced_lyrics_in_time_order() {
        let lyrics = Lyrics::parse(LRC);
        assert!(lyrics.is_synced());
        let texts: Vec<_> = lyrics.lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(
            texts,
            ["I don't know where to turn", "I feel the empty space", "", "I feel the empty space"]
        );
        assert_eq!(lyrics.lines[0].time, Some(Duration::from_millis(12_500)));
        assert_eq!(lyrics.lines[2].time, Some(Duration::from_millis(18_200)));
    }

    #[test]
    fn active_line_follows_the_position() {
        let lyrics = Lyrics::parse(LRC);
        assert_eq!(lyrics.active_line(Duration::from_secs(5)), None);
        assert_eq!(lyrics.active_line(Duration::from_millis(12_500)), Some(0));
        assert_eq!(lyrics.active_line(Duration::from_secs(16)), Some(1));
        assert_eq!(lyrics.active_line(Duration::from_secs(90)), Some(3));
    }

    #[test]
    fn section_headers_stay_but_lrc_tags_go() {
        let lyrics = Lyrics::parse("[ar:Above & Beyond]\r\n[Verse 1]\r\nI've stayed up late\r\n\r\n[Chorus]");
        let texts: Vec<_> = lyrics.lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["[Verse 1]", "I've stayed up late", "", "[Chorus]"]);
        assert!(!lyrics.is_synced());
    }

    #[test]
    fn plain_text_is_unsynced() {
        let lyrics = Lyrics::parse("First line\n\nThird line");
        assert!(!lyrics.is_synced());
        assert_eq!(lyrics.lines.len(), 3);
        assert_eq!(lyrics.active_line(Duration::from_secs(10)), None);
    }

    #[test]
    fn timestamps_accept_common_variants_and_reject_garbage() {
        assert_eq!(parse_timestamp("01:02"), Some(Duration::from_secs(62)));
        assert_eq!(parse_timestamp("01:02.345"), Some(Duration::from_millis(62_345)));
        assert_eq!(parse_timestamp("01:02:50"), Some(Duration::from_millis(62_500)));
        assert_eq!(parse_timestamp("ar:Someone"), None);
        assert_eq!(parse_timestamp("00:75"), None);
    }

    #[test]
    fn syncing_stamps_the_cursor_line_and_moves_on() {
        let text = "First line\nSecond line\nThird";
        // Cursor somewhere in "Second line".
        let (text, cursor) = sync_line(text, 14, Duration::from_millis(83_456));
        assert_eq!(text, "First line\n[01:23.45]Second line\nThird");
        assert_eq!(&text[cursor..], "Third");
        // Re-syncing replaces the old stamp instead of stacking.
        let (text, _) = sync_line(&text, 12, Duration::from_secs(5));
        assert_eq!(text, "First line\n[00:05.00]Second line\nThird");
        // The last line: the cursor stays at its end.
        let (text, cursor) = sync_line(&text, text.len(), Duration::from_secs(9));
        assert!(text.ends_with("[00:09.00]Third"));
        assert_eq!(cursor, text.len());
        // And the result parses as synced lyrics.
        assert!(Lyrics::parse(&text).is_synced());
    }

    #[test]
    fn syncing_handles_multibyte_text_and_empty_lines() {
        let (text, cursor) = sync_line("Zoë\n\nÜber", 1, Duration::from_secs(1));
        assert_eq!(text, "[00:01.00]Zoë\n\nÜber");
        assert_eq!(&text[cursor..], "\nÜber");
        let (text, _) = sync_line(&text, cursor, Duration::from_secs(2));
        assert_eq!(text, "[00:01.00]Zoë\n[00:02.00]\nÜber");
    }

    #[test]
    fn removing_a_timestamp_keeps_the_line() {
        let (text, cursor) = remove_timestamp("[00:01.00]A\n[00:02.00][01:00.00]B", 13);
        assert_eq!(text, "[00:01.00]A\nB");
        assert_eq!(cursor, 12);
        assert_eq!(format_timestamp(Duration::from_millis(3_723_450)), "[62:03.45]");
    }

    #[test]
    fn line_progress_runs_from_a_lines_start_to_the_next() {
        let lyrics = Lyrics::parse("[00:10.00]one\n[00:20.00]two\n[00:20.00]same time\n[00:30.00]three");
        let at = |s: f32| Duration::from_secs_f32(s);
        assert_eq!(lyrics.line_progress(0, at(15.0), at(60.0)), 0.5);
        assert_eq!(lyrics.line_progress(0, at(5.0), at(60.0)), 0.0);
        // Lines sharing a time run to the next later one.
        assert_eq!(lyrics.line_progress(1, at(25.0), at(60.0)), 0.5);
        // The last runs to the song's end.
        assert_eq!(lyrics.line_progress(3, at(45.0), at(60.0)), 0.5);
        assert_eq!(lyrics.line_progress(9, at(45.0), at(60.0)), 0.0);
    }


    #[test]
    fn shifting_moves_every_timestamp_and_nothing_else() {
        let text = "[ar:Someone]\n[00:01.00]one\n[00:00.05][01:59.95]two\nplain";
        assert_eq!(
            shift_timestamps(text, 100),
            "[ar:Someone]\n[00:01.10]one\n[00:00.15][02:00.05]two\nplain"
        );
        // Never before the start.
        assert_eq!(shift_timestamps("[00:00.05]x", -100), "[00:00.00]x");
    }

}
