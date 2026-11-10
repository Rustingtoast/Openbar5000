//! Crash reports (issue #42): when the app crashes, a window shows what
//! happened, ready to copy into a GitHub issue.
//!
//! A crashed process can't be trusted to show anything, so a second copy of
//! the program watches it: started with `--crash-watch`, it waits on a pipe
//! the app writes its log and any panic to, idle until the app ends. If that
//! was a crash, it saves a report and shows it. On Windows the app's exit
//! code tells a crash from a normal end -- which also catches what no code
//! in the app gets to see: aborts, access violations, stack overflows. Linux
//! and macOS give a process's exit status only to its parent, so there the
//! app says so itself: a panic on its main thread, or a crash signal.
//!
//! The window takes no GPU (the software renderer draws it) and no focus,
//! and on Windows waits for a full-screen game to be left: a crash of the
//! app shouldn't disturb anything else running.

use std::fmt::Write as _;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::Mutex;

/// The watcher's argument, followed by the app's process id.
const WATCH_FLAG: &str = "--crash-watch";
/// Set to anything, the app runs without a watcher: under a debugger or a
/// profiler, say.
const DISABLE_VAR: &str = "OPENBAR5000_NO_CRASH_REPORTS";
/// Starts a line on the pipe that isn't the log's.
const CONTROL: char = '\u{1e}';
/// Log lines kept for a report.
const LOG_LINES: usize = 200;
/// Panics kept for a report: the first ones, which the rest tend to follow.
const PANICS: usize = 4;
/// Reports kept on disk.
const KEPT_REPORTS: usize = 10;
/// What waits for the watcher to start, at most.
const EARLY_BYTES: usize = 64 * 1024;

/// The app's end of the pipe to its watcher.
enum Pipe {
    /// The watcher is starting; what's for it waits here.
    Starting(Vec<u8>),
    Open(ChildStdin),
    /// No watcher.
    Closed,
}

static PIPE: Mutex<Pipe> = Mutex::new(Pipe::Starting(Vec::new()));

fn send(bytes: &[u8]) {
    // A watcher that's gone isn't the sender's concern.
    match &mut *PIPE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) {
        Pipe::Starting(early) if early.len() + bytes.len() <= EARLY_BYTES => early.extend_from_slice(bytes),
        Pipe::Open(pipe) => {
            let _ = pipe.write_all(bytes);
        }
        _ => {}
    }
}

// --- In the app ----------------------------------------------------------

/// Whether the app gets a watcher: unless turned off by `DISABLE_VAR`.
pub fn wanted() -> bool {
    std::env::var_os(DISABLE_VAR).is_none()
}

/// Starts the app's watcher and reports panics to it -- crashes too, where
/// the system says what crashed (see `native`) -- along with the log, which
/// goes to it through [`WatcherLog`]. Started on a thread of its own, as
/// starting a process takes a while (16 ms on Windows) and the window
/// doesn't need to wait for it; what comes before waits in `PIPE`.
pub fn start() {
    let started = std::thread::Builder::new().name("crash watcher start".into()).spawn(|| {
        let watcher = std::env::current_exe().and_then(|exe| {
            Command::new(exe)
                .arg(WATCH_FLAG)
                .arg(std::process::id().to_string())
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .spawn()
        });
        let mut pipe = PIPE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        match watcher.map(|mut watcher| watcher.stdin.take()) {
            Ok(Some(mut stdin)) => {
                if let Pipe::Starting(early) = &*pipe {
                    let _ = stdin.write_all(early);
                }
                native::install(&stdin);
                *pipe = Pipe::Open(stdin);
            }
            failed => {
                *pipe = Pipe::Closed;
                drop(pipe);
                let e = failed.err().unwrap_or_else(|| std::io::Error::other("no pipe to it"));
                tracing::warn!("crash reports are off: couldn't start the crash watcher: {e}");
            }
        }
    });
    if let Err(e) = started {
        *PIPE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Pipe::Closed;
        tracing::warn!("crash reports are off: {e}");
        return;
    }
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default_hook(info);
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("<unnamed>");
        // On the main thread a panic ends the app; elsewhere only its thread.
        let kind = if name == "main" { "fatal-panic" } else { "panic" };
        let backtrace = std::backtrace::Backtrace::force_capture();
        send_record(kind, &format!("thread '{name}' {info}\n\nstack backtrace:\n{backtrace}"));
    }));
}

fn send_record(kind: &str, text: &str) {
    let mut record = format!("{CONTROL}begin {kind}\n");
    for line in text.lines() {
        record.push_str(line);
        record.push('\n');
    }
    record.push_str(&format!("{CONTROL}end\n"));
    send(record.as_bytes());
}

/// The log's copy for the watcher (see `main::init_logging`).
#[derive(Default)]
pub struct WatcherLog;

impl Write for WatcherLog {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        send(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Notes of crashes no Rust code sees, written straight to the pipe by the
/// crashing thread. The heap may be what's broken by then, so nothing here
/// allocates.
#[cfg(windows)]
mod native {
    use super::CONTROL;
    use std::fmt::Write as _;
    use std::io::Write as _;
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use std::sync::atomic::{AtomicPtr, Ordering};
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::System::Diagnostics::Debug::{SetUnhandledExceptionFilter, EXCEPTION_POINTERS};
    use windows::Win32::System::LibraryLoader::{
        GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
        GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    };

    static PIPE: AtomicPtr<std::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());

    pub fn install(pipe: &std::process::ChildStdin) {
        PIPE.store(pipe.as_raw_handle(), Ordering::Release);
        // SAFETY: `on_crash` stays valid for the life of the process.
        unsafe { SetUnhandledExceptionFilter(Some(on_crash)) };
    }

    /// Notes the exception and which module it came from -- a graphics
    /// driver's, say -- which the exit code alone doesn't tell. Leaves the
    /// crash itself to go on as it would have.
    unsafe extern "system" fn on_crash(pointers: *const EXCEPTION_POINTERS) -> i32 {
        // Continues the search: to Windows Error Reporting, then the end.
        const EXCEPTION_CONTINUE_SEARCH: i32 = 0;
        // SAFETY: Windows hands over a valid record.
        let (code, address) = unsafe {
            let Some(record) = pointers.as_ref().and_then(|pointers| pointers.ExceptionRecord.as_ref()) else {
                return EXCEPTION_CONTINUE_SEARCH;
            };
            (record.ExceptionCode.0 as u32, record.ExceptionAddress as usize)
        };
        let mut module = HMODULE::default();
        let mut path = [0u16; 260];
        // SAFETY: plain calls; `address` is only looked up, not read.
        let path_len = unsafe {
            let flags = GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT;
            match GetModuleHandleExW(flags, PCWSTR(address as *const u16), &mut module) {
                Ok(()) => GetModuleFileNameW(Some(module), &mut path) as usize,
                Err(_) => 0,
            }
        };
        let path = &path[..path_len.min(path.len())];
        let file_name = path.rsplit(|&c| c == u16::from(b'\\')).next().unwrap_or(path);

        // Starting on a new line, in case the crash cut one short.
        let mut note = Line::default();
        let _ = write!(note, "\n{CONTROL}exception 0x{code:08X} at ");
        if file_name.is_empty() {
            let _ = write!(note, "0x{address:X}");
        } else {
            for c in char::decode_utf16(file_name.iter().copied()) {
                let _ = note.write_char(c.unwrap_or('?'));
            }
            let _ = write!(note, "+0x{:X}", address.wrapping_sub(module.0 as usize));
        }
        let _ = note.write_char('\n');

        let pipe = PIPE.load(Ordering::Acquire);
        if !pipe.is_null() {
            // SAFETY: the pipe stays open; ManuallyDrop leaves it so.
            let mut pipe = std::mem::ManuallyDrop::new(unsafe { std::fs::File::from_raw_handle(pipe) });
            let _ = pipe.write_all(note.as_bytes());
        }
        EXCEPTION_CONTINUE_SEARCH
    }

    /// A line of text in a fixed buffer, cut short if it doesn't fit.
    struct Line {
        bytes: [u8; 512],
        len: usize,
    }

    impl Default for Line {
        fn default() -> Self {
            Self { bytes: [0; 512], len: 0 }
        }
    }

    impl Line {
        fn as_bytes(&self) -> &[u8] {
            &self.bytes[..self.len]
        }
    }

    impl std::fmt::Write for Line {
        fn write_str(&mut self, s: &str) -> std::fmt::Result {
            let n = s.len().min(self.bytes.len() - self.len);
            self.bytes[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
            self.len += n;
            Ok(())
        }
    }
}

/// Notes of crash signals, written straight to the pipe from the signal
/// handler -- `write` being one of the few calls allowed there.
#[cfg(unix)]
mod native {
    use std::os::fd::AsRawFd;
    use std::sync::atomic::{AtomicI32, Ordering};
    use std::sync::OnceLock;

    static PIPE: AtomicI32 = AtomicI32::new(-1);

    /// Each signal with its note.
    const SIGNALS: [(libc::c_int, &[u8]); 5] = [
        (libc::SIGSEGV, b"\n\x1esignal SIGSEGV (segmentation fault)\n"),
        (libc::SIGBUS, b"\n\x1esignal SIGBUS (bus error)\n"),
        (libc::SIGILL, b"\n\x1esignal SIGILL (illegal instruction)\n"),
        (libc::SIGFPE, b"\n\x1esignal SIGFPE (arithmetic error)\n"),
        (libc::SIGABRT, b"\n\x1esignal SIGABRT (abort)\n"),
    ];

    /// What each signal did before.
    static PREVIOUS: [OnceLock<libc::sigaction>; 5] =
        [OnceLock::new(), OnceLock::new(), OnceLock::new(), OnceLock::new(), OnceLock::new()];

    pub fn install(pipe: &std::process::ChildStdin) {
        PIPE.store(pipe.as_raw_fd(), Ordering::Release);
        let handler: extern "C" fn(libc::c_int, *mut libc::siginfo_t, *mut libc::c_void) = on_signal;
        for (index, (signal, _)) in SIGNALS.iter().enumerate() {
            // SAFETY: plain sigaction calls on zeroed structs.
            unsafe {
                let mut action: libc::sigaction = std::mem::zeroed();
                action.sa_sigaction = handler as libc::sighandler_t;
                // On the alternate stack Rust gives its threads, so a stack
                // overflow gets here too.
                action.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK;
                libc::sigemptyset(&mut action.sa_mask);
                let mut previous: libc::sigaction = std::mem::zeroed();
                if libc::sigaction(*signal, &action, &mut previous) == 0 {
                    let _ = PREVIOUS[index].set(previous);
                }
            }
        }
    }

    /// Notes the signal, then hands it on to whoever had it: Rust's own
    /// handler for SIGSEGV and SIGBUS, which tells a stack overflow; or the
    /// default, which ends the process -- once this returns, as a fault
    /// happens again, and abort() raises its signal again.
    extern "C" fn on_signal(signal: libc::c_int, info: *mut libc::siginfo_t, context: *mut libc::c_void) {
        let Some(index) = SIGNALS.iter().position(|(s, _)| *s == signal) else {
            return;
        };
        let pipe = PIPE.load(Ordering::Acquire);
        let note = SIGNALS[index].1;
        // SAFETY: async-signal-safe calls only.
        unsafe {
            if pipe >= 0 {
                libc::write(pipe, note.as_ptr().cast(), note.len());
            }
            match PREVIOUS[index].get() {
                Some(previous) if previous.sa_sigaction != libc::SIG_DFL && previous.sa_sigaction != libc::SIG_IGN => {
                    if previous.sa_flags & libc::SA_SIGINFO != 0 {
                        let handler: extern "C" fn(libc::c_int, *mut libc::siginfo_t, *mut libc::c_void) =
                            std::mem::transmute(previous.sa_sigaction);
                        handler(signal, info, context);
                    } else {
                        let handler: extern "C" fn(libc::c_int) = std::mem::transmute(previous.sa_sigaction);
                        handler(signal);
                    }
                }
                _ => {
                    let mut default: libc::sigaction = std::mem::zeroed();
                    default.sa_sigaction = libc::SIG_DFL;
                    libc::sigaction(signal, &default, std::ptr::null_mut());
                }
            }
        }
    }
}

// --- The watcher ---------------------------------------------------------

/// The app's process id, when this process is its watcher.
pub fn watcher_args() -> Option<u32> {
    let mut args = std::env::args_os().skip(1);
    if args.next()? != WATCH_FLAG {
        return None;
    }
    args.next()?.to_str()?.parse().ok()
}

/// Waits for the app to end, and reports it if that was a crash.
pub fn watch(app_pid: u32) {
    #[cfg(windows)]
    let app = windows_exit::App::open(app_pid);
    #[cfg(windows)]
    end_start_up_cursor();
    #[cfg(not(windows))]
    let _ = app_pid;

    let mut session = Session::default();
    let mut stdin = std::io::stdin().lock();
    let mut line = Vec::new();
    loop {
        line.clear();
        match stdin.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => session.add(&String::from_utf8_lossy(&line)),
        }
    }
    #[cfg(windows)]
    let crash = app.and_then(|app| app.exit_code()).and_then(describe_exit_code);
    #[cfg(not(windows))]
    let crash = session.crash();
    let Some(crash) = crash else {
        return;
    };
    let report = session.report(&crash, &system_description());
    let saved = save(&report);
    show(&crash, report, saved);
}

/// What the watcher has heard from the app.
#[derive(Default)]
struct Session {
    log: std::collections::VecDeque<String>,
    panics: Vec<String>,
    /// The panic being received.
    record: Option<String>,
    /// Notes from the crash itself: an exception and where, a signal.
    notes: Vec<String>,
    /// Off Windows: a panic on the main thread.
    #[cfg_attr(windows, allow(dead_code))]
    fatal: Option<String>,
    /// Off Windows: the last crash signal, and when it came.
    #[cfg_attr(windows, allow(dead_code))]
    signal: Option<(String, std::time::Instant)>,
}

impl Session {
    fn add(&mut self, line: &str) {
        let line = line.trim_end_matches(['\n', '\r']);
        if let Some(record) = &mut self.record {
            if line.strip_prefix(CONTROL) == Some("end") {
                let record = std::mem::take(record);
                self.record = None;
                if self.panics.len() < PANICS {
                    self.panics.push(record);
                }
            } else {
                record.push_str(line);
                record.push('\n');
            }
            return;
        }
        match line.strip_prefix(CONTROL) {
            Some(control) => {
                if let Some(kind) = control.strip_prefix("begin ") {
                    if kind == "fatal-panic" {
                        self.fatal.get_or_insert_with(|| "panicked".to_string());
                    }
                    self.record = Some(String::new());
                } else if let Some(signal) = control.strip_prefix("signal ") {
                    self.signal = Some((signal.to_string(), std::time::Instant::now()));
                    self.notes.push(format!("signal {signal}"));
                } else if let Some(exception) = control.strip_prefix("exception ") {
                    self.notes.push(format!("exception {exception}"));
                }
            }
            None if !line.is_empty() => {
                if self.log.len() == LOG_LINES {
                    self.log.pop_front();
                }
                self.log.push_back(line.to_string());
            }
            None => {}
        }
    }

    /// Off Windows, once the app ended: what shows it crashed. A signal only
    /// right before the end: one a library handles and carries on from (some
    /// use SIGSEGV so) wasn't a crash.
    #[cfg_attr(windows, allow(dead_code))]
    fn crash(&self) -> Option<String> {
        self.fatal.clone().or_else(|| {
            let (signal, at) = self.signal.as_ref()?;
            (at.elapsed() < std::time::Duration::from_secs(10)).then(|| format!("crashed with {signal}"))
        })
    }

    fn report(&self, crash: &str, system: &str) -> String {
        let version = crate::wiring::display_version(env!("CARGO_PKG_VERSION"));
        let mut report = format!("Openbar5000 {version} crash report\n");
        let _ = writeln!(report, "When: {}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S (UTC%:z)"));
        let _ = writeln!(report, "System: {system}");
        let _ = writeln!(report, "What happened: Openbar5000 {crash}");
        for note in &self.notes {
            let _ = writeln!(report, "  {note}");
        }
        for panic in &self.panics {
            let _ = write!(report, "\n--- Panic ---\n{panic}");
        }
        if !self.log.is_empty() {
            let _ = writeln!(report, "\n--- Log (last {} lines) ---", self.log.len());
            for line in &self.log {
                let _ = writeln!(report, "{line}");
            }
        }
        report
    }
}

/// What a Windows exit code says happened, if it was a crash: the code of a
/// panic or an abort, or an exception's. Others -- 0, and 1 from Task
/// Manager's "End task" -- are ends someone meant.
#[cfg_attr(not(windows), allow(dead_code))]
fn describe_exit_code(code: u32) -> Option<String> {
    let what = match code {
        // What Rust exits with after a panic.
        101 => "panicked (exit code 101)",
        3 => "aborted (exit code 3)",
        0xC0000005 => "crashed with an access violation (0xC0000005)",
        0xC00000FD => "crashed with a stack overflow (0xC00000FD)",
        // Rust's abort: a panic while panicking, or running out of memory.
        0xC0000409 => "aborted (0xC0000409: a fail-fast exit, as after running out of memory)",
        0xC0000374 => "crashed with a corrupted heap (0xC0000374)",
        0xC000001D => "crashed with an illegal instruction (0xC000001D)",
        // Ctrl+C, or the console it was started from closed.
        0xC000013A => return None,
        // The system's own error codes. Not 0xFFFFFFFF, say, what .NET's
        // Process.Kill (PowerShell's Stop-Process) ends a process with.
        code if code & 0xF000_0000 == 0xC000_0000 => return Some(format!("crashed with exception 0x{code:08X}")),
        _ => return None,
    };
    Some(what.to_string())
}

/// Windows shows the "app starting" cursor after starting a program until
/// it looks for window messages, or for a few seconds: here a while after
/// the app is up already. Looking once ends it.
#[cfg(windows)]
fn end_start_up_cursor() {
    use windows::Win32::UI::WindowsAndMessaging::{PeekMessageW, MSG, PM_NOREMOVE};
    let mut message = MSG::default();
    // SAFETY: plain call; nothing is taken off the (empty) queue.
    let _ = unsafe { PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE) };
}

#[cfg(windows)]
mod windows_exit {
    use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    };

    pub struct App(HANDLE);

    impl App {
        /// Opened right away, while the app -- which just started the
        /// watcher -- is certainly still the process with that id.
        pub fn open(pid: u32) -> Option<Self> {
            // SAFETY: plain call.
            unsafe { OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
                .ok()
                .map(Self)
        }

        /// Once the app has ended: its pipe closing says it's ending.
        pub fn exit_code(&self) -> Option<u32> {
            // SAFETY: plain calls on an open handle.
            unsafe {
                if WaitForSingleObject(self.0, 10_000) != WAIT_OBJECT_0 {
                    return None;
                }
                let mut code = 0;
                GetExitCodeProcess(self.0, &mut code).ok()?;
                Some(code)
            }
        }
    }

    impl Drop for App {
        fn drop(&mut self) {
            // SAFETY: the handle is ours.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

/// The system, for the report.
#[cfg(windows)]
fn system_description() -> String {
    use windows::Wdk::System::SystemServices::RtlGetVersion;
    use windows::Win32::System::SystemInformation::OSVERSIONINFOW;
    let mut info = OSVERSIONINFOW {
        dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    // SAFETY: a sized struct to fill. (Unlike GetVersionEx, this isn't
    // capped at what the program's manifest declares.)
    let _ = unsafe { RtlGetVersion(&mut info) };
    let name = match (info.dwMajorVersion, info.dwBuildNumber) {
        (10, build) if build >= 22000 => "11".to_string(),
        (10, _) => "10".to_string(),
        (major, _) => format!("{major}.{}", info.dwMinorVersion),
    };
    format!("Windows {name} (build {}), {}", info.dwBuildNumber, std::env::consts::ARCH)
}

#[cfg(target_os = "macos")]
fn system_description() -> String {
    let version = Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|version| version.trim().to_string())
        .unwrap_or_default();
    format!("macOS {version}, {}", std::env::consts::ARCH)
}

/// With how the desktop runs it: X11 or Wayland, which desktop, Flatpak.
#[cfg(all(unix, not(target_os = "macos")))]
fn system_description() -> String {
    // In a Flatpak, /etc is the runtime's; the host's is here.
    let os = ["/run/host/os-release", "/etc/os-release"]
        .iter()
        .find_map(|path| std::fs::read_to_string(path).ok())
        .and_then(|release| {
            release
                .lines()
                .find_map(|line| Some(line.strip_prefix("PRETTY_NAME=")?.trim_matches('"').to_string()))
        })
        .unwrap_or_else(|| "Linux".to_string());
    let var = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
    let mut description = format!("{os}, {}", std::env::consts::ARCH);
    if let Some(session) = var("XDG_SESSION_TYPE") {
        let _ = write!(description, ", {session}");
    }
    if let Some(desktop) = var("XDG_CURRENT_DESKTOP") {
        let _ = write!(description, ", {desktop}");
    }
    if std::env::var_os("FLATPAK_ID").is_some() {
        description.push_str(", Flatpak");
    }
    description
}

/// Saves `report` with the others, keeping the newest few.
fn save(report: &str) -> Option<PathBuf> {
    let dir = crate::settings::config_dir().join("crash-reports");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("crash-{}.txt", chrono::Local::now().format("%Y-%m-%d_%H-%M-%S")));
    std::fs::write(&path, report).ok()?;
    prune(&dir);
    Some(path)
}

fn prune(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    // Named by time, so by name is oldest first.
    let mut reports: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            let name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default();
            name.starts_with("crash-") && name.ends_with(".txt")
        })
        .collect();
    reports.sort();
    let excess = reports.len().saturating_sub(KEPT_REPORTS);
    for old in &reports[..excess] {
        let _ = std::fs::remove_file(old);
    }
}

/// Shows the report, once nothing full screen would be disturbed by it.
fn show(crash: &str, report: String, saved: Option<PathBuf>) {
    use slint::winit_030::{winit, WinitWindowAccessor};
    use slint::ComponentHandle;

    #[cfg(windows)]
    wait_while_full_screen();

    let selected = slint::BackendSelector::new()
        .renderer_name("software".into())
        // Never taking focus from what's in use: a game, say. Its task bar
        // entry asks for attention instead (below).
        .with_winit_window_attributes_hook(|attributes| attributes.with_active(false))
        .select();
    if let Err(e) = selected {
        eprintln!("crash report: couldn't set up the window system: {e}");
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = slint::set_xdg_app_id(crate::platform::desktop_id());

    let window = match ui::CrashReportWindow::new() {
        Ok(window) => window,
        Err(e) => {
            eprintln!("crash report: couldn't open its window: {e}");
            return;
        }
    };
    window.set_report(report.into());
    if let Some(saved) = &saved {
        window.set_saved_to(saved.display().to_string().into());
    }
    {
        let url = new_issue_url(crash);
        window.on_report_clicked(move || crate::platform::open_url(&url));
    }
    {
        let window_weak = window.as_weak();
        window.on_restart_clicked(move || {
            match std::env::current_exe().and_then(|exe| Command::new(exe).stdin(Stdio::null()).spawn()) {
                Ok(_) => {
                    if let Some(window) = window_weak.upgrade() {
                        let _ = window.hide();
                    }
                }
                Err(e) => eprintln!("crash report: couldn't restart Openbar5000: {e}"),
            }
        });
    }
    {
        let window_weak = window.as_weak();
        window.on_close_clicked(move || {
            if let Some(window) = window_weak.upgrade() {
                let _ = window.hide();
            }
        });
    }
    if let Err(e) = window.show() {
        eprintln!("crash report: couldn't show its window: {e}");
        return;
    }
    {
        let window_weak = window.as_weak();
        let _ = slint::spawn_local(async move {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            if let Ok(winit_window) = window.window().winit_window().await {
                winit_window.request_user_attention(Some(winit::window::UserAttentionType::Informational));
            }
        });
    }
    let _ = slint::run_event_loop();
}

/// Waits while a full-screen program is in front -- a game, or a video --
/// which a window showing up could knock out of full screen.
#[cfg(windows)]
fn wait_while_full_screen() {
    use windows::Win32::UI::Shell::{
        SHQueryUserNotificationState, QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN,
    };
    // SAFETY: plain call.
    while let Ok(QUNS_BUSY | QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_PRESENTATION_MODE) =
        unsafe { SHQueryUserNotificationState() }
    {
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}

/// A new GitHub issue for the crash, the report to be pasted in: too long
/// for a link, it's put on the clipboard instead.
fn new_issue_url(crash: &str) -> String {
    let title = format!("Crash: Openbar5000 {crash}");
    let body = "<!-- The crash report is on your clipboard: paste it here. \
                What were you doing when it happened? -->\n\n";
    format!(
        "{}/issues/new?title={}&body={}",
        env!("CARGO_PKG_REPOSITORY"),
        percent_encode(&title),
        percent_encode(body)
    )
}

fn percent_encode(text: &str) -> String {
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            encoded.push(byte as char);
        } else {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(lines: &[&str]) -> Session {
        let mut session = Session::default();
        for line in lines {
            session.add(&format!("{line}\n"));
        }
        session
    }

    #[test]
    fn panics_and_notes_are_told_from_the_log() {
        let session = session(&[
            "INFO renderer: Skia on Direct3D 12",
            "\u{1e}begin panic",
            "thread 'lyrics' panicked at src/lyrics.rs:1:1:",
            "oops",
            "\u{1e}end",
            "INFO later",
            "\u{1e}exception 0xC0000005 at nvwgf2umx.dll+0x1A2B",
        ]);
        assert_eq!(session.log, ["INFO renderer: Skia on Direct3D 12", "INFO later"]);
        assert_eq!(session.panics, ["thread 'lyrics' panicked at src/lyrics.rs:1:1:\noops\n"]);
        assert_eq!(session.notes, ["exception 0xC0000005 at nvwgf2umx.dll+0x1A2B"]);
        // A panic elsewhere than on the main thread doesn't end the app.
        assert_eq!(session.crash(), None);
    }

    #[test]
    fn a_main_thread_panic_or_a_crash_signal_is_fatal() {
        assert_eq!(
            session(&["\u{1e}begin fatal-panic", "x", "\u{1e}end"]).crash().as_deref(),
            Some("panicked")
        );
        let mut signalled = session(&["", "\u{1e}signal SIGSEGV (segmentation fault)"]);
        assert_eq!(signalled.crash().as_deref(), Some("crashed with SIGSEGV (segmentation fault)"));
        assert!(signalled.log.is_empty());
        // Long before the end, it was handled and the app carried on.
        let long_ago = std::time::Instant::now().checked_sub(std::time::Duration::from_secs(60));
        if let (Some((_, at)), Some(long_ago)) = (signalled.signal.as_mut(), long_ago) {
            *at = long_ago;
            assert_eq!(signalled.crash(), None);
        }
    }

    #[test]
    fn the_log_keeps_its_last_lines() {
        let lines: Vec<String> = (0..LOG_LINES + 5).map(|i| format!("line {i}")).collect();
        let session = session(&lines.iter().map(String::as_str).collect::<Vec<_>>());
        assert_eq!(session.log.len(), LOG_LINES);
        assert_eq!(session.log.front().map(String::as_str), Some("line 5"));
    }

    #[test]
    fn the_report_has_what_happened_then_the_panic_then_the_log() {
        let report = session(&["\u{1e}begin fatal-panic", "boom", "\u{1e}end", "INFO a"])
            .report("panicked (exit code 101)", "Windows 11 (build 26300), x86_64");
        let what = report.find("What happened: Openbar5000 panicked (exit code 101)").unwrap();
        let panic = report.find("--- Panic ---\nboom").unwrap();
        let log = report.find("--- Log (last 1 lines) ---\nINFO a").unwrap();
        assert!(report.contains("System: Windows 11 (build 26300), x86_64"));
        assert!(what < panic && panic < log);
    }

    #[test]
    fn exit_codes_tell_crashes_from_ends_someone_meant() {
        assert_eq!(describe_exit_code(0), None);
        // Task Manager's "End task", and PowerShell's Stop-Process.
        assert_eq!(describe_exit_code(1), None);
        assert_eq!(describe_exit_code(0xFFFF_FFFF), None);
        assert_eq!(describe_exit_code(0xC000013A), None);
        assert!(describe_exit_code(101).unwrap().starts_with("panicked"));
        assert!(describe_exit_code(0xC0000005).unwrap().contains("access violation"));
        assert_eq!(describe_exit_code(0xC0000142).as_deref(), Some("crashed with exception 0xC0000142"));
    }

    #[test]
    fn the_issue_link_is_encoded() {
        let url = new_issue_url("panicked (exit code 101)");
        assert!(url.starts_with("https://github.com/Rustingtoast/Openbar5000/issues/new?title=Crash%3A%20Openbar5000%20panicked%20%28exit%20code%20101%29&body="));
        assert!(!url[url.find("&body=").unwrap() + 6..].contains([' ', '\n', '<', '&']));
    }

    #[test]
    fn only_the_newest_reports_are_kept() {
        let dir = std::env::temp_dir().join(format!("openbar5000-crash-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..KEPT_REPORTS + 3 {
            std::fs::write(dir.join(format!("crash-2026-01-{i:02}_00-00-00.txt")), "").unwrap();
        }
        std::fs::write(dir.join("notes.txt"), "").unwrap();
        prune(&dir);
        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(left.len(), KEPT_REPORTS + 1);
        assert_eq!(left[0], "crash-2026-01-03_00-00-00.txt");
        assert_eq!(left.last().map(String::as_str), Some("notes.txt"));
    }
}
