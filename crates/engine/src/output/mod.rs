mod cpal_sink;
#[cfg(target_os = "linux")]
mod realtime;

pub use cpal_sink::{default_output_device, output_devices, CpalSink, OutputDeviceInfo};
