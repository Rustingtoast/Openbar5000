# Patched dependencies

Crates here replace their crates.io release through `[patch.crates-io]` in
the workspace `Cargo.toml`. Each is the published source, unchanged except
for what's listed below; drop the copy (and its `[patch]` line) once the fix
ships upstream.

## `i-slint-backend-winit` 1.18.1

**What (1):** `frame_throttle.rs`, `TimerBasedFrameThrottle::request_throttled_redraw`.
Its timer asks winit for a frame only while one is still pending, and
otherwise stops.

**Why:** upstream asks winit for a frame first and checks whether to keep
the timer running afterwards. At that point winit hasn't delivered the frame
yet, so the check always finds it pending, and every redraw is drawn a second
time, unchanged, one refresh interval later. This throttle is used on X11 and
Windows. Wayland uses compositor frame callbacks and macOS the display link,
so neither is affected.

**Effect** (Linux, X11, while playing): the 15 fps visualization setting
really drew 30 frames per second. With the fix it draws 15, and the UI
thread's CPU use roughly halves at that setting.

**What (2):** `winitwindowadapter.rs`, `update_window_properties` and the new
`same_icon`. Whether the window icon changed is decided by comparing the
images themselves, not their cache keys.

**Why:** an image loaded from data at run time, as the app's logo is
(`Image::load_from_svg_data`, so the holiday logos can be swapped in), has
no cache key. Upstream remembers "no key" both for "nothing set yet" and for
such an icon, so with the logo set before the window first shows, the icon
was never handed to the window.

**Effect** (Windows): the title bar and taskbar showed the generic program
icon instead of the logo. Linux window managers that show window icons were
affected the same way.

**What (3):** the new `wayland_icon.rs`, called from `winitwindowadapter.rs`
where the icon changes (`update_window_properties`) and where the window is
made (`ensure_window`); `window_icon_size`; the `wayland_icon` cfg in
`build.rs`; the `rustix`, `wayland-client` and `wayland-protocols`
dependencies of the `wayland` feature in `Cargo.toml` (all three already
built for winit). On Wayland, the icon is sent with the
`xdg-toplevel-icon-v1` protocol, over winit's own connection.

**Why:** winit 0.30 only sets window icons on X11 and Windows. On Wayland
the compositor fell back to the desktop entry matching the window's app id,
and with none -- `cargo run`, the bare binary -- showed a generic icon;
the holiday logos never showed at all.

**Effect** (Linux, KDE Plasma 6.8 on Wayland): the logo shows in the title
bars and the task bar, and follows the holiday logos. Compositors without
the protocol behave as before.

**What (4):** `winitwindowadapter.rs`, `WinitWindowAdapter::new`. A new
window is no longer registered to be made as soon as the event loop runs;
`set_visibility` registers it the first time it's shown.

**Why:** upstream makes a native window, and sets up its GPU surface, for
every window as the event loop starts -- shown or not. The app creates six
dialogs up front (Preferences, About, Properties, ...), all made before the
main window. On Wayland, hiding a window destroys it and showing it again
already takes this lazy path; now the first showing does too, everywhere.

**Effect** (Linux, Wayland, Radeon RX 9070 XT): the main window's renderer
is ready after ~190 ms instead of ~224 ms, and dialogs never opened cost
nothing.

**What (5):** `clipboard.rs`, `create_clipboard` and the new `wayland`
module; the `smithay-clipboard` dependency of the `wayland` feature in
`Cargo.toml` (0.7.2 exactly, on winit's smithay-client-toolkit). Where
arboard reaches no clipboard on Wayland, the Wayland clipboard itself
(`wl_data_device`, as the focused window) is used instead of none.

**Why:** arboard uses the data-control protocols on Wayland, else X11. In a
Flatpak on Wayland it has neither: compositors keep data control from
sandboxed apps (KWin does; GNOME has none at all), and with
`--socket=fallback-x11` there's no X server.

**Effect** (Flatpak, KDE Plasma 6.8 on Wayland): copy and paste work in text
fields; before, they did nothing. Elsewhere arboard is used as before.

**What (6):** `winitwindowadapter.rs`, `ensure_window`. On Windows, with a
native menu bar, a size the application gave (`Window::set_size`) is asked
for again once the menu bar is in, and a window to be maximized is
maximized only after that.

**Why:** muda puts the menu bar in once winit has made the window, and the
menu bar takes its height from the window's contents. A preferred size is
asked for again on showing, so it comes out right; a given size wasn't, so
the window came out short by the menu bar. When maximized, that was the
size it went back to.

**Effect** (Windows, 150%): the main window, put back where it was closed
(issue #39), is 30 pixels shorter no more -- before, it shrank by that much
at every start.

**What (7):** `lib.rs`, the new `SharedBackendData::recover_rendering` and
`frame_drawn`, and `can_recreate_surface`/`recreate_surface` on
`WinitCompatibleRenderer`; `winitwindowadapter.rs`, `draw` and the new
`recreate_surface`; `renderer/skia.rs`, the two implemented. A frame that
fails no longer ends the event loop: every window lets go of its surface,
and so of the GPU device they share; shown windows get new surfaces, on a
new device, and hidden ones are destroyed, to be made anew when shown.
Should that fail, or frames keep failing with none drawn, it's tried again
a second later.

**Why:** once the GPU device is lost -- in a driver reset (TDR), which a game
working the GPU hard can cause, or a driver update -- every frame fails.
Upstream ends the event loop at the first, and the app with it. Losing the
device usually ends the game too, so the two going down together looked
like Openbar5000 taking the game with it.

**Effect** (Windows, Radeon iGPU; the D3D12 device removed with
`ID3D12Device5::RemoveDevice`, five times in a row, while playing): the
window draws again within a quarter of a second each time, playback
uninterrupted. Before, the next frame ended the app.

**Updating Slint:** if a newer `i-slint-backend-winit` still has the old
`request_throttled_redraw`, still compares the icon's cache keys, is still
on a winit without window icons on Wayland, still makes every window at
start-up, still has no clipboard in a Flatpak on Wayland, still sizes a
window before its menu bar goes in, or still ends the event loop when a
frame fails, copy the new release here and re-apply what's still needed. If
none is, delete this folder and the `[patch.crates-io]` entry. Either way,
keep the version in step with `slint`'s.

## `i-slint-renderer-skia` 1.18.1

**What (1):** `wgpu_30_surface.rs`, `SharedWgpuState::new` (a flag set by
wgpu's device-lost callback, which also logs it), `WGPUSurface::device_lost`
and its uses in `render` and `resize_event`, and a `Drop` for `WGPUSurface`;
`lib.rs`, `Surface::abandon_if_device_lost`, called first in
`SkiaRenderer::clear_surface`; the `log` dependency in `Cargo.toml` (already
built for wgpu). Once the device is lost, Skia is told so (`abandon`) before
it does anything more with it, and frames fail from then on, for the winit
backend to set drawing up anew (its change 7).

**Why:** after the device is lost, Skia's Direct3D backend fails to make its
next command list and keeps the null one, which it dereferences at the next
frame, resize, or when its context is dropped -- an access violation in
`GrD3DCommandList::close`.

**Effect** (Windows, as for the winit backend's change 7): closing the app,
or setting drawing up anew, right after the device was lost no longer
crashes it.

**Updating Slint:** if a newer `i-slint-renderer-skia` still doesn't abandon
Skia's context when the wgpu device is lost, copy the new release here and
re-apply the change; otherwise delete this folder and its
`[patch.crates-io]` entry. Keep the version in step with `slint`'s.
