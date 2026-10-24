// Openbar5000 patch (see `patches/README.md`): this whole file is new.

//! The window icon on Wayland, through the `xdg-toplevel-icon-v1` protocol
//! (KDE Plasma 6.2 and later, among others): winit 0.30 only sets window
//! icons on X11 and Windows. Compositors without the protocol keep showing
//! the icon of the app's desktop entry, as before.
//!
//! The icon is sent over winit's own connection to the compositor, on an
//! event queue of its own, for the `xdg_toplevel` winit made.

use std::cell::RefCell;
use std::ffi::c_void;
use std::io::Write;
use std::mem::ManuallyDrop;
use std::os::fd::AsFd;

use i_slint_core::graphics::{Image, ImageInner, SharedImageBuffer, euclid};
use wayland_client::backend::{Backend, ObjectId};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_shm::{self, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, delegate_noop};
use wayland_protocols::xdg::shell::client::xdg_toplevel::XdgToplevel;
use wayland_protocols::xdg::toplevel_icon::v1::client::xdg_toplevel_icon_manager_v1::XdgToplevelIconManagerV1;
use wayland_protocols::xdg::toplevel_icon::v1::client::xdg_toplevel_icon_v1::XdgToplevelIconV1;
use winit::platform::wayland::WindowExtWayland;
use winit::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};

type Error = Box<dyn std::error::Error>;

/// What's bound once on a connection: the icon manager and shared memory,
/// if the compositor has them.
struct Globals {
    /// The `wl_display` this is for.
    display: *mut c_void,
    connection: Connection,
    queue: EventQueue<State>,
    icons: Option<(XdgToplevelIconManagerV1, WlShm)>,
}

thread_local! {
    /// Never dropped: at exit, winit may have closed the connection before
    /// thread-locals are dropped, and these objects would go after it.
    static GLOBALS: RefCell<Option<ManuallyDrop<Globals>>> = const { RefCell::new(None) };
}

/// Shows `icon`, drawn `size` pixels square, as `window`'s icon, if it's a
/// Wayland window and the compositor takes icons. It's applied with the
/// window's next frame.
pub fn set(window: &winit::window::Window, icon: &Image, size: u32) {
    let Some(toplevel) = window.xdg_toplevel() else { return };
    let Ok(handle) = window.display_handle() else { return };
    let RawDisplayHandle::Wayland(display) = handle.as_raw() else { return };
    match set_icon(display.display.as_ptr(), toplevel.as_ptr(), icon, size) {
        Ok(true) => window.request_redraw(),
        Ok(false) => {}
        Err(err) => i_slint_core::debug_log!("Error setting the window icon: {err}"),
    }
}

/// Whether the icon was set.
fn set_icon(display: *mut c_void, toplevel: *mut c_void, icon: &Image, size: u32) -> Result<bool, Error> {
    GLOBALS.with_borrow_mut(|globals| {
        if globals.as_ref().is_none_or(|globals| globals.display != display) {
            // A previous display's are left as they are (see `GLOBALS`).
            *globals = Some(ManuallyDrop::new(Globals::bind(display)?));
        }
        let globals = globals.as_mut().expect("bound above");
        let Some((manager, shm)) = &globals.icons else { return Ok(false) };
        let Some(pixels) = premultiplied_bgra(icon, size) else { return Ok(false) };

        // SAFETY: the live `xdg_toplevel` of a winit window, on `display`.
        // This connection only sends a request naming it, never takes it over.
        let id = unsafe { ObjectId::from_ptr(XdgToplevel::interface(), toplevel.cast()) }?;
        let toplevel = XdgToplevel::from_id(&globals.connection, id)?;

        let qh = globals.queue.handle();
        let side = size as i32;
        let file = shared_memory(&pixels)?;
        let pool = shm.create_pool(file.as_fd(), pixels.len() as i32, &qh, ());
        let buffer = pool.create_buffer(0, side, side, side * 4, wl_shm::Format::Argb8888, &qh, ());
        let icon = manager.create_icon(&qh, ());
        icon.add_buffer(&buffer, 1);
        manager.set_icon(&toplevel, Some(&icon));
        // The window keeps its icon without these; the buffer has to go
        // after the icon.
        icon.destroy();
        buffer.destroy();
        pool.destroy();
        globals.connection.flush()?;
        // The icon sizes the compositor prefers, of no use: one size is
        // drawn, and scaled as needed.
        globals.queue.dispatch_pending(&mut State)?;
        Ok(true)
    })
}

impl Globals {
    fn bind(display: *mut c_void) -> Result<Self, Error> {
        // SAFETY: winit's display, which outlives its windows; this
        // connection never closes it.
        let backend = unsafe { Backend::from_foreign_display(display.cast()) };
        let connection = Connection::from_backend(backend);
        let (globals, mut queue) = registry_queue_init::<State>(&connection)?;
        let qh = queue.handle();
        let icons = globals
            .bind::<XdgToplevelIconManagerV1, _, _>(&qh, 1..=1, ())
            .ok()
            .zip(globals.bind::<WlShm, _, _>(&qh, 1..=1, ()).ok());
        queue.roundtrip(&mut State)?;
        Ok(Self { display, connection, queue, icons })
    }
}

/// `icon` drawn `size` pixels square, as `wl_shm`'s ARGB8888 takes it:
/// premultiplied, and in memory blue, green, red, alpha. `None` if it
/// can't be drawn square -- the protocol takes square icons only.
fn premultiplied_bgra(icon: &Image, size: u32) -> Option<Vec<u8>> {
    let inner: &ImageInner = icon.into();
    let buffer = inner.render_to_buffer(Some(euclid::size2(size, size)))?;
    if buffer.width() != buffer.height() || buffer.width() == 0 {
        return None;
    }
    let pixels: Vec<u8> = match &buffer {
        SharedImageBuffer::RGB8(pixels) => {
            pixels.as_slice().iter().flat_map(|p| [p.b, p.g, p.r, 255]).collect()
        }
        SharedImageBuffer::RGBA8(pixels) => pixels
            .as_slice()
            .iter()
            .flat_map(|p| {
                let premultiply = |c: u8| (c as u32 * p.a as u32 / 255) as u8;
                [premultiply(p.b), premultiply(p.g), premultiply(p.r), p.a]
            })
            .collect(),
        SharedImageBuffer::RGBA8Premultiplied(pixels) => {
            pixels.as_slice().iter().flat_map(|p| [p.b, p.g, p.r, p.a]).collect()
        }
    };
    Some(pixels)
}

/// An anonymous file holding `bytes`, to share with the compositor.
fn shared_memory(bytes: &[u8]) -> std::io::Result<std::fs::File> {
    let fd = rustix::fs::memfd_create("slint-window-icon", rustix::fs::MemfdFlags::CLOEXEC)?;
    let mut file = std::fs::File::from(fd);
    file.write_all(bytes)?;
    Ok(file)
}

/// Every event on this queue is ignored.
struct State;

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(State: ignore XdgToplevelIconManagerV1);
delegate_noop!(State: XdgToplevelIconV1);
delegate_noop!(State: ignore WlShm);
delegate_noop!(State: WlShmPool);
delegate_noop!(State: ignore WlBuffer);
