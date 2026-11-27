//! How a record looks on the VNL panel (issue #40): its color and design,
//! chosen in a song's Properties. A look is kept for the whole album -- by
//! default, since the album is one record -- or for one song that has its
//! own. Stored in the `vinyl_styles` table under `album_key` / `track_key`;
//! an album's under each album name on its record exactly as tagged, so
//! with smart grouping it covers every tagging smart grouping gathered.

use crate::store::{Store, StoredVinylStyle};
use crate::theme::{color_to_hex, parse_hex_color};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Design {
    /// Plain vinyl in the color.
    #[default]
    Classic,
    /// See-through colored vinyl, the platter showing under it.
    Translucent,
    /// Swirls of a second color through it.
    Marbled,
    /// Flecks of other colors, as if paint were splashed on.
    Splatter,
    /// Two halves: the color and a contrasting one.
    Split,
    /// The cover art printed across the whole record.
    Picture,
}

impl Design {
    /// In the Properties window's order.
    pub const ALL: [Design; 6] =
        [Self::Classic, Self::Translucent, Self::Marbled, Self::Splatter, Self::Split, Self::Picture];

    /// Shared with Slint (`VinylDisc.design`) and stored.
    pub fn id(self) -> &'static str {
        match self {
            Self::Classic => "classic",
            Self::Translucent => "translucent",
            Self::Marbled => "marbled",
            Self::Splatter => "splatter",
            Self::Split => "split",
            Self::Picture => "picture",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Classic => "Classic",
            Self::Translucent => "Translucent",
            Self::Marbled => "Marbled",
            Self::Splatter => "Splatter",
            Self::Split => "Split",
            Self::Picture => "Picture disc",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|design| design.id() == id)
    }
}

/// The colors offered as swatches: (name, "#rrggbb"). The first is the
/// default.
pub const PRESETS: [(&str, &str); 12] = [
    ("Black", "#161616"),
    ("Smoke", "#5c5c5c"),
    ("White", "#e9e6df"),
    ("Red", "#b3202a"),
    ("Orange", "#e0701a"),
    ("Gold", "#c9a13b"),
    ("Yellow", "#e8c124"),
    ("Green", "#2f8f4e"),
    ("Teal", "#1f8a8a"),
    ("Blue", "#2556b8"),
    ("Purple", "#6b3fa0"),
    ("Pink", "#e07aa8"),
];

#[derive(Debug, Clone, PartialEq)]
pub struct VinylStyle {
    pub color: slint::Color,
    pub design: Design,
    pub label: Label,
    /// The cover art printed on the label (when there's one); off, the
    /// label is plain, in its colour.
    pub label_art: bool,
}

impl Default for VinylStyle {
    fn default() -> Self {
        Self { color: default_color(), design: Design::Classic, label: Label::Accent, label_art: true }
    }
}

/// What the record's label is (under the cover art, if that's printed on
/// it).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Label {
    /// The theme's accent colour.
    #[default]
    Accent,
    Color(slint::Color),
    /// The vinyl itself, its colour and design going on to the middle.
    Match,
    /// Clear: the platter shows through.
    Clear,
}

impl Label {
    /// Shared with Slint (`VinylDisc.label-kind`).
    pub fn kind(self) -> &'static str {
        match self {
            Self::Accent => "accent",
            Self::Color(_) => "color",
            Self::Match => "match",
            Self::Clear => "clear",
        }
    }

    pub fn color(self) -> Option<slint::Color> {
        match self {
            Self::Color(color) => Some(color),
            _ => None,
        }
    }

    /// As the `label` column keeps it: NULL, "#rrggbb", "match" or
    /// "clear".
    fn to_stored(self) -> Option<String> {
        match self {
            Self::Accent => None,
            Self::Color(color) => Some(color_to_hex(color)),
            Self::Match => Some("match".into()),
            Self::Clear => Some("clear".into()),
        }
    }

    fn from_stored(stored: Option<&str>) -> Self {
        match stored {
            None => Self::Accent,
            Some("match") => Self::Match,
            Some("clear") => Self::Clear,
            Some(hex) => parse_hex_color(hex).map_or(Self::Accent, Self::Color),
        }
    }
}

/// The vinyl's colour unless one's picked: black.
pub fn default_color() -> slint::Color {
    parse_hex_color(PRESETS[0].1).expect("a valid preset")
}

impl VinylStyle {
    pub fn hex(&self) -> String {
        color_to_hex(self.color)
    }

    /// The second color of the marbled, splatter and split designs: near
    /// white on dark vinyl, near black on light.
    pub fn contrast(&self) -> slint::Color {
        let c = self.color;
        let luma = 0.2126 * f32::from(c.red()) + 0.7152 * f32::from(c.green()) + 0.0722 * f32::from(c.blue());
        if luma < 110.0 {
            slint::Color::from_rgb_u8(0xee, 0xec, 0xe6)
        } else {
            slint::Color::from_rgb_u8(0x14, 0x14, 0x14)
        }
    }

    fn load(store: &Store, key: &str) -> Option<Self> {
        let stored = store.vinyl_style(key)?;
        Some(Self {
            color: parse_hex_color(&stored.color)?,
            design: Design::from_id(&stored.design).unwrap_or_default(),
            label: Label::from_stored(stored.label.as_deref()),
            label_art: stored.label_art,
        })
    }

    fn save(&self, store: &Store, key: &str) {
        store.set_vinyl_style(
            key,
            &StoredVinylStyle {
                color: self.hex(),
                design: self.design.id().to_string(),
                label: self.label.to_stored(),
                label_art: self.label_art,
            },
        );
    }
}

/// Where an album's look is stored; `album` is a
/// `playlist_rows::vinyl_style_key`.
fn album_key(album: &str) -> String {
    format!("album\u{1f}{album}")
}

/// Where a song's own look is stored.
fn track_key(path: &Path) -> String {
    format!("track\u{1f}{}", path.to_string_lossy())
}

/// What `path`'s record looks like, and whether that's its album's look
/// (false: the song has its own).
pub fn style_for(store: &Store, path: &Path, album: &str) -> (VinylStyle, bool) {
    match VinylStyle::load(store, &track_key(path)) {
        Some(own) => (own, false),
        None => (VinylStyle::load(store, &album_key(album)).unwrap_or_default(), true),
    }
}

/// The look `album` (a `playlist_rows::vinyl_style_key`) has: its own, or
/// the default.
pub fn album_look(store: &Store, album: &str) -> VinylStyle {
    VinylStyle::load(store, &album_key(album)).unwrap_or_default()
}

/// Keeps `style` for `album`.
pub fn save_album_look(store: &Store, album: &str, style: &VinylStyle) {
    style.save(store, &album_key(album));
}

/// Gives `tracks` (path, the albums on its record) the look: their
/// records' (`synced`) -- the songs themselves losing any look of their
/// own, the rest of the record keeping theirs -- or each its own.
pub fn save(store: &Store, tracks: &[(std::path::PathBuf, Vec<String>)], style: &VinylStyle, synced: bool) {
    for (path, albums) in tracks {
        if synced {
            store.remove_vinyl_style(&track_key(path));
            for album in albums {
                style.save(store, &album_key(album));
            }
        } else {
            style.save(store, &track_key(path));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn a_song_follows_its_album_until_it_has_its_own_look() {
        let store = Store::open_in_memory().unwrap();
        // Smart grouping put "a" and "A (Disc 2)" on one record.
        let record = vec!["a".to_string(), "A (Disc 2)".to_string()];
        let song = (PathBuf::from("/m/a/1.flac"), "a");
        let other = (PathBuf::from("/m/a/2.flac"), "A (Disc 2)");
        assert_eq!(style_for(&store, &song.0, song.1), (VinylStyle::default(), true));

        let red_marble = VinylStyle {
            color: parse_hex_color("#b3202a").unwrap(),
            design: Design::Marbled,
            label: Label::Color(parse_hex_color("#e9e6df").unwrap()),
            label_art: false,
        };
        save(&store, &[(song.0.clone(), record.clone())], &red_marble, true);
        // The whole record has it.
        assert_eq!(style_for(&store, &other.0, other.1), (red_marble.clone(), true));

        let blue_split = VinylStyle {
            color: parse_hex_color("#2556b8").unwrap(),
            design: Design::Split,
            label: Label::Clear,
            ..VinylStyle::default()
        };
        save(&store, &[(song.0.clone(), record.clone())], &blue_split, false);
        assert_eq!(style_for(&store, &song.0, song.1), (blue_split.clone(), false));
        assert_eq!(style_for(&store, &other.0, other.1), (red_marble.clone(), true));

        // The other song's album changes look: the song with its own keeps
        // it.
        let gold = VinylStyle { color: parse_hex_color("#c9a13b").unwrap(), label: Label::Match, ..VinylStyle::default() };
        save(&store, &[(other.0.clone(), record.clone())], &gold, true);
        assert_eq!(style_for(&store, &song.0, song.1), (blue_split, false));
        assert_eq!(style_for(&store, &other.0, other.1), (gold, true));

        // Back in sync: its own look is gone.
        save(&store, &[(song.0.clone(), record)], &red_marble, true);
        assert_eq!(style_for(&store, &song.0, song.1), (red_marble.clone(), true));
        assert_eq!(style_for(&store, &other.0, other.1), (red_marble, true));
    }

    #[test]
    fn designs_round_trip_and_contrast_suits_the_color() {
        for design in Design::ALL {
            assert_eq!(Design::from_id(design.id()), Some(design));
        }
        assert_eq!(Design::from_id("nope"), None);
        let black = VinylStyle::default();
        let white = VinylStyle { color: parse_hex_color("#e9e6df").unwrap(), design: Design::Split, ..VinylStyle::default() };
        assert!(black.contrast().red() > 200);
        assert!(white.contrast().red() < 50);
        assert_eq!(black.hex(), "#161616");
    }
}
