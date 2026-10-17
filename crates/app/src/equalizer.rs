//! The graphic equalizer (issue #7): one set of 18 band gains, edited from
//! two places -- the Equalizer tab in Preferences (foobar2000's "simple"
//! equalizer, with `.feq` presets; also Playback > Equalizer) and the Vintage Equalizer
//! panel, a hardware look-alike whose vacuum fluorescent display is drawn
//! from here. The filtering itself is `engine::EqualizerSettings`' job.

use crate::settings::Settings;
use crate::store::Store;
use engine::{EngineCommand, EqualizerSettings, PlaybackEngine, EQ_BAND_COUNT, EQ_MAX_DB};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::cell::{Cell, RefCell};
use std::fmt::Write as _;
use std::rc::Rc;
use ui::{MainWindow, SettingsWindow, Vfd};

pub type Gains = [i32; EQ_BAND_COUNT];

/// Band names, in the order of `engine::EQ_BANDS_HZ`.
pub const BAND_LABELS: [&str; EQ_BAND_COUNT] = [
    "55 Hz", "77 Hz", "110 Hz", "156 Hz", "220 Hz", "311 Hz", "440 Hz", "622 Hz", "880 Hz",
    "1.2 kHz", "1.8 kHz", "2.5 kHz", "3.5 kHz", "5 kHz", "7 kHz", "10 kHz", "14 kHz", "20 kHz",
];

/// The shorter names printed under the vintage panel's keys.
const SHORT_LABELS: [&str; EQ_BAND_COUNT] = [
    "55", "77", "110", "156", "220", "311", "440", "622", "880", "1.2k", "1.8k", "2.5k", "3.5k",
    "5k", "7k", "10k", "14k", "20k",
];

/// Built-in presets, offered on the Equalizer tab in Preferences.
pub const PRESETS: [(&str, Gains); 6] = [
    ("Flat", [0; EQ_BAND_COUNT]),
    ("Bass Boost", [7, 7, 6, 5, 3, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
    ("Treble Boost", [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 6, 6]),
    ("Loudness", [6, 5, 4, 2, 1, 0, 0, -1, -1, -1, 0, 0, 1, 2, 3, 4, 4, 3]),
    ("Vocal", [-3, -3, -2, -1, 0, 1, 2, 3, 4, 4, 3, 2, 1, 0, -1, -1, -2, -2]),
    // A nod to a certain pair of bass-shaking headphones.
    ("Skullcrusher", [20, 20, 20, 20, 20, 20, 20, 20, 20, 20, 0, 0, 0, 0, 0, 0, 0, 0]),
];

/// Curves the vintage display recognises, and what it says about them.
const EASTER_EGGS: [(Gains, &str); 2] = [
    (
        [6, 9, 4, 2, 0, -6, -9, -4, -2, 0, 6, 9, 4, 2, 0, -4, -2, 0],
        "VERY MATURE OF YOU",
    ),
    (PRESETS[5].1, "SKULLCRUSHER"),
];

pub fn easter_egg(gains: &Gains) -> Option<&'static str> {
    EASTER_EGGS
        .iter()
        .find(|(egg, _)| egg == gains)
        .map(|(_, message)| *message)
}

/// Reads a foobar2000 equalizer preset: one gain in dB per line, 18 lines,
/// lowest band first. Decimals are rounded and values clamped to ±20.
pub fn parse_feq(text: &str) -> Result<Gains, String> {
    let values = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            line.parse::<f64>()
                .ok()
                .filter(|value| value.is_finite())
                .map(|value| (value.round() as i32).clamp(-EQ_MAX_DB, EQ_MAX_DB))
                .ok_or_else(|| format!("\u{201c}{line}\u{201d} is not a number"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    values
        .try_into()
        .map_err(|values: Vec<i32>| format!("expected {EQ_BAND_COUNT} bands, found {}", values.len()))
}

/// Writes gains as a foobar2000 `.feq` preset.
pub fn format_feq(gains: &Gains) -> String {
    gains.iter().map(|gain| format!("{gain}\n")).collect()
}

/// Shifts every band by the same amount so the highest sits at 0 dB:
/// keeps the curve's shape while taking boosts out of clipping range.
pub fn auto_level(gains: &Gains) -> Gains {
    let top = gains.iter().copied().max().unwrap_or(0);
    gains.map(|gain| (gain - top).clamp(-EQ_MAX_DB, EQ_MAX_DB))
}

fn format_gain(gain: i32) -> String {
    if gain > 0 {
        format!("+{gain} dB")
    } else {
        format!("{gain} dB")
    }
}

// --- The vacuum fluorescent display ---------------------------------------

/// Rows of the band display: 10 segments of 2 dB above the 0 dB row and 10
/// below. Each band is 10 units wide and each row 4 units tall, matching
/// the 180 x 84 viewbox in `vintage_equalizer.slint`.
const DISPLAY_ROWS: i32 = 21;

fn push_rect(path: &mut String, x: f32, y: f32, width: f32, height: f32) {
    let _ = write!(path, "M{x} {y}h{width}v{height}h-{width}z");
}

/// The band display as three paths: lit, half-lit and unlit segments. A
/// band lights the segments from the 0 dB row out to its gain; an odd gain
/// lights the last one at half brightness.
pub fn band_dots(gains: &Gains) -> [String; 3] {
    let [mut lit, mut half, mut dim] = [String::new(), String::new(), String::new()];
    for (band, &gain) in gains.iter().enumerate() {
        let x = band as f32 * 10.0 + 2.0;
        for row in 0..DISPLAY_ROWS {
            let level = DISPLAY_ROWS / 2 - row; // +10 at the top, -10 at the bottom
            let needed = 2 * level.abs();
            let target = if level == 0 || (level.signum() == gain.signum() && gain.abs() >= needed) {
                &mut lit
            } else if level.signum() == gain.signum() && gain.abs() == needed - 1 {
                &mut half
            } else {
                &mut dim
            };
            push_rect(target, x, row as f32 * 4.0 + 0.8, 6.0, 2.4);
        }
    }
    [lit, half, dim]
}

/// 5x7 dot-matrix glyphs, top row first; anything missing shows as `?`.
const FONT: [(char, [&str; 7]); 44] = [
    ('A', [".###.", "#...#", "#...#", "#####", "#...#", "#...#", "#...#"]),
    ('B', ["####.", "#...#", "#...#", "####.", "#...#", "#...#", "####."]),
    ('C', [".###.", "#...#", "#....", "#....", "#....", "#...#", ".###."]),
    ('D', ["###..", "#..#.", "#...#", "#...#", "#...#", "#..#.", "###.."]),
    ('E', ["#####", "#....", "#....", "####.", "#....", "#....", "#####"]),
    ('F', ["#####", "#....", "#....", "####.", "#....", "#....", "#...."]),
    ('G', [".###.", "#...#", "#....", "#.###", "#...#", "#...#", ".####"]),
    ('H', ["#...#", "#...#", "#...#", "#####", "#...#", "#...#", "#...#"]),
    ('I', [".###.", "..#..", "..#..", "..#..", "..#..", "..#..", ".###."]),
    ('J', ["..###", "...#.", "...#.", "...#.", "...#.", "#..#.", ".##.."]),
    ('K', ["#...#", "#..#.", "#.#..", "##...", "#.#..", "#..#.", "#...#"]),
    ('L', ["#....", "#....", "#....", "#....", "#....", "#....", "#####"]),
    ('M', ["#...#", "##.##", "#.#.#", "#.#.#", "#...#", "#...#", "#...#"]),
    ('N', ["#...#", "#...#", "##..#", "#.#.#", "#..##", "#...#", "#...#"]),
    ('O', [".###.", "#...#", "#...#", "#...#", "#...#", "#...#", ".###."]),
    ('P', ["####.", "#...#", "#...#", "####.", "#....", "#....", "#...."]),
    ('Q', [".###.", "#...#", "#...#", "#...#", "#.#.#", "#..#.", ".##.#"]),
    ('R', ["####.", "#...#", "#...#", "####.", "#.#..", "#..#.", "#...#"]),
    ('S', [".####", "#....", "#....", ".###.", "....#", "....#", "####."]),
    ('T', ["#####", "..#..", "..#..", "..#..", "..#..", "..#..", "..#.."]),
    ('U', ["#...#", "#...#", "#...#", "#...#", "#...#", "#...#", ".###."]),
    ('V', ["#...#", "#...#", "#...#", "#...#", "#...#", ".#.#.", "..#.."]),
    ('W', ["#...#", "#...#", "#...#", "#.#.#", "#.#.#", "#.#.#", ".#.#."]),
    ('X', ["#...#", "#...#", ".#.#.", "..#..", ".#.#.", "#...#", "#...#"]),
    ('Y', ["#...#", "#...#", ".#.#.", "..#..", "..#..", "..#..", "..#.."]),
    ('Z', ["#####", "....#", "...#.", "..#..", ".#...", "#....", "#####"]),
    ('0', [".###.", "#...#", "#..##", "#.#.#", "##..#", "#...#", ".###."]),
    ('1', ["..#..", ".##..", "..#..", "..#..", "..#..", "..#..", ".###."]),
    ('2', [".###.", "#...#", "....#", "...#.", "..#..", ".#...", "#####"]),
    ('3', ["#####", "...#.", "..#..", "...#.", "....#", "#...#", ".###."]),
    ('4', ["...#.", "..##.", ".#.#.", "#..#.", "#####", "...#.", "...#."]),
    ('5', ["#####", "#....", "####.", "....#", "....#", "#...#", ".###."]),
    ('6', ["..##.", ".#...", "#....", "####.", "#...#", "#...#", ".###."]),
    ('7', ["#####", "....#", "...#.", "..#..", ".#...", ".#...", ".#..."]),
    ('8', [".###.", "#...#", "#...#", ".###.", "#...#", "#...#", ".###."]),
    ('9', [".###.", "#...#", "#...#", ".####", "....#", "...#.", ".##.."]),
    (' ', [".....", ".....", ".....", ".....", ".....", ".....", "....."]),
    ('-', [".....", ".....", ".....", ".###.", ".....", ".....", "....."]),
    ('+', [".....", "..#..", "..#..", "#####", "..#..", "..#..", "....."]),
    ('.', [".....", ".....", ".....", ".....", ".....", ".....", "..#.."]),
    ('!', ["..#..", "..#..", "..#..", "..#..", "..#..", ".....", "..#.."]),
    ('?', [".###.", "#...#", "....#", "...#.", "..#..", ".....", "..#.."]),
    ('\'', ["..#..", "..#..", ".#...", ".....", ".....", ".....", "....."]),
    (',', [".....", ".....", ".....", ".....", ".....", "..#..", ".#..."]),
];

fn glyph(c: char) -> &'static [&'static str; 7] {
    let c = c.to_ascii_uppercase();
    let find = |c| FONT.iter().find(|(f, _)| *f == c).map(|(_, rows)| rows);
    find(c).or_else(|| find('?')).expect("the font has '?'")
}

/// Each character is 5 dots wide plus a 1-dot gap; the text sits on rows
/// 1-7 of a 9-row strip.
const GLYPH_PITCH: usize = 6;

fn push_dot(path: &mut String, column: usize, row: usize) {
    push_rect(path, column as f32 + 0.12, row as f32 + 0.12, 0.76, 0.76);
}

/// `text` in dot-matrix form, and how many columns wide it is.
pub fn text_dots(text: &str) -> (String, usize) {
    let mut path = String::new();
    let mut columns = 0;
    for c in text.chars() {
        for (row, bits) in glyph(c).iter().enumerate() {
            for (column, bit) in bits.chars().enumerate() {
                if bit == '#' {
                    push_dot(&mut path, columns + column, row + 1);
                }
            }
        }
        columns += GLYPH_PITCH;
    }
    (path, columns)
}

/// The unlit dot grid behind scrolling text -- unbroken, since the text
/// moves a dot at a time rather than a character at a time.
pub fn grid_dots(columns: usize) -> String {
    let mut path = String::new();
    for row in 1..=7 {
        for column in 0..columns {
            push_dot(&mut path, column, row);
        }
    }
    path
}

/// Columns of unlit grid generated -- wider than any display gets.
const GRID_COLUMNS: usize = 240;

/// The Equalizer tab's index in the Preferences `TabWidget`.
const EQUALIZER_TAB: i32 = 3;

// --- Wiring ----------------------------------------------------------------

/// Keeps the engine, the saved settings, the Preferences tab and the
/// vintage panel showing the same equalizer.
pub struct Equalizer {
    main: slint::Weak<MainWindow>,
    window: SettingsWindow,
    engine: Rc<PlaybackEngine>,
    store: Rc<Store>,
    settings: Rc<RefCell<Settings>>,
    gains_model: Rc<VecModel<i32>>,
    /// The band last changed on the vintage panel, for its readout.
    touched: Cell<Option<usize>>,
    /// The message currently scrolling on the display, so an unchanged one
    /// isn't restarted.
    message: RefCell<Option<&'static str>>,
}

impl Equalizer {
    pub fn wire(
        main: &MainWindow,
        window: &SettingsWindow,
        engine: Rc<PlaybackEngine>,
        store: Rc<Store>,
        settings: Rc<RefCell<Settings>>,
    ) -> Rc<Self> {
        let current = settings.borrow().equalizer;
        let eq = Rc::new(Self {
            main: main.as_weak(),
            window: window.clone_strong(),
            engine,
            store,
            settings,
            gains_model: Rc::new(VecModel::from(current.gains_db.to_vec())),
            touched: Cell::new(None),
            message: RefCell::new(None),
        });
        eq.engine.send(EngineCommand::SetEqualizer(current));

        let labels = |labels: &[&str]| {
            ModelRc::from(Rc::new(VecModel::from(
                labels.iter().map(|label| SharedString::from(*label)).collect::<Vec<_>>(),
            )))
        };
        main.set_eq_band_labels(labels(&SHORT_LABELS));
        main.set_eq_grid_dots(grid_dots(GRID_COLUMNS).into());
        main.set_eq_grid_columns(GRID_COLUMNS as i32);
        window.set_eq_band_labels(labels(&BAND_LABELS));
        window.set_eq_gains(ModelRc::from(eq.gains_model.clone()));
        window.set_eq_max_db(EQ_MAX_DB);
        window.set_eq_vintage(eq.settings.borrow().equalizer_vintage_look);
        let saved_color = eq.settings.borrow().equalizer_vfd_color.clone();
        eq.show_vfd_color(&saved_color);
        {
            let weak = Rc::downgrade(&eq);
            window.on_eq_vfd_color_chosen(move |color| {
                if let Some(eq) = weak.upgrade() {
                    eq.set_vfd_color(&crate::theme::color_to_hex(color));
                }
            });
        }
        {
            let weak = Rc::downgrade(&eq);
            window.on_eq_vfd_color_edited(move |text| {
                let Some(eq) = weak.upgrade() else { return };
                let text = text.trim();
                let hex = if text.starts_with('#') { text.to_string() } else { format!("#{text}") };
                if crate::theme::parse_hex_color(&hex).is_some() {
                    eq.set_vfd_color(&hex.to_lowercase());
                } else {
                    eq.window.set_eq_status(format!("\u{201c}{text}\u{201d} isn\u{2019}t a color. Use hex, like #6fd6ff.").into());
                    let current = eq.settings.borrow().equalizer_vfd_color.clone();
                    eq.show_vfd_color(&current);
                }
            });
        }
        let mut names = vec!["Custom"];
        names.extend(PRESETS.iter().map(|(name, _)| *name));
        window.set_eq_preset_names(labels(&names));
        eq.refresh();

        // Main window: menu and vintage panel.
        let weak = Rc::downgrade(&eq);
        let with = move |action: fn(&Equalizer)| {
            let weak = weak.clone();
            move || {
                if let Some(eq) = weak.upgrade() {
                    action(&eq);
                }
            }
        };
        main.on_eq_settings_clicked(with(|eq| eq.open_window()));
        main.on_eq_enabled_toggled(with(|eq| {
            let current = eq.current();
            eq.set(EqualizerSettings { enabled: !current.enabled, ..current });
        }));
        main.on_eq_flat_clicked(with(|eq| {
            eq.touched.set(None);
            eq.set_gains([0; EQ_BAND_COUNT]);
        }));
        {
            let weak = Rc::downgrade(&eq);
            main.on_eq_step(move |band, delta| {
                let Some(eq) = weak.upgrade() else { return };
                let Some(band) = usize::try_from(band).ok().filter(|&b| b < EQ_BAND_COUNT) else {
                    return;
                };
                let mut gains = eq.current().gains_db;
                gains[band] = (gains[band] + delta).clamp(-EQ_MAX_DB, EQ_MAX_DB);
                eq.touched.set(Some(band));
                eq.set_gains(gains);
            });
        }

        // The Equalizer tab in Preferences.
        {
            let weak = Rc::downgrade(&eq);
            window.on_eq_band_changed(move |band, gain| {
                let Some(eq) = weak.upgrade() else { return };
                let Some(band) = usize::try_from(band).ok().filter(|&b| b < EQ_BAND_COUNT) else {
                    return;
                };
                let mut gains = eq.current().gains_db;
                if gains[band] != gain {
                    gains[band] = gain.clamp(-EQ_MAX_DB, EQ_MAX_DB);
                    eq.touched.set(Some(band));
                    eq.set_gains(gains);
                }
            });
        }
        {
            let weak = Rc::downgrade(&eq);
            window.on_eq_enabled_toggled(move |enabled| {
                if let Some(eq) = weak.upgrade() {
                    let current = eq.current();
                    eq.set(EqualizerSettings { enabled, ..current });
                }
            });
        }
        {
            let weak = Rc::downgrade(&eq);
            window.on_eq_vintage_toggled(move |on| {
                let Some(eq) = weak.upgrade() else { return };
                eq.settings.borrow_mut().equalizer_vintage_look = on;
                eq.settings.borrow().save(&eq.store);
                eq.window.set_eq_vintage(on);
            });
        }
        {
            let weak = Rc::downgrade(&eq);
            window.on_eq_preset_chosen(move |index| {
                let Some(eq) = weak.upgrade() else { return };
                // Index 0 is "Custom": nothing to load.
                if let Some((name, gains)) = usize::try_from(index - 1).ok().and_then(|i| PRESETS.get(i)) {
                    eq.touched.set(None);
                    eq.set_gains(*gains);
                    eq.window.set_eq_status(format!("Loaded \u{201c}{name}\u{201d}.").into());
                }
            });
        }
        window.on_eq_zero_all_clicked(with(|eq| {
            eq.touched.set(None);
            eq.set_gains([0; EQ_BAND_COUNT]);
        }));
        window.on_eq_auto_level_clicked(with(|eq| eq.set_gains(auto_level(&eq.current().gains_db))));
        window.on_eq_load_preset_clicked(with(|eq| eq.load_preset()));
        window.on_eq_save_preset_clicked(with(|eq| eq.save_preset()));
        eq
    }

    fn current(&self) -> EqualizerSettings {
        self.settings.borrow().equalizer
    }

    fn set_gains(&self, gains_db: Gains) {
        let current = self.current();
        self.set(EqualizerSettings { gains_db, ..current });
    }

    /// Applies, saves and shows `new`.
    fn set(&self, new: EqualizerSettings) {
        if new != self.current() {
            self.settings.borrow_mut().equalizer = new;
            self.settings.borrow().save(&self.store);
            self.engine.send(EngineCommand::SetEqualizer(new));
        }
        self.refresh();
    }

    /// Saves and shows the vintage display's colour.
    fn set_vfd_color(&self, hex: &str) {
        self.settings.borrow_mut().equalizer_vfd_color = hex.to_string();
        self.settings.borrow().save(&self.store);
        self.window.set_eq_status(SharedString::new());
        self.show_vfd_color(hex);
    }

    /// Paints `hex` into both windows' display (their `Vfd` globals are
    /// separate) and the picker.
    fn show_vfd_color(&self, hex: &str) {
        let color = crate::theme::parse_hex_color(hex)
            .or_else(|| crate::theme::parse_hex_color(crate::settings::DEFAULT_VFD_COLOR))
            .expect("the default colour parses");
        self.window.global::<Vfd>().set_glow(color);
        self.window.set_eq_vfd_color(color);
        self.window.set_eq_vfd_color_hex(hex.into());
        if let Some(main) = self.main.upgrade() {
            main.global::<Vfd>().set_glow(color);
        }
    }

    /// Playback > Equalizer > Equalizer Settings…: Preferences, on the
    /// Equalizer tab.
    fn open_window(&self) {
        self.window.set_eq_status(SharedString::new());
        self.window.set_current_tab(EQUALIZER_TAB);
        let _ = self.window.show();
    }

    fn refresh(&self) {
        let current = self.current();
        let gains = current.gains_db;
        for (i, &gain) in gains.iter().enumerate() {
            if self.gains_model.row_data(i) != Some(gain) {
                self.gains_model.set_row_data(i, gain);
            }
        }
        self.window.set_eq_enabled(current.enabled);
        self.window.set_eq_preset_index(
            PRESETS
                .iter()
                .position(|(_, preset)| *preset == gains)
                .map_or(0, |i| i as i32 + 1),
        );

        let [lit, half, dim] = band_dots(&gains);
        self.window.set_eq_lit_dots(lit.as_str().into());
        self.window.set_eq_half_dots(half.as_str().into());
        self.window.set_eq_dim_dots(dim.as_str().into());

        let Some(main) = self.main.upgrade() else { return };
        main.set_eq_enabled(current.enabled);
        main.set_eq_lit_dots(lit.into());
        main.set_eq_half_dots(half.into());
        main.set_eq_dim_dots(dim.into());
        let (band, gain) = match self.touched.get() {
            Some(band) => (BAND_LABELS[band].to_string(), format_gain(gains[band])),
            None if gains == [0; EQ_BAND_COUNT] => ("FLAT".to_string(), String::new()),
            None => (String::new(), String::new()),
        };
        main.set_eq_readout_band(band.into());
        main.set_eq_readout_gain(gain.into());

        let message = easter_egg(&gains);
        if *self.message.borrow() != message {
            *self.message.borrow_mut() = message;
            let (dots, columns) = message.map(text_dots).unwrap_or_default();
            main.set_eq_message_columns(columns as i32);
            main.set_eq_message_dots(dots.into());
        }
    }

    fn load_preset(&self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Load equalizer preset")
            .add_filter("Equalizer preset", &["feq"])
            .pick_file()
        else {
            return;
        };
        let result = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|text| parse_feq(&text));
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        match result {
            Ok(gains) => {
                self.touched.set(None);
                self.set_gains(gains);
                self.window.set_eq_status(format!("Loaded {name}.").into());
            }
            Err(e) => self.window.set_eq_status(format!("Couldn\u{2019}t load {name}: {e}").into()),
        }
    }

    fn save_preset(&self) {
        let Some(mut path) = rfd::FileDialog::new()
            .set_title("Save equalizer preset")
            .add_filter("Equalizer preset", &["feq"])
            .set_file_name("Preset.feq")
            .save_file()
        else {
            return;
        };
        if path.extension().is_none() {
            path.set_extension("feq");
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let status = match std::fs::write(&path, format_feq(&self.current().gains_db)) {
            Ok(()) => format!("Saved {name}."),
            Err(e) => format!("Couldn\u{2019}t save {name}: {e}"),
        };
        self.window.set_eq_status(status.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feq_presets_round_trip_and_tolerate_foobar_quirks() {
        let gains = PRESETS[4].1;
        assert_eq!(parse_feq(&format_feq(&gains)), Ok(gains));
        // Windows line endings, blank lines, decimals and out-of-range
        // values.
        let text = "1\r\n-2\r\n\r\n3.6\n25\n-40\n0\n0\n0\n0\n0\n0\n0\n0\n0\n0\n0\n0\n0\n";
        let parsed = parse_feq(text).unwrap();
        assert_eq!(parsed[..5], [1, -2, 4, 20, -20]);
        assert!(parse_feq("1\n2\n3").unwrap_err().contains("expected 18 bands, found 3"));
        assert!(parse_feq(&"x\n".repeat(18)).unwrap_err().contains("not a number"));
    }

    #[test]
    fn auto_level_puts_the_highest_band_at_zero() {
        let leveled = auto_level(&PRESETS[5].1);
        assert_eq!(leveled.iter().max(), Some(&0));
        assert_eq!(leveled[0], 0);
        assert_eq!(leveled[17], -20);
        assert_eq!(auto_level(&[0; EQ_BAND_COUNT]), [0; EQ_BAND_COUNT]);
    }

    #[test]
    fn the_display_knows_its_easter_eggs() {
        let mature = [6, 9, 4, 2, 0, -6, -9, -4, -2, 0, 6, 9, 4, 2, 0, -4, -2, 0];
        assert_eq!(easter_egg(&mature), Some("VERY MATURE OF YOU"));
        assert_eq!(easter_egg(&PRESETS[5].1), Some("SKULLCRUSHER"));
        assert_eq!(easter_egg(&[0; EQ_BAND_COUNT]), None);
    }

    #[test]
    fn band_display_lights_from_zero_out_to_the_gain() {
        let mut gains = [0; EQ_BAND_COUNT];
        gains[0] = 5; // two full segments and a half one above the 0 row
        gains[1] = -20; // every segment below
        let [lit, half, dim] = band_dots(&gains);
        let count = |path: &str| path.matches('M').count();
        // 18 zero rows + band 0's 2 segments + band 1's 10.
        assert_eq!(count(&lit), 18 + 2 + 10);
        assert_eq!(count(&half), 1);
        assert_eq!(count(&lit) + count(&half) + count(&dim), 18 * 21);
    }

    #[test]
    fn the_font_covers_every_message_and_its_glyphs_are_well_formed() {
        for (c, rows) in FONT {
            assert!(rows.iter().all(|row| row.len() == 5), "{c:?} rows must be 5 dots");
        }
        for (_, message) in EASTER_EGGS {
            assert!(message.chars().all(|c| FONT.iter().any(|(f, _)| *f == c)), "{message}");
        }
        let (dots, columns) = text_dots("Hi");
        assert_eq!(columns, 12);
        // H has 17 dots, I 11.
        assert_eq!(dots.matches('M').count(), 17 + 11);
    }
}
