mod platform;
mod playlist_rows;
mod settings;
mod wiring;

use engine::PlaybackEngine;
use plugin_host::PluginRegistry;
use slint::ComponentHandle;
use std::rc::Rc;
use ui::MainWindow;

fn main() {
    tracing_subscriber::fmt::init();

    let plugins = PluginRegistry::scan();
    tracing::info!("loaded {} plugin(s)", plugins.loaded().len());

    let window = MainWindow::new().expect("failed to create main window");
    let engine = Rc::new(PlaybackEngine::spawn());

    // Kept alive for the duration of `run()` below -- see wiring::wire.
    let _ui_handles = wiring::wire(&window, engine);

    window.run().expect("UI event loop failed");
}
