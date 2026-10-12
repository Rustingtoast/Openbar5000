//! Manual ABI round-trip checkpoint (milestone-1 step 5): loads a plugin
//! `cdylib` and exercises whichever services it exposes. Run with:
//!     cargo build -p gain-dsp
//!     cargo run -p plugin-host --example load_plugin -- \
//!         target/debug/libgain_dsp.dylib   (or .so / .dll)

use plugin_api::Dsp;
use std::path::PathBuf;

fn main() {
    let path = PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("usage: load_plugin <path-to-cdylib>"),
    );

    // Safety: this example is the explicit manual trust boundary for a
    // locally-built plugin passed on the command line.
    let mut plugin = unsafe { plugin_host::load_plugin(&path) }.expect("failed to load plugin");

    println!("loaded: {} {}", plugin.name, plugin.version);

    if let Some(dsp) = plugin.dsp.as_mut() {
        let mut samples = vec![1.0_f32, -1.0, 0.5, -0.5];
        println!("dsp \"{}\" before: {samples:?}", dsp.name());
        dsp.process(&mut samples, 2);
        println!("dsp \"{}\" after:  {samples:?}", dsp.name());
    } else {
        println!("plugin does not expose a Dsp service");
    }
}
