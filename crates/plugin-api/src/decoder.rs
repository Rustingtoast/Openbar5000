use crate::error::DecodeError;
use crate::types::AudioInfo;
use std::path::Path;
use std::time::Duration;

/// Decodes a single audio stream into interleaved `f32` PCM.
///
/// Implemented by built-ins (e.g. the symphonia-backed decoder in `engine`) and
/// by third-party decoder plugins loaded through `plugin-host`. Callers never
/// need to know which: both are handed around as `Box<dyn Decoder>`.
pub trait Decoder: Send {
    /// Opens `path` and returns the stream's format info. Must be called before
    /// any other method.
    fn open(&mut self, path: &Path) -> Result<AudioInfo, DecodeError>;

    /// Decodes the next chunk of interleaved samples into `out`, returning how
    /// many samples (not frames) were written. Returns `Ok(0)` only at end of
    /// stream.
    fn next_frame(&mut self, out: &mut [f32]) -> Result<usize, DecodeError>;

    /// Seeks to `pos`. Returns `DecodeError::SeekUnsupported` if this stream
    /// cannot seek (e.g. a live network source).
    fn seek(&mut self, pos: Duration) -> Result<(), DecodeError>;

    /// The format info returned by the most recent successful `open`.
    fn info(&self) -> AudioInfo;
}
