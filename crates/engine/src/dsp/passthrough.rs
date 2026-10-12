use plugin_api::Dsp;

/// A no-op effect used as a placeholder slot in the DSP chain and in tests.
#[derive(Default)]
pub struct PassthroughDsp;

impl Dsp for PassthroughDsp {
    fn name(&self) -> &str {
        "Passthrough"
    }

    fn process(&mut self, _samples: &mut [f32], _channels: u16) {}

    fn reset(&mut self) {}
}
