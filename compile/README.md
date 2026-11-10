# Build scripts

Per-platform scripts that produce a release build of the `openbar5000` binary
under `dist/<platform>/`. Each one wraps a plain `cargo build --release -p app`
and copies the resulting binary to a predictable location — nothing magic.

| Platform | Script | Output |
|---|---|---|
| macOS | `./compile/build-macos.sh` | `dist/macos/openbar5000` + `dist/macos/Openbar5000.app` + `dist/macos/Openbar5000-<version>-<arch>.dmg` |
| Linux (Flatpak) | `./compile/build-flatpak.sh` | `dist/flatpak/Openbar5000-<version>-<arch>.flatpak` |
| Windows | `.\compile\build-windows.ps1` | `dist\windows\openbar5000.exe` + `dist\windows\Openbar5000-Setup-<version>.msi` |

Linux is shipped only as a Flatpak, which runs on every distribution.

## Important: these build natively, not cross-platform

Because Openbar5000 links against platform-native audio (CoreAudio/ALSA/WASAPI
via `cpal`) and GUI backends (via Slint/winit), **each script must be run on
its own target OS** — there's no single machine that cross-compiles all
three. Run the macOS script on a Mac, the Flatpak script on Linux, and the
Windows script on Windows. The release workflow below does exactly that,
with one GitHub Actions machine per OS.

## Publishing a release

`.github/workflows/release.yml` runs all the scripts on GitHub's machines and
publishes what they make on a GitHub Release, so people install Openbar5000
by downloading one file:

| Platform | Release file |
|---|---|
| Windows | `Openbar5000-Windows-x64.msi` |
| macOS (Intel and Apple Silicon) | `Openbar5000-macOS-universal.dmg` |
| Linux | `Openbar5000-Linux-x86_64.flatpak`, `Openbar5000-Linux-aarch64.flatpak` |

The names carry no version, so
`https://github.com/Rustingtoast/Openbar5000/releases/latest/download/<file>`
always gets the newest one; the main README links to those.

To release a version:

1. Set `version` in the root `Cargo.toml` (e.g. `0.9.0`), and run
   `cargo update --workspace` so `Cargo.lock` matches (the Flatpak builds
   with `--locked`).
2. Add a `<release>` line for it to
   `flatpak/io.github.rustingtoast.Openbar5000.metainfo.xml`. A pre-release
   takes a `~` instead of the `-` (`0.9.0-beta` is `0.9.0~beta`).
3. Merge that into `main`.
4. On GitHub, go to **Releases > Draft a new release**. Under **Choose a
   tag**, type `v0.9.0` and pick **Create new tag on publish** (target:
   `main`). Add a title and notes (**Generate release notes** lists the
   merged pull requests), then **Publish release**.

Publishing starts the workflow, which builds the installers and attaches
them to the release about 20 to 30 minutes later (watch it under
**Actions > Release**). Until then the release has no installers, and the
README's download links don't work yet.

The workflow stops before building anything if the tag isn't the
`Cargo.toml` version (with or without the `v`), or if the metainfo has no
entry for it: delete the release and its tag, fix it, and publish again. If
a build fails for another reason, fix it and use **Re-run failed jobs** on
that run.

Leave **Set as a pre-release** unticked, even for a `-beta` version: GitHub's
"latest release" skips pre-releases, so the README's download links would
keep pointing at the previous one.

To try the installers without releasing, run the workflow by hand
(**Actions > Release > Run workflow**): it builds everything and attaches the
files to the run as artifacts, without creating a release. It also runs
this way on every pull request that changes `compile/` or the workflow, so
packaging that no longer builds shows up before a release does.

On the Windows machine the workflow installs WiX 7 and accepts its
[Open Source Maintenance Fee EULA](https://docs.firegiant.com/wix/osmf/)
(`wix eula accept wix7`); WiX 7 won't run otherwise.

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

## macOS: disk image (.dmg)

To share the app, `build-macos.sh` then puts it on a disk image,
`dist/macos/Openbar5000-<version>-<arch>.dmg` (`<arch>` is `arm64`,
`x86_64` or `universal`). Opened, it shows the app next to a link to
Applications over an MP3-player-style background, with an arrow from one to
the other: dragging the app onto Applications installs it.

- Everything it uses comes with macOS (`hdiutil`, `codesign`, `osascript`).
- The window's layout is saved by Finder itself, so the script asks Finder
  to set it up. The first time, macOS asks whether your terminal may control
  Finder: allow it. Without that permission, or on a machine with no desktop
  session (CI), the image still works but opens as a plain folder.
- Eject any open `Openbar5000` disk image before building; the script stops
  if one is mounted.
- The app is signed ad hoc (no developer certificate). macOS then asks once
  before opening it, instead of calling it damaged. If it's blocked, go to
  System Settings > Privacy & Security and choose **Open Anyway**. Opening
  without being asked needs a paid Apple Developer ID and notarization.
- The background is `assets/macos/dmg-background.svg`. After editing it,
  render `dmg-background.tiff` again with the commands in its header comment
  (they need `rsvg-convert`, from Homebrew's `librsvg`).

## Linux: Flatpak (any distribution)

`build-flatpak.sh` packages Openbar5000 as a Flatpak, which installs and runs
the same way on every distribution with Flatpak (Fedora, Arch, openSUSE,
Debian/Ubuntu, Mint, ...).

```bash
./compile/build-flatpak.sh            # dist/flatpak/Openbar5000-<version>-<arch>.flatpak
./compile/build-flatpak.sh --install  # ...and install it for the current user
```

It needs `flatpak`, `flatpak-builder` (or the `org.flatpak.Builder` Flatpak)
and `cargo`. On first run it adds Flathub as a per-user remote and installs
the freedesktop runtime/SDK 26.08 and the Rust SDK extension from there.
Flatpak builds run without network, so the script first copies every Rust
crate into `dist/flatpak/vendor` (`cargo vendor`) and points cargo at those
copies.

Install the bundle on any machine (the runtime comes from Flathub) and run it:

```bash
flatpak install --user Openbar5000-<version>-<arch>.flatpak
flatpak run io.github.rustingtoast.Openbar5000
```

The packaging files live in `compile/flatpak/`: the manifest
(`io.github.rustingtoast.Openbar5000.yml`), the desktop entry and the
AppStream metadata (`.metainfo.xml` -- add a `<release>` line there for each
new version; the script warns if the current one is missing).

**Sandbox permissions** (`finish-args` in the manifest): Wayland/X11 and the
GPU, PulseAudio/PipeWire for sound, network for artist info, read/write
access to the Music folder (lyrics are saved into files) and read-only
access to the rest of home and to external drives (`/media`, `/run/media`),
MPRIS for media keys, and Discord's IPC socket for Rich Presence. Files
chosen in the Open dialogs work from anywhere, through the desktop portal.
Settings live in `~/.var/app/io.github.rustingtoast.Openbar5000/config/openbar5000/`,
separate from `~/.config/openbar5000`, which a build run outside Flatpak
(`cargo run`) uses.

The bundle is built for the build machine's architecture (`x86_64` on a
normal PC, `aarch64` on ARM).

## Windows: MSI packaging

`build-windows.ps1` additionally packages the binary into a real MSI
installer, `dist\windows\Openbar5000-Setup-<version>.msi`, using the
[WiX Toolset](https://wixtoolset.org) (`compile\installer.wxs`). The
installer shows a license agreement that has to be accepted before going
on, lets you choose the install folder (`Program Files\Openbar5000` by
default), adds a Start Menu shortcut and a desktop shortcut, sets the app
icon, installs `LICENSE.txt` next to the app, and registers a proper
uninstaller with Windows (Add/Remove Programs).

The license agreement is built by the script on every run: the terms and
privacy statement in `compile\installer-terms.txt`, then the repository's
`LICENSE`, turned into the RTF WiX's license page takes (written to
`target\wix\License.rtf`). Edit the text file, not the RTF: blank lines
separate paragraphs, `# ` starts a heading and `- ` a bullet point. If what
the app sends over the network changes, update its privacy section.

Windows doesn't come with an MSI packager, so this step requires installing
the WiX CLI first, as a global .NET tool (the .NET
SDK is the only prerequisite -- [dotnet.microsoft.com/download](https://dotnet.microsoft.com/download)):

```powershell
dotnet tool install --global wix
```

`build-windows.ps1` installs the required `WixToolset.UI.wixext` extension
itself on first run if it's missing, in the same version as `wix`. If `wix` isn't found at all, MSI
packaging is skipped with a warning pointing at the install command above --
the raw `.exe` is still produced either way.

WiX only supports running on Windows (off it, its own path validation fails
even on WiX's canonical examples). On Windows 11 with WiX 7, the MSI builds,
passes `wix msi validate`, and its dialogs (welcome, license, install folder,
ready to install) were walked through.

## Prerequisites

All platforms need a Rust toolchain (via [rustup](https://rustup.rs)). See
the main [README](../README.md#prerequisites) for platform-specific native
dependencies (Linux needs ALSA + windowing dev headers; macOS needs the Xcode
Command Line Tools; Windows needs the MSVC build tools).
