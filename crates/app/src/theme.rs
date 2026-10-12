use slint::ComponentHandle;
use std::path::PathBuf;
use ui::{AboutWindow, MainWindow, PropertiesWindow, SettingsWindow, Theme, ThemeEditorWindow};

/// The semantic color tokens every window reads via the `Theme` Slint
/// global (see `crates/ui/ui/theme.slint`). Each top-level window is a
/// separate Slint component instance with its own copy of that global, so
/// `apply_theme` below writes these into all of them.
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
/// user-created custom one (persisted to `themes.txt`).
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
                spectrum_bar: rgb(0x3f, 0xb8, 0xe8),
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
    pub fn load() -> Self {
        let mut entries = builtin_entries();
        entries.extend(load_custom_themes());
        Self { entries }
    }

    pub fn find(&self, id: &str) -> Option<&ThemeEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// Adds (or replaces, if the name slugifies to an existing custom id) a
    /// custom theme and persists all custom themes to disk.
    pub fn add_custom(&mut self, name: &str, colors: ThemeColors) -> String {
        let id = self.upsert_custom(name, colors);
        self.save_custom();
        id
    }

    /// The in-memory half of `add_custom`, without the file write -- split
    /// out so it's testable without touching the real config directory.
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

    pub fn remove_custom(&mut self, id: &str) {
        self.entries.retain(|e| e.id != id);
        self.save_custom();
    }

    fn save_custom(&self) {
        let contents = serialize_custom_themes(&self.entries);
        let path = custom_themes_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&path, contents) {
            tracing::warn!("failed to save custom themes to {}: {e}", path.display());
        }
    }
}

fn slugify(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect()
}

/// Serializes every custom (`is_custom`) entry as `[Name]` blocks of
/// `key=hex` lines. Pure text transform, kept separate from `save_custom`'s
/// file I/O so it's directly testable.
fn serialize_custom_themes(entries: &[ThemeEntry]) -> String {
    let mut contents = String::new();
    for entry in entries.iter().filter(|e| e.is_custom) {
        contents.push_str(&format!("[{}]\n", entry.label));
        for (key, _) in ThemeColors::FIELDS {
            let value = entry.colors.get(key).expect("field key must be valid");
            contents.push_str(&format!("{key}={}\n", color_to_hex(value)));
        }
        contents.push('\n');
    }
    contents
}

/// Parses the format `serialize_custom_themes` writes. Unknown/malformed
/// lines are skipped rather than failing the whole file, so a hand-edited
/// `themes.txt` degrades gracefully instead of losing every theme in it.
fn parse_custom_themes(contents: &str) -> Vec<ThemeEntry> {
    let mut entries = Vec::new();
    let mut current_label: Option<String> = None;
    let mut current_colors = builtin_entries()[0].colors;

    for line in contents.lines() {
        let line = line.trim();
        if let Some(label) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            if let Some(label) = current_label.take() {
                entries.push(ThemeEntry {
                    id: format!("custom:{}", slugify(&label)),
                    label,
                    colors: current_colors,
                    is_custom: true,
                });
            }
            current_label = Some(label.to_string());
            current_colors = builtin_entries()[0].colors;
        } else if let Some((key, value)) = line.split_once('=') {
            if let Some(color) = parse_hex_color(value) {
                current_colors.set(key.trim(), color);
            }
        }
    }
    if let Some(label) = current_label {
        entries.push(ThemeEntry {
            id: format!("custom:{}", slugify(&label)),
            label,
            colors: current_colors,
            is_custom: true,
        });
    }
    entries
}

fn load_custom_themes() -> Vec<ThemeEntry> {
    let Ok(contents) = std::fs::read_to_string(custom_themes_path()) else {
        return Vec::new();
    };
    parse_custom_themes(&contents)
}

fn custom_themes_path() -> PathBuf {
    crate::settings::config_dir().join("themes.txt")
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
pub fn apply_theme(
    main_window: &MainWindow,
    properties_window: &PropertiesWindow,
    about_window: &AboutWindow,
    settings_window: &SettingsWindow,
    theme_editor_window: &ThemeEditorWindow,
    colors: &ThemeColors,
) {
    let is_dark = is_dark_background(colors.bg_base);

    set_theme_global(main_window.global::<Theme>(), colors);
    main_window.set_theme_is_dark(is_dark);

    set_theme_global(properties_window.global::<Theme>(), colors);
    properties_window.set_theme_is_dark(is_dark);

    set_theme_global(about_window.global::<Theme>(), colors);
    about_window.set_theme_is_dark(is_dark);

    set_theme_global(settings_window.global::<Theme>(), colors);
    settings_window.set_theme_is_dark(is_dark);

    set_theme_global(theme_editor_window.global::<Theme>(), colors);
    theme_editor_window.set_theme_is_dark(is_dark);
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn custom_theme_survives_a_serialize_parse_round_trip() {
        let mut colors = builtin_entries()[0].colors;
        colors.accent = rgb(0x12, 0x34, 0x56);
        let entries = vec![ThemeEntry {
            id: "custom:ocean-deep".into(),
            label: "Ocean Deep".into(),
            colors,
            is_custom: true,
        }];

        let text = serialize_custom_themes(&entries);
        let parsed = parse_custom_themes(&text);

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].label, "Ocean Deep");
        assert_eq!(parsed[0].id, "custom:ocean-deep");
        assert!(parsed[0].is_custom);
        assert_eq!(parsed[0].colors.accent, colors.accent);
        assert_eq!(parsed[0].colors.bg_base, colors.bg_base);
    }

    #[test]
    fn upsert_custom_replaces_an_existing_theme_with_the_same_name_instead_of_duplicating() {
        // Exercises the in-memory half of `add_custom` only, so this test
        // never touches the real config directory.
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
