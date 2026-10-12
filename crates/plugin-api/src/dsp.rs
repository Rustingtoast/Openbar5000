/// An in-place effect in the playback DSP chain (e.g. gain, EQ, a future VST
/// bridge). Processes interleaved `f32` PCM in place so effects can be chained
/// without extra copies.
pub trait Dsp: Send {
    /// Human-readable name shown in the UI's DSP chain editor.
    fn name(&self) -> &str;

    /// Processes `samples` (interleaved, `channels` channels) in place.
    fn process(&mut self, samples: &mut [f32], channels: u16);

    /// Clears any internal state (filter history, envelope followers, etc.),
    /// called on seek/track-change so effects don't leak audio across tracks.
    fn reset(&mut self);
}
