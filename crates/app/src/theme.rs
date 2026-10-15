use crate::store::Store;
use slint::ComponentHandle;
use ui::{
    AboutWindow, LyricsEditorWindow, PlaylistManagerWindow, MainWindow, PropertiesWindow, SettingsWindow, Theme,
    ThemeEditorWindow,
};

/// The semantic color tokens every window reads via the `Theme` Slint
/// global (see `crates/ui/ui/theme.slint`). Each top-level window is a
/// separate Slint component instance with its own copy of that global, so
/// `ThemedWindows::apply` below writes these into all of them.
#[derive(Debug, Clone, Copy)]
pub struct ThemeColors {
    pub bg_base: slint::Color,
    pub bg_raised: slint::Color,
    pub bg_sunken: slint::Color,
    pub text_primary: slint::Color,
    pub text_muted: slint::Color,
    pub accent: slint::Color,
    pub accent_text: slint::Color,
    pub waveform_unplayed: slint::Color,
    pub spectrum_bar: slint::Color,
}

impl ThemeColors {
    /// Looks up a field by the same key used in persistence/the editor UI.
    pub fn get(&self, key: &str) -> Option<slint::Color> {
        Some(match key {
            "bg_base" => self.bg_base,
            "bg_raised" => self.bg_raised,
            "bg_sunken" => self.bg_sunken,
            "text_primary" => self.text_primary,
            "text_muted" => self.text_muted,
            "accent" => self.accent,
            "accent_text" => self.accent_text,
            "waveform_unplayed" => self.waveform_unplayed,
            "spectrum_bar" => self.spectrum_bar,
            _ => return None,
        })
    }

    pub fn set(&mut self, key: &str, color: slint::Color) {
        match key {
            "bg_base" => self.bg_base = color,
            "bg_raised" => self.bg_raised = color,
            "bg_sunken" => self.bg_sunken = color,
            "text_primary" => self.text_primary = color,
            "text_muted" => self.text_muted = color,
            "accent" => self.accent = color,
            "accent_text" => self.accent_text = color,
            "waveform_unplayed" => self.waveform_unplayed = color,
            "spectrum_bar" => self.spectrum_bar = color,
            _ => {}
        }
    }

    /// All field keys in the order the theme editor displays them.
    pub const FIELDS: [(&'static str, &'static str); 9] = [
        ("bg_base", "Background"),
        ("bg_raised", "Panel background"),
        ("bg_sunken", "Sunken background"),
        ("text_primary", "Text"),
        ("text_muted", "Muted text"),
        ("accent", "Accent"),
        ("accent_text", "Accent text"),
        ("waveform_unplayed", "Waveform (unplayed)"),
        ("spectrum_bar", "Spectrum bar"),
    ];
}

fn rgb(r: u8, g: u8, b: u8) -> slint::Color {
    slint::Color::from_rgb_u8(r, g, b)
}

pub fn parse_hex_color(input: &str) -> Option<slint::Color> {
    let hex = input.trim().trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(rgb(r, g, b))
}

pub fn color_to_hex(color: slint::Color) -> String {
    format!(
        "#{:02x}{:02x}{:02x}",
        color.red(),
        color.green(),
        color.blue()
    )
}

/// One selectable theme: a built-in (fixed id, not persisted) or a
/// user-created custom one (persisted to the `themes` table in [`Store`]).
#[derive(Debug, Clone)]
pub struct ThemeEntry {
    pub id: String,
    pub label: String,
    pub colors: ThemeColors,
    pub is_custom: bool,
}

fn builtin_entries() -> Vec<ThemeEntry> {
    vec![
        ThemeEntry {
            id: "dark".into(),
            label: "Dark".into(),
            is_custom: false,
            colors: ThemeColors {
                bg_base: rgb(0x1b, 0x1b, 0x1b),
                bg_raised: rgb(0x20, 0x20, 0x20),
                bg_sunken: rgb(0x14, 0x14, 0x14),
                text_primary: rgb(0xe8, 0xe8, 0xe8),
                text_muted: rgb(0x9a, 0x9a, 0x9a),
                accent: rgb(0xc9, 0x7a, 0x26),
                accent_text: rgb(0x1b, 0x1b, 0x1b),
                waveform_unplayed: rgb(0x4a, 0x4a, 0x4a),
                // Quiet bars stay grey and blend into the accent as they
                // get louder (see spectrum_view.slint), like the reference.
                spectrum_bar: rgb(0x3a, 0x3a, 0x3a),
            },
        },
        ThemeEntry {
            id: "light".into(),
            label: "Light".into(),
            is_custom: false,
            colors: ThemeColors {
                bg_base: rgb(0xf4, 0xf4, 0xf4),
                bg_raised: rgb(0xe8, 0xe8, 0xe8),
                bg_sunken: rgb(0xde, 0xde, 0xde),
                text_primary: rgb(0x1b, 0x1b, 0x1b),
                text_muted: rgb(0x6a, 0x6a, 0x6a),
                accent: rgb(0x1a, 0x73, 0xe8),
                accent_text: rgb(0xff, 0xff, 0xff),
                waveform_unplayed: rgb(0xc7, 0xc7, 0xc7),
                spectrum_bar: rgb(0x0d, 0x94, 0x88),
            },
        },
        ThemeEntry {
            id: "one_dark".into(),
            label: "One Dark".into(),
            is_custom: false,
            colors: ThemeColors {
                bg_base: rgb(0x28, 0x2c, 0x34),
                bg_raised: rgb(0x2c, 0x31, 0x3a),
                bg_sunken: rgb(0x21, 0x25, 0x2b),
                text_primary: rgb(0xab, 0xb2, 0xbf),
                text_muted: rgb(0x5c, 0x63, 0x70),
                accent: rgb(0x61, 0xaf, 0xef),
                accent_text: rgb(0x28, 0x2c, 0x34),
                waveform_unplayed: rgb(0x3e, 0x44, 0x51),
                spectrum_bar: rgb(0xc6, 0x78, 0xdd),
            },
        },
        ThemeEntry {
            id: "dracula".into(),
            label: "Dracula".into(),
            is_custom: false,
            colors: ThemeColors {
                bg_base: rgb(0x28, 0x2a, 0x36),
                bg_raised: rgb(0x34, 0x37, 0x46),
                bg_sunken: rgb(0x21, 0x22, 0x2c),
                text_primary: rgb(0xf8, 0xf8, 0xf2),
                text_muted: rgb(0x62, 0x72, 0xa4),
                accent: rgb(0xbd, 0x93, 0xf9),
                accent_text: rgb(0x28, 0x2a, 0x36),
                waveform_unplayed: rgb(0x44, 0x47, 0x5a),
                spectrum_bar: rgb(0x50, 0xfa, 0x7b),
            },
        },
        // Inspired by Stellaris' interface: deep blue-black panels, teal
        // highlights, and the gold its UI uses for what matters most -- here
        // the loudest spectrum bars and peak markers blend into it.
        ThemeEntry {
            id: "stellaris".into(),
            label: "Stellaris".into(),
            is_custom: false,
            colors: ThemeColors {
                bg_base: rgb(0x0a, 0x14, 0x1b),
                bg_raised: rgb(0x10, 0x21, 0x2b),
                bg_sunken: rgb(0x06, 0x0d, 0x12),
                text_primary: rgb(0xcf, 0xe7, 0xe9),
                text_muted: rgb(0x6d, 0x8d, 0x94),
                accent: rgb(0xd9, 0xa6, 0x40),
                accent_text: rgb(0x0a, 0x14, 0x1b),
                waveform_unplayed: rgb(0x1d, 0x3a, 0x44),
                spectrum_bar: rgb(0x1f, 0x8f, 0x8c),
            },
        },
        ThemeEntry {
            id: "nord".into(),
            label: "Nord".into(),
            is_custom: false,
            colors: ThemeColors {
                bg_base: rgb(0x2e, 0x34, 0x40),
                bg_raised: rgb(0x3b, 0x42, 0x52),
                bg_sunken: rgb(0x27, 0x2c, 0x36),
                text_primary: rgb(0xec, 0xef, 0xf4),
                text_muted: rgb(0x7b, 0x88, 0xa1),
                accent: rgb(0x88, 0xc0, 0xd0),
                accent_text: rgb(0x2e, 0x34, 0x40),
                waveform_unplayed: rgb(0x43, 0x4c, 0x5e),
                spectrum_bar: rgb(0x81, 0xa1, 0xc1),
            },
        },
        ThemeEntry {
            id: "solarized_dark".into(),
            label: "Solarized Dark".into(),
            is_custom: false,
            colors: ThemeColors {
                bg_base: rgb(0x00, 0x2b, 0x36),
                bg_raised: rgb(0x07, 0x36, 0x42),
                bg_sunken: rgb(0x00, 0x21, 0x2b),
                text_primary: rgb(0x93, 0xa1, 0xa1),
                text_muted: rgb(0x58, 0x6e, 0x75),
                accent: rgb(0x26, 0x8b, 0xd2),
                accent_text: rgb(0xfd, 0xf6, 0xe3),
                waveform_unplayed: rgb(0x1d, 0x4a, 0x55),
                spectrum_bar: rgb(0x2a, 0xa1, 0x98),
            },
        },
        ThemeEntry {
            id: "solarized_light".into(),
            label: "Solarized Light".into(),
            is_custom: false,
            colors: ThemeColors {
                bg_base: rgb(0xfd, 0xf6, 0xe3),
                bg_raised: rgb(0xee, 0xe8, 0xd5),
                bg_sunken: rgb(0xe4, 0xdd, 0xc8),
                text_primary: rgb(0x07, 0x36, 0x42),
                text_muted: rgb(0x93, 0xa1, 0xa1),
                accent: rgb(0x26, 0x8b, 0xd2),
                accent_text: rgb(0xfd, 0xf6, 0xe3),
                waveform_unplayed: rgb(0xd3, 0xcb, 0xb7),
                spectrum_bar: rgb(0x2a, 0xa1, 0x98),
            },
        },
        ThemeEntry {
            id: "gruvbox_dark".into(),
            label: "Gruvbox Dark".into(),
            is_custom: false,
            colors: ThemeColors {
                bg_base: rgb(0x28, 0x28, 0x28),
                bg_raised: rgb(0x32, 0x30, 0x2f),
                bg_sunken: rgb(0x1d, 0x20, 0x21),
                text_primary: rgb(0xeb, 0xdb, 0xb2),
                text_muted: rgb(0xa8, 0x99, 0x84),
                accent: rgb(0xfe, 0x80, 0x19),
                accent_text: rgb(0x28, 0x28, 0x28),
                waveform_unplayed: rgb(0x50, 0x49, 0x45),
                spectrum_bar: rgb(0xb8, 0xbb, 0x26),
            },
        },
        ThemeEntry {
            id: "monokai".into(),
            label: "Monokai".into(),
            is_custom: false,
            colors: ThemeColors {
                bg_base: rgb(0x27, 0x28, 0x22),
                bg_raised: rgb(0x2f, 0x30, 0x2a),
                bg_sunken: rgb(0x1e, 0x1f, 0x1c),
                text_primary: rgb(0xf8, 0xf8, 0xf2),
                text_muted: rgb(0x75, 0x71, 0x5e),
                accent: rgb(0xa6, 0xe2, 0x2e),
                accent_text: rgb(0x27, 0x28, 0x22),
                waveform_unplayed: rgb(0x49, 0x48, 0x3e),
                spectrum_bar: rgb(0xf9, 0x26, 0x72),
            },
        },
        ThemeEntry {
            id: "catppuccin_mocha".into(),
            label: "Catppuccin Mocha".into(),
            is_custom: false,
            colors: ThemeColors {
                bg_base: rgb(0x1e, 0x1e, 0x2e),
                bg_raised: rgb(0x26, 0x26, 0x37),
                bg_sunken: rgb(0x18, 0x18, 0x25),
                text_primary: rgb(0xcd, 0xd6, 0xf4),
                text_muted: rgb(0x7f, 0x84, 0x9c),
                accent: rgb(0xcb, 0xa6, 0xf7),
                accent_text: rgb(0x1e, 0x1e, 0x2e),
                waveform_unplayed: rgb(0x45, 0x47, 0x5a),
                spectrum_bar: rgb(0x89, 0xb4, 0xfa),
            },
        },
        ThemeEntry {
            id: "tokyo_night".into(),
            label: "Tokyo Night".into(),
            is_custom: false,
            colors: ThemeColors {
                bg_base: rgb(0x1a, 0x1b, 0x26),
                bg_raised: rgb(0x1f, 0x23, 0x35),
                bg_sunken: rgb(0x16, 0x16, 0x1e),
                text_primary: rgb(0xc0, 0xca, 0xf5),
                text_muted: rgb(0x56, 0x5f, 0x89),
                accent: rgb(0x7a, 0xa2, 0xf7),
                accent_text: rgb(0x1a, 0x1b, 0x26),
                waveform_unplayed: rgb(0x29, 0x2e, 0x42),
                spectrum_bar: rgb(0xbb, 0x9a, 0xf7),
            },
        },
        ThemeEntry {
            id: "github_dark".into(),
            label: "GitHub Dark".into(),
            is_custom: false,
            colors: ThemeColors {
                bg_base: rgb(0x0d, 0x11, 0x17),
                bg_raised: rgb(0x16, 0x1b, 0x22),
                bg_sunken: rgb(0x01, 0x04, 0x09),
                text_primary: rgb(0xe6, 0xed, 0xf3),
                text_muted: rgb(0x7d, 0x85, 0x90),
                accent: rgb(0x2f, 0x81, 0xf7),
                accent_text: rgb(0xff, 0xff, 0xff),
                waveform_unplayed: rgb(0x30, 0x36, 0x3d),
                spectrum_bar: rgb(0x3f, 0xb9, 0x50),
            },
        },
        // Clair Obscur: Expedition 33: Belle Époque/Art Deco -- warm
        // near-black, ivory text, gold ornament, and the Paintress's crimson
        // in the spectrum, flaring into gold as it gets loud.
        ThemeEntry {
            id: "clair_obscur".into(),
            label: "Clair Obscur: Expedition 33".into(),
            is_custom: false,
            colors: ThemeColors {
                bg_base: rgb(0x0d, 0x0b, 0x0a),
                bg_raised: rgb(0x17, 0x13, 0x11),
                bg_sunken: rgb(0x08, 0x07, 0x06),
                text_primary: rgb(0xec, 0xe3, 0xd0),
                text_muted: rgb(0x8c, 0x82, 0x73),
                accent: rgb(0xc9, 0xa4, 0x5c),
                accent_text: rgb(0x0d, 0x0b, 0x0a),
                waveform_unplayed: rgb(0x3a, 0x30, 0x2a),
                spectrum_bar: rgb(0x8e, 0x1f, 0x26),
            },
        },
        // Fallout's Pip-Boy: everything in one green phosphor on a dark CRT,
        // brighter where it matters.
        ThemeEntry {
            id: "pip_boy".into(),
            label: "Pip-Boy".into(),
            is_custom: false,
            colors: ThemeColors {
                bg_base: rgb(0x07, 0x12, 0x0a),
                bg_raised: rgb(0x0b, 0x1b, 0x0f),
                bg_sunken: rgb(0x04, 0x0b, 0x06),
                text_primary: rgb(0x3c, 0xff, 0x7a),
                text_muted: rgb(0x2a, 0x9a, 0x4c),
                accent: rgb(0x3c, 0xff, 0x7a),
                accent_text: rgb(0x04, 0x10, 0x0a),
                waveform_unplayed: rgb(0x14, 0x38, 0x1f),
                spectrum_bar: rgb(0x1b, 0x5c, 0x30),
            },
        },
        // Cyberpunk 2077: near-black with a cold blue tint, cyan text,
        // neon yellow for what's active, and the spectrum running from hot
        // red into that yellow as it gets loud.
        ThemeEntry {
            id: "cyberpunk".into(),
            label: "Cyberpunk 2077".into(),
            is_custom: false,
            colors: ThemeColors {
                bg_base: rgb(0x0a, 0x0b, 0x10),
                bg_raised: rgb(0x12, 0x14, 0x1d),
                bg_sunken: rgb(0x05, 0x06, 0x09),
                text_primary: rgb(0x5e, 0xf6, 0xff),
                text_muted: rgb(0x4a, 0x7a, 0x86),
                accent: rgb(0xfc, 0xee, 0x0a),
                accent_text: rgb(0x0a, 0x0b, 0x10),
                waveform_unplayed: rgb(0x26, 0x2a, 0x38),
                spectrum_bar: rgb(0x8a, 0x0f, 0x2b),
            },
        },
    ]
}

/// A theme counts as visually dark when its base background is dark --
/// used to decide whether std-widgets (Button/CheckBox/LineEdit/...) should
/// render in Slint's dark or light built-in style.
fn is_dark_background(color: slint::Color) -> bool {
    let luma =
        0.299 * color.red() as f32 + 0.587 * color.green() as f32 + 0.114 * color.blue() as f32;
    luma < 128.0
}

pub struct ThemeRegistry {
    pub entries: Vec<ThemeEntry>,
}

impl ThemeRegistry {
    pub fn load(store: &Store) -> Self {
        let mut entries = builtin_entries();
        entries.extend(store.load_custom_themes());
        Self { entries }
    }

    pub fn find(&self, id: &str) -> Option<&ThemeEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// Adds (or replaces, if the name slugifies to an existing custom id) a
    /// custom theme and persists every custom theme to `store`.
    pub fn add_custom(&mut self, store: &Store, name: &str, colors: ThemeColors) -> String {
        let id = self.upsert_custom(name, colors);
        self.save_custom(store);
        id
    }

    /// The in-memory half of `add_custom`, without the store write -- split
    /// out so it's testable without touching a real database.
    fn upsert_custom(&mut self, name: &str, colors: ThemeColors) -> String {
        let id = format!("custom:{}", slugify(name));
        if let Some(existing) = self.entries.iter_mut().find(|e| e.id == id) {
            existing.label = name.to_string();
            existing.colors = colors;
        } else {
            self.entries.push(ThemeEntry {
                id: id.clone(),
                label: name.to_string(),
                colors,
                is_custom: true,
            });
        }
        id
    }

    pub fn remove_custom(&mut self, store: &Store, id: &str) {
        self.entries.retain(|e| e.id != id);
        self.save_custom(store);
    }

    fn save_custom(&self, store: &Store) {
        let custom: Vec<ThemeEntry> = self
            .entries
            .iter()
            .filter(|e| e.is_custom)
            .cloned()
            .collect();
        store.save_custom_themes(&custom);
    }
}

fn slugify(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect()
}

fn set_theme_global(theme: Theme<'_>, colors: &ThemeColors) {
    theme.set_bg_base(colors.bg_base);
    theme.set_bg_raised(colors.bg_raised);
    theme.set_bg_sunken(colors.bg_sunken);
    theme.set_text_primary(colors.text_primary);
    theme.set_text_muted(colors.text_muted);
    theme.set_accent(colors.accent);
    theme.set_accent_text(colors.accent_text);
    theme.set_waveform_unplayed(colors.waveform_unplayed);
    theme.set_spectrum_bar(colors.spectrum_bar);
}

/// Writes `colors` into every top-level window's own `Theme` global
/// instance (each is a separate Slint component instantiation, so each has
/// independent global state) and syncs `theme-is-dark`, which each window's
/// own `.slint` file uses to keep std-widgets (Button/CheckBox/LineEdit/
/// MenuBar/...) in the matching built-in dark/light style.
/// Every top-level window, so a theme can be applied to all of them at
/// once (each has its own copy of the `Theme` global). Holds weak handles:
/// a window that's gone is simply skipped.
#[derive(Clone)]
pub struct ThemedWindows {
    main: slint::Weak<MainWindow>,
    properties: slint::Weak<PropertiesWindow>,
    about: slint::Weak<AboutWindow>,
    settings: slint::Weak<SettingsWindow>,
    theme_editor: slint::Weak<ThemeEditorWindow>,
    lyrics_editor: slint::Weak<LyricsEditorWindow>,
    playlist_manager: slint::Weak<PlaylistManagerWindow>,
}

impl ThemedWindows {
    pub fn new(
        main: &MainWindow,
        properties: &PropertiesWindow,
        about: &AboutWindow,
        settings: &SettingsWindow,
        theme_editor: &ThemeEditorWindow,
        lyrics_editor: &LyricsEditorWindow,
        playlist_manager: &PlaylistManagerWindow,
    ) -> Self {
        Self {
            main: main.as_weak(),
            properties: properties.as_weak(),
            about: about.as_weak(),
            settings: settings.as_weak(),
            theme_editor: theme_editor.as_weak(),
            lyrics_editor: lyrics_editor.as_weak(),
            playlist_manager: playlist_manager.as_weak(),
        }
    }

    pub fn apply(&self, colors: &ThemeColors) {
        let is_dark = is_dark_background(colors.bg_base);
        macro_rules! apply_to {
            ($($window:ident),*) => {$(
                if let Some(window) = self.$window.upgrade() {
                    set_theme_global(window.global::<Theme>(), colors);
                    window.set_theme_is_dark(is_dark);
                }
            )*};
        }
        apply_to!(main, properties, about, settings, theme_editor, lyrics_editor, playlist_manager);
    }

    /// Shows `logo` in every window (title bar, now-playing bar, About).
    pub fn set_logo(&self, logo: &slint::Image) {
        macro_rules! apply_to {
            ($($window:ident),*) => {$(
                if let Some(window) = self.$window.upgrade() {
                    window.global::<Theme>().set_logo(logo.clone());
                }
            )*};
        }
        apply_to!(main, properties, about, settings, theme_editor, lyrics_editor, playlist_manager);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_themes_have_unique_ids_and_readable_text() {
        let entries = builtin_entries();
        let ids: std::collections::HashSet<_> = entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids.len(), entries.len(), "duplicate built-in theme id");
        let luma = |c: slint::Color| {
            0.299 * c.red() as f32 + 0.587 * c.green() as f32 + 0.114 * c.blue() as f32
        };
        for entry in &entries {
            let colors = entry.colors;
            for (what, fg, bg) in [
                ("text", colors.text_primary, colors.bg_base),
                ("accent text", colors.accent_text, colors.accent),
            ] {
                assert!(
                    (luma(fg) - luma(bg)).abs() > 80.0,
                    "{}: {what} too close to its background",
                    entry.label
                );
            }
        }
    }

    #[test]
    fn hex_color_round_trips_through_parse_and_format() {
        let color = parse_hex_color("#c97a26").unwrap();
        assert_eq!(color.red(), 0xc9);
        assert_eq!(color.green(), 0x7a);
        assert_eq!(color.blue(), 0x26);
        assert_eq!(color_to_hex(color), "#c97a26");
    }

    #[test]
    fn parse_hex_color_accepts_missing_hash_and_rejects_garbage() {
        assert!(parse_hex_color("1b1b1b").is_some());
        assert!(parse_hex_color("#zzzzzz").is_none());
        assert!(parse_hex_color("#abc").is_none());
    }

    #[test]
    fn upsert_custom_replaces_an_existing_theme_with_the_same_name_instead_of_duplicating() {
        // Exercises the in-memory half of `add_custom` only, so this test
        // never touches a real database.
        let mut registry = ThemeRegistry {
            entries: builtin_entries(),
        };
        let builtin_count = registry.entries.len();

        let first_id = registry.upsert_custom("My Theme", builtin_entries()[0].colors);
        assert_eq!(registry.entries.len(), builtin_count + 1);

        let mut second_colors = builtin_entries()[1].colors;
        second_colors.accent = rgb(0xaa, 0xbb, 0xcc);
        let second_id = registry.upsert_custom("My Theme", second_colors);

        assert_eq!(first_id, second_id);
        assert_eq!(registry.entries.len(), builtin_count + 1);
        assert_eq!(
            registry.find(&second_id).unwrap().colors.accent,
            second_colors.accent
        );
    }
}
