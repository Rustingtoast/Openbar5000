use std::path::Path;
use std::process::Command;

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
