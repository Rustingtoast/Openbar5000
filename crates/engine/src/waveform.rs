use crate::decoders::SymphoniaDecoder;
use plugin_api::{DecodeError, Decoder};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Frames per raw peak sample while decoding, before binning down to the
/// requested bucket count. Bounds memory use regardless of track length.
const RAW_CHUNK_FRAMES: usize = 2048;

/// Tracks shorter than this aren't worth splitting across threads.
const MIN_PARALLEL_DURATION: Duration = Duration::from_secs(20);
/// At most half the cores (and 4): a track change shouldn't saturate the
/// machine and make the UI stutter just to draw the waveform sooner.
const MAX_WORKERS: usize = 4;

/// One channel's waveform outline: per bucket, the highest and lowest
/// sample value, each in `[-1.0, 1.0]` (so `max >= 0 >= min` for audio
/// that crosses zero within the bucket).
#[derive(Debug, Clone, PartialEq)]
pub struct WaveformChannel {
    pub max: Vec<f32>,
    pub min: Vec<f32>,
}

impl WaveformChannel {
    fn new(bucket_count: usize) -> Self {
        Self {
            max: vec![0.0; bucket_count],
            min: vec![0.0; bucket_count],
        }
    }

    fn add(&mut self, bucket: usize, sample: f32) {
        self.max[bucket] = self.max[bucket].max(sample);
        self.min[bucket] = self.min[bucket].min(sample);
    }

    fn merge(&mut self, other: &WaveformChannel) {
        for (a, b) in self.max.iter_mut().zip(&other.max) {
            *a = a.max(*b);
        }
        for (a, b) in self.min.iter_mut().zip(&other.min) {
            *a = a.min(*b);
        }
    }
}

/// Fully decodes `path` (on whichever thread calls this -- callers should
/// run it off the realtime playback thread) and returns, per channel,
/// `bucket_count` min/max pairs: the outline a waveform seekbar draws.
pub fn compute_peaks(path: &Path, bucket_count: usize) -> Result<Vec<WaveformChannel>, DecodeError> {
    let never = AtomicBool::new(false);
    Ok(compute_peaks_cancellable(path, bucket_count, &never)?.expect("never cancelled"))
}

/// [`compute_peaks`], giving up -- `Ok(None)` -- soon after `cancel` is set
/// (checked every decoded chunk), e.g. because playback already moved on to
/// another track and nobody will see this waveform.
///
/// Decoding a whole track is the heaviest thing a track change does, so it
/// is split into contiguous time slices decoded on separate cores (each
/// with its own decoder seeked to its slice); falls back to one thread when
/// the duration is unknown, the track is short, or seeking fails.
pub fn compute_peaks_cancellable(
    path: &Path,
    bucket_count: usize,
    cancel: &AtomicBool,
) -> Result<Option<Vec<WaveformChannel>>, DecodeError> {
    let workers = std::thread::available_parallelism()
        .map(|n| n.get() / 2)
        .unwrap_or(1)
        .clamp(1, MAX_WORKERS);
    let info = SymphoniaDecoder::new().open(path)?;
    let known_duration = info.duration_ms != u64::MAX && info.duration_ms > 0;
    if bucket_count == 0
        || workers < 2
        || !known_duration
        || Duration::from_millis(info.duration_ms) < MIN_PARALLEL_DURATION
    {
        return compute_peaks_sequential(path, bucket_count, cancel);
    }

    let total_frames = info.duration_ms * info.sample_rate as u64 / 1000;
    let slices: Vec<Result<Option<Vec<WaveformChannel>>, DecodeError>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers as u64)
            .map(|i| {
                let start = total_frames * i / workers as u64;
                // The last slice runs to the real end of the stream, in case
                // the tagged duration is a little short.
                let end = (i + 1 < workers as u64).then(|| total_frames * (i + 1) / workers as u64);
                scope.spawn(move || {
                    peaks_for_slice(path, info.sample_rate, start, end, total_frames, bucket_count, cancel)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or(Err(DecodeError::EndOfStream)))
            .collect()
    });

    let mut merged: Option<Vec<WaveformChannel>> = None;
    for slice in slices {
        let slice = match slice {
            Ok(Some(slice)) => slice,
            Ok(None) => return Ok(None),
            Err(_) => return compute_peaks_sequential(path, bucket_count, cancel),
        };
        match merged.as_mut() {
            None => merged = Some(slice),
            Some(channels) => {
                for (channel, other) in channels.iter_mut().zip(&slice) {
                    channel.merge(other);
                }
            }
        }
    }
    Ok(Some(merged.unwrap_or_default()))
}

/// Decodes frames `start..end` (or to the end of the stream) and pools
/// them straight into `bucket_count` buckets spanning `total_frames`.
fn peaks_for_slice(
    path: &Path,
    sample_rate: u32,
    start: u64,
    end: Option<u64>,
    total_frames: u64,
    bucket_count: usize,
    cancel: &AtomicBool,
) -> Result<Option<Vec<WaveformChannel>>, DecodeError> {
    let mut decoder = SymphoniaDecoder::new();
    let channels = decoder.open(path)?.channels.max(1) as usize;
    if start > 0 {
        decoder.seek(Duration::from_secs_f64(start as f64 / sample_rate as f64))?;
    }

    let mut out = vec![WaveformChannel::new(bucket_count); channels];
    let mut buf = vec![0f32; 4096];
    let mut frame = start;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let written = decoder.next_frame(&mut buf)?;
        if written == 0 {
            break;
        }
        for samples in buf[..written].chunks_exact(channels) {
            if end.is_some_and(|end| frame >= end) {
                return Ok(Some(out));
            }
            let bucket = ((frame * bucket_count as u64 / total_frames) as usize).min(bucket_count - 1);
            for (channel, &sample) in out.iter_mut().zip(samples) {
                channel.add(bucket, sample);
            }
            frame += 1;
        }
    }
    Ok(Some(out))
}

fn compute_peaks_sequential(
    path: &Path,
    bucket_count: usize,
    cancel: &AtomicBool,
) -> Result<Option<Vec<WaveformChannel>>, DecodeError> {
    let mut decoder = SymphoniaDecoder::new();
    let info = decoder.open(path)?;
    let channels = info.channels.max(1) as usize;

    // Unknown length: pool fixed-size chunks first, then fit those to the
    // bucket count once the real length is known.
    let mut raw: Vec<WaveformChannel> = vec![WaveformChannel { max: Vec::new(), min: Vec::new() }; channels];
    let mut buf = vec![0f32; 4096];
    let mut frames_in_chunk = 0usize;

    loop {
        if cancel.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let written = decoder.next_frame(&mut buf)?;
        if written == 0 {
            break;
        }
        for samples in buf[..written].chunks_exact(channels) {
            if frames_in_chunk == 0 {
                for channel in raw.iter_mut() {
                    channel.max.push(0.0);
                    channel.min.push(0.0);
                }
            }
            let chunk = raw[0].max.len() - 1;
            for (channel, &sample) in raw.iter_mut().zip(samples) {
                channel.add(chunk, sample);
            }
            frames_in_chunk = (frames_in_chunk + 1) % RAW_CHUNK_FRAMES;
        }
    }

    let raw_len = raw[0].max.len();
    Ok(Some(raw
        .iter()
        .map(|channel| {
            let mut out = WaveformChannel::new(bucket_count);
            if bucket_count == 0 {
                return out;
            }
            for i in 0..raw_len {
                let bucket = (i * bucket_count / raw_len).min(bucket_count - 1);
                out.add(bucket, channel.max[i]);
                out.add(bucket, channel.min[i]);
            }
            out
        })
        .collect()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parallel peaks equal a one-pass decode, and how much faster they
    /// are. Needs a real (20s+) track -- the repo has no long fixture:
    /// `OPENBAR_TEST_TRACK=song.mp3 cargo test --release -p engine
    /// waveform -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn parallel_peaks_match_sequential_on_a_real_track() {
        let Some(path) = std::env::var_os("OPENBAR_TEST_TRACK") else {
            return;
        };
        let path = std::path::PathBuf::from(path);
        let t = std::time::Instant::now();
        compute_peaks_sequential(&path, 400, &AtomicBool::new(false)).unwrap();
        let sequential_time = t.elapsed();
        let t = std::time::Instant::now();
        let parallel = compute_peaks(&path, 400).unwrap();
        let parallel_time = t.elapsed();
        println!("sequential {sequential_time:.2?}, parallel {parallel_time:.2?}");

        // Exact reference: the same per-frame bucketing in one pass. (The
        // sequential path pools 2048-frame chunks first, so a chunk that
        // straddles a bucket boundary lands wholly in one bucket -- it's
        // only approximately equal.)
        let info = SymphoniaDecoder::new().open(&path).unwrap();
        let total = info.duration_ms * info.sample_rate as u64 / 1000;
        let exact = peaks_for_slice(&path, info.sample_rate, 0, None, total, 400, &AtomicBool::new(false))
            .unwrap()
            .unwrap();
        assert_eq!(parallel, exact, "parallel peaks must match a one-pass decode exactly");
    }

    #[test]
    fn outlines_every_channel_within_range() {
        let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/test_tone.wav");
        let channels = compute_peaks(&fixture, 50).unwrap();
        assert!(!channels.is_empty());
        for channel in &channels {
            assert_eq!((channel.max.len(), channel.min.len()), (50, 50));
            assert!(channel.max.iter().any(|&p| p > 0.0), "a tone has positive peaks");
            assert!(channel.min.iter().any(|&p| p < 0.0), "and negative troughs");
            assert!(channel.max.iter().chain(&channel.min).all(|p| (-1.0..=1.0).contains(p)));
        }
        // The sequential fallback agrees on shape.
        let sequential = compute_peaks_sequential(&fixture, 50, &AtomicBool::new(false))
            .unwrap()
            .unwrap();
        assert_eq!(sequential.len(), channels.len());
    }

    #[test]
    fn a_cancelled_computation_stops_without_a_result() {
        let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/test_tone.wav");
        let cancelled = AtomicBool::new(true);
        assert_eq!(compute_peaks_cancellable(&fixture, 50, &cancelled).unwrap(), None);
    }
}
