use plugin_api::{AudioInfo, DecodeError, Decoder};
use std::fs::File;
use std::path::Path;
use std::time::Duration;
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::codecs::CodecParameters;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::Time;

/// The built-in [`Decoder`] implementation, backed by `symphonia`. Treated by
/// `engine::pipeline` exactly like a decoder loaded from a plugin -- it's
/// just the one that's always available.
pub struct SymphoniaDecoder {
    format: Option<Box<dyn FormatReader>>,
    decoder: Option<Box<dyn AudioDecoder>>,
    track_id: u32,
    info: AudioInfo,
    /// Leftover interleaved samples from the last decoded packet that didn't
    /// fit in the caller's `out` buffer yet.
    pending: Vec<f32>,
    pending_pos: usize,
}

impl Default for SymphoniaDecoder {
    fn default() -> Self {
        Self {
            format: None,
            decoder: None,
            track_id: 0,
            info: AudioInfo {
                sample_rate: 0,
                channels: 0,
                bits_per_sample: 32,
                duration_ms: u64::MAX,
            },
            pending: Vec::new(),
            pending_pos: 0,
        }
    }
}

impl SymphoniaDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    fn decode_next_packet(&mut self) -> Result<bool, DecodeError> {
        let format = self.format.as_mut().expect("open() must be called first");
        let decoder = self.decoder.as_mut().expect("open() must be called first");

        loop {
            let packet = match format.next_packet() {
                Ok(Some(packet)) => packet,
                Ok(None) => return Ok(false),
                Err(SymError::ResetRequired) => return Ok(false),
                Err(e) => return Err(DecodeError::CorruptStream(e.to_string())),
            };

            if packet.track_id != self.track_id {
                continue;
            }

            match decoder.decode(&packet) {
                Ok(decoded) => {
                    let channels = decoded.spec().channels().count();
                    let frames = decoded.frames();
                    self.pending.resize(frames * channels, 0.0);
                    decoded.copy_to_slice_interleaved::<f32, _>(self.pending.as_mut_slice());
                    self.pending_pos = 0;
                    return Ok(true);
                }
                Err(SymError::DecodeError(_)) => continue,
                Err(e) => return Err(DecodeError::CorruptStream(e.to_string())),
            }
        }
    }
}

impl Decoder for SymphoniaDecoder {
    fn open(&mut self, path: &Path) -> Result<AudioInfo, DecodeError> {
        let file = File::open(path).map_err(|e| DecodeError::IoError(e.to_string()))?;
        let mss = MediaSourceStream::new(Box::new(file), MediaSourceStreamOptions::default());

        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }

        let format = symphonia::default::get_probe()
            .probe(
                &hint,
                mss,
                FormatOptions::default(),
                MetadataOptions::default(),
            )
            .map_err(|_| DecodeError::UnsupportedFormat)?;

        let track = format
            .default_track(TrackType::Audio)
            .ok_or(DecodeError::UnsupportedFormat)?
            .clone();
        let track_id = track.id;

        let Some(CodecParameters::Audio(audio_params)) = track.codec_params.clone() else {
            return Err(DecodeError::UnsupportedFormat);
        };

        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(&audio_params, &AudioDecoderOptions::default())
            .map_err(|_| DecodeError::UnsupportedFormat)?;

        let sample_rate = audio_params.sample_rate.unwrap_or(44_100);
        let channels = audio_params
            .channels
            .as_ref()
            .map(|c| c.count() as u16)
            .unwrap_or(2);
        let bits_per_sample = audio_params.bits_per_sample.unwrap_or(16) as u16;
        let duration_ms = track
            .num_frames
            .map(|frames| frames * 1000 / sample_rate as u64)
            .unwrap_or(u64::MAX);

        self.info = AudioInfo {
            sample_rate,
            channels,
            bits_per_sample,
            duration_ms,
        };
        self.track_id = track_id;
        self.format = Some(format);
        self.decoder = Some(decoder);
        self.pending.clear();
        self.pending_pos = 0;

        Ok(self.info)
    }

    fn next_frame(&mut self, out: &mut [f32]) -> Result<usize, DecodeError> {
        if self.pending_pos >= self.pending.len() && !self.decode_next_packet()? {
            return Ok(0);
        }

        let available = self.pending.len() - self.pending_pos;
        let to_copy = available.min(out.len());
        out[..to_copy].copy_from_slice(&self.pending[self.pending_pos..self.pending_pos + to_copy]);
        self.pending_pos += to_copy;
        Ok(to_copy)
    }

    fn seek(&mut self, pos: Duration) -> Result<(), DecodeError> {
        let format = self.format.as_mut().ok_or(DecodeError::SeekUnsupported)?;
        let time = Time::try_from_secs_f64(pos.as_secs_f64()).unwrap_or(Time::ZERO);
        format
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time,
                    track_id: Some(self.track_id),
                },
            )
            .map_err(|e| DecodeError::CorruptStream(e.to_string()))?;
        if let Some(decoder) = self.decoder.as_mut() {
            decoder.reset();
        }
        self.pending.clear();
        self.pending_pos = 0;
        Ok(())
    }

    fn info(&self) -> AudioInfo {
        self.info
    }
}
