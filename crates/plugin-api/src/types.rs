/// Describes the PCM format produced by a decoder or expected by an output sink.
///
/// Samples crossing this boundary are always interleaved `f32` in `[-1.0, 1.0]`,
/// which keeps the plugin ABI (see [`crate::abi`]) free of per-format branching.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct AudioInfo {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
    /// Total duration in milliseconds, or `u64::MAX` if unknown (e.g. a live stream).
    pub duration_ms: u64,
}
