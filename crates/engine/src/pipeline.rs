use crate::dsp::{EqualizerDsp, EqualizerSettings, GainDsp};
use crate::spectrum::{SpectrumAnalyzer, SpectrumConfig};
use plugin_api::{AudioInfo, DecodeError, Decoder, Dsp, OutputError, OutputSink};
use std::fmt;

const CHUNK_SAMPLES: usize = 4096;

/// Largest FFT the spectrum settings offer, in frames -- sizes the sample
/// history so any configured window fits.
const MAX_FFT_FRAMES: usize = 16384;

/// The most recent samples written to the sink, oldest first, so the
/// spectrum can analyze exactly what's coming out of the speakers -- the
/// sink still holds up to ~0.5s of audio that was decoded earlier than
/// it's heard.
struct SampleHistory {
    samples: Vec<f32>,
    /// Index the next sample is written to (the oldest once full).
    head: usize,
    filled: usize,
}

impl SampleHistory {
    fn new(capacity: usize) -> Self {
        Self {
            samples: vec![0.0; capacity.max(1)],
            head: 0,
            filled: 0,
        }
    }

    fn clear(&mut self) {
        self.head = 0;
        self.filled = 0;
    }

    fn push(&mut self, chunk: &[f32]) {
        let capacity = self.samples.len();
        // A chunk longer than the history only needs its tail.
        let chunk = &chunk[chunk.len().saturating_sub(capacity)..];
        let first = chunk.len().min(capacity - self.head);
        self.samples[self.head..self.head + first].copy_from_slice(&chunk[..first]);
        self.samples[..chunk.len() - first].copy_from_slice(&chunk[first..]);
        self.head = (self.head + chunk.len()) % capacity;
        self.filled = (self.filled + chunk.len()).min(capacity);
    }

    /// Copies the `len` samples that end `skip_newest` samples before the
    /// newest one into `out` (fewer if the history doesn't reach back that
    /// far).
    fn copy_window(&self, skip_newest: usize, len: usize, out: &mut Vec<f32>) {
        out.clear();
        let capacity = self.samples.len();
        let available = self.filled.saturating_sub(skip_newest);
        let len = len.min(available);
        let end = (self.head + capacity - skip_newest.min(self.filled)) % capacity;
        let start = (end + capacity - len) % capacity;
        if start + len <= capacity {
            out.extend_from_slice(&self.samples[start..start + len]);
        } else {
            out.extend_from_slice(&self.samples[start..]);
            out.extend_from_slice(&self.samples[..len - (capacity - start)]);
        }
    }
}

#[derive(Debug)]
pub enum PipelineError {
    Decode(DecodeError),
    Output(OutputError),
}

impl fmt::Display for PipelineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode(e) => write!(f, "{e}"),
            Self::Output(e) => write!(f, "{e}"),
        }
    }
}

impl From<DecodeError> for PipelineError {
    fn from(e: DecodeError) -> Self {
        Self::Decode(e)
    }
}

impl From<OutputError> for PipelineError {
    fn from(e: OutputError) -> Self {
        Self::Output(e)
    }
}

/// Bundles the active track's decoder with the DSP chain and output sink.
///
/// Volume is a fixed, always-present final stage rather than a plugin
/// extension point (per the milestone-1 scope); `plugin_dsp_chain` is where
/// third-party `Dsp` plugins loaded by `plugin-host` will be inserted ahead
/// of it.
pub struct Pipeline {
    decoder: Option<Box<dyn Decoder>>,
    plugin_dsp_chain: Vec<Box<dyn Dsp>>,
    equalizer: EqualizerDsp,
    gain: GainDsp,
    sink: Box<dyn OutputSink>,
    channels: u16,
    sample_rate: u32,
    buffer: Vec<f32>,
    spectrum: SpectrumAnalyzer,
    spectrum_enabled: bool,
    history: SampleHistory,
    /// Reused buffer for the window handed to the analyzer.
    spectrum_window: Vec<f32>,
}

impl Pipeline {
    pub fn new(sink: Box<dyn OutputSink>) -> Self {
        Self {
            decoder: None,
            plugin_dsp_chain: Vec::new(),
            equalizer: EqualizerDsp::new(),
            gain: GainDsp::new(1.0),
            sink,
            channels: 2,
            sample_rate: 44_100,
            buffer: vec![0.0; CHUNK_SAMPLES],
            spectrum: SpectrumAnalyzer::new(SpectrumConfig::default()),
            spectrum_enabled: false,
            history: SampleHistory::new(1),
            spectrum_window: Vec::new(),
        }
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.gain.set_volume(volume);
    }

    pub fn set_equalizer(&mut self, settings: EqualizerSettings) {
        self.equalizer.set_settings(settings);
    }

    pub fn set_spectrum_enabled(&mut self, enabled: bool) {
        self.spectrum_enabled = enabled;
        if !enabled {
            self.history.clear();
        }
    }

    pub fn set_spectrum_config(&mut self, config: SpectrumConfig) {
        self.spectrum.set_config(config);
    }

    /// Analyzes the audio currently audible (the newest `fft_size` frames
    /// the sink has actually played, not just been handed) and returns
    /// `(bars, peaks)` -- `None` when disabled, or when the configured
    /// interval hasn't elapsed since the last analysis. Independent of
    /// decoding, so it can run as often as the interval asks.
    pub fn audible_spectrum(&mut self) -> Option<(Vec<f32>, Vec<f32>)> {
        if !self.spectrum_enabled || !self.spectrum.due() {
            return None;
        }
        let channels = self.channels.max(1) as usize;
        let window = self.spectrum.fft_size() * channels;
        self.history
            .copy_window(self.sink.queued_samples(), window, &mut self.spectrum_window);
        Some(
            self.spectrum
                .analyze(&self.spectrum_window, channels, self.sample_rate),
        )
    }

    /// Whether the sink can take a whole chunk without `process_chunk`
    /// blocking in `write`. Sinks that can't tell always report room.
    pub fn has_room_for_chunk(&self) -> bool {
        self.sink
            .free_samples()
            .is_none_or(|free| free >= CHUNK_SAMPLES)
    }

    /// Roughly how long until the sink has drained enough to take a whole
    /// chunk -- zero if it already can (or can't tell).
    pub fn time_until_room(&self) -> std::time::Duration {
        let Some(free) = self.sink.free_samples() else {
            return std::time::Duration::ZERO;
        };
        let missing = CHUNK_SAMPLES.saturating_sub(free);
        let samples_per_sec = self.sample_rate.max(1) as f64 * self.channels.max(1) as f64;
        std::time::Duration::from_secs_f64(missing as f64 / samples_per_sec)
    }

    /// How long until the next spectrum analysis is due, or `None` when the
    /// spectrum is off.
    pub fn time_until_spectrum(&self) -> Option<std::time::Duration> {
        self.spectrum_enabled.then(|| self.spectrum.time_until_due())
    }

    /// Samples written to the sink that haven't been heard yet.
    pub fn queued_samples(&self) -> usize {
        self.sink.queued_samples()
    }

    /// Inserts a plugin-provided effect ahead of the fixed volume stage.
    ///
    /// Not called yet: wiring loaded plugins (see `plugin-host::PluginRegistry`)
    /// into a live pipeline safely means keeping each plugin's backing
    /// `Library` alive for exactly as long as its proxy is in use, which
    /// isn't settled yet -- see the milestone plan's open risks. The FFI
    /// round-trip itself is instead verified standalone via
    /// `cargo run -p plugin-host --example load_plugin`.
    #[allow(dead_code)]
    pub fn push_dsp(&mut self, dsp: Box<dyn Dsp>) {
        self.plugin_dsp_chain.push(dsp);
    }

    /// Switches to a new track's decoder, (re)configuring the output sink if
    /// its format differs from the previous track's.
    pub fn open_track(
        &mut self,
        mut decoder: Box<dyn Decoder>,
        path: &std::path::Path,
    ) -> Result<AudioInfo, PipelineError> {
        let info = decoder.open(path)?;
        self.sink.configure(info.sample_rate, info.channels)?;
        if info.channels != self.channels || self.history.samples.len() <= 1 {
            // Room for the largest FFT window plus a full second of
            // output latency; the cpal sink buffers half a second.
            self.history = SampleHistory::new(
                (MAX_FFT_FRAMES + info.sample_rate as usize) * info.channels as usize,
            );
        }
        self.channels = info.channels;
        self.sample_rate = info.sample_rate;
        for dsp in self.plugin_dsp_chain.iter_mut() {
            dsp.reset();
        }
        self.equalizer.set_format(info.sample_rate, info.channels);
        self.equalizer.reset();
        self.gain.reset();
        self.decoder = Some(decoder);
        Ok(info)
    }

    pub fn seek(&mut self, pos: std::time::Duration) -> Result<(), PipelineError> {
        self.decoder
            .as_mut()
            .ok_or(PipelineError::Decode(DecodeError::EndOfStream))?
            .seek(pos)?;
        Ok(())
    }

    /// Decodes, processes, and plays one chunk. Returns the number of
    /// interleaved samples processed; `0` means the track has ended.
    pub fn process_chunk(&mut self) -> Result<usize, PipelineError> {
        let decoder = self
            .decoder
            .as_mut()
            .ok_or(PipelineError::Decode(DecodeError::EndOfStream))?;

        let written = decoder.next_frame(&mut self.buffer)?;
        if written == 0 {
            return Ok(0);
        }

        let chunk = &mut self.buffer[..written];
        for dsp in self.plugin_dsp_chain.iter_mut() {
            dsp.process(chunk, self.channels);
        }
        self.equalizer.process(chunk, self.channels);
        self.gain.process(chunk, self.channels);

        if self.spectrum_enabled {
            self.history.push(chunk);
        }

        self.sink.write(chunk)?;
        Ok(written)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(history: &SampleHistory, skip: usize, len: usize) -> Vec<f32> {
        let mut out = Vec::new();
        history.copy_window(skip, len, &mut out);
        out
    }

    #[test]
    fn history_windows_skip_the_not_yet_audible_tail_and_wrap() {
        let mut history = SampleHistory::new(5);
        history.push(&[1.0, 2.0, 3.0]);
        assert_eq!(window(&history, 0, 2), [2.0, 3.0]);
        assert_eq!(window(&history, 1, 5), [1.0, 2.0]);

        history.push(&[4.0, 5.0, 6.0, 7.0]); // wraps; holds 3..=7
        assert_eq!(window(&history, 0, 5), [3.0, 4.0, 5.0, 6.0, 7.0]);
        assert_eq!(window(&history, 2, 2), [4.0, 5.0]);
        assert_eq!(window(&history, 9, 2), Vec::<f32>::new());

        history.push(&[10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0]); // longer than capacity
        assert_eq!(window(&history, 0, 5), [12.0, 13.0, 14.0, 15.0, 16.0]);
    }
}
