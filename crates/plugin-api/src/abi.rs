//! The stable C-ABI boundary that dynamically loaded plugins compile against.
//!
//! This exists because Rust gives no ABI stability across compiler versions —
//! a plugin and the host app could be built with different rustc releases and
//! must still interoperate. So every service crossing a `cdylib` boundary is
//! described here as a plain `#[repr(C)]` vtable of `extern "C" fn` pointers
//! plus an opaque instance pointer, the same shape as a COM/Win32 interface.
//!
//! Compatibility rule: vtable and manifest structs only ever grow by appending
//! fields after the `header`. Never reorder or remove a field. The `header`'s
//! `struct_size` lets a newer host detect an older (smaller) struct and simply
//! not touch the fields it doesn't have, and lets an older host reject a
//! struct that is missing a field it unconditionally needs — see
//! [`AbiHeader::has_field`].

use crate::types::AudioInfo;
use std::ffi::c_void;

/// Binary-compatibility version of this ABI, independent of the crate's own
/// source-level semver. Packed as `major << 16 | minor`.
///
/// Bump `ABI_VERSION_MAJOR` for a breaking change (field reordered/removed,
/// semantics changed) — hosts must reject plugins built against a different
/// major version. Bump `ABI_VERSION_MINOR` for additive-only changes (new
/// trailing fields) — old and new major-compatible versions may mix, gated by
/// [`AbiHeader::struct_size`].
pub const ABI_VERSION_MAJOR: u16 = 0;
pub const ABI_VERSION_MINOR: u16 = 1;
pub const ABI_VERSION: u32 = ((ABI_VERSION_MAJOR as u32) << 16) | (ABI_VERSION_MINOR as u32);

pub const fn abi_major(version: u32) -> u16 {
    (version >> 16) as u16
}

/// Prefixes every ABI struct so a host can tell which fields are actually
/// present before reading them.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AbiHeader {
    pub abi_version: u32,
    pub struct_size: usize,
}

impl AbiHeader {
    pub const fn new(struct_size: usize) -> Self {
        Self {
            abi_version: ABI_VERSION,
            struct_size,
        }
    }

    /// Whether a struct of this reported `struct_size` is large enough to
    /// contain the field whose byte offset + size is `field_end`.
    pub const fn has_field(&self, field_end: usize) -> bool {
        self.struct_size >= field_end
    }
}

/// Status codes returned by vtable functions in place of Rust's `Result`,
/// which has no stable ABI. Mirrors [`crate::error::DecodeError`] /
/// [`crate::error::OutputError`]; `plugin-host` maps these back to the real
/// error enums when proxying a loaded plugin as a Rust trait object.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbiStatus {
    Ok = 0,
    EndOfStream = 1,
    UnsupportedFormat = -1,
    IoError = -2,
    CorruptStream = -3,
    SeekUnsupported = -4,
    DeviceUnavailable = -5,
    UnsupportedConfig = -6,
    StreamError = -7,
}

/// A loaded service instance: an opaque pointer owned by the plugin, paired
/// with the vtable describing how to call into it. `instance` is passed as
/// the first argument to every function in `vtable`.
#[repr(C)]
pub struct ServiceHandle<V> {
    pub instance: *mut c_void,
    pub vtable: *const V,
}

#[repr(C)]
pub struct DecoderVtable {
    pub header: AbiHeader,
    /// `path_utf8`/`path_len` is the track path as UTF-8 bytes (not a C
    /// string — paths may legitimately contain embedded structure we don't
    /// want to rely on NUL-termination for). Writes the opened format info
    /// through `out_info`.
    pub open: unsafe extern "C" fn(
        instance: *mut c_void,
        path_utf8: *const u8,
        path_len: usize,
        out_info: *mut AudioInfo,
    ) -> AbiStatus,
    /// Decodes into `out_samples` (capacity `out_capacity` f32s), writing the
    /// number of samples actually produced through `out_written`.
    pub next_frame: unsafe extern "C" fn(
        instance: *mut c_void,
        out_samples: *mut f32,
        out_capacity: usize,
        out_written: *mut usize,
    ) -> AbiStatus,
    pub seek: unsafe extern "C" fn(instance: *mut c_void, pos_ms: u64) -> AbiStatus,
    pub info: unsafe extern "C" fn(instance: *mut c_void, out_info: *mut AudioInfo),
    pub destroy: unsafe extern "C" fn(instance: *mut c_void),
}

#[repr(C)]
pub struct DspVtable {
    pub header: AbiHeader,
    /// Writes the effect's display name as UTF-8 into `out_buf` (capacity
    /// `out_capacity`), returning the number of bytes written (truncated if
    /// the buffer is too small).
    pub name:
        unsafe extern "C" fn(instance: *mut c_void, out_buf: *mut u8, out_capacity: usize) -> usize,
    pub process: unsafe extern "C" fn(
        instance: *mut c_void,
        samples: *mut f32,
        sample_count: usize,
        channels: u16,
    ),
    pub reset: unsafe extern "C" fn(instance: *mut c_void),
    pub destroy: unsafe extern "C" fn(instance: *mut c_void),
}

#[repr(C)]
pub struct OutputVtable {
    pub header: AbiHeader,
    pub configure:
        unsafe extern "C" fn(instance: *mut c_void, sample_rate: u32, channels: u16) -> AbiStatus,
    pub write: unsafe extern "C" fn(
        instance: *mut c_void,
        samples: *const f32,
        sample_count: usize,
    ) -> AbiStatus,
    pub destroy: unsafe extern "C" fn(instance: *mut c_void),
}

/// A plugin's self-description, returned by its [`PluginEntryFn`]. Each
/// `create_*` field is `None` if the plugin doesn't implement that service.
#[repr(C)]
pub struct PluginManifest {
    pub header: AbiHeader,
    pub name_utf8: *const u8,
    pub name_len: usize,
    pub version_utf8: *const u8,
    pub version_len: usize,
    /// Inclusive range of host [`ABI_VERSION`]s this plugin supports.
    pub min_host_abi: u32,
    pub max_host_abi: u32,
    pub create_decoder: Option<unsafe extern "C" fn() -> ServiceHandle<DecoderVtable>>,
    pub create_dsp: Option<unsafe extern "C" fn() -> ServiceHandle<DspVtable>>,
    pub create_output: Option<unsafe extern "C" fn() -> ServiceHandle<OutputVtable>>,
}

// A plugin's `PluginManifest` is declared as a `'static` so this is always
// read-only, immutable, process-lifetime data -- the raw pointer fields
// (`name_utf8`/`version_utf8`) are never mutated, so sharing the manifest
// across threads is sound despite raw pointers not being `Sync` by default.
unsafe impl Sync for PluginManifest {}

/// Every plugin `cdylib` exports exactly one symbol with this signature, named
/// [`PLUGIN_ENTRY_SYMBOL`]. The host passes its own [`ABI_VERSION`] in; the
/// plugin returns `null` if it can't support that version, otherwise a
/// pointer to a `'static` [`PluginManifest`].
pub type PluginEntryFn = unsafe extern "C" fn(host_abi_version: u32) -> *const PluginManifest;

pub const PLUGIN_ENTRY_SYMBOL: &str = "openbar5000_plugin_entry";
