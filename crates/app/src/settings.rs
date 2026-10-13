use crate::store::Store;
use engine::{SpectrumChannelMode, SpectrumConfig};
use std::path::PathBuf;

/// Persisted, user-facing toggles and preferences. Backed by the `settings`
/// table in [`Store`] -- a flat key/value table, so adding a new field here
/// never needs a schema migration.
#[derive(Debug, Clone)]
pub struct Settings {
    pub show_waveform: bool,
    pub show_spectrum: bool,
    pub theme_id: String,
    pub spectrum: SpectrumConfig,
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
            theme_id: "dark".to_string(),
            spectrum: SpectrumConfig::default(),
            discord_presence_enabled: false,
            discord_client_id: String::new(),
        }
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
        if let Some(v) = get("discord_presence_enabled") {
            settings.discord_presence_enabled = v == "true";
        }
        if let Some(v) = get("discord_client_id") {
            settings.discord_client_id = v;
        }
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
                "discord_presence_enabled",
                self.discord_presence_enabled.to_string(),
            ),
            ("discord_client_id", self.discord_client_id.clone()),
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
