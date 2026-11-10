//! The main window opening where it was closed (issue #39): its position,
//! size and maximized state are saved on exit and restored at start-up --
//! unless the screens have changed so that it wouldn't be wholly on them
//! any more, when it opens at its default size on the main screen, as on a
//! first start.
//!
//! Both ends go through winit's event loop (see [`WindowPlacement::handler`]):
//! it lists the screens as it starts, before any window is made, so the
//! window is made in its place instead of moving there once shown; and its
//! move and resize events say when to note where the window is.

use crate::store::Store;
use serde::{Deserialize, Serialize};
use slint::winit_030::winit::{event::WindowEvent, event_loop::ActiveEventLoop, window::WindowId};
use slint::winit_030::{CustomApplicationHandler, EventResult, WinitWindowAccessor};
use slint::ComponentHandle;
use std::cell::{Cell, OnceCell};
use std::rc::Rc;
use ui::MainWindow;

/// The `settings` key the placement is saved under, as JSON.
const SETTING: &str = "window_placement";

/// Where the window is, as saved.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
struct Placement {
    /// Where the window is, by the part winit places a window it makes by:
    /// the frame, title bar included, on Windows and X11; on macOS what's
    /// below the title bar. In desktop coordinates (see [`to_desktop`]);
    /// `None` where the window system keeps windows' positions to itself
    /// (Wayland).
    frame: Option<Rect>,
    /// The window's scale factor: a screen set to another scale since
    /// counts as a changed setup.
    scale: f64,
    /// The size of the window's contents, in logical pixels.
    width: f32,
    height: f32,
    /// When maximized, `frame` and the size are where it goes back to.
    maximized: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct Rect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

impl Rect {
    fn right(&self) -> i32 {
        self.x.saturating_add(self.width)
    }

    fn bottom(&self) -> i32 {
        self.y.saturating_add(self.height)
    }

    fn contains(&self, (x, y): (i32, i32)) -> bool {
        self.x <= x && x < self.right() && self.y <= y && y < self.bottom()
    }

    fn center(&self) -> (i32, i32) {
        (self.x + self.width / 2, self.y + self.height / 2)
    }

    fn shrunk_by(&self, margin: i32) -> Rect {
        Rect {
            x: self.x + margin,
            y: self.y + margin,
            width: self.width - 2 * margin,
            height: self.height - 2 * margin,
        }
    }
}

/// A connected screen, in desktop coordinates.
#[derive(Debug, Clone, Copy)]
struct Screen {
    rect: Rect,
    scale: f64,
}

/// Desktop coordinates: what window positions are given in and screens are
/// compared in. Physical pixels on Windows and X11, where the desktop is
/// laid out in them; on macOS points, which winit turns into pixels at each
/// screen's own scale, so screens of different scales would overlap.
fn to_desktop(pixels: f64, scale: f64) -> i32 {
    if cfg!(target_os = "macos") {
        (pixels / scale).round() as i32
    } else {
        pixels.round() as i32
    }
}

fn desktop_position(x: i32, y: i32) -> slint::WindowPosition {
    if cfg!(target_os = "macos") {
        slint::LogicalPosition::new(x as f32, y as f32).into()
    } else {
        slint::PhysicalPosition::new(x, y).into()
    }
}

impl Placement {
    /// Whether the window can go back here with `screens` connected: all
    /// of it on them -- give or take Windows' invisible resize borders --
    /// and its middle on a screen of the scale it had. Where its position
    /// isn't known (Wayland), whether its size fits on a screen.
    fn fits(&self, screens: &[Screen]) -> bool {
        match self.frame {
            Some(frame) => {
                // Windows 11's borders are 7 pixels at 100%.
                let margin = (8.0 * self.scale).ceil() as i32;
                let rects: Vec<Rect> = screens.iter().map(|screen| screen.rect).collect();
                screens
                    .iter()
                    .find(|screen| screen.rect.contains(frame.center()))
                    .is_some_and(|screen| (screen.scale - self.scale).abs() < 0.01)
                    && covered(frame.shrunk_by(margin), &rects)
            }
            None => screens.iter().any(|screen| {
                let width = screen.rect.width as f64 / screen.scale;
                let height = screen.rect.height as f64 / screen.scale;
                self.width as f64 <= width && self.height as f64 <= height
            }),
        }
    }
}

/// Whether every point of `area` is on one of `screens`.
fn covered(area: Rect, screens: &[Rect]) -> bool {
    if area.width <= 0 || area.height <= 0 {
        return false;
    }
    // Cut along every screen edge crossing `area`: each piece is then on a
    // screen or off all of them as a whole, which its middle shows. In
    // doubled coordinates, so the middles are whole numbers.
    let cuts = |start: i32, end: i32, edges: &mut dyn Iterator<Item = i32>| {
        let mut cuts: Vec<i64> = std::iter::once(start)
            .chain(std::iter::once(end))
            .chain(edges.filter(|&edge| start < edge && edge < end))
            .map(|cut| 2 * cut as i64)
            .collect();
        cuts.sort_unstable();
        cuts.dedup();
        cuts
    };
    let xs = cuts(area.x, area.right(), &mut screens.iter().flat_map(|s| [s.x, s.right()]));
    let ys = cuts(area.y, area.bottom(), &mut screens.iter().flat_map(|s| [s.y, s.bottom()]));
    xs.windows(2).all(|x| {
        ys.windows(2).all(|y| {
            let (mid_x, mid_y) = ((x[0] + x[1]) / 2, (y[0] + y[1]) / 2);
            screens.iter().any(|s| {
                2 * s.x as i64 <= mid_x
                    && mid_x < 2 * s.right() as i64
                    && 2 * s.y as i64 <= mid_y
                    && mid_y < 2 * s.bottom() as i64
            })
        })
    })
}

fn screens(event_loop: &ActiveEventLoop) -> Vec<Screen> {
    event_loop
        .available_monitors()
        .map(|monitor| {
            let scale = monitor.scale_factor();
            let (position, size) = (monitor.position(), monitor.size());
            Screen {
                rect: Rect {
                    x: to_desktop(position.x as f64, scale),
                    y: to_desktop(position.y as f64, scale),
                    width: to_desktop(size.width as f64, scale),
                    height: to_desktop(size.height as f64, scale),
                },
                scale,
            }
        })
        .collect()
}

/// The main window's placement: restored as the event loop starts, followed
/// while it runs, saved as it ends.
pub struct WindowPlacement {
    store: Rc<Store>,
    window: OnceCell<slint::Weak<MainWindow>>,
    /// As saved last time, until restored (or found not to fit).
    saved: Cell<Option<Placement>>,
    /// Where the window is now, or was when last not minimized.
    current: Cell<Option<Placement>>,
    /// The window moved or was resized since `current` was noted.
    moved: Cell<bool>,
}

impl WindowPlacement {
    pub fn load(store: &Rc<Store>) -> Rc<Self> {
        let saved = store
            .get_setting(SETTING)
            .and_then(|json| serde_json::from_str::<Placement>(&json).ok())
            .filter(|saved| saved.width > 0.0 && saved.height > 0.0 && saved.scale > 0.0);
        Rc::new(Self {
            store: store.clone(),
            window: OnceCell::new(),
            saved: Cell::new(saved),
            current: Cell::new(None),
            moved: Cell::new(false),
        })
    }

    /// For Slint's winit backend: restores the placement when the event
    /// loop starts, then follows the window's moves and resizes.
    pub fn handler(self: &Rc<Self>) -> impl CustomApplicationHandler + 'static {
        Handler(self.clone())
    }

    /// The window to place, made after the backend is set up.
    pub fn attach(&self, window: &MainWindow) {
        let _ = self.window.set(window.as_weak());
    }

    pub fn save(&self) {
        if let Some(json) = self.current.get().and_then(|current| serde_json::to_string(&current).ok()) {
            self.store.set_settings(&[(SETTING, json)]);
        }
    }

    fn window(&self) -> Option<MainWindow> {
        self.window.get().and_then(|window| window.upgrade())
    }

    /// Before the window is made: where it was, if that still fits.
    fn restore(&self, event_loop: &ActiveEventLoop) {
        let (Some(saved), Some(window)) = (self.saved.take(), self.window()) else {
            return;
        };
        if !saved.fits(&screens(event_loop)) {
            tracing::info!("the screens changed since the window was closed: opening it in its default place");
            return;
        }
        let window = window.window();
        if let Some(frame) = saved.frame {
            window.set_position(desktop_position(frame.x, frame.y));
        }
        window.set_size(slint::LogicalSize::new(saved.width, saved.height));
        if saved.maximized {
            window.set_maximized(true);
        }
        self.current.set(Some(saved));
    }

    /// Notes where the window is -- once winit has handled the whole move
    /// or resize, so that a maximize, say, is no longer half applied. While
    /// it's minimized or full screen, nothing is noted; while maximized, only
    /// that, keeping the place it goes back to.
    fn note(&self) {
        let Some(window) = self.window() else {
            return;
        };
        let current = self.current.get();
        let noted = window.window().with_winit_window(|winit| {
            if winit.is_minimized() == Some(true) || winit.fullscreen().is_some() {
                return current;
            }
            if winit.is_maximized() {
                return current.map(|current| Placement { maximized: true, ..current });
            }
            let scale = winit.scale_factor();
            // See `Placement::frame`: otherwise every start would put the
            // window a title bar's height higher on macOS.
            let (position, size) = if cfg!(target_os = "macos") {
                (winit.inner_position(), winit.inner_size())
            } else {
                (winit.outer_position(), winit.outer_size())
            };
            let frame = position.ok().map(|position| {
                Rect {
                    x: to_desktop(position.x as f64, scale),
                    y: to_desktop(position.y as f64, scale),
                    width: to_desktop(size.width as f64, scale),
                    height: to_desktop(size.height as f64, scale),
                }
            });
            // In Slint's own logical pixels, which a scale factor set with
            // SLINT_SCALE_FACTOR changes, as `set_size` takes them.
            let size = window.window().size().to_logical(window.window().scale_factor());
            Some(Placement { frame, scale, width: size.width, height: size.height, maximized: false })
        });
        if let Some(noted) = noted.flatten() {
            self.current.set(Some(noted));
        }
    }
}

struct Handler(Rc<WindowPlacement>);

impl CustomApplicationHandler for Handler {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) -> EventResult {
        self.0.restore(event_loop);
        EventResult::Propagate
    }

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        _winit_window: Option<&slint::winit_030::winit::window::Window>,
        slint_window: Option<&slint::Window>,
        event: &WindowEvent,
    ) -> EventResult {
        if matches!(event, WindowEvent::Moved(_) | WindowEvent::Resized(_))
            && slint_window.is_some_and(|slint_window| {
                self.0.window().is_some_and(|window| std::ptr::eq(window.window(), slint_window))
            })
        {
            self.0.moved.set(true);
        }
        EventResult::Propagate
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) -> EventResult {
        if self.0.moved.take() {
            self.0.note();
        }
        EventResult::Propagate
    }

    /// On macOS, quitting from the app menu (Cmd+Q), the Dock or by logging
    /// out ends here: AppKit ends the process right after, never returning
    /// to `main` to save it there.
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) -> EventResult {
        self.0.save();
        EventResult::Propagate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, width: i32, height: i32) -> Rect {
        Rect { x, y, width, height }
    }

    fn screen(x: i32, y: i32, width: i32, height: i32, scale: f64) -> Screen {
        Screen { rect: rect(x, y, width, height), scale }
    }

    fn placed(frame: Rect, scale: f64) -> Placement {
        Placement { frame: Some(frame), scale, width: 800.0, height: 600.0, maximized: false }
    }

    #[test]
    fn coverage_spans_screens_side_by_side_but_not_gaps() {
        let screens = [rect(0, 0, 1920, 1080), rect(1920, 0, 1920, 1080)];
        assert!(covered(rect(1500, 100, 800, 600), &screens));
        assert!(!covered(rect(3500, 100, 800, 600), &screens));
        // A taller screen beside a shorter one: the corner below the short
        // one is off screen.
        let screens = [rect(0, 0, 1920, 1080), rect(1920, 0, 2560, 1440)];
        assert!(covered(rect(2000, 1000, 800, 400), &screens));
        assert!(!covered(rect(1500, 1000, 800, 400), &screens));
        assert!(!covered(rect(0, 0, 0, 600), &screens));
    }

    #[test]
    fn a_window_on_a_screen_since_disconnected_falls_back() {
        // Issue #39: closed on the lower half of a second screen.
        let lower_half = placed(rect(1913, 540, 1934, 547), 1.0);
        let both = [screen(0, 0, 1920, 1080, 1.0), screen(1920, 0, 1920, 1080, 1.0)];
        assert!(lower_half.fits(&both));
        assert!(!lower_half.fits(&both[..1]));
    }

    #[test]
    fn a_rearranged_or_rescaled_screen_falls_back() {
        let window = placed(rect(2100, 200, 1000, 700), 1.5);
        assert!(window.fits(&[screen(0, 0, 1920, 1080, 1.0), screen(1920, 0, 2560, 1440, 1.5)]));
        // Now left of the main screen instead of right of it.
        assert!(!window.fits(&[screen(0, 0, 1920, 1080, 1.0), screen(-2560, 0, 2560, 1440, 1.5)]));
        // Set to 100% since.
        assert!(!window.fits(&[screen(0, 0, 1920, 1080, 1.0), screen(1920, 0, 2560, 1440, 1.0)]));
    }

    #[test]
    fn invisible_borders_past_the_screen_edge_still_fit() {
        // Snapped to the left half at 150%: the borders stick out 11 pixels.
        let snapped = placed(rect(-11, 0, 1302, 1451), 1.5);
        assert!(snapped.fits(&[screen(0, 0, 2560, 1440, 1.5)]));
    }

    #[test]
    fn without_a_position_only_the_size_has_to_fit() {
        let wayland = Placement { frame: None, scale: 2.0, width: 1400.0, height: 700.0, maximized: false };
        // 2880x1800 at 200% is 1440x900 logical.
        assert!(wayland.fits(&[screen(0, 0, 2880, 1800, 2.0)]));
        assert!(!wayland.fits(&[screen(0, 0, 2560, 1440, 2.0)]));
        assert!(!wayland.fits(&[]));
    }

    #[test]
    fn placements_round_trip_through_the_store() {
        let store = Rc::new(Store::open_in_memory().unwrap());
        assert!(WindowPlacement::load(&store).saved.get().is_none());
        let placement = WindowPlacement::load(&store);
        placement.current.set(Some(Placement { maximized: true, ..placed(rect(10, 20, 900, 650), 1.25) }));
        placement.save();
        assert_eq!(WindowPlacement::load(&store).saved.get(), placement.current.get());
        store.set_settings(&[(SETTING, "not json".to_string())]);
        assert!(WindowPlacement::load(&store).saved.get().is_none());
    }
}
