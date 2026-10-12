use std::path::PathBuf;

/// Directories scanned for plugin `cdylib`s, plus `./plugins` next to the
/// running executable for development convenience.
pub fn plugin_search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if let Some(dir) = platform_plugin_dir() {
        dirs.push(dir);
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            dirs.push(parent.join("plugins"));
        }
    }

    dirs
}

#[cfg(target_os = "macos")]
fn platform_plugin_dir() -> Option<PathBuf> {
    dirs_home().map(|home| home.join("Library/Application Support/Openbar5000/plugins"))
}

#[cfg(target_os = "windows")]
fn platform_plugin_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA")
        .map(|appdata| PathBuf::from(appdata).join("Openbar5000").join("plugins"))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_plugin_dir() -> Option<PathBuf> {
    dirs_home().map(|home| home.join(".local/share/openbar5000/plugins"))
}

#[cfg(any(target_os = "macos", all(unix, not(target_os = "macos"))))]
fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}
