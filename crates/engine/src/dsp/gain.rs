use plugin_api::Dsp;

/// The always-present volume stage at the end of the DSP chain (see the
/// milestone plan: volume is a built-in, not a plugin extension point).
pub struct GainDsp {
    gain: f32,
}

impl GainDsp {
    pub fn new(initial_volume: f32) -> Self {
        Self {
            gain: initial_volume.clamp(0.0, 1.0),
        }
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.gain = volume.clamp(0.0, 1.0);
    }
}

impl Dsp for GainDsp {
    fn name(&self) -> &str {
        "Volume"
    }

    fn process(&mut self, samples: &mut [f32], _channels: u16) {
        if self.gain == 1.0 {
            return;
        }
        for sample in samples.iter_mut() {
            *sample *= self.gain;
        }
    }

    fn reset(&mut self) {}
}
