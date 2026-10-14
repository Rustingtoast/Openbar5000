use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};
use std::sync::Arc;
use std::time::Instant;

/// Which channel(s) feed the analyzer -- mirrors the reference UI's
/// "Channels" dropdown (we only implement the common cases, not full
/// per-speaker selection).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpectrumChannelMode {
    All,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpectrumConfig {
    pub channel_mode: SpectrumChannelMode,
    /// dB range the magnitude is mapped onto before normalizing to the
    /// `[0, 1]` bar heights the UI draws.
    pub min_db: f32,
    pub max_db: f32,
    pub fft_size: usize,
    pub peaks_enabled: bool,
    pub peak_hold_ms: f32,
    /// How fast a peak accelerates downward once it starts falling, in
    /// dB/s² -- true gravity-style acceleration (velocity builds up over
    /// time), not a fixed linear decay rate.
    pub gravity_db_per_s2: f32,
    /// Minimum time between UI-visible updates; throttled independently of
    /// how often `analyze` is actually called from the decode chunk cadence.
    pub interval_ms: u32,
    /// MIDI notes of the lowest and highest bar (one bar per semitone),
    /// like the reference UI's "Notes: F#0 - A8".
    pub first_note: u8,
    pub last_note: u8,
    /// Reference pitch of A4 every note's frequency is derived from (e.g.
    /// 432 for alternative tunings) -- the reference UI's "Pitch (A4)".
    pub a4_hz: f32,
}

impl SpectrumConfig {
    /// One bar per note from `first_note` to `last_note`.
    pub fn bar_count(&self) -> usize {
        self.last_note.saturating_sub(self.first_note) as usize + 1
    }
}

impl Default for SpectrumConfig {
    fn default() -> Self {
        Self {
            channel_mode: SpectrumChannelMode::All,
            min_db: -80.0,
            max_db: 0.0,
            fft_size: 2048,
            peaks_enabled: true,
            peak_hold_ms: 250.0,
            gravity_db_per_s2: 50.0,
            interval_ms: 10,
            first_note: DEFAULT_FIRST_NOTE,
            last_note: DEFAULT_LAST_NOTE,
            a4_hz: 440.0,
        }
    }
}

/// Default MIDI notes of the lowest bar (F#0, ~23 Hz) and the highest (A8,
/// 7040 Hz) -- the reference UI's default "Notes: F#0 - A8" range.
pub const DEFAULT_FIRST_NOTE: u8 = 18;
pub const DEFAULT_LAST_NOTE: u8 = 117;

/// Frequency (Hz) of MIDI note `note` (fractional for band edges), tuned so
/// A4 (note 69) is `a4_hz`.
fn note_frequency(note: f32, a4_hz: f32) -> f32 {
    a4_hz * 2f32.powf((note - 69.0) / 12.0)
}

/// Live magnitude-spectrum analyzer: windows and FFTs a chunk of audio, then
/// maps the bins onto one bar per musical note (semitone) from
/// [`FIRST_NOTE`] to [`LAST_NOTE`], like a piano-roll analyzer. Also tracks a
/// falling peak marker per bar with hold-then-gravity physics.
pub struct SpectrumAnalyzer {
    config: SpectrumConfig,
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    buffer: Vec<Complex32>,
    mono_scratch: Vec<f32>,
    peak_db: Vec<f32>,
    peak_velocity: Vec<f32>,
    peak_hold_remaining_ms: Vec<f32>,
    last_analyze: Option<Instant>,
}

impl SpectrumAnalyzer {
    pub fn new(config: SpectrumConfig) -> Self {
        let mut analyzer = Self {
            config,
            fft: FftPlanner::new().plan_fft_forward(config.fft_size),
            window: Vec::new(),
            buffer: Vec::new(),
            mono_scratch: Vec::new(),
            peak_db: vec![config.min_db; config.bar_count()],
            peak_velocity: vec![0.0; config.bar_count()],
            peak_hold_remaining_ms: vec![0.0; config.bar_count()],
            last_analyze: None,
        };
        analyzer.rebuild_fft(config.fft_size);
        analyzer
    }

    pub fn set_config(&mut self, config: SpectrumConfig) {
        if config.fft_size != self.config.fft_size {
            self.rebuild_fft(config.fft_size);
        }
        if config.bar_count() != self.config.bar_count() {
            // A different note range: start the peak markers afresh.
            let bars = config.bar_count();
            self.peak_db = vec![config.min_db; bars];
            self.peak_velocity = vec![0.0; bars];
            self.peak_hold_remaining_ms = vec![0.0; bars];
        }
        self.config = config;
    }

    fn rebuild_fft(&mut self, fft_size: usize) {
        self.fft = FftPlanner::new().plan_fft_forward(fft_size);
        // Hann window to reduce spectral leakage from the chunk boundary.
        self.window = (0..fft_size)
            .map(|i| {
                0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (fft_size as f32 - 1.0)).cos()
            })
            .collect();
        self.buffer = vec![Complex32::new(0.0, 0.0); fft_size];
        self.mono_scratch = vec![0.0; fft_size];
    }

    pub fn fft_size(&self) -> usize {
        self.config.fft_size
    }

    /// How long until `due` turns true.
    pub fn time_until_due(&self) -> std::time::Duration {
        let interval = std::time::Duration::from_millis(self.config.interval_ms as u64);
        self.last_analyze
            .map_or(std::time::Duration::ZERO, |last| interval.saturating_sub(last.elapsed()))
    }

    /// Whether enough time has passed since the last update to honor
    /// `interval_ms` -- callers should skip `analyze` entirely when this is
    /// false, to avoid paying for FFTs nobody will see.
    pub fn due(&self) -> bool {
        match self.last_analyze {
            None => true,
            Some(last) => last.elapsed().as_secs_f32() * 1000.0 >= self.config.interval_ms as f32,
        }
    }

    /// Analyzes one chunk of interleaved PCM recorded at `sample_rate` and
    /// returns `(bars, peaks)`, each `bar_count()` magnitudes normalized to
    /// `[0, 1]` for typical program material (not strictly bounded -- loud
    /// peaks can exceed 1.0 before the dB clamp catches up). `peaks` is
    /// empty when disabled.
    pub fn analyze(
        &mut self,
        interleaved: &[f32],
        channels: usize,
        sample_rate: u32,
    ) -> (Vec<f32>, Vec<f32>) {
        let channels = channels.max(1);
        let fft_size = self.config.fft_size;
        let now = Instant::now();
        let dt_s = self
            .last_analyze
            .map(|last| now.duration_since(last).as_secs_f32())
            .unwrap_or(0.0);
        self.last_analyze = Some(now);

        self.mono_scratch.fill(0.0);
        let frames = (interleaved.len() / channels).min(fft_size);
        for (frame_idx, frame) in interleaved.chunks_exact(channels).take(frames).enumerate() {
            let sample = match self.config.channel_mode {
                SpectrumChannelMode::All => frame.iter().sum::<f32>() / channels as f32,
                SpectrumChannelMode::Left => frame[0],
                SpectrumChannelMode::Right => frame[channels.min(2) - 1],
            };
            self.mono_scratch[frame_idx] = sample * self.window[frame_idx];
        }
        for i in 0..fft_size {
            self.buffer[i] = Complex32::new(self.mono_scratch[i], 0.0);
        }
        self.fft.process(&mut self.buffer);

        let half = fft_size / 2;
        let normalize = (half as f32).sqrt().max(1.0);
        let range = (self.config.max_db - self.config.min_db).max(1.0);

        let bin_hz = sample_rate.max(1) as f32 / fft_size as f32;
        let mut bars = vec![0f32; self.config.bar_count()];
        for (bar, bar_value) in bars.iter_mut().enumerate() {
            let note = self.config.first_note as usize + bar;
            let (start, end) = note_bins(note, self.config.a4_hz, bin_hz, half);
            let mut peak_mag = 0f32;
            for bin in start..end {
                peak_mag = peak_mag.max(self.buffer[bin].norm() / normalize);
            }
            let db = 20.0 * peak_mag.max(1e-9).log10();
            *bar_value = ((db - self.config.min_db) / range).clamp(0.0, 1.0);

            if self.config.peaks_enabled {
                self.step_peak(bar, db, dt_s);
            }
        }

        let peaks = if self.config.peaks_enabled {
            self.peak_db
                .iter()
                .map(|&db| ((db - self.config.min_db) / range).clamp(0.0, 1.0))
                .collect()
        } else {
            Vec::new()
        };

        (bars, peaks)
    }

    fn step_peak(&mut self, bar: usize, db: f32, dt_s: f32) {
        if db >= self.peak_db[bar] {
            self.peak_db[bar] = db;
            self.peak_velocity[bar] = 0.0;
            self.peak_hold_remaining_ms[bar] = self.config.peak_hold_ms;
            return;
        }

        // Split dt across hold and fall so a step that straddles the hold
        // deadline still applies gravity for the leftover time, instead of
        // silently eating a whole tick of fall on the boundary step.
        let mut remaining_dt_s = dt_s;
        if self.peak_hold_remaining_ms[bar] > 0.0 {
            let hold_dt_s = (self.peak_hold_remaining_ms[bar] / 1000.0).min(remaining_dt_s);
            self.peak_hold_remaining_ms[bar] -= hold_dt_s * 1000.0;
            remaining_dt_s -= hold_dt_s;
            if remaining_dt_s <= 0.0 {
                return;
            }
        }

        self.peak_velocity[bar] += self.config.gravity_db_per_s2 * remaining_dt_s;
        self.peak_db[bar] -= self.peak_velocity[bar] * remaining_dt_s;
        if self.peak_db[bar] < db {
            self.peak_db[bar] = db;
            self.peak_velocity[bar] = 0.0;
        }
        if self.peak_db[bar] < self.config.min_db {
            self.peak_db[bar] = self.config.min_db;
            self.peak_velocity[bar] = 0.0;
        }
    }
}

/// FFT bins `start..end` covering `note`'s band (a quarter-tone either side
/// of its pitch). Low notes are narrower than one bin at typical FFT sizes,
/// so a band always gets at least the bin nearest its pitch -- neighbouring
/// bass notes may then share a bin, as on any FFT-based note analyzer.
fn note_bins(note: usize, a4_hz: f32, bin_hz: f32, half: usize) -> (usize, usize) {
    let note = note as f32;
    let lo = (note_frequency(note - 0.5, a4_hz) / bin_hz).ceil() as usize;
    let hi = (note_frequency(note + 0.5, a4_hz) / bin_hz).ceil() as usize;
    if lo < hi {
        (lo.min(half), hi.min(half))
    } else {
        let nearest = ((note_frequency(note, a4_hz) / bin_hz).round() as usize).min(half.saturating_sub(1));
        (nearest, nearest + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peak_holds_then_falls_with_accelerating_gravity() {
        let config = SpectrumConfig {
            peak_hold_ms: 100.0,
            gravity_db_per_s2: 50.0,
            min_db: -80.0,
            max_db: 0.0,
            ..SpectrumConfig::default()
        };
        let mut analyzer = SpectrumAnalyzer::new(config);

        analyzer.step_peak(0, -10.0, 0.0);
        assert_eq!(analyzer.peak_db[0], -10.0);

        // Still within hold time: peak must not move even though the
        // "signal" dropped.
        analyzer.step_peak(0, -40.0, 0.05);
        assert_eq!(analyzer.peak_db[0], -10.0);

        // Past hold time: gravity should now be pulling the peak down, and
        // accelerating (velocity grows), so each successive step falls
        // further than the last.
        analyzer.step_peak(0, -40.0, 0.06);
        let after_first_fall = analyzer.peak_db[0];
        assert!(after_first_fall < -10.0);

        let drop_1 = -10.0 - after_first_fall;
        analyzer.step_peak(0, -40.0, 0.05);
        let drop_2 = after_first_fall - analyzer.peak_db[0];
        assert!(
            drop_2 > drop_1,
            "gravity should accelerate: {drop_2} should exceed {drop_1}"
        );
    }

    #[test]
    fn peak_snaps_back_up_when_signal_rises_above_it() {
        let mut analyzer = SpectrumAnalyzer::new(SpectrumConfig::default());
        analyzer.step_peak(0, -20.0, 0.0);
        analyzer.step_peak(0, -5.0, 0.3);
        assert_eq!(analyzer.peak_db[0], -5.0);
    }

    #[test]
    fn a_pure_tone_peaks_at_its_own_note() {
        let mut analyzer = SpectrumAnalyzer::new(SpectrumConfig {
            fft_size: 8192,
            ..SpectrumConfig::default()
        });
        let rate = 44_100.0;
        let tone: Vec<f32> = (0..8192)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / rate).sin() * 0.5)
            .collect();
        let (bars, _) = analyzer.analyze(&tone, 1, 44_100);
        assert_eq!(bars.len(), SpectrumConfig::default().bar_count());
        let loudest = bars
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap()
            .0;
        assert_eq!(DEFAULT_FIRST_NOTE as usize + loudest, 69, "A4 is MIDI note 69");
    }

    fn loudest_note(config: SpectrumConfig, tone_hz: f32) -> usize {
        let mut analyzer = SpectrumAnalyzer::new(config);
        let tone: Vec<f32> = (0..8192)
            .map(|i| (2.0 * std::f32::consts::PI * tone_hz * i as f32 / 44_100.0).sin() * 0.5)
            .collect();
        let (bars, _) = analyzer.analyze(&tone, 1, 44_100);
        let loudest = bars.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
        config.first_note as usize + loudest
    }

    #[test]
    fn note_range_and_reference_pitch_are_configurable() {
        // A narrower range: C4 (60) to C6 (84) -> 25 bars, and A4 still
        // lands on note 69.
        let range = SpectrumConfig {
            fft_size: 8192,
            first_note: 60,
            last_note: 84,
            ..SpectrumConfig::default()
        };
        assert_eq!(range.bar_count(), 25);
        assert_eq!(loudest_note(range, 440.0), 69);
        // Tuned to A4 = 432 Hz, a 432 Hz tone is A4 too...
        let tuned = SpectrumConfig { a4_hz: 432.0, ..range };
        assert_eq!(loudest_note(tuned, 432.0), 69);
        // ...and changing the range resizes the peak markers.
        let mut analyzer = SpectrumAnalyzer::new(SpectrumConfig::default());
        analyzer.set_config(range);
        let silence = vec![0.0; 8192];
        let (bars, peaks) = analyzer.analyze(&silence, 1, 44_100);
        assert_eq!((bars.len(), peaks.len()), (25, 25));
    }
}
