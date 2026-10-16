use crate::layout::Layout;
use crate::playlist_rows::PlaylistGrouping;
use crate::store::Store;
use engine::{PlaybackMode, SpectrumChannelMode, SpectrumConfig};
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
    /// How often the spectrum/seekbar redraw while playing: 60 or 30 (caps
    /// the redraw rate; see `wiring::poll_interval`).
    pub visualization_fps: u32,
    /// Off by default: it does nothing until the user supplies their own
    /// Discord Application client ID (see `discord_client_id`), which only
    /// they can create -- see README for the one-time setup steps.
    pub discord_presence_enabled: bool,
    pub discord_client_id: String,
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
            visualization_fps: 60,
            discord_presence_enabled: false,
            discord_client_id: String::new(),
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

/// Only 30 and 60 are offered; anything else (hand-edited or from a future
/// build) falls back to 60.
pub fn normalize_fps(fps: u32) -> u32 {
    if fps == 30 {
        30
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
        if let Some(v) = get("playback_mode") {
            settings.playback_mode = playback_mode_from_str(&v);
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
            ("layout", self.layout.to_json()),
        ]);
    }
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
