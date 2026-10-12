//! Decode-correctness check (milestone-1 step 3): feeds a committed fixture
//! through the real `SymphoniaDecoder` and asserts on its reported format and
//! the samples it actually produces. Runs in CI without any audio hardware.

use engine::decoders::SymphoniaDecoder;
use plugin_api::Decoder;
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn decodes_wav_fixture_to_expected_format_and_sample_count() {
    let mut decoder = SymphoniaDecoder::new();
    let info = decoder
        .open(&fixture("test_tone.wav"))
        .expect("fixture should decode");

    assert_eq!(info.sample_rate, 44_100);
    assert_eq!(info.channels, 2);

    let mut total_samples = 0usize;
    let mut buf = [0.0f32; 4096];
    loop {
        let written = decoder
            .next_frame(&mut buf)
            .expect("decode should not error mid-stream");
        if written == 0 {
            break;
        }
        total_samples += written;
    }

    // ~2.16s @ 44.1kHz stereo, allow slack for container framing/padding.
    let expected = (info.sample_rate as usize) * (info.channels as usize) * 2;
    assert!(
        total_samples > expected / 2 && total_samples < expected * 2,
        "decoded {total_samples} samples, expected roughly {expected}"
    );
}
