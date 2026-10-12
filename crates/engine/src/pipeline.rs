use crate::dsp::GainDsp;
use plugin_api::{AudioInfo, DecodeError, Decoder, Dsp, OutputError, OutputSink};
use std::fmt;

const CHUNK_SAMPLES: usize = 4096;

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
    gain: GainDsp,
    sink: Box<dyn OutputSink>,
    channels: u16,
    buffer: Vec<f32>,
}

impl Pipeline {
    pub fn new(sink: Box<dyn OutputSink>) -> Self {
        Self {
            decoder: None,
            plugin_dsp_chain: Vec::new(),
            gain: GainDsp::new(1.0),
            sink,
            channels: 2,
            buffer: vec![0.0; CHUNK_SAMPLES],
        }
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.gain.set_volume(volume);
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
        self.channels = info.channels;
        for dsp in self.plugin_dsp_chain.iter_mut() {
            dsp.reset();
        }
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
        self.gain.process(chunk, self.channels);

        self.sink.write(chunk)?;
        Ok(written)
    }
}
