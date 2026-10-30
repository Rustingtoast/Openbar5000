//! Whether the displays are on, as the system reports it. While they're
//! off -- the screen timed out, or the PC was locked and its screen went
//! dark -- nothing on them can be seen, so the window is treated as out of
//! sight (see `wiring`). Windows says so even of a window that's still
//! open and on top; there, music playing on a locked PC would otherwise be
//! drawn at full rate for no one. Elsewhere the window system's own
//! occlusion events cover this, and the displays count as on.

/// Registered for as long as it's kept.
pub struct DisplayPower {
    #[cfg(windows)]
    registration: Option<windows_impl::Registration>,
}

impl DisplayPower {
    pub fn watch() -> Self {
        Self {
            #[cfg(windows)]
            registration: windows_impl::Registration::new()
                .inspect_err(|e| tracing::warn!("not told when the displays turn off: {e}"))
                .ok(),
        }
    }

    /// Whether the displays are known to be off.
    pub fn is_off(&self) -> bool {
        #[cfg(windows)]
        if self.registration.is_some() {
            return windows_impl::DISPLAYS_OFF.load(std::sync::atomic::Ordering::Relaxed);
        }
        false
    }
}

#[cfg(windows)]
mod windows_impl {
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, Ordering};
    use windows::Win32::Foundation::{ERROR_SUCCESS, HANDLE};
    use windows::Win32::System::Power::{
        PowerSettingRegisterNotification, PowerSettingUnregisterNotification, DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS,
        HPOWERNOTIFY, POWERBROADCAST_SETTING,
    };
    use windows::Win32::System::SystemServices::GUID_CONSOLE_DISPLAY_STATE;
    use windows::Win32::UI::WindowsAndMessaging::{DEVICE_NOTIFY_CALLBACK, PBT_POWERSETTINGCHANGE};

    /// The displays' state is the whole system's, so it's kept once for the
    /// process; the callback needs nothing else.
    pub static DISPLAYS_OFF: AtomicBool = AtomicBool::new(false);

    /// `GUID_CONSOLE_DISPLAY_STATE`'s value for "off" (1 is on, 2 dimmed --
    /// still showing).
    const DISPLAY_OFF: u32 = 0;

    pub struct Registration {
        handle: HPOWERNOTIFY,
        // Handed to the system by address: kept in place until unregistered.
        _parameters: Box<DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS>,
    }

    impl Registration {
        pub fn new() -> windows::core::Result<Self> {
            let parameters = Box::new(DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
                Callback: Some(display_state_changed),
                Context: std::ptr::null_mut(),
            });
            let mut handle = std::ptr::null_mut();
            // SAFETY: `parameters` outlives the registration (see `drop`).
            // The system calls back at once with the current state.
            unsafe {
                PowerSettingRegisterNotification(
                    &GUID_CONSOLE_DISPLAY_STATE,
                    DEVICE_NOTIFY_CALLBACK,
                    HANDLE(&*parameters as *const DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS as *mut c_void),
                    &mut handle,
                )
            }
            .ok()?;
            Ok(Self { handle: HPOWERNOTIFY(handle as isize), _parameters: parameters })
        }
    }

    impl Drop for Registration {
        fn drop(&mut self) {
            // SAFETY: registered in `new`.
            unsafe {
                let _ = PowerSettingUnregisterNotification(self.handle);
            }
            DISPLAYS_OFF.store(false, Ordering::Relaxed);
        }
    }

    /// Called by the system on a thread of its own.
    unsafe extern "system" fn display_state_changed(_context: *const c_void, kind: u32, setting: *const c_void) -> u32 {
        if kind == PBT_POWERSETTINGCHANGE && !setting.is_null() {
            // SAFETY: for this message, the system passes the setting that
            // changed, followed by `DataLength` bytes of its value.
            let setting = unsafe { &*setting.cast::<POWERBROADCAST_SETTING>() };
            if setting.PowerSetting == GUID_CONSOLE_DISPLAY_STATE && setting.DataLength as usize >= size_of::<u32>() {
                let state = unsafe { std::ptr::read_unaligned(setting.Data.as_ptr().cast::<u32>()) };
                DISPLAYS_OFF.store(state == DISPLAY_OFF, Ordering::Relaxed);
            }
        }
        ERROR_SUCCESS.0
    }
}
