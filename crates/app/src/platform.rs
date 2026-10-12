use std::path::Path;
use std::process::Command;

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
