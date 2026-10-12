//! Manual playback checkpoint (milestone-1 step 4): plays a single audio file
//! through the real output device. Run with:
//!     cargo run -p engine --example play_file -- path/to/track.flac

use engine::{EngineCommand, EngineEvent, PlaybackEngine, PlaybackState};
use std::time::Duration;

fn main() {
    tracing_subscriber::fmt::init();

    let path = std::env::args().nth(1).expect("usage: play_file <path>");
    let engine = PlaybackEngine::spawn();

    engine.send(EngineCommand::LoadPlaylist(vec![path.into()]));
    engine.send(EngineCommand::Play);

    let mut started = false;
    loop {
        while let Some(event) = engine.try_recv_event() {
            println!("{event:?}");
            match event {
                EngineEvent::PlaybackStateChanged(PlaybackState::Playing) => started = true,
                EngineEvent::PlaybackStateChanged(PlaybackState::Stopped) if started => return,
                EngineEvent::Error(_) => return,
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
