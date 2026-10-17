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

**Updating Slint:** if a newer `i-slint-backend-winit` still has the old
`request_throttled_redraw`, or still compares the icon's cache keys, copy
the new release here and re-apply what's still needed. If both are fixed,
delete this folder and the `[patch.crates-io]` entry. Either way, keep the
version in step with `slint`'s.
