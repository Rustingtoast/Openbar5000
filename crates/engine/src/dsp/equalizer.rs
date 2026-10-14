use plugin_api::Dsp;

/// Centre frequencies of the 18 graphic-equalizer bands (issue #7) -- the
/// same half-octave set foobar2000 uses, so its `.feq` presets carry over.
pub const EQ_BANDS_HZ: [f64; EQ_BAND_COUNT] = [
    55.0, 77.0, 110.0, 156.0, 220.0, 311.0, 440.0, 622.0, 880.0, 1200.0, 1800.0, 2500.0, 3500.0,
    5000.0, 7000.0, 10000.0, 14000.0, 20000.0,
];
pub const EQ_BAND_COUNT: usize = 18;
/// Each band's gain is limited to ±this many dB.
pub const EQ_MAX_DB: i32 = 20;

/// Each band's bandwidth: the bands sit half an octave apart, and at half
/// an octave wide neighbouring bands meet at half gain, so raising them all
/// alike gives a fairly even boost instead of a row of humps.
const BANDWIDTH_OCTAVES: f64 = 0.5;

/// What the equalizer should do: per-band gains in whole dB, and whether
/// it's on at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EqualizerSettings {
    pub enabled: bool,
    pub gains_db: [i32; EQ_BAND_COUNT],
}

impl EqualizerSettings {
    /// Whether processing would change anything.
    pub fn is_active(&self) -> bool {
        self.enabled && self.gains_db.iter().any(|&gain| gain != 0)
    }
}

/// Peaking-filter coefficients (RBJ audio-EQ cookbook), normalised so
/// `a0 == 1`.
#[derive(Debug, Clone, Copy)]
struct Coefficients {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

impl Coefficients {
    fn peaking(frequency: f64, gain_db: f64, sample_rate: f64) -> Self {
        let a = 10f64.powf(gain_db / 40.0);
        let w0 = std::f64::consts::TAU * frequency / sample_rate;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin * (std::f64::consts::LN_2 / 2.0 * BANDWIDTH_OCTAVES * w0 / sin).sinh();
        let a0 = 1.0 + alpha / a;
        Self {
            b0: (1.0 + alpha * a) / a0,
            b1: -2.0 * cos / a0,
            b2: (1.0 - alpha * a) / a0,
            a1: -2.0 * cos / a0,
            a2: (1.0 - alpha / a) / a0,
        }
    }
}

/// One band: its coefficients and, per channel, the filter's memory
/// (transposed direct form II, in f64 so low bands at high sample rates
/// stay clean).
struct Band {
    /// Index into [`EQ_BANDS_HZ`].
    index: usize,
    coefficients: Coefficients,
    state: Vec<[f64; 2]>,
}

/// The 18-band graphic equalizer stage in the playback chain.
pub struct EqualizerDsp {
    settings: EqualizerSettings,
    sample_rate: u32,
    channels: u16,
    /// Only the bands with a non-zero gain (that a flat band would leave
    /// the signal alone, so it isn't worth the work).
    bands: Vec<Band>,
}

impl Default for EqualizerDsp {
    fn default() -> Self {
        Self::new()
    }
}

impl EqualizerDsp {
    pub fn new() -> Self {
        Self {
            settings: EqualizerSettings::default(),
            sample_rate: 44_100,
            channels: 2,
            bands: Vec::new(),
        }
    }

    pub fn set_settings(&mut self, settings: EqualizerSettings) {
        if settings != self.settings {
            self.settings = settings;
            self.rebuild();
        }
    }

    /// Adapts to a new track's format.
    pub fn set_format(&mut self, sample_rate: u32, channels: u16) {
        if sample_rate != self.sample_rate || channels != self.channels {
            self.sample_rate = sample_rate;
            self.channels = channels;
            self.rebuild();
        }
    }

    /// Recomputes the filters, keeping each band's memory where it can so a
    /// gain change mid-song doesn't click.
    fn rebuild(&mut self) {
        let channels = self.channels.max(1) as usize;
        let sample_rate = self.sample_rate.max(1) as f64;
        let mut old = std::mem::take(&mut self.bands);
        if !self.settings.enabled {
            return;
        }
        for (index, (&frequency, &gain)) in EQ_BANDS_HZ.iter().zip(&self.settings.gains_db).enumerate() {
            // Bands at or above Nyquist can't be represented -- e.g. 20 kHz
            // in a 32 kHz file.
            if gain == 0 || frequency >= sample_rate * 0.49 {
                continue;
            }
            let gain = gain.clamp(-EQ_MAX_DB, EQ_MAX_DB) as f64;
            let state = old
                .iter_mut()
                .find(|band| band.index == index)
                .map(|band| std::mem::take(&mut band.state))
                .filter(|state| state.len() == channels)
                .unwrap_or_else(|| vec![[0.0; 2]; channels]);
            self.bands.push(Band {
                index,
                coefficients: Coefficients::peaking(frequency, gain, sample_rate),
                state,
            });
        }
    }
}

impl Dsp for EqualizerDsp {
    fn name(&self) -> &str {
        "Equalizer"
    }

    fn process(&mut self, samples: &mut [f32], channels: u16) {
        if self.bands.is_empty() {
            return;
        }
        let channels = channels.max(1) as usize;
        for frame in samples.chunks_exact_mut(channels) {
            for (channel, sample) in frame.iter_mut().enumerate() {
                let mut x = *sample as f64;
                for band in self.bands.iter_mut() {
                    let Some(state) = band.state.get_mut(channel) else {
                        continue;
                    };
                    let c = band.coefficients;
                    let y = c.b0 * x + state[0];
                    state[0] = c.b1 * x - c.a1 * y + state[1];
                    state[1] = c.b2 * x - c.a2 * y;
                    x = y;
                }
                // Boosts can push past full scale; clip here rather than
                // let the output wrap or distort unpredictably.
                *sample = (x as f32).clamp(-1.0, 1.0);
            }
        }
    }

    fn reset(&mut self) {
        for band in self.bands.iter_mut() {
            band.state.iter_mut().for_each(|state| *state = [0.0; 2]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 44_100;

    /// RMS level, in dB, of a stereo sine at `frequency` after the EQ (the
    /// first half second skipped while the filters settle).
    fn level_after(eq: &mut EqualizerDsp, frequency: f64) -> f64 {
        let frames = RATE as usize;
        let mut samples: Vec<f32> = (0..frames)
            .flat_map(|i| {
                let s = (std::f64::consts::TAU * frequency * i as f64 / RATE as f64).sin() as f32 * 0.01;
                [s, s]
            })
            .collect();
        eq.reset();
        eq.process(&mut samples, 2);
        let tail = &samples[frames..];
        let rms = (tail.iter().map(|&s| (s as f64).powi(2)).sum::<f64>() / tail.len() as f64).sqrt();
        20.0 * (rms / (0.01 / 2f64.sqrt())).log10()
    }

    fn eq_with(gains: &[(usize, i32)]) -> EqualizerDsp {
        let mut settings = EqualizerSettings {
            enabled: true,
            ..Default::default()
        };
        for &(band, gain) in gains {
            settings.gains_db[band] = gain;
        }
        let mut eq = EqualizerDsp::new();
        eq.set_format(RATE, 2);
        eq.set_settings(settings);
        eq
    }

    #[test]
    fn a_flat_or_disabled_equalizer_leaves_the_signal_alone() {
        let mut eq = eq_with(&[]);
        let mut samples = vec![0.25f32, -0.5, 0.75, -1.0];
        eq.process(&mut samples, 2);
        assert_eq!(samples, [0.25, -0.5, 0.75, -1.0]);

        let mut eq = eq_with(&[(6, 12)]);
        eq.set_settings(EqualizerSettings {
            enabled: false,
            ..eq.settings
        });
        eq.process(&mut samples, 2);
        assert_eq!(samples, [0.25, -0.5, 0.75, -1.0]);
    }

    #[test]
    fn a_band_boosts_and_cuts_its_own_frequency() {
        // 440 Hz is band 6.
        let boosted = level_after(&mut eq_with(&[(6, 6)]), 440.0);
        assert!((5.5..7.0).contains(&boosted), "+6 dB band gave {boosted:.2} dB");
        let cut = level_after(&mut eq_with(&[(6, -12)]), 440.0);
        assert!((-12.5..-11.0).contains(&cut), "-12 dB band gave {cut:.2} dB");
        // Far away from the band, nothing changes.
        let elsewhere = level_after(&mut eq_with(&[(6, 6)]), 7000.0);
        assert!(elsewhere.abs() < 0.1, "7 kHz moved by {elsewhere:.2} dB");
    }

    #[test]
    fn boosts_clip_at_full_scale() {
        let mut eq = eq_with(&[(9, 20)]);
        let mut samples: Vec<f32> = (0..20_000)
            .map(|i| (std::f64::consts::TAU * 1200.0 * i as f64 / RATE as f64).sin() as f32 * 0.9)
            .collect();
        eq.process(&mut samples, 1);
        assert!(samples.iter().all(|s| (-1.0..=1.0).contains(s)));
    }

    #[test]
    fn bands_above_nyquist_are_skipped() {
        let mut eq = eq_with(&[(17, 10)]);
        eq.set_format(32_000, 2);
        assert!(eq.bands.is_empty(), "20 kHz can't exist at 32 kHz");
        eq.set_format(48_000, 2);
        assert_eq!(eq.bands.len(), 1);
    }
}
