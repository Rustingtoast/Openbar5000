//! Output device changes on Windows, as the system reports them: the
//! default output moving, or a device being plugged in, unplugged, enabled
//! or disabled. Otherwise the playback thread asks every couple of seconds
//! -- on Windows each time a call into the audio service, made even with
//! nothing playing.

use crossbeam_channel::{Receiver, Sender};
use std::time::Instant;
use windows::core::{implement, Result, PCWSTR};
use windows::Win32::Foundation::{PROPERTYKEY, RPC_E_CHANGED_MODE};
use windows::Win32::Media::Audio::{
    eConsole, eRender, EDataFlow, ERole, IMMDeviceEnumerator, IMMNotificationClient, IMMNotificationClient_Impl,
    MMDeviceEnumerator, DEVICE_STATE,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
};

/// Registered with the system for as long as it's kept; drop it on the
/// thread that created it.
pub struct DeviceWatch {
    enumerator: IMMDeviceEnumerator,
    client: IMMNotificationClient,
    changes: Receiver<Instant>,
    // Last, so COM is let go of after everything that uses it.
    _com: Com,
}

impl DeviceWatch {
    pub fn new() -> Result<Self> {
        let com = Com::init()?;
        // SAFETY: COM is initialized on this thread (`com`).
        let enumerator: IMMDeviceEnumerator = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
        // One pending message is enough: any number of changes in a row
        // call for a single check.
        let (sender, changes) = crossbeam_channel::bounded(1);
        let client: IMMNotificationClient = Notifier(sender).into();
        // SAFETY: `client` stays registered no longer than it's kept here.
        unsafe { enumerator.RegisterEndpointNotificationCallback(&client)? };
        Ok(Self { enumerator, client, changes, _com: com })
    }

    /// Gets a message (when it was sent) after any change that may call for
    /// playback to move to another device.
    pub fn changes(&self) -> &Receiver<Instant> {
        &self.changes
    }
}

impl Drop for DeviceWatch {
    fn drop(&mut self) {
        // SAFETY: registered in `new`. Waits for a callback in progress,
        // which only ever sends on a channel, so it can't wait on us.
        unsafe {
            let _ = self.enumerator.UnregisterEndpointNotificationCallback(&self.client);
        }
    }
}

/// Called by the system on a thread of its own, so it only passes the
/// news on. Changes to recording devices come through too; checking the
/// output once more for those is cheaper than telling them apart here.
#[implement(IMMNotificationClient)]
struct Notifier(Sender<Instant>);

impl Notifier {
    fn changed(&self) -> Result<()> {
        let _ = self.0.try_send(Instant::now());
        Ok(())
    }
}

impl IMMNotificationClient_Impl for Notifier_Impl {
    fn OnDeviceStateChanged(&self, _device: &PCWSTR, _state: DEVICE_STATE) -> Result<()> {
        self.changed()
    }

    fn OnDeviceAdded(&self, _device: &PCWSTR) -> Result<()> {
        self.changed()
    }

    fn OnDeviceRemoved(&self, _device: &PCWSTR) -> Result<()> {
        self.changed()
    }

    fn OnDefaultDeviceChanged(&self, flow: EDataFlow, role: ERole, _device: &PCWSTR) -> Result<()> {
        // The default output, as cpal picks it.
        if flow == eRender && role == eConsole {
            self.changed()?;
        }
        Ok(())
    }

    fn OnPropertyValueChanged(&self, _device: &PCWSTR, _key: &PROPERTYKEY) -> Result<()> {
        Ok(())
    }
}

/// COM, initialized on this thread the way cpal does it (single-threaded
/// apartment) unless it already was otherwise.
struct Com {
    initialized: bool,
}

impl Com {
    fn init() -> Result<Self> {
        // SAFETY: balanced by `drop` when it succeeds.
        let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if result == RPC_E_CHANGED_MODE {
            return Ok(Self { initialized: false });
        }
        result.ok()?;
        Ok(Self { initialized: true })
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.initialized {
            // SAFETY: matches the successful `CoInitializeEx` in `init`.
            unsafe { CoUninitialize() };
        }
    }
}
