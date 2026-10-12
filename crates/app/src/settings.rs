use std::path::PathBuf;

/// Persisted, user-facing toggles (currently just the visualization panels).
/// Stored as a tiny manual `key=value` text file rather than pulling in a
/// serialization crate for two booleans.
#[derive(Debug, Clone, Copy)]
pub struct Settings {
    pub show_waveform: bool,
    pub show_spectrum: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            show_waveform: true,
            show_spectrum: true,
        }
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
            let value = value.trim() == "true";
            match key.trim() {
                "show_waveform" => settings.show_waveform = value,
                "show_spectrum" => settings.show_spectrum = value,
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
            "show_waveform={}\nshow_spectrum={}\n",
            self.show_waveform, self.show_spectrum
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
fn config_dir() -> PathBuf {
    home_dir().join("Library/Application Support/Openbar5000")
}

#[cfg(target_os = "windows")]
fn config_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join("Openbar5000")
}

#[cfg(all(unix, not(target_os = "macos")))]
fn config_dir() -> PathBuf {
    home_dir().join(".config/openbar5000")
}

#[cfg(any(target_os = "macos", all(unix, not(target_os = "macos"))))]
fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}
