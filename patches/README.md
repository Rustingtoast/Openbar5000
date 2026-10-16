# Patched dependencies

Crates here replace their crates.io release through `[patch.crates-io]` in
the workspace `Cargo.toml`. Each is the published source, unchanged except
for what's listed below; drop the copy (and its `[patch]` line) once the fix
ships upstream.

## `i-slint-backend-winit` 1.18.1

**What:** `frame_throttle.rs`, `TimerBasedFrameThrottle::request_throttled_redraw`.
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

**Updating Slint:** if a newer `i-slint-backend-winit` still has the old
`request_throttled_redraw`, copy the new release here and re-apply the
change. If it's fixed, delete this folder and the `[patch.crates-io]` entry.
Either way, keep the version in step with `slint`'s.
