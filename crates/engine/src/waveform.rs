use crate::decoders::SymphoniaDecoder;
use plugin_api::{DecodeError, Decoder};
use std::path::Path;

/// Frames per raw peak sample while decoding, before binning down to the
/// requested bucket count. Bounds memory use regardless of track length.
const RAW_CHUNK_FRAMES: usize = 2048;

/// Fully decodes `path` (on whichever thread calls this -- callers should
/// run it off the realtime playback thread) and returns `bucket_count` peak
/// amplitudes in `[0.0, 1.0]`, suitable for drawing a waveform seekbar.
pub fn compute_peaks(path: &Path, bucket_count: usize) -> Result<Vec<f32>, DecodeError> {
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
