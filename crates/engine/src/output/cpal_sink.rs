use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Stream, StreamConfig};
use plugin_api::{OutputError, OutputSink};
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::HeapRb;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

/// The built-in [`OutputSink`], backed by `cpal`. Decoding/DSP happen on the
/// engine's control thread; this only ever *produces* into a lock-free ring
/// buffer -- the realtime audio callback (spawned by `configure`) is the sole
/// consumer and never locks, allocates, or blocks.
pub struct CpalSink {
    device: cpal::Device,
    /// The device's stable id (see [`OutputDeviceInfo::id`]), if it has one.
    device_id: Option<String>,
    device_name: String,
    stream: Option<Stream>,
    producer: Option<ringbuf::HeapProd<f32>>,
    /// Set to have the audio callback drop everything queued; it clears
    /// the flag once it has.
    flush_requested: Arc<AtomicBool>,
    /// Applied in the audio callback (f32 bits), so changes are heard at
    /// once instead of after the queued audio.
    volume: Arc<AtomicU32>,
    configured_rate: u32,
    configured_channels: u16,
}

/// An audio output the user can pick (Preferences > Output).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputDeviceInfo {
    /// Stable across runs and reconnects where the OS allows: what the
    /// choice is saved as.
    pub id: String,
    /// Human-readable, e.g. "MacBook Pro Speakers".
    pub name: String,
}

fn device_id(device: &cpal::Device) -> Option<String> {
    device.id().ok().map(|id| id.to_string())
}

fn device_name(device: &cpal::Device) -> String {
    device
        .description()
        .map(|description| description.name().to_string())
        .unwrap_or_else(|_| "Unknown device".to_string())
}

/// Every output device the system currently offers.
pub fn output_devices() -> Vec<OutputDeviceInfo> {
    let host = cpal::default_host();
    let Ok(devices) = host.output_devices() else {
        return Vec::new();
    };
    devices
        .filter_map(|device| {
            Some(OutputDeviceInfo {
                id: device_id(&device)?,
                name: device_name(&device),
            })
        })
        .collect()
}

/// The system's current default output.
pub fn default_output_device() -> Option<OutputDeviceInfo> {
    let device = cpal::default_host().default_output_device()?;
    Some(OutputDeviceInfo {
        id: device_id(&device)?,
        name: device_name(&device),
    })
}

/// The device with this id, if it's connected.
fn find_device(id: &str) -> Option<cpal::Device> {
    let id: cpal::DeviceId = id.parse().ok()?;
    cpal::default_host().device_by_id(&id)
}

impl CpalSink {
    pub fn default_device() -> Result<Self, OutputError> {
        Self::open(None)
    }

    /// Opens the device with `id`, or the system default for `None` -- or
    /// when that device isn't connected (see [`CpalSink::device_id`] to
    /// tell which one was opened).
    pub fn open(id: Option<&str>) -> Result<Self, OutputError> {
        let device = match id.and_then(find_device) {
            Some(device) => device,
            None => cpal::default_host()
                .default_output_device()
                .ok_or_else(|| OutputError::DeviceUnavailable("no default output device".into()))?,
        };
        Ok(Self {
            device_id: device_id(&device),
            device_name: device_name(&device),
            device,
            stream: None,
            producer: None,
            flush_requested: Arc::new(AtomicBool::new(false)),
            volume: Arc::new(AtomicU32::new(1.0f32.to_bits())),
            configured_rate: 0,
            configured_channels: 0,
        })
    }

    pub fn device_id(&self) -> Option<&str> {
        self.device_id.as_deref()
    }

    pub fn device_name(&self) -> &str {
        &self.device_name
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

        // ~0.3s of buffering: enough to absorb decode-thread jitter. (Pause,
        // seek and track changes flush it, and volume is applied on the way
        // out, so only effects like the EQ wait for it.)
        let capacity = (sample_rate as usize) * (channels as usize) * 3 / 10;
        let ring = HeapRb::<f32>::new(capacity.max(1024));
        let (producer, mut consumer) = ring.split();

        let config = StreamConfig {
            channels,
            sample_rate,
            buffer_size: cpal::BufferSize::Default,
        };

        let err_fn = |err| tracing::error!("cpal output stream error: {err}");
        let flush_requested = self.flush_requested.clone();
        flush_requested.store(false, Ordering::Release);
        let volume = self.volume.clone();
        // Realtime scheduling for the callback's thread, asked for once
        // it has run (see `realtime`).
        #[cfg(target_os = "linux")]
        let mut callback_thread = super::realtime::request_for_callback();

        let stream = self
            .device
            .build_output_stream(
                config,
                move |data: &mut [f32], _info| {
                    #[cfg(target_os = "linux")]
                    callback_thread.report();
                    if flush_requested.load(Ordering::Acquire) {
                        consumer.clear();
                        flush_requested.store(false, Ordering::Release);
                    }
                    let written = consumer.pop_slice(data);
                    let gain = f32::from_bits(volume.load(Ordering::Relaxed));
                    if gain != 1.0 {
                        for sample in &mut data[..written] {
                            *sample *= gain;
                        }
                    }
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

    fn free_samples(&self) -> Option<usize> {
        self.producer.as_ref().map(|p| p.vacant_len())
    }

    fn queued_samples(&self) -> usize {
        self.producer.as_ref().map_or(0, |p| p.occupied_len())
    }

    fn flush(&mut self) {
        if self.stream.is_none() {
            return;
        }
        self.flush_requested.store(true, Ordering::Release);
        // The callback runs every few milliseconds; wait for it so the
        // queue is really empty (and `queued_samples` says so) on return.
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(100);
        while self.flush_requested.load(Ordering::Acquire) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    fn set_volume(&mut self, volume: f32) -> bool {
        self.volume.store(volume.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
        true
    }
}
