mod cpal_sink;
#[cfg(windows)]
mod device_watch;
#[cfg(target_os = "linux")]
mod realtime;

pub use cpal_sink::{default_output_device, output_devices, CpalSink, OutputDeviceInfo};
#[cfg(windows)]
pub(crate) use device_watch::DeviceWatch;
