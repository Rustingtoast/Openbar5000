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

    /// How many interleaved samples `write` could accept right now without
    /// waiting, if the sink knows. Lets the engine decode only when there's
    /// room, instead of blocking inside `write` (which would stall its
    /// visualizations and command handling). `None` (the default): unknown,
    /// so the engine just writes and accepts that `write` may wait.
    fn free_samples(&self) -> Option<usize> {
        None
    }

    /// Interleaved samples already written but not yet audible (the sink's
    /// output latency). The engine subtracts this so the seekbar and
    /// spectrum follow what's heard rather than what's been decoded.
    fn queued_samples(&self) -> usize {
        0
    }
}
