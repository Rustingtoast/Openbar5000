use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Stream, StreamConfig};
use plugin_api::{OutputError, OutputSink};
use ringbuf::traits::{Consumer, Producer, Split};
use ringbuf::HeapRb;

/// The built-in [`OutputSink`], backed by `cpal`. Decoding/DSP happen on the
/// engine's control thread; this only ever *produces* into a lock-free ring
/// buffer -- the realtime audio callback (spawned by `configure`) is the sole
/// consumer and never locks, allocates, or blocks.
pub struct CpalSink {
    device: cpal::Device,
    stream: Option<Stream>,
    producer: Option<ringbuf::HeapProd<f32>>,
    configured_rate: u32,
    configured_channels: u16,
}

impl CpalSink {
    pub fn default_device() -> Result<Self, OutputError> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| OutputError::DeviceUnavailable("no default output device".into()))?;
        Ok(Self {
            device,
            stream: None,
            producer: None,
            configured_rate: 0,
            configured_channels: 0,
        })
    }
}

impl OutputSink for CpalSink {
    fn configure(&mut self, sample_rate: u32, channels: u16) -> Result<(), OutputError> {
        if self.configured_rate == sample_rate
            && self.configured_channels == channels
            && self.stream.is_some()
        {
            return Ok(());
        }

        // ~0.5s of buffering: enough to absorb decode-thread jitter without
        // adding noticeable output latency.
        let capacity = (sample_rate as usize) * (channels as usize) / 2;
        let ring = HeapRb::<f32>::new(capacity.max(1024));
        let (producer, mut consumer) = ring.split();

        let config = StreamConfig {
            channels,
            sample_rate,
            buffer_size: cpal::BufferSize::Default,
        };

        let err_fn = |err| tracing::error!("cpal output stream error: {err}");

        let stream = self
            .device
            .build_output_stream(
                config,
                move |data: &mut [f32], _info| {
                    let written = consumer.pop_slice(data);
                    for sample in &mut data[written..] {
                        *sample = 0.0;
                    }
                },
                err_fn,
                None,
            )
            .map_err(|_| OutputError::UnsupportedConfig {
                sample_rate,
                channels,
            })?;

        stream
            .play()
            .map_err(|e| OutputError::StreamError(e.to_string()))?;

        self.stream = Some(stream);
        self.producer = Some(producer);
        self.configured_rate = sample_rate;
        self.configured_channels = channels;
        Ok(())
    }

    fn write(&mut self, samples: &[f32]) -> Result<(), OutputError> {
        let producer = self
            .producer
            .as_mut()
            .ok_or_else(|| OutputError::StreamError("sink not configured".into()))?;

        let mut remaining = samples;
        while !remaining.is_empty() {
            let pushed = producer.push_slice(remaining);
            remaining = &remaining[pushed..];
            if pushed == 0 {
                // Ring buffer is full; back off briefly rather than busy-spinning.
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        Ok(())
    }
}
