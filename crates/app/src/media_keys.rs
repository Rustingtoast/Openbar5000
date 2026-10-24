//! OS media controls (issue #17): keyboard media keys, headset buttons and
//! the system's now-playing widget -- MPRIS on Linux (served directly over
//! zbus, see `mpris`), the System Media Transport Controls on Windows
//! (through `souvlaki`), and Apple's MediaPlayer framework on macOS
//! (directly, through the objc2 bindings: souvlaki's macOS side depends on
//! `block` 0.1, which future Rust versions reject). OS events arrive on a
//! channel the UI thread drains (see `wiring`), and are turned into the
//! same actions as the on-screen buttons; in the other direction the OS is
//! told what's playing.

#[cfg(windows)]
use crossbeam_channel::{Receiver, Sender};
#[cfg(windows)]
use engine::{PlaybackState, Track};
#[cfg(windows)]
use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
    SeekDirection,
};
use std::time::Duration;

/// How far the plain "seek forward/backward" media command jumps. (MPRIS
/// always says how far.)
#[cfg(any(windows, target_os = "macos"))]
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

#[cfg(windows)]
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

#[cfg(windows)]
pub struct MediaKeys {
    controls: MediaControls,
    actions: Receiver<MediaAction>,
}

#[cfg(windows)]
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

    /// Playback jumped to `position` (a seek). Only MPRIS has a way to say
    /// so (see `mpris`).
    pub fn seeked(&mut self, _position: Duration) {}
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

        /// Playback jumped to `position` (a seek). Only MPRIS has a way to
        /// say so (see `mpris`).
        pub fn seeked(&mut self, _position: Duration) {}

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

#[cfg(all(unix, not(target_os = "macos")))]
pub use mpris::MediaKeys;

/// Linux: MPRIS, the D-Bus interface the desktop's media keys and
/// now-playing widgets use, served directly over zbus. (souvlaki's Linux
/// side looks for something to tell the bus a hundred times a second, even
/// with nothing playing.) Calls from the bus run on zbus's own thread and
/// only send actions on a channel; the UI thread updates what's reported,
/// and the bus is told about each change as it's made.
#[cfg(all(unix, not(target_os = "macos")))]
mod mpris {
    use super::MediaAction;
    use crossbeam_channel::{Receiver, Sender};
    use engine::{PlaybackState, Track};
    use std::collections::HashMap;
    use std::time::{Duration, Instant};
    use zbus::blocking::object_server::InterfaceRef;
    use zbus::blocking::Connection;
    use zbus::object_server::SignalEmitter;
    use zbus::zvariant::{ObjectPath, OwnedValue, Str, Value};

    const BUS_NAME: &str = "org.mpris.MediaPlayer2.openbar5000";
    const PATH: &str = "/org/mpris/MediaPlayer2";
    /// The track id MPRIS reserves for "nothing loaded".
    const NO_TRACK: &str = "/org/mpris/MediaPlayer2/TrackList/NoTrack";

    pub struct MediaKeys {
        connection: Connection,
        actions: Receiver<MediaAction>,
        /// How many tracks have been shown, numbering their ids.
        tracks_shown: u64,
    }

    impl MediaKeys {
        /// Takes the player's name on the session bus. `_hwnd` is only
        /// needed on Windows. Failing is not fatal -- the player just won't
        /// react to media keys -- so this logs and returns `None`.
        pub fn new(_hwnd: Option<*mut std::ffi::c_void>) -> Option<Self> {
            let (tx, actions) = crossbeam_channel::unbounded();
            let root = Root {
                actions: tx.clone(),
                desktop_entry: crate::platform::desktop_id(),
            };
            let connection = zbus::blocking::connection::Builder::session()
                .and_then(|builder| builder.serve_at(PATH, root))
                .and_then(|builder| builder.serve_at(PATH, Player::new(tx)))
                .and_then(|builder| builder.name(BUS_NAME))
                .and_then(|builder| builder.build());
            match connection {
                Ok(connection) => Some(Self { connection, actions, tracks_shown: 0 }),
                Err(e) => {
                    tracing::warn!("media keys unavailable: {e}");
                    None
                }
            }
        }

        pub fn try_recv(&self) -> Option<MediaAction> {
            self.actions.try_recv().ok()
        }

        /// Tells the OS what's playing.
        pub fn set_track(&mut self, track: &Track) {
            self.tracks_shown += 1;
            let shown = ShownTrack::new(self.tracks_shown, track);
            self.update("metadata", |player, emitter| {
                player.track = Some(shown);
                player.position = Position::new(Duration::ZERO, player.status == PlaybackState::Playing);
                zbus::block_on(player.metadata_changed(emitter))
            });
        }

        /// Tells the OS whether (and where) we're playing.
        pub fn set_state(&mut self, state: PlaybackState, position: Duration) {
            self.update("playback state", |player, emitter| {
                player.status = state;
                player.position = Position::new(position, state == PlaybackState::Playing);
                zbus::block_on(player.playback_status_changed(emitter))
            });
        }

        /// Playback jumped to `position` (a seek): widgets keep moving the
        /// position on by themselves, from where they were last told.
        pub fn seeked(&mut self, position: Duration) {
            self.update("position", |player, emitter| {
                player.position = Position::new(position, player.status == PlaybackState::Playing);
                zbus::block_on(Player::seeked(emitter, micros(position)))
            });
        }

        /// Changes what the player interface reports with `change`, which
        /// also tells the bus about it.
        fn update(&self, what: &str, change: impl FnOnce(&mut Player, &SignalEmitter<'static>) -> zbus::Result<()>) {
            let result = self
                .connection
                .object_server()
                .interface::<_, Player>(PATH)
                .and_then(|player: InterfaceRef<Player>| change(&mut player.get_mut(), player.signal_emitter()));
            if let Err(e) = result {
                tracing::debug!("couldn't update the media {what}: {e}");
            }
        }
    }

    /// `org.mpris.MediaPlayer2`: the player as an application.
    struct Root {
        actions: Sender<MediaAction>,
        desktop_entry: String,
    }

    #[zbus::interface(name = "org.mpris.MediaPlayer2")]
    impl Root {
        fn raise(&self) {
            let _ = self.actions.send(MediaAction::Raise);
        }

        fn quit(&self) {
            let _ = self.actions.send(MediaAction::Quit);
        }

        #[zbus(property)]
        fn can_quit(&self) -> bool {
            true
        }

        #[zbus(property)]
        fn can_raise(&self) -> bool {
            true
        }

        #[zbus(property)]
        fn has_track_list(&self) -> bool {
            false
        }

        #[zbus(property)]
        fn identity(&self) -> &str {
            "Openbar5000"
        }

        /// Where the widget finds the app's icon.
        #[zbus(property)]
        fn desktop_entry(&self) -> &str {
            &self.desktop_entry
        }

        #[zbus(property)]
        fn supported_uri_schemes(&self) -> Vec<String> {
            Vec::new()
        }

        #[zbus(property)]
        fn supported_mime_types(&self) -> Vec<String> {
            Vec::new()
        }
    }

    /// `org.mpris.MediaPlayer2.Player`: playback.
    struct Player {
        actions: Sender<MediaAction>,
        status: PlaybackState,
        track: Option<ShownTrack>,
        position: Position,
        /// As last set over the bus.
        volume: f64,
    }

    impl Player {
        fn new(actions: Sender<MediaAction>) -> Self {
            Self {
                actions,
                status: PlaybackState::Stopped,
                track: None,
                position: Position::new(Duration::ZERO, false),
                volume: 1.0,
            }
        }

        fn send(&self, action: MediaAction) {
            let _ = self.actions.send(action);
        }
    }

    #[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
    impl Player {
        fn next(&self) {
            self.send(MediaAction::Next);
        }

        fn previous(&self) {
            self.send(MediaAction::Previous);
        }

        fn pause(&self) {
            self.send(MediaAction::Pause);
        }

        fn play_pause(&self) {
            self.send(MediaAction::TogglePlayPause);
        }

        fn stop(&self) {
            self.send(MediaAction::Stop);
        }

        fn play(&self) {
            self.send(MediaAction::Play);
        }

        /// `offset` in microseconds, either way.
        fn seek(&self, offset: i64) {
            self.send(MediaAction::SeekBy(offset as f64 / 1_000_000.0));
        }

        /// Ignored unless it's for the track playing now, and within it --
        /// a widget can still be showing the previous one.
        fn set_position(&self, track_id: ObjectPath<'_>, position: i64) {
            if let Some(position) = self.track.as_ref().and_then(|track| track.seek_target(&track_id, position)) {
                self.send(MediaAction::SeekTo(position));
            }
        }

        /// We don't open arbitrary URIs from other apps.
        fn open_uri(&self, _uri: &str) {}

        #[zbus(signal)]
        async fn seeked(emitter: &SignalEmitter<'_>, position: i64) -> zbus::Result<()>;

        #[zbus(property)]
        fn playback_status(&self) -> &str {
            match self.status {
                PlaybackState::Playing => "Playing",
                PlaybackState::Paused => "Paused",
                PlaybackState::Stopped => "Stopped",
            }
        }

        #[zbus(property)]
        fn rate(&self) -> f64 {
            1.0
        }

        #[zbus(property)]
        fn minimum_rate(&self) -> f64 {
            1.0
        }

        #[zbus(property)]
        fn maximum_rate(&self) -> f64 {
            1.0
        }

        #[zbus(property)]
        fn metadata(&self) -> HashMap<String, OwnedValue> {
            metadata(self.track.as_ref())
        }

        #[zbus(property)]
        fn volume(&self) -> f64 {
            self.volume
        }

        #[zbus(property)]
        fn set_volume(&mut self, volume: f64) {
            self.volume = volume.clamp(0.0, 1.0);
            self.send(MediaAction::SetVolume(self.volume as f32));
        }

        /// Read when asked, never announced: widgets move it on themselves
        /// while playing (see `seeked`).
        #[zbus(property(emits_changed_signal = "false"))]
        fn position(&self) -> i64 {
            let length = self.track.as_ref().map_or(Duration::ZERO, |track| track.length);
            micros(self.position.now(length))
        }

        #[zbus(property)]
        fn can_go_next(&self) -> bool {
            true
        }

        #[zbus(property)]
        fn can_go_previous(&self) -> bool {
            true
        }

        #[zbus(property)]
        fn can_play(&self) -> bool {
            true
        }

        #[zbus(property)]
        fn can_pause(&self) -> bool {
            true
        }

        #[zbus(property)]
        fn can_seek(&self) -> bool {
            true
        }

        #[zbus(property)]
        fn can_control(&self) -> bool {
            true
        }
    }

    /// The track as reported: its tags, and an id of its own.
    #[derive(Debug, Clone, PartialEq)]
    struct ShownTrack {
        id: String,
        title: String,
        artist: String,
        album_artist: String,
        album: String,
        length: Duration,
    }

    impl ShownTrack {
        /// The `number`th track shown.
        fn new(number: u64, track: &Track) -> Self {
            let m = &track.metadata;
            Self {
                id: format!("/io/github/rustingtoast/Openbar5000/Track/{number}"),
                title: m.title.clone(),
                artist: m.artist.clone(),
                album_artist: m.album_artist.clone(),
                album: m.album.clone(),
                length: m.duration,
            }
        }

        /// Where a `SetPosition(track_id, position)` asks to go, if it's
        /// for this track and inside it.
        fn seek_target(&self, track_id: &str, position: i64) -> Option<Duration> {
            let position = Duration::from_micros(u64::try_from(position).ok()?);
            let inside = self.length.is_zero() || position <= self.length;
            (track_id == self.id && inside).then_some(position)
        }
    }

    /// The `Metadata` property: the id alone with nothing loaded.
    fn metadata(track: Option<&ShownTrack>) -> HashMap<String, OwnedValue> {
        let mut metadata = HashMap::new();
        let path = |id: &str| OwnedValue::from(ObjectPath::try_from(id.to_string()).expect("a valid object path"));
        let Some(track) = track else {
            metadata.insert("mpris:trackid".to_string(), path(NO_TRACK));
            return metadata;
        };
        metadata.insert("mpris:trackid".to_string(), path(&track.id));
        if !track.length.is_zero() {
            metadata.insert("mpris:length".to_string(), OwnedValue::from(micros(track.length)));
        }
        let text = |s: &str| OwnedValue::from(Str::from(s.to_string()));
        // Lists of names, of which we only ever have one.
        let names = |s: &str| OwnedValue::try_from(Value::from(vec![s.to_string()])).expect("no file descriptors");
        for (key, value, list) in [
            ("xesam:title", &track.title, false),
            ("xesam:album", &track.album, false),
            ("xesam:artist", &track.artist, true),
            ("xesam:albumArtist", &track.album_artist, true),
        ] {
            if !value.is_empty() {
                metadata.insert(key.to_string(), if list { names(value) } else { text(value) });
            }
        }
        metadata
    }

    /// Where playback is: `at` a moment, and moving on from there while
    /// playing.
    #[derive(Debug, Clone, Copy)]
    struct Position {
        at: Duration,
        since: Instant,
        moving: bool,
    }

    impl Position {
        fn new(at: Duration, moving: bool) -> Self {
            Self { at, since: Instant::now(), moving }
        }

        /// Now, but not past the end of a track `length` long (if known).
        fn now(&self, length: Duration) -> Duration {
            self.at_time(Instant::now(), length)
        }

        fn at_time(&self, now: Instant, length: Duration) -> Duration {
            let position = if self.moving { self.at + now.saturating_duration_since(self.since) } else { self.at };
            if length.is_zero() {
                position
            } else {
                position.min(length)
            }
        }
    }

    /// MPRIS gives times in microseconds.
    fn micros(time: Duration) -> i64 {
        i64::try_from(time.as_micros()).unwrap_or(i64::MAX)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn track() -> ShownTrack {
            ShownTrack {
                id: "/io/github/rustingtoast/Openbar5000/Track/3".to_string(),
                title: "Lumière".to_string(),
                artist: "Lorien Testard".to_string(),
                album_artist: String::new(),
                album: "Clair Obscur: Expedition 33".to_string(),
                length: Duration::from_secs(200),
            }
        }

        #[test]
        fn metadata_lists_the_tags_there_are() {
            let metadata = metadata(Some(&track()));
            let get = |key: &str| Value::try_from(&metadata[key]).unwrap();
            assert_eq!(get("mpris:trackid"), Value::from(ObjectPath::try_from(track().id).unwrap()));
            assert_eq!(get("mpris:length"), Value::from(200_000_000i64));
            assert_eq!(get("xesam:title"), Value::from("Lumière"));
            assert_eq!(get("xesam:artist"), Value::from(vec!["Lorien Testard".to_string()]));
            assert!(!metadata.contains_key("xesam:albumArtist"));
        }

        #[test]
        fn nothing_loaded_is_the_reserved_track_id_alone() {
            let metadata = metadata(None);
            assert_eq!(metadata.len(), 1);
            assert_eq!(
                Value::try_from(&metadata["mpris:trackid"]).unwrap(),
                Value::from(ObjectPath::try_from(NO_TRACK).unwrap())
            );
        }

        #[test]
        fn set_position_only_moves_within_the_current_track() {
            let track = track();
            assert_eq!(track.seek_target(&track.id, 5_000_000), Some(Duration::from_secs(5)));
            assert_eq!(track.seek_target("/io/github/rustingtoast/Openbar5000/Track/2", 5_000_000), None);
            assert_eq!(track.seek_target(&track.id, -1), None);
            assert_eq!(track.seek_target(&track.id, 201_000_000), None);
        }

        #[test]
        fn position_moves_on_only_while_playing_and_stops_at_the_end() {
            let length = Duration::from_secs(200);
            let playing = Position::new(Duration::from_secs(10), true);
            let later = playing.since + Duration::from_secs(5);
            assert_eq!(playing.at_time(later, length), Duration::from_secs(15));
            assert_eq!(playing.at_time(playing.since + Duration::from_secs(500), length), length);
            let paused = Position::new(Duration::from_secs(10), false);
            assert_eq!(paused.at_time(later, length), Duration::from_secs(10));
        }
    }
}

#[cfg(all(test, windows))]
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
