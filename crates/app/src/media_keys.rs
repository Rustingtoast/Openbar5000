//! OS media controls (issue #17): keyboard media keys, headset buttons and
//! the system's now-playing widget -- MPRIS on Linux, the System Media
//! Transport Controls on Windows (both through `souvlaki`), and Apple's
//! MediaPlayer framework on macOS (directly, through the objc2 bindings:
//! souvlaki's macOS side depends on `block` 0.1, which future Rust
//! versions reject). OS events arrive on a channel the UI thread drains
//! (see `wiring`), and are turned into the same actions as the on-screen
//! buttons; in the other direction the OS is told what's playing.

#[cfg(not(target_os = "macos"))]
use crossbeam_channel::{Receiver, Sender};
#[cfg(not(target_os = "macos"))]
use engine::{PlaybackState, Track};
#[cfg(not(target_os = "macos"))]
use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
    SeekDirection,
};
use std::time::Duration;

/// How far the plain "seek forward/backward" media command jumps.
const SEEK_STEP: Duration = Duration::from_secs(10);

/// What the player should do for an OS media event -- kept separate from
/// the platforms' event types so the mapping is testable without an OS.
#[derive(Debug, Clone, PartialEq)]
// macOS has no volume, raise or quit commands.
#[cfg_attr(target_os = "macos", allow(dead_code))]
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

#[cfg(not(target_os = "macos"))]
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

#[cfg(not(target_os = "macos"))]
pub struct MediaKeys {
    controls: MediaControls,
    actions: Receiver<MediaAction>,
}

#[cfg(not(target_os = "macos"))]
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

#[cfg(target_os = "macos")]
pub use mac::MediaKeys;

/// macOS: MPRemoteCommandCenter for the keys and headset buttons,
/// MPNowPlayingInfoCenter for Control Center's now-playing widget.
#[cfg(target_os = "macos")]
mod mac {
    use super::{MediaAction, SEEK_STEP};
    use block2::RcBlock;
    use crossbeam_channel::{Receiver, Sender};
    use engine::{PlaybackState, Track};
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSDictionary, NSNumber, NSString};
    use objc2_media_player::{
        MPChangePlaybackPositionCommandEvent, MPMediaItemPropertyAlbumTitle, MPMediaItemPropertyArtist,
        MPMediaItemPropertyPlaybackDuration, MPMediaItemPropertyTitle, MPNowPlayingInfoCenter,
        MPNowPlayingInfoPropertyElapsedPlaybackTime, MPNowPlayingInfoPropertyPlaybackRate, MPNowPlayingPlaybackState,
        MPRemoteCommand, MPRemoteCommandCenter, MPRemoteCommandEvent, MPRemoteCommandHandlerStatus,
    };
    use std::ptr::NonNull;
    use std::time::Duration;

    pub struct MediaKeys {
        actions: Receiver<MediaAction>,
        /// Each command with the target its handler was registered as, to
        /// unregister on drop.
        targets: Vec<(Retained<MPRemoteCommand>, Retained<AnyObject>)>,
        /// What's playing, re-sent with every state change (the widget
        /// takes the whole dictionary each time).
        track: Option<(String, String, String, f64)>,
    }

    impl MediaKeys {
        /// `_hwnd` is only needed on Windows.
        pub fn new(_hwnd: Option<*mut std::ffi::c_void>) -> Option<Self> {
            let (tx, actions) = crossbeam_channel::unbounded::<MediaAction>();
            // SAFETY: plain calls into MediaPlayer.framework on the main
            // thread; the handlers only send on a channel.
            let center = unsafe { MPRemoteCommandCenter::sharedCommandCenter() };
            let mut targets = Vec::new();
            let mut on = |command: Retained<MPRemoteCommand>, action: MediaAction| {
                let tx: Sender<MediaAction> = tx.clone();
                let handler = RcBlock::new(move |_event: NonNull<MPRemoteCommandEvent>| {
                    let _ = tx.send(action.clone());
                    MPRemoteCommandHandlerStatus::Success
                });
                unsafe { command.setEnabled(true) };
                let target = unsafe { command.addTargetWithHandler(&handler) };
                targets.push((command, target));
            };
            unsafe {
                on(center.playCommand(), MediaAction::Play);
                on(center.pauseCommand(), MediaAction::Pause);
                on(center.togglePlayPauseCommand(), MediaAction::TogglePlayPause);
                on(center.stopCommand(), MediaAction::Stop);
                on(center.nextTrackCommand(), MediaAction::Next);
                on(center.previousTrackCommand(), MediaAction::Previous);
                on(Retained::into_super(center.skipForwardCommand()), MediaAction::SeekBy(SEEK_STEP.as_secs_f64()));
                on(Retained::into_super(center.skipBackwardCommand()), MediaAction::SeekBy(-SEEK_STEP.as_secs_f64()));
            }
            // Dragging the widget's progress bar.
            let position = unsafe { center.changePlaybackPositionCommand() };
            let seek_tx = tx.clone();
            let handler = RcBlock::new(move |event: NonNull<MPRemoteCommandEvent>| {
                // SAFETY: this command only ever sends its own event type.
                let event = unsafe { &*(event.as_ptr() as *const MPChangePlaybackPositionCommandEvent) };
                let seconds = unsafe { event.positionTime() };
                let _ = seek_tx.send(MediaAction::SeekTo(Duration::from_secs_f64(seconds.max(0.0))));
                MPRemoteCommandHandlerStatus::Success
            });
            unsafe { position.setEnabled(true) };
            let target = unsafe { position.addTargetWithHandler(&handler) };
            targets.push((Retained::into_super(position), target));
            Some(Self { actions, targets, track: None })
        }

        pub fn try_recv(&self) -> Option<MediaAction> {
            self.actions.try_recv().ok()
        }

        /// Tells the OS what's playing.
        pub fn set_track(&mut self, track: &Track) {
            let m = &track.metadata;
            self.track = Some((m.title.clone(), m.artist.clone(), m.album.clone(), m.duration.as_secs_f64()));
            self.publish(0.0, 1.0);
        }

        /// Tells the OS whether (and where) we're playing.
        pub fn set_state(&mut self, state: PlaybackState, position: Duration) {
            let (playback, rate) = match state {
                PlaybackState::Playing => (MPNowPlayingPlaybackState::Playing, 1.0),
                PlaybackState::Paused => (MPNowPlayingPlaybackState::Paused, 0.0),
                PlaybackState::Stopped => (MPNowPlayingPlaybackState::Stopped, 0.0),
            };
            if state == PlaybackState::Stopped {
                let center = unsafe { MPNowPlayingInfoCenter::defaultCenter() };
                unsafe {
                    center.setNowPlayingInfo(None);
                    center.setPlaybackState(playback);
                }
                return;
            }
            self.publish(position.as_secs_f64(), rate);
            let center = unsafe { MPNowPlayingInfoCenter::defaultCenter() };
            unsafe { center.setPlaybackState(playback) };
        }

        fn publish(&self, elapsed: f64, rate: f64) {
            let Some((title, artist, album, duration)) = &self.track else { return };
            let strings = [NSString::from_str(title), NSString::from_str(artist), NSString::from_str(album)];
            let numbers = [NSNumber::new_f64(*duration), NSNumber::new_f64(elapsed), NSNumber::new_f64(rate)];
            // SAFETY: MediaPlayer's documented, immutable key constants.
            let keys: [&NSString; 6] = unsafe {
                [
                    MPMediaItemPropertyTitle,
                    MPMediaItemPropertyArtist,
                    MPMediaItemPropertyAlbumTitle,
                    MPMediaItemPropertyPlaybackDuration,
                    MPNowPlayingInfoPropertyElapsedPlaybackTime,
                    MPNowPlayingInfoPropertyPlaybackRate,
                ]
            };
            let values: [&AnyObject; 6] = [
                &strings[0], &strings[1], &strings[2], &numbers[0], &numbers[1], &numbers[2],
            ];
            let info = NSDictionary::from_slices(&keys, &values);
            let center = unsafe { MPNowPlayingInfoCenter::defaultCenter() };
            unsafe { center.setNowPlayingInfo(Some(&info)) };
        }
    }

    impl Drop for MediaKeys {
        fn drop(&mut self) {
            for (command, target) in &self.targets {
                unsafe { command.removeTarget(Some(target)) };
            }
            unsafe { MPNowPlayingInfoCenter::defaultCenter().setNowPlayingInfo(None) };
        }
    }
}

#[cfg(all(test, not(target_os = "macos")))]
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
