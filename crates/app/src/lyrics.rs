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

    // No lyric source calls this yet (see the module docs); the tests do.
    #[allow(dead_code)]
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
                match parse_timestamp(&after_bracket[..close]) {
                    Some(time) => times.push(time),
                    // `[ar:Artist]`, `[offset:...]`, ...: metadata, not lyrics.
                    None => is_tag = true,
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
}
