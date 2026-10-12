use engine::{SpectrumChannelMode, SpectrumConfig};
use std::path::PathBuf;

/// Persisted, user-facing toggles and preferences. Stored as a tiny manual
/// `key=value` text file rather than pulling in a serialization crate.
#[derive(Debug, Clone)]
pub struct Settings {
    pub show_waveform: bool,
    pub show_spectrum: bool,
    pub theme_id: String,
    pub spectrum: SpectrumConfig,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            show_waveform: true,
            show_spectrum: true,
            theme_id: "dark".to_string(),
            spectrum: SpectrumConfig::default(),
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
    pub fn load() -> Self {
        let mut settings = Self::default();
        let Ok(contents) = std::fs::read_to_string(settings_path()) else {
            return settings;
        };
        for line in contents.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "show_waveform" => settings.show_waveform = value == "true",
                "show_spectrum" => settings.show_spectrum = value == "true",
                "theme_id" => settings.theme_id = value.to_string(),
                "spectrum_channels" => {
                    settings.spectrum.channel_mode = channel_mode_from_str(value)
                }
                "spectrum_min_db" => {
                    if let Ok(v) = value.parse() {
                        settings.spectrum.min_db = v;
                    }
                }
                "spectrum_max_db" => {
                    if let Ok(v) = value.parse() {
                        settings.spectrum.max_db = v;
                    }
                }
                "spectrum_fft_size" => {
                    if let Ok(v) = value.parse() {
                        settings.spectrum.fft_size = v;
                    }
                }
                "spectrum_peaks_enabled" => settings.spectrum.peaks_enabled = value == "true",
                "spectrum_peak_hold_ms" => {
                    if let Ok(v) = value.parse() {
                        settings.spectrum.peak_hold_ms = v;
                    }
                }
                "spectrum_gravity" => {
                    if let Ok(v) = value.parse() {
                        settings.spectrum.gravity_db_per_s2 = v;
                    }
                }
                "spectrum_interval_ms" => {
                    if let Ok(v) = value.parse() {
                        settings.spectrum.interval_ms = v;
                    }
                }
                _ => {}
            }
        }
        settings
    }

    pub fn save(&self) {
        let path = settings_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let contents = format!(
            "show_waveform={}\n\
             show_spectrum={}\n\
             theme_id={}\n\
             spectrum_channels={}\n\
             spectrum_min_db={}\n\
             spectrum_max_db={}\n\
             spectrum_fft_size={}\n\
             spectrum_peaks_enabled={}\n\
             spectrum_peak_hold_ms={}\n\
             spectrum_gravity={}\n\
             spectrum_interval_ms={}\n",
            self.show_waveform,
            self.show_spectrum,
            self.theme_id,
            channel_mode_to_str(self.spectrum.channel_mode),
            self.spectrum.min_db,
            self.spectrum.max_db,
            self.spectrum.fft_size,
            self.spectrum.peaks_enabled,
            self.spectrum.peak_hold_ms,
            self.spectrum.gravity_db_per_s2,
            self.spectrum.interval_ms,
        );
        if let Err(e) = std::fs::write(&path, contents) {
            tracing::warn!("failed to save settings to {}: {e}", path.display());
        }
    }
}

fn settings_path() -> PathBuf {
    config_dir().join("settings.txt")
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
