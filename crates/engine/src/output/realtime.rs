//! Realtime scheduling for the audio callback thread on Linux.
//!
//! The output's own buffer is only a couple of PipeWire periods long
//! (~25ms), refilled by cpal's callback thread from our ring buffer. At
//! normal priority, a busy machine (a game, a build) can keep that thread
//! off the CPU long enough for the buffer to run dry: a click. Realtime
//! scheduling lets it preempt all of that, the way PipeWire's own threads
//! do. An unprivileged process gets it from RTKit, over the system bus --
//! or, inside a Flatpak, from the desktop portal, which asks RTKit for it.
//! Either may refuse (or be missing); playback then just carries on as
//! before.

use std::time::Duration;
use zbus::blocking::Connection;
use zbus::zvariant::OwnedValue;

/// The priority asked for, at most -- RTKit's own limit is usually 20, and
/// this stays well under PipeWire's threads, which this one feeds.
const PRIORITY: u32 = 10;

/// How long to wait for the callback thread to report its id.
const THREAD_ID_TIMEOUT: Duration = Duration::from_secs(2);

/// The calling thread's kernel id -- what RTKit and the portal take.
fn current_thread_id() -> i64 {
    // SAFETY: gettid has no preconditions.
    unsafe { libc::gettid() as i64 }
}

/// Handed to the audio callback, which reports its thread on every run;
/// the first report starts the request for realtime scheduling, the rest
/// do nothing. Dropped unreported (the stream never started), the request
/// is abandoned.
pub struct CallbackThread(Option<crossbeam_channel::Sender<i64>>);

impl CallbackThread {
    /// Cheap enough for every callback: after the first, an empty check.
    /// The first is a single send that never blocks.
    pub fn report(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.try_send(current_thread_id());
        }
    }
}

/// Starts a background request for realtime scheduling of the thread that
/// will run the audio callback; the callback reports which one that is.
/// Logs the outcome; never blocks the caller.
pub fn request_for_callback() -> CallbackThread {
    let (sender, receiver) = crossbeam_channel::bounded(1);
    let spawned = std::thread::Builder::new()
        .name("openbar5000-rt".into())
        .spawn(move || {
            let Ok(tid) = receiver.recv_timeout(THREAD_ID_TIMEOUT) else {
                tracing::debug!("audio callback never ran; not asking for realtime");
                return;
            };
            match make_realtime(tid) {
                Ok(priority) => tracing::info!("audio thread runs realtime (priority {priority})"),
                Err(e) => tracing::info!("audio thread stays at normal priority: {e}"),
            }
        });
    match spawned {
        Ok(_) => CallbackThread(Some(sender)),
        Err(e) => {
            tracing::debug!("couldn't start the realtime request: {e}");
            CallbackThread(None)
        }
    }
}

/// Makes thread `tid` of this process realtime, returning the priority
/// granted.
fn make_realtime(tid: i64) -> Result<u32, String> {
    let in_flatpak = std::path::Path::new("/.flatpak-info").exists();
    let service = if in_flatpak { Service::portal() } else { Service::rtkit() };
    let connection = if in_flatpak { Connection::session() } else { Connection::system() }
        .map_err(|e| format!("no D-Bus connection: {e}"))?;

    let max_priority: i32 = service.property(&connection, "MaxRealtimePriority")?;
    let max_rttime: i64 = service.property(&connection, "RTTimeUSecMax")?;
    let priority = PRIORITY.min(max_priority.max(0) as u32);
    if priority == 0 {
        return Err("realtime isn't allowed".to_string());
    }
    // RTKit only grants realtime to a process that caps how long its
    // realtime threads may run without sleeping, so a runaway one can't
    // freeze the machine. The callback sleeps between every period.
    limit_realtime_runtime(max_rttime)?;

    let result = if in_flatpak {
        // Ids as seen inside the sandbox; the portal translates them.
        let pid = std::process::id() as u64;
        service.call(&connection, "MakeThreadRealtimeWithPID", &(pid, tid as u64, priority))
    } else {
        service.call(&connection, "MakeThreadRealtime", &(tid as u64, priority))
    };
    result.map(|()| priority)
}

/// Sets `RLIMIT_RTTIME` to `max_us`, unless it's lower already.
fn limit_realtime_runtime(max_us: i64) -> Result<(), String> {
    let max_us = max_us.max(0) as libc::rlim_t;
    let mut limit = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
    // SAFETY: plain syscalls on a local struct.
    unsafe {
        if libc::getrlimit(libc::RLIMIT_RTTIME, &mut limit) != 0 {
            return Err("can't read RLIMIT_RTTIME".to_string());
        }
        if limit.rlim_max <= max_us && limit.rlim_cur <= max_us {
            return Ok(());
        }
        let capped = libc::rlimit {
            rlim_cur: limit.rlim_cur.min(max_us),
            rlim_max: limit.rlim_max.min(max_us),
        };
        if libc::setrlimit(libc::RLIMIT_RTTIME, &capped) != 0 {
            return Err("can't set RLIMIT_RTTIME".to_string());
        }
    }
    Ok(())
}

/// RTKit itself, or the portal's interface to it -- the same methods and
/// properties either way.
struct Service {
    destination: &'static str,
    path: &'static str,
    interface: &'static str,
}

impl Service {
    fn rtkit() -> Self {
        Self {
            destination: "org.freedesktop.RealtimeKit1",
            path: "/org/freedesktop/RealtimeKit1",
            interface: "org.freedesktop.RealtimeKit1",
        }
    }

    fn portal() -> Self {
        Self {
            destination: "org.freedesktop.portal.Desktop",
            path: "/org/freedesktop/portal/desktop",
            interface: "org.freedesktop.portal.Realtime",
        }
    }

    fn property<T>(&self, connection: &Connection, name: &str) -> Result<T, String>
    where
        T: TryFrom<OwnedValue>,
        T::Error: std::fmt::Display,
    {
        let reply = connection
            .call_method(
                Some(self.destination),
                self.path,
                Some("org.freedesktop.DBus.Properties"),
                "Get",
                &(self.interface, name),
            )
            .map_err(|e| format!("{} unavailable: {e}", self.interface))?;
        let value: OwnedValue = reply.body().deserialize().map_err(|e| format!("{name}: {e}"))?;
        T::try_from(value).map_err(|e| format!("{name}: {e}"))
    }

    fn call<B>(&self, connection: &Connection, method: &str, body: &B) -> Result<(), String>
    where
        B: zbus::export::serde::Serialize + zbus::zvariant::DynamicType,
    {
        connection
            .call_method(Some(self.destination), self.path, Some(self.interface), method, body)
            .map(|_| ())
            .map_err(|e| format!("{method} refused: {e}"))
    }
}
