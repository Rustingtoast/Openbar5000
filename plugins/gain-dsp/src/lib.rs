//! Proof-of-concept Openbar5000 plugin: a fixed-attenuation gain effect.
//!
//! This crate's only purpose is to exercise the `plugin-api` C-ABI boundary
//! end-to-end (build order step 5) before any UI exists -- it proves a real
//! `cdylib`, loaded with `libloading`, can be called through the hand-rolled
//! vtables and round-trip correctly.

use plugin_api::abi::{AbiHeader, DspVtable, PluginManifest, ServiceHandle, ABI_VERSION};
use std::ffi::c_void;

/// Attenuation applied by this plugin; fixed for the proof-of-concept rather
/// than exposed as a setting.
const FIXED_GAIN: f32 = 0.8;

struct GainState {
    gain: f32,
}

unsafe extern "C" fn dsp_name(
    _instance: *mut c_void,
    out_buf: *mut u8,
    out_capacity: usize,
) -> usize {
    let name = b"Gain (plugin)";
    let len = name.len().min(out_capacity);
    std::ptr::copy_nonoverlapping(name.as_ptr(), out_buf, len);
    len
}

unsafe extern "C" fn dsp_process(
    instance: *mut c_void,
    samples: *mut f32,
    sample_count: usize,
    _channels: u16,
) {
    let state = &*(instance as *const GainState);
    let slice = std::slice::from_raw_parts_mut(samples, sample_count);
    for sample in slice.iter_mut() {
        *sample *= state.gain;
    }
}

unsafe extern "C" fn dsp_reset(_instance: *mut c_void) {}

unsafe extern "C" fn dsp_destroy(instance: *mut c_void) {
    drop(Box::from_raw(instance as *mut GainState));
}

static DSP_VTABLE: DspVtable = DspVtable {
    header: AbiHeader::new(std::mem::size_of::<DspVtable>()),
    name: dsp_name,
    process: dsp_process,
    reset: dsp_reset,
    destroy: dsp_destroy,
};

unsafe extern "C" fn create_dsp() -> ServiceHandle<DspVtable> {
    let state = Box::new(GainState { gain: FIXED_GAIN });
    ServiceHandle {
        instance: Box::into_raw(state) as *mut c_void,
        vtable: &DSP_VTABLE,
    }
}

const PLUGIN_NAME: &[u8] = b"gain-dsp";
const PLUGIN_VERSION: &[u8] = b"0.1.0";

static MANIFEST: PluginManifest = PluginManifest {
    header: AbiHeader::new(std::mem::size_of::<PluginManifest>()),
    name_utf8: PLUGIN_NAME.as_ptr(),
    name_len: PLUGIN_NAME.len(),
    version_utf8: PLUGIN_VERSION.as_ptr(),
    version_len: PLUGIN_VERSION.len(),
    min_host_abi: ABI_VERSION,
    max_host_abi: ABI_VERSION,
    create_decoder: None,
    create_dsp: Some(create_dsp),
    create_output: None,
};

#[no_mangle]
pub extern "C" fn openbar5000_plugin_entry(host_abi_version: u32) -> *const PluginManifest {
    if plugin_api::abi::abi_major(host_abi_version) != plugin_api::abi::abi_major(ABI_VERSION) {
        return std::ptr::null();
    }
    &MANIFEST
}
