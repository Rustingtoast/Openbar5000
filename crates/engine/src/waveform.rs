use crate::decoders::SymphoniaDecoder;
use plugin_api::{DecodeError, Decoder};
use std::path::Path;
use std::time::Duration;

/// Frames per raw peak sample while decoding, before binning down to the
/// requested bucket count. Bounds memory use regardless of track length.
const RAW_CHUNK_FRAMES: usize = 2048;

/// Tracks shorter than this aren't worth splitting across threads.
const MIN_PARALLEL_DURATION: Duration = Duration::from_secs(20);
const MAX_WORKERS: usize = 8;

/// Fully decodes `path` (on whichever thread calls this -- callers should
/// run it off the realtime playback thread) and returns `bucket_count` peak
/// amplitudes in `[0.0, 1.0]`, suitable for drawing a waveform seekbar.
///
/// Decoding a whole track is the heaviest thing a track change does, so it
/// is split into contiguous time slices decoded on separate cores (each
/// with its own decoder seeked to its slice); falls back to one thread when
/// the duration is unknown, the track is short, or seeking fails.
pub fn compute_peaks(path: &Path, bucket_count: usize) -> Result<Vec<f32>, DecodeError> {
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(MAX_WORKERS);
    let info = SymphoniaDecoder::new().open(path)?;
    let known_duration = info.duration_ms != u64::MAX && info.duration_ms > 0;
    if bucket_count == 0
        || workers < 2
        || !known_duration
        || Duration::from_millis(info.duration_ms) < MIN_PARALLEL_DURATION
    {
        return compute_peaks_sequential(path, bucket_count);
    }

    let total_frames = info.duration_ms * info.sample_rate as u64 / 1000;
    let slices: Vec<Result<Vec<f32>, DecodeError>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers as u64)
            .map(|i| {
                let start = total_frames * i / workers as u64;
                // The last slice runs to the real end of the stream, in case
                // the tagged duration is a little short.
                let end = (i + 1 < workers as u64).then(|| total_frames * (i + 1) / workers as u64);
                scope.spawn(move || {
                    peaks_for_slice(path, info.sample_rate, start, end, total_frames, bucket_count)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or(Err(DecodeError::EndOfStream)))
            .collect()
    });

    let mut buckets = vec![0f32; bucket_count];
    for slice in slices {
        let Ok(slice) = slice else {
            return compute_peaks_sequential(path, bucket_count);
        };
        for (bucket, peak) in buckets.iter_mut().zip(slice) {
            *bucket = bucket.max(peak);
        }
    }
    Ok(buckets)
}

/// Decodes frames `start..end` (or to the end of the stream) and max-pools
/// them straight into `bucket_count` buckets spanning `total_frames`.
fn peaks_for_slice(
    path: &Path,
    sample_rate: u32,
    start: u64,
    end: Option<u64>,
    total_frames: u64,
    bucket_count: usize,
) -> Result<Vec<f32>, DecodeError> {
    let mut decoder = SymphoniaDecoder::new();
    let channels = decoder.open(path)?.channels.max(1) as usize;
    if start > 0 {
        decoder.seek(Duration::from_secs_f64(start as f64 / sample_rate as f64))?;
    }

    let mut buckets = vec![0f32; bucket_count];
    let mut buf = vec![0f32; 4096];
    let mut frame = start;
    loop {
        let written = decoder.next_frame(&mut buf)?;
        if written == 0 {
            break;
        }
        for samples in buf[..written].chunks_exact(channels) {
            if end.is_some_and(|end| frame >= end) {
                return Ok(buckets);
            }
            let bucket = ((frame * bucket_count as u64 / total_frames) as usize).min(bucket_count - 1);
            let peak = samples.iter().fold(0f32, |acc, s| acc.max(s.abs()));
            buckets[bucket] = buckets[bucket].max(peak);
            frame += 1;
        }
    }
    Ok(buckets)
}

fn compute_peaks_sequential(path: &Path, bucket_count: usize) -> Result<Vec<f32>, DecodeError> {
    let mut decoder = SymphoniaDecoder::new();
    let info = decoder.open(path)?;
    let channels = info.channels.max(1) as usize;

    let mut raw_peaks = Vec::new();
    let mut buf = vec![0f32; 4096];
    let mut chunk_max = 0f32;
    let mut frames_in_chunk = 0usize;

    loop {
        let written = decoder.next_frame(&mut buf)?;
        if written == 0 {
            break;
        }
        for frame in buf[..written].chunks_exact(channels) {
            let sample_max = frame.iter().fold(0f32, |acc, s| acc.max(s.abs()));
            chunk_max = chunk_max.max(sample_max);
            frames_in_chunk += 1;
            if frames_in_chunk >= RAW_CHUNK_FRAMES {
                raw_peaks.push(chunk_max);
                chunk_max = 0.0;
                frames_in_chunk = 0;
            }
        }
    }
    if frames_in_chunk > 0 {
        raw_peaks.push(chunk_max);
    }
    if raw_peaks.is_empty() {
        return Ok(vec![0.0; bucket_count]);
    }

    // Max-pool the raw peak stream down to exactly `bucket_count` buckets.
    let mut buckets = vec![0f32; bucket_count];
    for (i, &peak) in raw_peaks.iter().enumerate() {
        let bucket = (i * bucket_count / raw_peaks.len()).min(bucket_count - 1);
        buckets[bucket] = buckets[bucket].max(peak);
    }
    Ok(buckets)
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
        let sequential = compute_peaks_sequential(&path, 400).unwrap();
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
        let exact = peaks_for_slice(&path, info.sample_rate, 0, None, total, 400).unwrap();
        let diff = |a: &[f32], b: &[f32]| {
            a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0f32, f32::max)
        };
        println!(
            "max bucket difference vs exact: parallel {}, sequential {}",
            diff(&parallel, &exact),
            diff(&sequential, &exact)
        );
        assert_eq!(parallel, exact, "parallel peaks must match a one-pass decode exactly");
    }

    #[test]
    fn produces_nonzero_peaks_for_a_real_fixture() {
        let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/test_tone.wav");
        let peaks = compute_peaks(&fixture, 50).unwrap();
        assert_eq!(peaks.len(), 50);
        assert!(
            peaks.iter().any(|&p| p > 0.0),
            "expected at least one nonzero peak"
        );
        assert!(
            peaks.iter().all(|&p| p <= 1.0),
            "peaks should stay within [0, 1]"
        );
    }
}

