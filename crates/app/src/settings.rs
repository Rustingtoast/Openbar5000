use crate::artist_info::{LookupOptions, WIKIPEDIA_LANGUAGES};
use crate::layout::Layout;
use crate::playlist_rows::{PlaylistGrouping, SortMode};
use crate::store::Store;
use engine::{EqualizerSettings, PlaybackMode, SpectrumChannelMode, SpectrumConfig, EQ_MAX_DB};
use std::path::PathBuf;

/// Persisted, user-facing toggles and preferences. Backed by the `settings`
/// table in [`Store`] -- a flat key/value table, so adding a new field here
/// never needs a schema migration.
#[derive(Debug, Clone)]
pub struct Settings {
    /// Pre-panels visibility toggles: now only seed the default `layout`
    /// for settings saved before layouts existed.
    pub show_waveform: bool,
    pub show_spectrum: bool,
    /// The main window's panel arrangement (issue #22).
    pub layout: Layout,
    pub theme_id: String,
    pub spectrum: SpectrumConfig,
    pub playback_mode: PlaybackMode,
    pub playlist_grouping: PlaylistGrouping,
    /// Smart grouping (Preferences > Appearance): albums matched loosely and
    /// their tracks gathered together (see `playlist_rows::smart_order`).
    pub playlist_smart_grouping: bool,
    /// The Sorting menu's kept sort order.
    pub playlist_sort: SortMode,
    /// Plain song list instead of album groups (Preferences > Appearance).
    pub playlist_list_view: bool,
    /// How often the spectrum/seekbar redraw while playing: 60 or 30 (caps
    /// the redraw rate; see `wiring::poll_interval`).
    pub visualization_fps: u32,
    /// Off by default: it does nothing until the user supplies their own
    /// Discord Application client ID (see `discord_client_id`), which only
    /// they can create -- see README for the one-time setup steps.
    pub discord_presence_enabled: bool,
    pub discord_client_id: String,
    /// Whether an artist lookup also fetches a biography and photo from
    /// Wikipedia (MusicBrainz is always used to identify the artist).
    pub artist_wikipedia: bool,
    /// Wikipedia edition to read (falls back to English).
    pub artist_wikipedia_language: String,
    /// Playback volume, 0.0 to 1.0.
    pub volume: f32,
    /// The chosen output device's id; `None` follows the system default.
    pub output_device: Option<String>,
    /// Its name, to show while it isn't connected.
    pub output_device_name: String,
    /// The graphic equalizer (issue #7).
    pub equalizer: EqualizerSettings,
    /// Shows the Equalizer tab in Preferences as a hardware faceplate.
    pub equalizer_vintage_look: bool,
    /// The media library's music folders (see `media_library`).
    pub library_folders: Vec<PathBuf>,
    /// How the LIB panel groups the library (`media_library::Grouping` id).
    pub library_grouping: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            show_waveform: true,
            show_spectrum: true,
            layout: Layout::default_with(true, true),
            theme_id: "dark".to_string(),
            spectrum: SpectrumConfig::default(),
            playback_mode: PlaybackMode::Normal,
            playlist_grouping: PlaylistGrouping::default(),
            playlist_list_view: false,
            playlist_sort: SortMode::Manual,
            playlist_smart_grouping: true,
            visualization_fps: 60,
            discord_presence_enabled: false,
            discord_client_id: String::new(),
            artist_wikipedia: true,
            artist_wikipedia_language: "en".to_string(),
            volume: 1.0,
            output_device: None,
            output_device_name: String::new(),
            equalizer: EqualizerSettings::default(),
            equalizer_vintage_look: false,
            library_folders: Vec::new(),
            library_grouping: "artist-album".to_string(),
        }
    }
}

fn playback_mode_to_str(mode: PlaybackMode) -> &'static str {
    match mode {
        PlaybackMode::Normal => "normal",
        PlaybackMode::RepeatAll => "repeat_all",
        PlaybackMode::RepeatOne => "repeat_one",
        PlaybackMode::Shuffle => "shuffle",
    }
}

fn playback_mode_from_str(value: &str) -> PlaybackMode {
    match value {
        "repeat_all" => PlaybackMode::RepeatAll,
        // "loop" is the pre-3-state-cycle value this field used to store;
        // kept so settings saved by an older build still load sensibly
        // instead of silently resetting to Normal.
        "repeat_one" | "loop" => PlaybackMode::RepeatOne,
        "shuffle" => PlaybackMode::Shuffle,
        _ => PlaybackMode::Normal,
    }
}

/// Notes the spectrum's range can be set to: C0 (MIDI 12) to G9 (127).
pub const LOWEST_NOTE: u8 = 12;
pub const HIGHEST_NOTE: u8 = 127;
/// Reference pitches offered for A4.
pub const A4_MIN_HZ: f32 = 400.0;
pub const A4_MAX_HZ: f32 = 480.0;

/// At least two notes, within what the settings offer.
pub fn valid_note_range(first: u8, last: u8) -> bool {
    LOWEST_NOTE <= first && first < last && last <= HIGHEST_NOTE
}

/// "C4", "F#0", ... for MIDI note `note`.
pub fn note_name(note: u8) -> String {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    format!("{}{}", NAMES[note as usize % 12], note as i32 / 12 - 1)
}

/// The inverse of [`note_name`].
pub fn note_from_name(name: &str) -> Option<u8> {
    (LOWEST_NOTE..=HIGHEST_NOTE).find(|&note| note_name(note) == name)
}

/// The visualization frame rates Preferences offers, in its dropdown's
/// order.
pub const FPS_CHOICES: [u32; 4] = [120, 60, 30, 15];

/// Anything not offered (hand-edited, or from another build) falls back to
/// 60.
pub fn normalize_fps(fps: u32) -> u32 {
    if FPS_CHOICES.contains(&fps) {
        fps
    } else {
        60
    }
}

fn channel_mode_to_str(mode: SpectrumChannelMode) -> &'static str {
    match mode {
        SpectrumChannelMode::All => "all",
        SpectrumChannelMode::Left => "left",
        SpectrumChannelMode::Right => "right",
    }
}

fn channel_mode_from_str(value: &str) -> SpectrumChannelMode {
    match value {
        "left" => SpectrumChannelMode::Left,
        "right" => SpectrumChannelMode::Right,
        _ => SpectrumChannelMode::All,
    }
}

impl Settings {
    /// The artist lookup options chosen in Preferences.
    pub fn artist_lookup(&self) -> LookupOptions {
        LookupOptions {
            wikipedia_language: self
                .artist_wikipedia
                .then(|| self.artist_wikipedia_language.clone()),
        }
    }

    pub fn load(store: &Store) -> Self {
        let mut settings = Self::default();
        let get = |key: &str| store.get_setting(key);

        if let Some(v) = get("show_waveform") {
            settings.show_waveform = v == "true";
        }
        if let Some(v) = get("show_spectrum") {
            settings.show_spectrum = v == "true";
        }
        if let Some(v) = get("theme_id") {
            settings.theme_id = v;
        }
        if let Some(v) = get("spectrum_channels") {
            settings.spectrum.channel_mode = channel_mode_from_str(&v);
        }
        if let Some(v) = get("spectrum_min_db").and_then(|v| v.parse().ok()) {
            settings.spectrum.min_db = v;
        }
        if let Some(v) = get("spectrum_max_db").and_then(|v| v.parse().ok()) {
            settings.spectrum.max_db = v;
        }
        if let Some(v) = get("spectrum_fft_size").and_then(|v| v.parse().ok()) {
            settings.spectrum.fft_size = v;
        }
        if let Some(v) = get("spectrum_peaks_enabled") {
            settings.spectrum.peaks_enabled = v == "true";
        }
        if let Some(v) = get("spectrum_peak_hold_ms").and_then(|v| v.parse().ok()) {
            settings.spectrum.peak_hold_ms = v;
        }
        if let Some(v) = get("spectrum_gravity").and_then(|v| v.parse().ok()) {
            settings.spectrum.gravity_db_per_s2 = v;
        }
        if let Some(v) = get("spectrum_interval_ms").and_then(|v| v.parse().ok()) {
            settings.spectrum.interval_ms = v;
        }
        let first = get("spectrum_first_note").and_then(|v| v.parse().ok());
        let last = get("spectrum_last_note").and_then(|v| v.parse().ok());
        if let (Some(first), Some(last)) = (first, last) {
            if valid_note_range(first, last) {
                settings.spectrum.first_note = first;
                settings.spectrum.last_note = last;
            }
        }
        if let Some(v) = get("spectrum_a4_hz").and_then(|v| v.parse::<f32>().ok()) {
            if (A4_MIN_HZ..=A4_MAX_HZ).contains(&v) {
                settings.spectrum.a4_hz = v;
            }
        }
        if let Some(v) = get("playback_mode") {
            settings.playback_mode = playback_mode_from_str(&v);
        }
        if let Some(v) = get("playlist_smart_grouping") {
            settings.playlist_smart_grouping = v == "true";
        }
        if let Some(v) = get("playlist_sort") {
            settings.playlist_sort = SortMode::from_str(&v);
        }
        if let Some(v) = get("playlist_view") {
            settings.playlist_list_view = v == "list";
        }
        if let Some(v) = get("playlist_grouping") {
            settings.playlist_grouping = PlaylistGrouping::from_str(&v);
        }
        if let Some(v) = get("visualization_fps").and_then(|v| v.parse().ok()) {
            settings.visualization_fps = normalize_fps(v);
        }
        if let Some(v) = get("discord_presence_enabled") {
            settings.discord_presence_enabled = v == "true";
        }
        if let Some(v) = get("discord_client_id") {
            settings.discord_client_id = v;
        }
        if let Some(v) = get("artist_wikipedia") {
            settings.artist_wikipedia = v == "true";
        }
        if let Some(v) = get("artist_wikipedia_language") {
            if WIKIPEDIA_LANGUAGES.iter().any(|(code, _)| *code == v) {
                settings.artist_wikipedia_language = v;
            }
        }
        if let Some(v) = get("volume").and_then(|v| v.parse::<f32>().ok()) {
            if v.is_finite() {
                settings.volume = v.clamp(0.0, 1.0);
            }
        }
        settings.output_device = get("output_device").filter(|id| !id.is_empty());
        if let Some(v) = get("output_device_name") {
            settings.output_device_name = v;
        }
        if let Some(v) = get("eq_enabled") {
            settings.equalizer.enabled = v == "true";
        }
        if let Some(folders) = get("library_folders")
            .and_then(|json| serde_json::from_str::<Vec<PathBuf>>(&json).ok())
        {
            settings.library_folders = folders;
        }
        if let Some(v) = get("library_grouping") {
            settings.library_grouping = v;
        }
        if let Some(v) = get("eq_vintage_look") {
            settings.equalizer_vintage_look = v == "true";
        }
        if let Some(gains) = get("eq_gains").and_then(|v| parse_eq_gains(&v)) {
            settings.equalizer.gains_db = gains;
        }
        settings.layout = get("layout")
            .and_then(|json| Layout::from_json(&json))
            .unwrap_or_else(|| Layout::default_with(settings.show_waveform, settings.show_spectrum));
        settings
    }

    pub fn save(&self, store: &Store) {
        store.set_settings(&[
            ("show_waveform", self.show_waveform.to_string()),
            ("show_spectrum", self.show_spectrum.to_string()),
            ("theme_id", self.theme_id.clone()),
            (
                "spectrum_channels",
                channel_mode_to_str(self.spectrum.channel_mode).to_string(),
            ),
            ("spectrum_min_db", self.spectrum.min_db.to_string()),
            ("spectrum_first_note", self.spectrum.first_note.to_string()),
            ("spectrum_last_note", self.spectrum.last_note.to_string()),
            ("spectrum_a4_hz", self.spectrum.a4_hz.to_string()),
            ("spectrum_max_db", self.spectrum.max_db.to_string()),
            ("spectrum_fft_size", self.spectrum.fft_size.to_string()),
            (
                "spectrum_peaks_enabled",
                self.spectrum.peaks_enabled.to_string(),
            ),
            (
                "spectrum_peak_hold_ms",
                self.spectrum.peak_hold_ms.to_string(),
            ),
            (
                "spectrum_gravity",
                self.spectrum.gravity_db_per_s2.to_string(),
            ),
            (
                "spectrum_interval_ms",
                self.spectrum.interval_ms.to_string(),
            ),
            (
                "playback_mode",
                playback_mode_to_str(self.playback_mode).to_string(),
            ),
            ("playlist_sort", self.playlist_sort.as_str().to_string()),
            ("playlist_smart_grouping", self.playlist_smart_grouping.to_string()),
            (
                "playlist_view",
                if self.playlist_list_view { "list" } else { "album" }.to_string(),
            ),
            (
                "playlist_grouping",
                self.playlist_grouping.as_str().to_string(),
            ),
            ("visualization_fps", self.visualization_fps.to_string()),
            (
                "discord_presence_enabled",
                self.discord_presence_enabled.to_string(),
            ),
            ("discord_client_id", self.discord_client_id.clone()),
            ("artist_wikipedia", self.artist_wikipedia.to_string()),
            ("artist_wikipedia_language", self.artist_wikipedia_language.clone()),
            ("volume", self.volume.to_string()),
            ("output_device", self.output_device.clone().unwrap_or_default()),
            ("output_device_name", self.output_device_name.clone()),
            ("eq_enabled", self.equalizer.enabled.to_string()),
            ("eq_vintage_look", self.equalizer_vintage_look.to_string()),
            (
                "library_folders",
                serde_json::to_string(&self.library_folders).unwrap_or_else(|_| "[]".into()),
            ),
            ("library_grouping", self.library_grouping.clone()),
            (
                "eq_gains",
                self.equalizer.gains_db.map(|gain| gain.to_string()).join(","),
            ),
            ("layout", self.layout.to_json()),
        ]);
    }
}

/// `eq_gains` as saved: comma-separated whole dB, lowest band first.
fn parse_eq_gains(value: &str) -> Option<[i32; engine::EQ_BAND_COUNT]> {
    value
        .split(',')
        .map(|gain| gain.trim().parse::<i32>().ok().map(|g| g.clamp(-EQ_MAX_DB, EQ_MAX_DB)))
        .collect::<Option<Vec<_>>>()?
        .try_into()
        .ok()
}

#[cfg(target_os = "macos")]
pub fn config_dir() -> PathBuf {
    home_dir().join("Library/Application Support/Openbar5000")
}

#[cfg(target_os = "windows")]
pub fn config_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join("Openbar5000")
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn config_dir() -> PathBuf {
    home_dir().join(".config/openbar5000")
}

#[cfg(any(target_os = "macos", all(unix, not(target_os = "macos"))))]
fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

#[cfg(test)]
mod fps_tests {
    use super::*;

    #[test]
    fn visualization_fps_round_trips_and_rejects_unknown_values() {
        let store = Store::open_in_memory().unwrap();
        let mut settings = Settings::load(&store);
        assert_eq!(settings.visualization_fps, 60);

        settings.visualization_fps = 30;
        settings.save(&store);
        assert_eq!(Settings::load(&store).visualization_fps, 30);

        for fps in [15, 120] {
            store.set_settings(&[("visualization_fps", fps.to_string())]);
            assert_eq!(Settings::load(&store).visualization_fps, fps);
        }
        store.set_settings(&[("visualization_fps", "144".to_string())]);
        assert_eq!(Settings::load(&store).visualization_fps, 60);
    }

    #[test]
    fn layout_migrates_old_visibility_toggles_then_round_trips() {
        use crate::layout::PanelKind;
        let store = Store::open_in_memory().unwrap();
        // Saved by a build from before panels: waveform hidden.
        store.set_settings(&[("show_waveform", "false".to_string())]);
        let mut settings = Settings::load(&store);
        assert!(!settings.layout.contains(PanelKind::Waveform));
        assert!(settings.layout.contains(PanelKind::Spectrum));

        settings.layout.replace(&[0], PanelKind::Spectrum);
        settings.save(&store);
        assert_eq!(Settings::load(&store).layout, settings.layout);

        // A corrupt saved layout falls back to the default.
        store.set_settings(&[("layout", "not json".to_string())]);
        assert!(Settings::load(&store).layout.contains(PanelKind::Playlist));
    }
}

#[cfg(test)]
mod spectrum_range_tests {
    use super::*;

    #[test]
    fn note_names_round_trip_and_ranges_are_validated() {
        assert_eq!(note_name(18), "F#0");
        assert_eq!(note_name(69), "A4");
        assert_eq!(note_name(117), "A8");
        assert_eq!(note_from_name("C4"), Some(60));
        assert_eq!(note_from_name("H4"), None);
        assert!(valid_note_range(18, 117));
        assert!(!valid_note_range(60, 60));
        assert!(!valid_note_range(5, 60));
    }

    #[test]
    fn note_range_and_a4_pitch_persist() {
        let store = Store::open_in_memory().unwrap();
        let mut settings = Settings::load(&store);
        settings.spectrum.first_note = 36;
        settings.spectrum.last_note = 96;
        settings.spectrum.a4_hz = 432.0;
        settings.save(&store);
        let loaded = Settings::load(&store).spectrum;
        assert_eq!((loaded.first_note, loaded.last_note, loaded.a4_hz), (36, 96, 432.0));

        // Nonsense from a hand-edited database falls back to the defaults.
        store.set_settings(&[
            ("spectrum_first_note", "90".to_string()),
            ("spectrum_last_note", "10".to_string()),
            ("spectrum_a4_hz", "9000".to_string()),
        ]);
        let loaded = Settings::load(&store).spectrum;
        let defaults = engine::SpectrumConfig::default();
        assert_eq!(
            (loaded.first_note, loaded.last_note, loaded.a4_hz),
            (defaults.first_note, defaults.last_note, defaults.a4_hz)
        );
    }
}
