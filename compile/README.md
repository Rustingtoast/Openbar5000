# Build scripts

Per-platform scripts that produce a release build of the `openbar5000` binary
under `dist/<platform>/`. Each one wraps a plain `cargo build --release -p app`
and copies the resulting binary to a predictable location — nothing magic.

| Platform | Script | Output |
|---|---|---|
| macOS | `./compile/build-macos.sh` | `dist/macos/openbar5000` + `dist/macos/Openbar5000.app` |
| Linux | `./compile/build-linux.sh` | `dist/linux/openbar5000` + `dist/linux/openbar5000_<version>_<arch>.deb` |
| Windows | `.\compile\build-windows.ps1` | `dist\windows\openbar5000.exe` + `dist\windows\Openbar5000-Setup-<version>.msi` |

## Important: these build natively, not cross-platform

Because Openbar5000 links against platform-native audio (CoreAudio/ALSA/WASAPI
via `cpal`) and GUI backends (via Slint/winit), **each script must be run on
its own target OS** — there's no single machine that cross-compiles all
three. Run the macOS script on a Mac, the Linux script on Linux, and the
Windows script on Windows (or use a CI matrix with one job per OS, which is
the standard way to produce all three from one trigger).

## macOS: universal binaries

`build-macos.sh` defaults to building for the host architecture only. Pass
`--universal` to build both `x86_64` and `aarch64` and combine them with
`lipo` into a single universal binary:

```bash
./compile/build-macos.sh --universal
```

## macOS: .app bundle and icon

`build-macos.sh` also wraps the binary into `dist/macos/Openbar5000.app`
(with `Contents/Info.plist` and `Contents/Resources/AppIcon.icns`), which is
what gives Openbar5000 its own icon in Finder/Launchpad/the Dock. This is
the only way to get a custom icon on macOS — `cargo run` won't show one in
the Dock, since macOS ties the Dock icon to the app bundle, not the window
(unlike Windows/Linux, where the window icon shows up even unbundled).
Open `dist/macos/Openbar5000.app` to see the real icon.

## Linux: .deb packaging

On Debian/Ubuntu-based systems (where `dpkg-deb` is available), `build-linux.sh`
additionally packages the binary into `dist/linux/openbar5000_<version>_<arch>.deb`,
with a `.desktop` entry (so it shows up in application launchers), the app
icon installed into the standard `hicolor` icon theme paths (both the
scalable SVG and a 256px PNG fallback), and a `Depends:` line listing the
runtime shared libraries it needs (ALSA, fontconfig, X11/libGL). Install it
with:

```bash
sudo dpkg -i dist/linux/openbar5000_*.deb
sudo apt-get install -f   # only needed if dpkg reports missing dependencies
```

On non-Debian distros (Fedora, Arch, etc.) `dpkg-deb` won't be present, so
this step is skipped automatically and only the raw binary is produced —
that's expected, not an error.

## Windows: MSI packaging

`build-windows.ps1` additionally packages the binary into a real MSI
installer, `dist\windows\Openbar5000-Setup-<version>.msi`, using the
[WiX Toolset](https://wixtoolset.org) (`compile\installer.wxs`). The
installer puts the app under `Program Files`, adds a Start Menu shortcut and
a desktop shortcut, sets the app icon, and registers a proper uninstaller
with Windows (Add/Remove Programs).

Windows doesn't ship a packager the way Debian ships `dpkg-deb`, so this
step requires installing the WiX CLI first, as a global .NET tool (the .NET
SDK is the only prerequisite -- [dotnet.microsoft.com/download](https://dotnet.microsoft.com/download)):

```powershell
dotnet tool install --global wix
```

`build-windows.ps1` installs the required `WixToolset.UI.wixext` extension
itself on first run if it's missing. If `wix` isn't found at all, MSI
packaging is skipped with a warning pointing at the install command above --
the raw `.exe` is still produced either way.

**A note on where this was tested:** WiX explicitly only supports running on
Windows (it prints its own warning and says behavior is undefined otherwise).
Everything up to and including the actual `wix build` invocation was verified
for real on this repo (dotnet tool install, the UI extension, version
extraction from `Cargo.toml`, the full script's control flow and error
handling) -- but the MSI compilation step itself needs to be sanity-checked
once on a real Windows machine, since WiX's own path-validation logic
behaves differently when hosted on a non-Windows .NET runtime (confirmed:
even WiX's own canonical example .wxs fails identically off-Windows, so
this is a WiX-on-this-host limitation, not anything specific to this repo's
installer script).

## Prerequisites

All platforms need a Rust toolchain (via [rustup](https://rustup.rs)). See
the main [README](../README.md#prerequisites) for platform-specific native
dependencies (Linux needs ALSA + windowing dev headers; macOS needs the Xcode
Command Line Tools; Windows needs the MSVC build tools).
