//! Discord Rich Presence ("Listening to ...") integration.
//!
//! This speaks Discord's own local IPC protocol (a unix socket on
//! macOS/Linux, a named pipe on Windows) via the `discord-rich-presence`
//! crate.
//!
//! All IPC happens on a dedicated thread -- `connect`/`set_activity` are
//! blocking calls over a local socket, and this way a slow or wedged
//! Discord process can never stall the UI thread.
use crate::settings::Settings;
use discord_rich_presence::{
    activity::{Activity, ActivityType, Assets, StatusDisplayType, Timestamps},
    DiscordIpc, DiscordIpcClient,
};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, SystemTime};

/// How often to retry connecting while enabled but not yet connected (e.g.
/// Discord hasn't been launched yet, or was just closed).
const RECONNECT_INTERVAL: Duration = Duration::from_secs(5);
/// How often to re-check the connection while already connected -- Discord
/// IPC has no "you got disconnected" push, so this just bounds how long a
/// silently-dropped connection can go unnoticed.
const IDLE_CHECK_INTERVAL: Duration = Duration::from_secs(15);

/// Openbar5000's own shared Discord Application ID, auto-filled into the
/// Client ID field the first time a user enables Rich Presence (see
/// `wiring::on_discord_presence_toggled`) so most people never have to
/// create their own Discord Application -- it's still a plain, editable
/// text field afterward, e.g. to point at a personal application instead.
///
/// A client ID is an opaque identifier tied to *someone's* Discord account
/// in the Developer Portal; nothing running locally can mint a working one
/// on the fly the way a UUID or API key could be generated. This constant
/// has to be replaced with a real Application ID (create one at
/// https://discord.com/developers/applications) before this auto-fill does
/// anything -- left empty, the field simply stays empty and the README's
/// manual setup steps apply, exactly as before this constant existed.
pub const DEFAULT_CLIENT_ID: &str = "";

#[derive(Debug, Clone)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub elapsed: Duration,
    pub is_playing: bool,
}

pub enum PresenceCommand {
    /// Applied whenever the user toggles the feature or edits the client ID
    /// in Preferences.
    Configure { enabled: bool, client_id: String },
    NowPlaying(NowPlaying),
    /// Nothing loaded, or the playlist was stopped/cleared.
    Cleared,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresenceEvent {
    Connected,
    Disconnected,
    /// Feature is off, or no client ID has been configured yet.
    Idle,
}

pub struct PresenceClient {
    command_tx: Sender<PresenceCommand>,
    event_rx: Receiver<PresenceEvent>,
    _thread: std::thread::JoinHandle<()>,
}

impl PresenceClient {
    pub fn spawn(settings: &Settings) -> Self {
        let (command_tx, command_rx) = crossbeam_channel::unbounded();
        let (event_tx, event_rx) = crossbeam_channel::unbounded();
        let initial = PresenceCommand::Configure {
            enabled: settings.discord_presence_enabled,
            client_id: settings.discord_client_id.clone(),
        };
        let _ = command_tx.send(initial);
        let thread = std::thread::Builder::new()
            .name("openbar5000-discord-presence".into())
            .spawn(move || run(command_rx, event_tx))
            .expect("failed to spawn discord presence thread");
        Self {
            command_tx,
            event_rx,
            _thread: thread,
        }
    }

    pub fn send(&self, command: PresenceCommand) {
        let _ = self.command_tx.send(command);
    }

    /// Non-blocking; call this from a UI timer to drain pending events.
    pub fn try_recv_event(&self) -> Option<PresenceEvent> {
        self.event_rx.try_recv().ok()
    }
}

impl Drop for PresenceClient {
    fn drop(&mut self) {
        let _ = self.command_tx.send(PresenceCommand::Configure {
            enabled: false,
            client_id: String::new(),
        });
    }
}

struct State {
    enabled: bool,
    client_id: String,
    client: Option<DiscordIpcClient>,
    connected: bool,
    /// Re-applied on every successful (re)connect, since connecting resets
    /// any activity Discord previously had for this client ID.
    last_activity: Option<ActivityState>,
}

#[derive(Clone)]
enum ActivityState {
    NowPlaying(NowPlaying),
    Cleared,
}

fn run(command_rx: Receiver<PresenceCommand>, event_tx: Sender<PresenceEvent>) {
    let mut state = State {
        enabled: false,
        client_id: String::new(),
        client: None,
        connected: false,
        last_activity: None,
    };

    loop {
        let timeout = if state.connected {
            IDLE_CHECK_INTERVAL
        } else {
            RECONNECT_INTERVAL
        };
        match command_rx.recv_timeout(timeout) {
            Ok(command) => handle_command(&mut state, command, &event_tx),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        if state.enabled && !state.client_id.is_empty() && !state.connected {
            try_connect(&mut state, &event_tx);
        }
    }

    disconnect(&mut state, &event_tx);
}

fn handle_command(state: &mut State, command: PresenceCommand, event_tx: &Sender<PresenceEvent>) {
    match command {
        PresenceCommand::Configure { enabled, client_id } => {
            let client_id_changed = client_id != state.client_id;
            state.enabled = enabled;
            state.client_id = client_id;

            if !enabled || client_id_changed {
                disconnect(state, event_tx);
            }
            if !enabled {
                let _ = event_tx.send(PresenceEvent::Idle);
            }
        }
        PresenceCommand::NowPlaying(now_playing) => {
            state.last_activity = Some(ActivityState::NowPlaying(now_playing.clone()));
            apply_activity(state, &ActivityState::NowPlaying(now_playing), event_tx);
        }
        PresenceCommand::Cleared => {
            state.last_activity = Some(ActivityState::Cleared);
            apply_activity(state, &ActivityState::Cleared, event_tx);
        }
    }
}

fn apply_activity(state: &mut State, activity: &ActivityState, event_tx: &Sender<PresenceEvent>) {
    if !state.enabled || !state.connected {
        return;
    }
    let Some(client) = state.client.as_mut() else {
        return;
    };
    let result = match activity {
        ActivityState::NowPlaying(now_playing) => {
            client.set_activity(build_activity(now_playing))
        }
        ActivityState::Cleared => client.clear_activity(),
    };
    if result.is_err() {
        // The pipe most likely died (Discord/the bridge closed) -- drop the
        // connection and let the next idle tick retry it.
        state.connected = false;
        state.client = None;
        let _ = event_tx.send(PresenceEvent::Disconnected);
    }
}

fn try_connect(state: &mut State, event_tx: &Sender<PresenceEvent>) {
    let mut client = DiscordIpcClient::new(&state.client_id);
    if client.connect().is_ok() {
        state.client = Some(client);
        state.connected = true;
        let _ = event_tx.send(PresenceEvent::Connected);
        if let Some(activity) = state.last_activity.clone() {
            apply_activity(state, &activity, event_tx);
        }
    }
}

fn disconnect(state: &mut State, event_tx: &Sender<PresenceEvent>) {
    if let Some(mut client) = state.client.take() {
        let _ = client.close();
    }
    if state.connected {
        state.connected = false;
        let _ = event_tx.send(PresenceEvent::Disconnected);
    }
}

/// Builds the `SET_ACTIVITY` payload for a now-playing track. Kept separate
/// from all IPC/socket code so it's testable without a live Discord
/// connection -- see the tests below, which inspect the serialized JSON.
fn build_activity(now_playing: &NowPlaying) -> Activity<'_> {
    let title = if now_playing.title.is_empty() {
        "Unknown title"
    } else {
        &now_playing.title
    };
    let state_text = match (now_playing.artist.is_empty(), now_playing.album.is_empty()) {
        (true, _) => "Unknown artist".to_string(),
        (false, true) => now_playing.artist.clone(),
        (false, false) => format!("{} \u{2014} {}", now_playing.artist, now_playing.album),
    };

    let mut activity = Activity::new()
        .details(title)
        .state(state_text)
        .activity_type(ActivityType::Listening)
        .status_display_type(StatusDisplayType::Details)
        .assets(
            Assets::new()
                .large_image("logo")
                .large_text("Openbar5000")
                .small_image(if now_playing.is_playing {
                    "play"
                } else {
                    "pause"
                })
                .small_text(if now_playing.is_playing {
                    "Playing"
                } else {
                    "Paused"
                }),
        );

    // Only show a running elapsed-time bar while actually playing -- a
    // ticking clock on a paused track would be misleading.
    if now_playing.is_playing {
        if let Some(start_ms) = unix_millis_now().checked_sub(now_playing.elapsed.as_millis() as i64)
        {
            activity = activity.timestamps(Timestamps::new().start(start_ms));
        }
    }

    activity
}

fn unix_millis_now() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(is_playing: bool) -> NowPlaying {
        NowPlaying {
            title: "Song Title".to_string(),
            artist: "Some Artist".to_string(),
            album: "Some Album".to_string(),
            elapsed: Duration::from_secs(30),
            is_playing,
        }
    }

    #[test]
    fn listening_activity_shows_title_as_details_and_artist_album_as_state() {
        let json = serde_json::to_value(build_activity(&sample(true))).unwrap();
        assert_eq!(json["details"], "Song Title");
        assert_eq!(json["state"], "Some Artist \u{2014} Some Album");
        assert_eq!(json["type"], 2); // ActivityType::Listening
        assert_eq!(json["status_display_type"], 2); // StatusDisplayType::Details
    }

    #[test]
    fn playing_includes_a_start_timestamp_but_paused_does_not() {
        let playing = serde_json::to_value(build_activity(&sample(true))).unwrap();
        assert!(playing["timestamps"]["start"].is_number());

        let paused = serde_json::to_value(build_activity(&sample(false))).unwrap();
        assert!(paused.get("timestamps").is_none());
    }

    #[test]
    fn missing_artist_or_album_degrades_gracefully() {
        let mut no_album = sample(true);
        no_album.album.clear();
        let json = serde_json::to_value(build_activity(&no_album)).unwrap();
        assert_eq!(json["state"], "Some Artist");

        let mut no_artist = sample(true);
        no_artist.artist.clear();
        no_artist.album.clear();
        let json = serde_json::to_value(build_activity(&no_artist)).unwrap();
        assert_eq!(json["state"], "Unknown artist");
    }
}
