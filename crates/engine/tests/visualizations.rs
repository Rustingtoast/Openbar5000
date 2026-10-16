//! Confirms the waveform-peak background computation and the live spectrum
//! FFT both actually fire during real playback through the real output
//! device -- the riskiest new logic added alongside the "as close as
//! possible to the reference UI" pass (background thread + per-chunk FFT).

use engine::{EngineCommand, EngineEvent, PlaybackEngine, SpectrumConfig};
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
            Some(EngineEvent::SpectrumUpdate { bars, peaks }) => {
                assert!(!bars.is_empty());
                assert!(!peaks.is_empty(), "peaks default to enabled");
                got_spectrum = true;
            }
            Some(EngineEvent::Error(e)) => panic!("engine error: {e}"),
            _ => std::thread::sleep(Duration::from_millis(10)),
        }
    }

    assert!(got_waveform, "expected at least one WaveformReady event");
    assert!(got_spectrum, "expected at least one SpectrumUpdate event");
}

/// Issue #16: with a 10ms interval the spectrum used to update only once
/// per decoded chunk (~10 fps) and the position every 200ms. Measured over
/// a steady stretch mid-track (the fixture is ~2.1s long).
#[test]
fn spectrum_and_position_update_at_their_configured_rates() {
    let engine = PlaybackEngine::spawn();
    engine.send(EngineCommand::SetSpectrumEnabled(true));
    engine.send(EngineCommand::SetSpectrumConfig(SpectrumConfig {
        interval_ms: 10,
        fft_size: 16384,
        ..SpectrumConfig::default()
    }));
    engine.send(EngineCommand::LoadPlaylist(vec![fixture()]));
    engine.send(EngineCommand::Play);

    let start = Instant::now();
    let (from, to) = (Duration::from_millis(500), Duration::from_millis(1500));
    let (mut spectrum, mut position) = (0u32, 0u32);
    while start.elapsed() < to {
        match engine.try_recv_event() {
            Some(EngineEvent::SpectrumUpdate { .. }) if start.elapsed() >= from => spectrum += 1,
            Some(EngineEvent::PositionUpdate { .. }) if start.elapsed() >= from => position += 1,
            Some(EngineEvent::Error(e)) => panic!("engine error: {e}"),
            Some(_) => {}
            None => std::thread::sleep(Duration::from_millis(1)),
        }
    }
    println!("per second: {spectrum} spectrum updates, {position} position updates");
    // Generous lower bounds (nominal 100 and 30) so a loaded CI box passes.
    assert!(spectrum >= 50, "only {spectrum} spectrum updates/s");
    assert!(position >= 20, "only {position} position updates/s");
}
