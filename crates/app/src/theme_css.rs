//! Themes as CSS files (Preferences > Appearance > Export / Import): a
//! `:root` block of custom properties, one per theme colour --
//!
//! ```css
//! /* Openbar5000 theme */
//! :root {
//!   --theme-name: "Midnight";
//!   --bg-base: #1b1b1b;
//!   ...
//! }
//! ```
//!
//! An imported file is only *read* for those values: the nine colours, as
//! hex codes, and the name, as plain text. Nothing else in it is used, kept
//! or passed on -- no `url()`, `@import`, selectors or scripts can do
//! anything -- and what's saved is written anew from the values read, never
//! the file itself.

use crate::theme::{color_to_hex, parse_hex_color, ThemeColors};

/// The largest file read: a theme is a dozen lines.
pub const MAX_FILE_BYTES: u64 = 64 * 1024;
/// The longest name kept.
const MAX_NAME_CHARS: usize = 40;

/// Where themes are kept as CSS files: imported ones (written anew, see the
/// module's comment), and any dropped in by hand.
pub fn themes_dir() -> std::path::PathBuf {
    crate::settings::config_dir().join("themes")
}

/// The file in `themes_dir` for a theme named `name`: its name made safe as
/// a file name (letters, digits and hyphens only, so it can't point
/// anywhere else).
pub fn file_name(name: &str) -> String {
    let slug: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .take(MAX_NAME_CHARS)
        .collect();
    let slug = slug.trim_matches('-');
    format!("{}.css", if slug.is_empty() { "theme" } else { slug })
}

/// The themes in `dir`'s CSS files: (name, colours), colours they lack from
/// `base`. Files too big, unreadable or not themes are skipped.
pub fn load_folder(dir: &std::path::Path, base: &ThemeColors) -> Vec<(String, ThemeColors, std::path::PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut themes = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let is_css = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("css"));
        let small = entry.metadata().is_ok_and(|m| m.is_file() && m.len() <= MAX_FILE_BYTES);
        if !is_css || !small {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        match from_css(&text, base) {
            Ok(theme) => {
                let stem = path.file_stem().map(|s| clean_name(&s.to_string_lossy())).unwrap_or_default();
                let name = theme.name.unwrap_or(stem);
                if !name.is_empty() {
                    themes.push((name, theme.colors, path));
                }
            }
            Err(e) => tracing::warn!("skipping theme {}: {e}", path.display()),
        }
    }
    themes.sort_by_key(|theme| theme.0.to_lowercase());
    themes
}

/// The CSS name of a theme colour: `bg_base` -> `--bg-base`.
fn property(key: &str) -> String {
    format!("--{}", key.replace('_', "-"))
}

/// `colors`, named `name`, as a CSS file.
pub fn to_css(name: &str, colors: &ThemeColors) -> String {
    let mut css = String::from("/* Openbar5000 theme -- Preferences > Appearance > Import to use it. */\n:root {\n");
    css.push_str(&format!("  --theme-name: \"{}\";\n", clean_name(name).replace('"', "'")));
    for (key, label) in ThemeColors::FIELDS {
        let color = colors.get(key).unwrap_or_default();
        css.push_str(&format!("  {}: {}; /* {label} */\n", property(key), color_to_hex(color)));
    }
    css.push_str("}\n");
    css
}

/// A theme read from a CSS file.
#[derive(Debug, Clone)]
pub struct Imported {
    /// From `--theme-name`, if the file has one.
    pub name: Option<String>,
    pub colors: ThemeColors,
    /// Colours the file didn't set (or set to something not a colour),
    /// taken from the theme it was imported over.
    pub missing: Vec<&'static str>,
}

/// Reads `css` for a theme; colours it lacks come from `base`. An error
/// when it sets none of the colours at all -- it isn't a theme.
pub fn from_css(css: &str, base: &ThemeColors) -> Result<Imported, String> {
    let text = strip_comments(css);
    let mut colors = *base;
    let mut name = None;
    let mut found = 0;
    let mut missing = Vec::new();
    let declarations = declarations(&text);
    for (key, label) in ThemeColors::FIELDS {
        let wanted = property(key);
        match declarations.iter().rev().find(|(property, _)| *property == wanted) {
            Some((_, value)) => match parse_color(value) {
                Some(color) => {
                    colors.set(key, color);
                    found += 1;
                }
                None => missing.push(label),
            },
            None => missing.push(label),
        }
    }
    if let Some((_, value)) = declarations.iter().rev().find(|(property, _)| property == "--theme-name") {
        let value = value.trim().trim_matches(|c| c == '"' || c == '\'');
        let value = clean_name(value);
        if !value.is_empty() {
            name = Some(value);
        }
    }
    if found == 0 {
        return Err("it sets none of the theme's colours (--bg-base, --accent, ...)".to_string());
    }
    Ok(Imported { name, colors, missing })
}

/// `name` as plain text: letters, digits, spaces and simple punctuation,
/// at most `MAX_NAME_CHARS` -- no control characters, no markup.
pub fn clean_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| c.is_alphanumeric() || " -_.,'()&+!".contains(*c))
        .take(MAX_NAME_CHARS)
        .collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `css` without its `/* ... */` comments (an unclosed one runs to the end).
fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        match rest[start + 2..].find("*/") {
            Some(end) => rest = &rest[start + 2 + end + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// Every `--property: value` declaration, wherever it is (inside `:root`
/// or not -- selectors don't matter, only these properties are read).
fn declarations(css: &str) -> Vec<(String, String)> {
    css.split(['{', '}', ';'])
        .filter_map(|part| {
            let (property, value) = part.split_once(':')?;
            let property = property.trim();
            // Only custom properties, by name: `--` then letters and
            // hyphens.
            let name_ok = property.len() > 2
                && property.starts_with("--")
                && property[2..].chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
            name_ok.then(|| (property.to_ascii_lowercase(), value.trim().to_string()))
        })
        .collect()
}

/// A hex colour: `#rrggbb` or `#rgb`; anything else (`url(...)`, a
/// variable, a keyword) is no colour.
fn parse_color(value: &str) -> Option<slint::Color> {
    let value = value.trim().trim_end_matches("!important").trim();
    let hex = value.strip_prefix('#')?;
    if hex.len() == 3 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        let long: String = hex.chars().flat_map(|c| [c, c]).collect();
        return parse_hex_color(&long);
    }
    parse_hex_color(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> ThemeColors {
        let black = slint::Color::from_rgb_u8(0, 0, 0);
        ThemeColors {
            bg_base: black,
            bg_raised: black,
            bg_sunken: black,
            text_primary: black,
            text_muted: black,
            accent: black,
            accent_text: black,
            waveform_unplayed: black,
            spectrum_bar: black,
        }
    }

    #[test]
    fn a_theme_round_trips_through_css() {
        let mut colors = base();
        colors.accent = slint::Color::from_rgb_u8(0xc9, 0x7a, 0x26);
        colors.text_primary = slint::Color::from_rgb_u8(0xe8, 0xe8, 0xe8);
        let css = to_css("Midnight Sun", &colors);
        let imported = from_css(&css, &base()).unwrap();
        assert_eq!(imported.name.as_deref(), Some("Midnight Sun"));
        assert!(imported.missing.is_empty());
        assert_eq!(color_to_hex(imported.colors.accent), "#c97a26");
        assert_eq!(color_to_hex(imported.colors.text_primary), "#e8e8e8");
    }

    #[test]
    fn only_hex_colours_for_known_properties_are_taken() {
        let css = r#"
            @import url("https://example.com/evil.css");
            body { background: url(javascript:alert(1)); }
            :root {
              --accent: url(https://example.com/x.png);
              --bg-base: #FfF;
              --text-primary: expression(alert(1));
              --spectrum-bar: #12345;
              --unknown-thing: #ffffff;
              --theme-name: "<script>alert('x')</script>";
            }
        "#;
        let imported = from_css(css, &base()).unwrap();
        assert_eq!(color_to_hex(imported.colors.bg_base), "#ffffff");
        assert_eq!(color_to_hex(imported.colors.accent), "#000000");
        assert_eq!(color_to_hex(imported.colors.text_primary), "#000000");
        assert_eq!(color_to_hex(imported.colors.spectrum_bar), "#000000");
        assert!(imported.missing.contains(&"Accent"));
        // Markup characters dropped from the name.
        let name = imported.name.unwrap();
        assert!(!name.contains('<') && !name.contains('>'), "{name}");
    }

    #[test]
    fn a_file_with_no_theme_colours_isnt_a_theme() {
        assert!(from_css("body { color: red; }", &base()).is_err());
        assert!(from_css("", &base()).is_err());
        assert!(from_css("/* --bg-base: #ffffff; */", &base()).is_err());
        // Unclosed comment: the rest is comment.
        assert!(from_css("/* :root { --bg-base: #ffffff; }", &base()).is_err());
    }

    #[test]
    fn names_are_plain_and_short() {
        assert_eq!(clean_name("  My\u{0007} Theme\n\t "), "My Theme");
        assert_eq!(clean_name(&"x".repeat(100)).chars().count(), MAX_NAME_CHARS);
        assert_eq!(clean_name("../../etc/passwd"), "....etcpasswd");
    }

    #[test]
    fn odd_input_never_panics() {
        for css in ["{{{{", ":::", "--a:", "--bg-base:#", "--bg-base: #€€€", "/*", "*/", ";;;--accent:#000"] {
            let _ = from_css(css, &base());
        }
    }

    #[test]
    fn file_names_stay_in_the_folder() {
        assert_eq!(file_name("My Theme"), "my-theme.css");
        assert_eq!(file_name("../../etc/passwd"), "etc-passwd.css");
        assert_eq!(file_name("///"), "theme.css");
        assert_eq!(file_name("C:\\Windows\\x"), "c--windows-x.css");
    }

    #[test]
    fn a_folder_of_themes_loads_and_skips_what_isnt_one() {
        let dir = std::env::temp_dir().join(format!("openbar5000-themes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ocean.css"), ":root { --accent: #0088ff; }").unwrap();
        std::fs::write(dir.join("named.css"), ":root { --theme-name: 'Deep Sea'; --bg-base: #000011; }").unwrap();
        std::fs::write(dir.join("junk.css"), "body { color: red }").unwrap();
        std::fs::write(dir.join("notes.txt"), ":root { --accent: #ffffff; }").unwrap();
        std::fs::write(dir.join("huge.css"), " ".repeat(MAX_FILE_BYTES as usize + 1)).unwrap();
        let themes = load_folder(&dir, &base());
        let names: Vec<_> = themes.iter().map(|(name, _, _)| name.as_str()).collect();
        assert_eq!(names, ["Deep Sea", "ocean"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

}
