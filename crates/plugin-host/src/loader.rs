use crate::proxy::{DecoderProxy, DspProxy, OutputProxy};
use libloading::{Library, Symbol};
use plugin_api::abi::{self, PluginEntryFn, PluginManifest};
use std::fmt;
use std::path::Path;

#[derive(Debug)]
pub enum LoadError {
    Io(String),
    MissingEntrySymbol,
    AbiRejected {
        plugin_min: u32,
        plugin_max: u32,
        host: u32,
    },
    NullManifest,
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(msg) => write!(f, "failed to load plugin library: {msg}"),
            Self::MissingEntrySymbol => {
                write!(
                    f,
                    "plugin is missing the `{}` entry symbol",
                    abi::PLUGIN_ENTRY_SYMBOL
                )
            }
            Self::AbiRejected {
                plugin_min,
                plugin_max,
                host,
            } => write!(
                f,
                "plugin supports host ABI {plugin_min}..={plugin_max}, host is {host}"
            ),
            Self::NullManifest => write!(f, "plugin entry point returned a null manifest"),
        }
    }
}

impl std::error::Error for LoadError {}

/// A loaded plugin `cdylib`, kept alive for as long as its proxied services
/// (`decoder`/`dsp`/`output`) are in use. Dropping this unloads the library,
/// which is only safe once all proxies obtained from it have themselves been
/// dropped -- `plugin-host` callers must not let a proxy outlive its
/// `LoadedPlugin`.
pub struct LoadedPlugin {
    _library: Library,
    pub name: String,
    pub version: String,
    pub decoder: Option<DecoderProxy>,
    pub dsp: Option<DspProxy>,
    pub output: Option<OutputProxy>,
}

/// Loads a single plugin `cdylib` from `path`, negotiating the ABI version
/// and instantiating whichever services it exposes.
///
/// # Safety
/// `path` must point to a library that correctly implements the Openbar5000
/// plugin ABI (see [`plugin_api::abi`]) -- loading an arbitrary shared
/// library is inherently unsafe, since we're trusting it to honor the vtable
/// contract.
pub unsafe fn load_plugin(path: &Path) -> Result<LoadedPlugin, LoadError> {
    let library = Library::new(path).map_err(|e| LoadError::Io(e.to_string()))?;

    let entry: Symbol<PluginEntryFn> = library
        .get(abi::PLUGIN_ENTRY_SYMBOL.as_bytes())
        .map_err(|_| LoadError::MissingEntrySymbol)?;

    let manifest_ptr = entry(abi::ABI_VERSION);
    if manifest_ptr.is_null() {
        return Err(LoadError::NullManifest);
    }
    let manifest: &PluginManifest = &*manifest_ptr;

    if abi::ABI_VERSION < manifest.min_host_abi || abi::ABI_VERSION > manifest.max_host_abi {
        return Err(LoadError::AbiRejected {
            plugin_min: manifest.min_host_abi,
            plugin_max: manifest.max_host_abi,
            host: abi::ABI_VERSION,
        });
    }

    let name = read_manifest_str(manifest.name_utf8, manifest.name_len);
    let version = read_manifest_str(manifest.version_utf8, manifest.version_len);

    let decoder = manifest
        .create_decoder
        .map(|create| DecoderProxy::new(create()));
    let dsp = manifest.create_dsp.map(|create| DspProxy::new(create()));
    let output = manifest
        .create_output
        .map(|create| OutputProxy::new(create()));

    Ok(LoadedPlugin {
        _library: library,
        name,
        version,
        decoder,
        dsp,
        output,
    })
}

unsafe fn read_manifest_str(ptr: *const u8, len: usize) -> String {
    if ptr.is_null() || len == 0 {
        return String::new();
    }
    let slice = std::slice::from_raw_parts(ptr, len);
    String::from_utf8_lossy(slice).into_owned()
}
