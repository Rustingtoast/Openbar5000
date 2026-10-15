//! OS media controls (issue #17): keyboard media keys, headset buttons and
//! the system's now-playing widget -- MPRIS on Linux, the System Media
//! Transport Controls on Windows, Now Playing / remote commands on macOS,
//! all through `souvlaki`. OS events arrive on a channel the UI thread
//! drains (see `wiring`), and are turned into the same actions as the
//! on-screen buttons; in the other direction the OS is told what's playing.

use crossbeam_channel::{Receiver, Sender};
use engine::{PlaybackState, Track};
use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
    SeekDirection,
};
use std::time::Duration;

/// How far the plain "seek forward/backward" media command jumps.
const SEEK_STEP: Duration = Duration::from_secs(10);

/// What the player should do for an OS media event -- kept separate from
/// `souvlaki`'s event type so the mapping is testable without an OS.
#[derive(Debug, Clone, PartialEq)]
pub enum MediaAction {
    Play,
    Pause,
    TogglePlayPause,
    Stop,
    Next,
    Previous,
    /// Jump by this many seconds (negative: backwards).
    SeekBy(f64),
    SeekTo(Duration),
    /// 0.0 to 1.0.
    SetVolume(f32),
    Raise,
    Quit,
}

pub fn action_for(event: MediaControlEvent) -> Option<MediaAction> {
    let signed = |direction: SeekDirection, amount: Duration| match direction {
        SeekDirection::Forward => amount.as_secs_f64(),
        SeekDirection::Backward => -amount.as_secs_f64(),
    };
    Some(match event {
        MediaControlEvent::Play => MediaAction::Play,
        MediaControlEvent::Pause => MediaAction::Pause,
        MediaControlEvent::Toggle => MediaAction::TogglePlayPause,
        MediaControlEvent::Stop => MediaAction::Stop,
        MediaControlEvent::Next => MediaAction::Next,
        MediaControlEvent::Previous => MediaAction::Previous,
        MediaControlEvent::Seek(direction) => MediaAction::SeekBy(signed(direction, SEEK_STEP)),
        MediaControlEvent::SeekBy(direction, amount) => MediaAction::SeekBy(signed(direction, amount)),
        MediaControlEvent::SetPosition(MediaPosition(position)) => MediaAction::SeekTo(position),
        MediaControlEvent::SetVolume(volume) => MediaAction::SetVolume(volume.clamp(0.0, 1.0) as f32),
        MediaControlEvent::Raise => MediaAction::Raise,
        MediaControlEvent::Quit => MediaAction::Quit,
        // We don't open arbitrary URIs from other apps.
        MediaControlEvent::OpenUri(_) => return None,
    })
}

pub struct MediaKeys {
    controls: MediaControls,
    actions: Receiver<MediaAction>,
}

impl MediaKeys {
    /// Registers with the OS. `hwnd` is the main window's handle, needed
    /// (only) on Windows. Failing is not fatal -- the player just won't
    /// react to media keys -- so this logs and returns `None`.
    pub fn new(hwnd: Option<*mut std::ffi::c_void>) -> Option<Self> {
        // souvlaki *panics* on Windows without a window handle (issue #31)
        // rather than returning an error -- never hand it `None` there.
        if cfg!(windows) && hwnd.is_none() {
            tracing::warn!("media keys unavailable: no window handle yet");
            return None;
        }
        let config = PlatformConfig {
            display_name: "Openbar5000",
            dbus_name: "openbar5000",
            hwnd,
        };
        let mut controls = match MediaControls::new(config) {
            Ok(controls) => controls,
            Err(e) => {
                tracing::warn!("media keys unavailable: {e:?}");
                return None;
            }
        };
        let (tx, actions): (Sender<MediaAction>, _) = crossbeam_channel::unbounded();
        if let Err(e) = controls.attach(move |event| {
            if let Some(action) = action_for(event) {
                let _ = tx.send(action);
            }
        }) {
            tracing::warn!("media keys unavailable: {e:?}");
            return None;
        }
        Some(Self { controls, actions })
    }

    pub fn try_recv(&self) -> Option<MediaAction> {
        self.actions.try_recv().ok()
    }

    /// Tells the OS what's playing.
    pub fn set_track(&mut self, track: &Track) {
        let m = &track.metadata;
        let non_empty = |s: &str| (!s.is_empty()).then_some(s.to_string());
        let (title, artist, album) = (non_empty(&m.title), non_empty(&m.artist), non_empty(&m.album));
        let result = self.controls.set_metadata(MediaMetadata {
            title: title.as_deref(),
            artist: artist.as_deref(),
            album: album.as_deref(),
            duration: (!m.duration.is_zero()).then_some(m.duration),
            cover_url: None,
        });
        if let Err(e) = result {
            tracing::debug!("couldn't update media metadata: {e:?}");
        }
    }

    /// Tells the OS whether (and where) we're playing.
    pub fn set_state(&mut self, state: PlaybackState, position: Duration) {
        let progress = Some(MediaPosition(position));
        let playback = match state {
            PlaybackState::Playing => MediaPlayback::Playing { progress },
            PlaybackState::Paused => MediaPlayback::Paused { progress },
            PlaybackState::Stopped => MediaPlayback::Stopped,
        };
        if let Err(e) = self.controls.set_playback(playback) {
            tracing::debug!("couldn't update media playback state: {e:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_events_map_to_player_actions() {
        assert_eq!(action_for(MediaControlEvent::Toggle), Some(MediaAction::TogglePlayPause));
        assert_eq!(action_for(MediaControlEvent::Next), Some(MediaAction::Next));
        assert_eq!(action_for(MediaControlEvent::Previous), Some(MediaAction::Previous));
        assert_eq!(
            action_for(MediaControlEvent::Seek(SeekDirection::Backward)),
            Some(MediaAction::SeekBy(-10.0))
        );
        assert_eq!(
            action_for(MediaControlEvent::SeekBy(SeekDirection::Forward, Duration::from_secs(5))),
            Some(MediaAction::SeekBy(5.0))
        );
        assert_eq!(
            action_for(MediaControlEvent::SetPosition(MediaPosition(Duration::from_secs(42)))),
            Some(MediaAction::SeekTo(Duration::from_secs(42)))
        );
        assert_eq!(action_for(MediaControlEvent::SetVolume(1.7)), Some(MediaAction::SetVolume(1.0)));
        assert_eq!(action_for(MediaControlEvent::OpenUri("https://x".into())), None);
    }
}
