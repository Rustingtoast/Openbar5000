use std::path::Path;
use std::process::Command;

/// The name the app's desktop entry is installed under, without the
/// `.desktop`: what Linux desktops go by to find the app's name and icon
/// for its windows (Wayland's app id, X11's WM_CLASS) and for the MPRIS
/// now-playing widget. Inside a Flatpak it's the Flatpak's own id; run
/// outside one (`cargo run`), `openbar5000`.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn desktop_id() -> String {
    std::env::var("FLATPAK_ID")
        .ok()
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| "openbar5000".to_string())
}

/// A file dialog attached to `window`, so it opens over it -- even over a
/// window kept on top, as Preferences and Properties are, which an
/// unattached one would open behind.
pub fn file_dialog_over(window: &slint::Window) -> rfd::FileDialog {
    rfd::FileDialog::new().set_parent(&window.window_handle())
}

/// Starts `work` on a thread of its own named `name`. When the system won't
/// give one -- short of memory or threads, as with a demanding game
/// running -- says so in the log and returns `false`, for the caller to
/// carry on without, instead of the app ending.
pub fn spawn(name: &str, work: impl FnOnce() + Send + 'static) -> bool {
    match std::thread::Builder::new().name(name.into()).spawn(work) {
        Ok(_) => true,
        Err(e) => {
            tracing::warn!("couldn't start {name}: {e}");
            false
        }
    }
}

/// Shows `window`, and -- if it was showing already, behind others --
/// brings it to the front with the keyboard focus. Wayland compositors
/// don't let a program raise its own window; a window shown anew does get
/// the focus there, so it's hidden and shown again (keeping what's in it).
pub fn show_in_front(window: &slint::Window) {
    use slint::winit_030::WinitWindowAccessor;
    if !window.is_visible() {
        if let Err(e) = window.show() {
            tracing::warn!("couldn't show a window: {e}");
        }
        return;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        use slint::winit_030::winit::platform::wayland::WindowExtWayland;
        let wayland = window.with_winit_window(|winit| winit.xdg_toplevel().is_some()).unwrap_or(false);
        if wayland {
            let _ = window.hide();
            if let Err(e) = window.show() {
                tracing::warn!("couldn't show a window: {e}");
            }
            return;
        }
    }
    window.with_winit_window(|winit| {
        if winit.is_minimized() == Some(true) {
            winit.set_minimized(false);
        }
        winit.focus_window();
    });
}

/// Opens folder `dir` in the file manager (made first if need be).
pub fn open_folder(dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
    #[cfg(target_os = "macos")]
    let result = Command::new("open").arg(dir).spawn();
    #[cfg(target_os = "windows")]
    let result = Command::new("explorer").arg(dir).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = Command::new("xdg-open").arg(dir).spawn();
    if let Err(e) = result {
        tracing::warn!("couldn't open {}: {e}", dir.display());
    }
}

/// Opens `url` in the default browser.
pub fn open_url(url: &str) {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return;
    }
    if let Err(e) = open_url_with_os(url) {
        tracing::warn!("failed to open {url}: {e}");
    }
}

#[cfg(target_os = "macos")]
fn open_url_with_os(url: &str) -> std::io::Result<()> {
    Command::new("open").arg(url).spawn().map(|_| ())
}

#[cfg(target_os = "windows")]
fn open_url_with_os(url: &str) -> std::io::Result<()> {
    // The shell's own "open", as a double-click does. Not `cmd /C start`:
    // run from the app, which has no console, that flashes one up -- and
    // `cmd` reads a `&` in the URL as the start of another command.
    use windows::core::{w, HSTRING, PCWSTR};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    // SAFETY: valid strings for the call's duration.
    let result = unsafe {
        ShellExecuteW(None, w!("open"), &HSTRING::from(url), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL)
    };
    // Anything above 32 means it worked.
    if result.0 as usize > 32 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn open_url_with_os(url: &str) -> std::io::Result<()> {
    Command::new("xdg-open").arg(url).spawn().map(|_| ())
}

/// Opens the OS file manager with `path` selected (Finder/Explorer), or just
/// opens its containing folder where the platform has no "select" verb.
pub fn reveal_in_file_manager(path: &Path) {
    let result = reveal(path);
    if let Err(e) = result {
        tracing::warn!("failed to reveal {} in file manager: {e}", path.display());
    }
}

#[cfg(target_os = "macos")]
fn reveal(path: &Path) -> std::io::Result<()> {
    Command::new("open").arg("-R").arg(path).spawn()?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn reveal(path: &Path) -> std::io::Result<()> {
    // `explorer` returns a non-zero exit status even on success, so don't
    // treat that as failure -- only a spawn error is actionable here.
    Command::new("explorer").arg("/select,").arg(path).spawn()?;
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn reveal(path: &Path) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(path);
    Command::new("xdg-open").arg(dir).spawn()?;
    Ok(())
}
