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
            Some(EngineEvent::WaveformReady { channels, .. }) => {
                assert!(!channels.is_empty());
                assert!(!channels[0].max.is_empty());
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
/// per decoded chunk (~10 fps). Measured over the first second of
/// playback (the fixture is ~2.1s long). The position has no events: the
/// clock is read whenever it's drawn, and must move smoothly between reads.
#[test]
fn spectrum_updates_at_its_configured_rate_and_the_clock_runs_smoothly() {
    let engine = PlaybackEngine::spawn();
    engine.send(EngineCommand::SetSpectrumEnabled(true));
    engine.send(EngineCommand::SetSpectrumConfig(SpectrumConfig {
        interval_ms: 10,
        fft_size: 16384,
        ..SpectrumConfig::default()
    }));
    engine.send(EngineCommand::LoadPlaylist(vec![fixture()]));
    engine.send(EngineCommand::Play);

    // Count from the first frame, not from a fixed moment: opening the
    // audio device can take a while (the first run of a new build, a busy
    // machine), and that's not what's measured here.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match engine.try_recv_event() {
            Some(EngineEvent::SpectrumUpdate { .. }) => break,
            Some(EngineEvent::Error(e)) => panic!("engine error: {e}"),
            Some(_) => {}
            None if Instant::now() > deadline => panic!("playback didn't start"),
            None => std::thread::sleep(Duration::from_millis(1)),
        }
    }
    // One second, well inside the ~2.1s fixture.
    let start = Instant::now();
    let mut spectrum = 0u32;
    let mut positions = Vec::new();
    let mut last_read = start;
    while start.elapsed() < Duration::from_secs(1) {
        match engine.try_recv_event() {
            Some(EngineEvent::SpectrumUpdate { .. }) => spectrum += 1,
            Some(EngineEvent::Error(e)) => panic!("engine error: {e}"),
            Some(_) => {}
            None => std::thread::sleep(Duration::from_millis(1)),
        }
        // Like a 60 fps UI.
        if last_read.elapsed() >= Duration::from_millis(16) {
            last_read = Instant::now();
            positions.push(engine.clock().position());
        }
    }
    println!("per second: {spectrum} spectrum updates, {} clock reads", positions.len());
    // Generous lower bound (nominal 100) so a loaded CI box passes.
    assert!(spectrum >= 50, "only {spectrum} spectrum updates/s");
    assert!(positions.windows(2).all(|w| w[1] >= w[0]), "the clock went backwards: {positions:?}");
    let distinct = positions.windows(2).filter(|w| w[1] > w[0]).count();
    assert!(
        distinct * 10 >= positions.len() * 8,
        "the clock moved on only {distinct} of {} reads",
        positions.len()
    );
    let advanced = *positions.last().unwrap() - positions[0];
    assert!(
        advanced >= Duration::from_millis(800) && advanced <= Duration::from_millis(1100),
        "about a second of playback should be heard in a second, got {advanced:?}"
    );
}
