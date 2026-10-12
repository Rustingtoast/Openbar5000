//! Confirms the waveform-peak background computation and the live spectrum
//! FFT both actually fire during real playback through the real output
//! device -- the riskiest new logic added alongside the "as close as
//! possible to the reference UI" pass (background thread + per-chunk FFT).

use engine::{EngineCommand, EngineEvent, PlaybackEngine};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/test_tone.wav")
}

#[test]
fn waveform_and_spectrum_events_fire_during_playback() {
    let engine = PlaybackEngine::spawn();
    engine.send(EngineCommand::SetWaveformEnabled(true));
    engine.send(EngineCommand::SetSpectrumEnabled(true));
    engine.send(EngineCommand::LoadPlaylist(vec![fixture()]));
    engine.send(EngineCommand::Play);

    let mut got_waveform = false;
    let mut got_spectrum = false;
    let deadline = Instant::now() + Duration::from_secs(10);

    while Instant::now() < deadline && !(got_waveform && got_spectrum) {
        match engine.try_recv_event() {
            Some(EngineEvent::WaveformReady { peaks, .. }) => {
                assert!(!peaks.is_empty());
                got_waveform = true;
            }
            Some(EngineEvent::SpectrumUpdate { bars }) => {
                assert!(!bars.is_empty());
                got_spectrum = true;
            }
            Some(EngineEvent::Error(e)) => panic!("engine error: {e}"),
            _ => std::thread::sleep(Duration::from_millis(10)),
        }
    }

    assert!(got_waveform, "expected at least one WaveformReady event");
    assert!(got_spectrum, "expected at least one SpectrumUpdate event");
}
