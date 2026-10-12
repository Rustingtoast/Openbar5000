use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};
use std::sync::Arc;

/// Live magnitude-spectrum analyzer: windows and FFTs a chunk of mono audio,
/// then groups the resulting bins into a fixed number of log-scaled bars
/// (more resolution in the bass, matching how spectrum visualizers usually
/// read) for display.
pub struct SpectrumAnalyzer {
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    buffer: Vec<Complex32>,
    mono_scratch: Vec<f32>,
    bar_count: usize,
    fft_size: usize,
}

impl SpectrumAnalyzer {
    pub fn new(fft_size: usize, bar_count: usize) -> Self {
        let fft = FftPlanner::new().plan_fft_forward(fft_size);
        // Hann window to reduce spectral leakage from the chunk boundary.
        let window: Vec<f32> = (0..fft_size)
            .map(|i| {
                0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (fft_size as f32 - 1.0)).cos()
            })
            .collect();
        Self {
            fft,
            window,
            buffer: vec![Complex32::new(0.0, 0.0); fft_size],
            mono_scratch: vec![0.0; fft_size],
            bar_count,
            fft_size,
        }
    }

    /// Analyzes one chunk of interleaved PCM (any channel count) and returns
    /// `bar_count` magnitudes in roughly `[0.0, 1.0]` for typical program
    /// material (not strictly bounded -- loud peaks can exceed 1.0).
    pub fn analyze(&mut self, interleaved: &[f32], channels: usize) -> Vec<f32> {
        let channels = channels.max(1);
        self.mono_scratch.fill(0.0);

        let frames = (interleaved.len() / channels).min(self.fft_size);
        for (frame_idx, frame) in interleaved.chunks_exact(channels).take(frames).enumerate() {
            let mono = frame.iter().sum::<f32>() / channels as f32;
            self.mono_scratch[frame_idx] = mono * self.window[frame_idx];
        }
        for i in 0..self.fft_size {
            self.buffer[i] = Complex32::new(self.mono_scratch[i], 0.0);
        }

        self.fft.process(&mut self.buffer);

        let half = self.fft_size / 2;
        let normalize = (half as f32).sqrt().max(1.0);
        let mut bars = vec![0f32; self.bar_count];
        for (bar, bar_value) in bars.iter_mut().enumerate() {
            let start = bin_edge(bar, self.bar_count, half);
            let end = bin_edge(bar + 1, self.bar_count, half).max(start + 1);
            let mut peak = 0f32;
            for bin in start..end.min(half) {
                peak = peak.max(self.buffer[bin].norm() / normalize);
            }
            *bar_value = peak;
        }
        bars
    }
}

/// Log-scaled bin boundary so lower bars cover fewer, lower-frequency bins
/// and higher bars sweep across much wider (higher-frequency) ranges.
fn bin_edge(bar: usize, bar_count: usize, half: usize) -> usize {
    let t = bar as f32 / bar_count as f32;
    ((half as f32).powf(t)).round() as usize
}
