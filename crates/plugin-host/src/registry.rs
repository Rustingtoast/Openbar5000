use crate::loader::{load_plugin, LoadedPlugin};
use crate::paths::plugin_search_dirs;

#[cfg(target_os = "windows")]
const PLUGIN_EXTENSION: &str = "dll";
#[cfg(target_os = "macos")]
const PLUGIN_EXTENSION: &str = "dylib";
#[cfg(all(unix, not(target_os = "macos")))]
const PLUGIN_EXTENSION: &str = "so";

/// Discovers and loads every plugin `cdylib` found in the platform plugin
/// directories (see [`plugin_search_dirs`]). A plugin that fails to load
/// (bad ABI, missing entry point, etc.) is logged and skipped rather than
/// failing the whole scan.
pub struct PluginRegistry {
    plugins: Vec<LoadedPlugin>,
}

impl PluginRegistry {
    pub fn scan() -> Self {
        let mut plugins = Vec::new();

        for dir in plugin_search_dirs() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some(PLUGIN_EXTENSION) {
                    continue;
                }
                // Safety: we only load files with the platform's native
                // library extension from our own plugin directories; the ABI
                // negotiation inside `load_plugin` rejects anything that
                // doesn't speak our entry-point protocol.
                match unsafe { load_plugin(&path) } {
                    Ok(plugin) => {
                        tracing::info!(
                            "loaded plugin: {} {} ({})",
                            plugin.name,
                            plugin.version,
                            path.display()
                        );
                        plugins.push(plugin);
                    }
                    Err(e) => tracing::warn!("skipping plugin {}: {e}", path.display()),
                }
            }
        }

        Self { plugins }
    }

    pub fn loaded(&self) -> &[LoadedPlugin] {
        &self.plugins
    }
}
