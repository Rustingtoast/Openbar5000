use crate::error::OutputError;

/// A sink that consumes the final interleaved `f32` PCM stream (e.g. the
/// cpal-backed default output, or a third-party sink like a network caster).
pub trait OutputSink: Send {
    /// (Re)configures the sink for a new sample rate/channel count. Called
    /// whenever the engine opens a track whose format differs from the
    /// currently configured one.
    fn configure(&mut self, sample_rate: u32, channels: u16) -> Result<(), OutputError>;

    /// Writes interleaved samples. Must not block for longer than real-time
    /// playback allows underruns to stay audible rather than stalling.
    fn write(&mut self, samples: &[f32]) -> Result<(), OutputError>;
}
