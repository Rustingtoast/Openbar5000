use crate::decoders::SymphoniaDecoder;
use crate::events::PlaybackState;
use crate::metadata::load_metadata;
use crate::output::CpalSink;
use crate::pipeline::Pipeline;
use crate::playlist::{Playlist, Track};
use crate::{EngineCommand, EngineEvent};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use plugin_api::OutputSink;
use std::time::{Duration, Instant};

const POSITION_UPDATE_INTERVAL: Duration = Duration::from_millis(200);
const IDLE_POLL_INTERVAL: Duration = Duration::from_millis(50);
const WAVEFORM_BUCKET_COUNT: usize = 400;

/// Owns the dedicated playback thread. The UI never touches decoding, DSP,
/// or output directly -- it only sends [`EngineCommand`]s and drains
/// [`EngineEvent`]s, keeping the realtime audio path fully decoupled from the
/// Slint main thread.
pub struct PlaybackEngine {
    command_tx: Sender<EngineCommand>,
    event_rx: Receiver<EngineEvent>,
    _thread: std::thread::JoinHandle<()>,
}

impl PlaybackEngine {
    pub fn spawn() -> Self {
        let (command_tx, command_rx) = crossbeam_channel::unbounded();
        let (event_tx, event_rx) = crossbeam_channel::unbounded();
        let thread = std::thread::Builder::new()
            .name("openbar5000-playback".into())
            .spawn(move || run(command_rx, event_tx))
            .expect("failed to spawn playback thread");
        Self {
            command_tx,
            event_rx,
            _thread: thread,
        }
    }

    pub fn send(&self, command: EngineCommand) {
        let _ = self.command_tx.send(command);
    }

    /// Non-blocking; call this from a UI timer to drain pending events.
    pub fn try_recv_event(&self) -> Option<EngineEvent> {
        self.event_rx.try_recv().ok()
    }
}

impl Drop for PlaybackEngine {
    fn drop(&mut self) {
        self.send(EngineCommand::Stop);
    }
}

struct EngineState {
    playlist: Playlist,
    pipeline: Pipeline,
    playback_state: PlaybackState,
    sample_rate: u32,
    channels: u16,
    samples_played: u64,
    volume: f32,
    waveform_enabled: bool,
}

fn run(command_rx: Receiver<EngineCommand>, event_tx: Sender<EngineEvent>) {
    let sink = match CpalSink::default_device() {
        Ok(sink) => Box::new(sink) as Box<dyn OutputSink>,
        Err(e) => {
            let _ = event_tx.send(EngineEvent::Error(e.to_string()));
            return;
        }
    };

    let mut state = EngineState {
        playlist: Playlist::new(),
        pipeline: Pipeline::new(sink),
        playback_state: PlaybackState::Stopped,
        sample_rate: 44_100,
        channels: 2,
        samples_played: 0,
        volume: 1.0,
        waveform_enabled: false,
    };

    let mut last_position_sent = Instant::now();

    loop {
        let poll_timeout = if state.playback_state == PlaybackState::Playing {
            Duration::from_millis(0)
        } else {
            IDLE_POLL_INTERVAL
        };

        match command_rx.recv_timeout(poll_timeout) {
            Ok(command) => {
                if !handle_command(&mut state, command, &event_tx) {
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        if state.playback_state != PlaybackState::Playing {
            continue;
        }

        match state.pipeline.process_chunk() {
            Ok(0) => advance_to_next_track(&mut state, &event_tx),
            Ok(written) => {
                state.samples_played += written as u64;
                if let Some((bars, peaks)) = state.pipeline.take_spectrum() {
                    let _ = event_tx.send(EngineEvent::SpectrumUpdate { bars, peaks });
                }
                if last_position_sent.elapsed() >= POSITION_UPDATE_INTERVAL {
                    send_position_update(&state, &event_tx);
                    last_position_sent = Instant::now();
                }
            }
            Err(e) => {
                let _ = event_tx.send(EngineEvent::Error(e.to_string()));
                set_playback_state(&mut state, PlaybackState::Stopped, &event_tx);
            }
        }
    }
}

/// Returns `false` when the thread should exit.
fn handle_command(
    state: &mut EngineState,
    command: EngineCommand,
    event_tx: &Sender<EngineEvent>,
) -> bool {
    match command {
        EngineCommand::Play => {
            if state.playlist.current().is_none() && !state.playlist.is_empty() {
                load_track(state, 0, event_tx);
            }
            if state.playlist.current().is_some() {
                set_playback_state(state, PlaybackState::Playing, event_tx);
            }
        }
        EngineCommand::Pause => set_playback_state(state, PlaybackState::Paused, event_tx),
        EngineCommand::Stop => {
            set_playback_state(state, PlaybackState::Stopped, event_tx);
            state.samples_played = 0;
        }
        EngineCommand::Seek(pos) => {
            if let Err(e) = state.pipeline.seek(pos) {
                let _ = event_tx.send(EngineEvent::Error(e.to_string()));
            } else {
                state.samples_played =
                    (pos.as_secs_f64() * state.sample_rate as f64 * state.channels as f64) as u64;
                send_position_update(state, event_tx);
            }
        }
        EngineCommand::Next => advance_to_next_track(state, event_tx),
        EngineCommand::Previous => {
            let was_playing = state.playback_state == PlaybackState::Playing;
            let index = state
                .playlist
                .current_index()
                .and_then(|i| i.checked_sub(1))
                .unwrap_or(0);
            load_track(state, index, event_tx);
            if was_playing {
                set_playback_state(state, PlaybackState::Playing, event_tx);
            }
        }
        EngineCommand::SetVolume(volume) => {
            state.volume = volume.clamp(0.0, 1.0);
            state.pipeline.set_volume(state.volume);
        }
        EngineCommand::LoadPlaylist(paths) => {
            let tracks: Vec<Track> = paths
                .into_iter()
                .map(|path| {
                    let metadata = load_metadata(&path);
                    Track { path, metadata }
                })
                .collect();
            state.playlist = Playlist::from_tracks(tracks);
            state.samples_played = 0;
            send_playlist_update(state, event_tx);
        }
        EngineCommand::PlayIndex(index) => {
            load_track(state, index, event_tx);
            set_playback_state(state, PlaybackState::Playing, event_tx);
        }
        EngineCommand::RemoveIndex(index) => {
            let removed_current = state.playlist.current_index() == Some(index);
            state.playlist.remove(index);
            if removed_current {
                set_playback_state(state, PlaybackState::Stopped, event_tx);
                state.samples_played = 0;
            }
            send_playlist_update(state, event_tx);
        }
        EngineCommand::ClearPlaylist => {
            state.playlist.clear();
            set_playback_state(state, PlaybackState::Stopped, event_tx);
            state.samples_played = 0;
            send_playlist_update(state, event_tx);
        }
        EngineCommand::SetSpectrumEnabled(enabled) => {
            state.pipeline.set_spectrum_enabled(enabled);
        }
        EngineCommand::SetWaveformEnabled(enabled) => {
            state.waveform_enabled = enabled;
        }
        EngineCommand::SetSpectrumConfig(config) => {
            state.pipeline.set_spectrum_config(config);
        }
    }
    true
}

fn send_playlist_update(state: &EngineState, event_tx: &Sender<EngineEvent>) {
    let _ = event_tx.send(EngineEvent::PlaylistUpdated {
        tracks: state.playlist.tracks().to_vec(),
        current_index: state.playlist.current_index(),
    });
}

fn load_track(state: &mut EngineState, index: usize, event_tx: &Sender<EngineEvent>) {
    let Some(track) = state.playlist.set_current_index(index).cloned() else {
        set_playback_state(state, PlaybackState::Stopped, event_tx);
        return;
    };

    let decoder = Box::new(SymphoniaDecoder::new());
    match state.pipeline.open_track(decoder, &track.path) {
        Ok(info) => {
            state.sample_rate = info.sample_rate;
            state.channels = info.channels;
            state.samples_played = 0;
            if state.waveform_enabled {
                spawn_waveform_computation(track.path.clone(), index, event_tx.clone());
            }
            let _ = event_tx.send(EngineEvent::TrackChanged { index, track });
        }
        Err(e) => {
            let _ = event_tx.send(EngineEvent::Error(e.to_string()));
            set_playback_state(state, PlaybackState::Stopped, event_tx);
        }
    }
}

/// Computes waveform peaks on a dedicated, short-lived thread with its own
/// decoder/file handle, so a multi-minute track doesn't stall playback (the
/// playback thread is busy decoding the *same* file for output at the same
/// time).
fn spawn_waveform_computation(
    path: std::path::PathBuf,
    index: usize,
    event_tx: Sender<EngineEvent>,
) {
    std::thread::Builder::new()
        .name("openbar5000-waveform".into())
        .spawn(move || {
            if let Ok(peaks) = crate::waveform::compute_peaks(&path, WAVEFORM_BUCKET_COUNT) {
                let _ = event_tx.send(EngineEvent::WaveformReady { index, peaks });
            }
        })
        .expect("failed to spawn waveform thread");
}

fn advance_to_next_track(state: &mut EngineState, event_tx: &Sender<EngineEvent>) {
    let next_index = state.playlist.current_index().map(|i| i + 1).unwrap_or(0);
    if next_index < state.playlist.len() {
        let was_playing = state.playback_state == PlaybackState::Playing;
        load_track(state, next_index, event_tx);
        if was_playing {
            set_playback_state(state, PlaybackState::Playing, event_tx);
        }
    } else {
        set_playback_state(state, PlaybackState::Stopped, event_tx);
    }
}

fn set_playback_state(
    state: &mut EngineState,
    new_state: PlaybackState,
    event_tx: &Sender<EngineEvent>,
) {
    if state.playback_state != new_state {
        state.playback_state = new_state;
        let _ = event_tx.send(EngineEvent::PlaybackStateChanged(new_state));
    }
}

fn send_position_update(state: &EngineState, event_tx: &Sender<EngineEvent>) {
    let frames_played = state.samples_played / state.channels.max(1) as u64;
    let position = Duration::from_secs_f64(frames_played as f64 / state.sample_rate.max(1) as f64);
    let duration = state
        .playlist
        .current()
        .map(|t| t.metadata.duration)
        .unwrap_or_default();
    let _ = event_tx.send(EngineEvent::PositionUpdate { position, duration });
}
