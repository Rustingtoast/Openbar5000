// `#[cfg(target_os = "windows")]` here reflects the *host* building this
// crate, not the cross-compilation target -- which is exactly right for us,
// since `compile/build-windows.ps1` always runs natively on Windows. (A
// `CARGO_CFG_TARGET_OS` runtime check was tried instead, to support
// cross-compiling from macOS/Linux, but `winresource` is only pulled in as
// a dependency at all when Cargo's own target-cfg resolution picks Windows,
// so the two mechanisms disagree across a real cross-compile and the build
// fails outright on non-Windows hosts without this `cfg` gate.)
fn main() {
    #[cfg(target_os = "windows")]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../compile/assets/windows/app-icon.ico");
        if let Err(e) = res.compile() {
            eprintln!("warning: failed to embed Windows icon resource: {e}");
        }
    }
}
