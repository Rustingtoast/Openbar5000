use plugin_api::abi::{AbiStatus, DecoderVtable, DspVtable, OutputVtable, ServiceHandle};
use plugin_api::{AudioInfo, DecodeError, Decoder, Dsp, OutputError, OutputSink};
use std::time::Duration;

fn status_to_decode_error(status: AbiStatus) -> DecodeError {
    match status {
        AbiStatus::EndOfStream => DecodeError::EndOfStream,
        AbiStatus::IoError => DecodeError::IoError("plugin io error".into()),
        AbiStatus::CorruptStream => {
            DecodeError::CorruptStream("plugin reported corrupt stream".into())
        }
        AbiStatus::SeekUnsupported => DecodeError::SeekUnsupported,
        _ => DecodeError::UnsupportedFormat,
    }
}

fn status_to_output_error(status: AbiStatus) -> OutputError {
    match status {
        AbiStatus::DeviceUnavailable => {
            OutputError::DeviceUnavailable("plugin device unavailable".into())
        }
        AbiStatus::StreamError => OutputError::StreamError("plugin stream error".into()),
        _ => OutputError::UnsupportedConfig {
            sample_rate: 0,
            channels: 0,
        },
    }
}

/// Wraps a loaded plugin's [`DecoderVtable`] so engine code can use it as an
/// ordinary `Box<dyn Decoder>`, indistinguishable from a built-in decoder.
///
/// # Safety
/// The caller must guarantee `handle` came from a successfully negotiated
/// plugin entry call and that the owning `libloading::Library` outlives this
/// proxy (held by [`crate::LoadedPlugin`]).
pub struct DecoderProxy {
    handle: ServiceHandle<DecoderVtable>,
}

impl DecoderProxy {
    /// # Safety
    /// See struct-level docs.
    pub unsafe fn new(handle: ServiceHandle<DecoderVtable>) -> Self {
        Self { handle }
    }
}

impl Decoder for DecoderProxy {
    fn open(&mut self, path: &std::path::Path) -> Result<AudioInfo, DecodeError> {
        let path_str = path.to_string_lossy();
        let bytes = path_str.as_bytes();
        let mut info = AudioInfo {
            sample_rate: 0,
            channels: 0,
            bits_per_sample: 0,
            duration_ms: 0,
        };
        let vtable = unsafe { &*self.handle.vtable };
        let status =
            unsafe { (vtable.open)(self.handle.instance, bytes.as_ptr(), bytes.len(), &mut info) };
        match status {
            AbiStatus::Ok => Ok(info),
            other => Err(status_to_decode_error(other)),
        }
    }

    fn next_frame(&mut self, out: &mut [f32]) -> Result<usize, DecodeError> {
        let vtable = unsafe { &*self.handle.vtable };
        let mut written = 0usize;
        let status = unsafe {
            (vtable.next_frame)(
                self.handle.instance,
                out.as_mut_ptr(),
                out.len(),
                &mut written,
            )
        };
        match status {
            AbiStatus::Ok => Ok(written),
            AbiStatus::EndOfStream => Ok(0),
            other => Err(status_to_decode_error(other)),
        }
    }

    fn seek(&mut self, pos: Duration) -> Result<(), DecodeError> {
        let vtable = unsafe { &*self.handle.vtable };
        let status = unsafe { (vtable.seek)(self.handle.instance, pos.as_millis() as u64) };
        match status {
            AbiStatus::Ok => Ok(()),
            other => Err(status_to_decode_error(other)),
        }
    }

    fn info(&self) -> AudioInfo {
        let vtable = unsafe { &*self.handle.vtable };
        let mut info = AudioInfo {
            sample_rate: 0,
            channels: 0,
            bits_per_sample: 0,
            duration_ms: 0,
        };
        unsafe { (vtable.info)(self.handle.instance, &mut info) };
        info
    }
}

impl Drop for DecoderProxy {
    fn drop(&mut self) {
        let vtable = unsafe { &*self.handle.vtable };
        unsafe { (vtable.destroy)(self.handle.instance) };
    }
}

unsafe impl Send for DecoderProxy {}

/// Proxy for a loaded [`DspVtable`] service; see [`DecoderProxy`] for the
/// safety contract.
pub struct DspProxy {
    handle: ServiceHandle<DspVtable>,
    name_buf: [u8; 64],
}

impl DspProxy {
    /// # Safety
    /// See [`DecoderProxy`].
    pub unsafe fn new(handle: ServiceHandle<DspVtable>) -> Self {
        Self {
            handle,
            name_buf: [0; 64],
        }
    }
}

impl Dsp for DspProxy {
    fn name(&self) -> &str {
        let vtable = unsafe { &*self.handle.vtable };
        let len = unsafe {
            (vtable.name)(
                self.handle.instance,
                self.name_buf.as_ptr() as *mut u8,
                self.name_buf.len(),
            )
        };
        std::str::from_utf8(&self.name_buf[..len.min(self.name_buf.len())])
            .unwrap_or("<invalid plugin name>")
    }

    fn process(&mut self, samples: &mut [f32], channels: u16) {
        let vtable = unsafe { &*self.handle.vtable };
        unsafe {
            (vtable.process)(
                self.handle.instance,
                samples.as_mut_ptr(),
                samples.len(),
                channels,
            )
        };
    }

    fn reset(&mut self) {
        let vtable = unsafe { &*self.handle.vtable };
        unsafe { (vtable.reset)(self.handle.instance) };
    }
}

impl Drop for DspProxy {
    fn drop(&mut self) {
        let vtable = unsafe { &*self.handle.vtable };
        unsafe { (vtable.destroy)(self.handle.instance) };
    }
}

unsafe impl Send for DspProxy {}

/// Proxy for a loaded [`OutputVtable`] service; see [`DecoderProxy`] for the
/// safety contract.
pub struct OutputProxy {
    handle: ServiceHandle<OutputVtable>,
}

impl OutputProxy {
    /// # Safety
    /// See [`DecoderProxy`].
    pub unsafe fn new(handle: ServiceHandle<OutputVtable>) -> Self {
        Self { handle }
    }
}

impl OutputSink for OutputProxy {
    fn configure(&mut self, sample_rate: u32, channels: u16) -> Result<(), OutputError> {
        let vtable = unsafe { &*self.handle.vtable };
        let status = unsafe { (vtable.configure)(self.handle.instance, sample_rate, channels) };
        match status {
            AbiStatus::Ok => Ok(()),
            other => Err(status_to_output_error(other)),
        }
    }

    fn write(&mut self, samples: &[f32]) -> Result<(), OutputError> {
        let vtable = unsafe { &*self.handle.vtable };
        let status =
            unsafe { (vtable.write)(self.handle.instance, samples.as_ptr(), samples.len()) };
        match status {
            AbiStatus::Ok => Ok(()),
            other => Err(status_to_output_error(other)),
        }
    }
}

impl Drop for OutputProxy {
    fn drop(&mut self) {
        let vtable = unsafe { &*self.handle.vtable };
        unsafe { (vtable.destroy)(self.handle.instance) };
    }
}

unsafe impl Send for OutputProxy {}
