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

**Updating Slint:** if a newer `i-slint-backend-winit` still has the old
`request_throttled_redraw`, still compares the icon's cache keys, is still
on a winit without window icons on Wayland, still makes every window at
start-up, or still has no clipboard in a Flatpak on Wayland, copy the new release here and re-apply what's still needed. If
none is, delete this folder and the `[patch.crates-io]` entry. Either way,
keep the version in step with `slint`'s.
