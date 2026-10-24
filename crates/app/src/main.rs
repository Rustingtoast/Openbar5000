// A window, not a console program, on Windows: otherwise starting it from
// the Start menu or a shortcut opens a terminal next to it for the log. Its
// log still shows when it's started from one (see `attach_parent_console`).
#![cfg_attr(windows, windows_subsystem = "windows")]

mod artist_info;
mod audio_output;
mod discord_presence;
mod display_power;
mod equalizer;
mod holiday;
mod layout;
mod lyrics;
mod media_library;
mod media_keys;
mod now_playing_file;
mod platform;
mod playlist_rows;
mod playlists;
mod properties;
mod settings;
mod spectrum_bars;
mod store;
mod theme;
mod wiring;

use engine::PlaybackEngine;
use plugin_host::PluginRegistry;
use slint::ComponentHandle;
use std::rc::Rc;
use store::Store;
use ui::MainWindow;

fn main() {
    // First, while no other thread could be reading the environment.
    #[cfg(all(unix, not(target_os = "macos")))]
    let startup_token = take_startup_token();
    #[cfg(windows)]
    attach_parent_console();
    init_logging();
    #[cfg(all(unix, not(target_os = "macos")))]
    set_up_linux_windows(startup_token);

    let plugins = PluginRegistry::scan();
    tracing::info!("loaded {} plugin(s)", plugins.loaded().len());

    let store = Rc::new(Store::open().expect("failed to open app database"));

    let window = MainWindow::new().expect("failed to create main window");
    let engine = Rc::new(PlaybackEngine::spawn());

    // Kept alive for the duration of `run()` below -- see wiring::wire.
    let _ui_handles = wiring::wire(&window, engine, store);

    window.run().expect("UI event loop failed");
}

/// The token a launcher (the app menu, the task bar) started the app with,
/// if any: Wayland's activation token, or X11's startup notification id --
/// the one for the window system the app will run on, as winit picks it.
/// Taken out of the environment, so programs the app opens later (a
/// browser, the file manager) don't present it as their own.
#[cfg(all(unix, not(target_os = "macos")))]
fn take_startup_token() -> Option<String> {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("WAYLAND_SOCKET").is_some();
    let name = if wayland { "XDG_ACTIVATION_TOKEN" } else { "DESKTOP_STARTUP_ID" };
    let token = std::env::var(name).ok().filter(|token| !token.is_empty());
    std::env::remove_var("XDG_ACTIVATION_TOKEN");
    std::env::remove_var("DESKTOP_STARTUP_ID");
    token
}

/// Names the windows after the desktop entry (see `platform::desktop_id`),
/// so the desktop shows them with its name and icon, and hands the main
/// window -- the first one made -- the launcher's token. With it, the
/// window takes focus as it opens, and the launcher's "starting" feedback
/// (a bouncing cursor, a spinner in the task bar) ends right then instead
/// of at a time-out. Neither Slint nor winit passes the token on by itself.
#[cfg(all(unix, not(target_os = "macos")))]
fn set_up_linux_windows(startup_token: Option<String>) {
    use slint::winit_030::winit::platform::startup_notify::WindowAttributesExtStartupNotify;
    use slint::winit_030::winit::window::ActivationToken;

    let token = std::cell::Cell::new(startup_token.map(ActivationToken::from_raw));
    let selected = slint::BackendSelector::new()
        .with_winit_window_attributes_hook(move |attributes| match token.take() {
            Some(token) => attributes.with_activation_token(token),
            None => attributes,
        })
        .select();
    if let Err(e) = selected {
        tracing::warn!("couldn't set up the window system backend: {e}");
    }
    if let Err(e) = slint::set_xdg_app_id(platform::desktop_id()) {
        tracing::warn!("couldn't set the app id: {e}");
    }
}

/// A Windows GUI program starts without a console, so the log would go
/// nowhere: started from a terminal (as by `cargo run`), it writes to that
/// one instead. Output redirected to a file or a pipe already has somewhere
/// to go, and is left there.
#[cfg(windows)]
fn attach_parent_console() {
    use windows::Win32::System::Console::{AttachConsole, GetStdHandle, ATTACH_PARENT_PROCESS, STD_OUTPUT_HANDLE};
    // SAFETY: plain calls; without a parent console, attaching just fails.
    unsafe {
        if GetStdHandle(STD_OUTPUT_HANDLE).is_err() {
            let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

/// Logs at INFO, except libraries whose warnings aren't actionable for a
/// user: lofty notes it *couldn't write back* tags it read fine (e.g. an
/// ID3v2 block inside a FLAC, issue #20), symphonia's MP3 decoder reports
/// expected warm-up after seeks, and zbus reports Linux desktop portal
/// internals around the file dialogs. `RUST_LOG` (e.g.
/// `RUST_LOG=debug` or `RUST_LOG=lofty=warn,info`) overrides all of this.
fn init_logging() {
    use tracing_subscriber::filter::Targets;
    use tracing_subscriber::prelude::*;

    let filter = std::env::var("RUST_LOG")
        .ok()
        .and_then(|spec| spec.parse::<Targets>().ok())
        .unwrap_or_else(|| {
            Targets::new()
                .with_default(tracing::Level::INFO)
                .with_target("lofty", tracing::Level::ERROR)
                // MP3 decoders note "using xing header" on every open and
                // "invalid main_data_begin" for the first frame or two after
                // starting mid-file (waveform slices, seeks): those frames
                // are decoder warm-up that's discarded anyway.
                .with_target("symphonia_bundle_mp3", tracing::Level::ERROR)
                .with_target("symphonia_core", tracing::Level::WARN)
                .with_target("zbus", tracing::Level::ERROR)
                // Warns "could not find pipe" on every connection attempt
                // while Discord isn't running -- expected, and shown as the
                // status in Preferences instead (issue #29).
                .with_target("discord_rich_presence", tracing::Level::ERROR)
        });
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(filter)
        .init();
}
