use crate::clock::PlaybackClock;
use crate::decoders::SymphoniaDecoder;
use crate::events::PlaybackState;
use crate::library::scan_audio_files;
use crate::metadata::load_metadata_parallel;
use crate::output::CpalSink;
use crate::pipeline::Pipeline;
use crate::playlist::{Playlist, Track};
use crate::shuffle::ShuffleOrder;
use crate::{EngineCommand, EngineEvent, PlaybackMode};
use crossbeam_channel::{Receiver, Sender};
use plugin_api::OutputSink;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Most chunks decoded in one go before checking for commands again, so a
/// refill never delays a pause or skip noticeably.
const MAX_CHUNKS_PER_REFILL: usize = 32;

const LOAD_PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
/// The longest the playback thread sleeps while playing.
const PLAYING_POLL_INTERVAL: Duration = Duration::from_millis(50);
/// How often to check whether the output should move to another device
/// (the system default changed, or a chosen device came back) where the
/// system doesn't say (see `output_checks`).
const OUTPUT_CHECK_INTERVAL: Duration = Duration::from_secs(2);
/// Points per channel in the waveform outline -- about one per pixel of a
/// full-width seekbar, so the outline stays smooth.
const WAVEFORM_BUCKET_COUNT: usize = 1000;
/// How long a track must stay current before its waveform is computed --
/// long enough that skipping through tracks never starts one.
const WAVEFORM_START_DELAY: Duration = Duration::from_millis(250);

/// Owns the dedicated playback thread. The UI never touches decoding, DSP,
/// or output directly -- it only sends [`EngineCommand`]s and drains
/// [`EngineEvent`]s, keeping the realtime audio path fully decoupled from the
/// Slint main thread.
pub struct PlaybackEngine {
    command_tx: Sender<EngineCommand>,
    event_rx: Receiver<EngineEvent>,
    clock: PlaybackClock,
    _thread: std::thread::JoinHandle<()>,
}

impl PlaybackEngine {
    pub fn spawn() -> Self {
        let (command_tx, command_rx) = crossbeam_channel::unbounded();
        let (event_tx, event_rx) = crossbeam_channel::unbounded();
        let clock = PlaybackClock::default();
        let engine_clock = clock.clone();
        let thread = std::thread::Builder::new()
            .name("openbar5000-playback".into())
            .spawn(move || run(command_rx, event_tx, engine_clock))
            .expect("failed to spawn playback thread");
        Self {
            command_tx,
            event_rx,
            clock,
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

    /// The playback position, to read whenever it's drawn -- there are no
    /// position events, so the playback thread never wakes just for them.
    pub fn clock(&self) -> &PlaybackClock {
        &self.clock
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
    playback_mode: PlaybackMode,
    /// `Some` only while `playback_mode == Shuffle`; rebuilt whenever the
    /// playlist's contents or length change so its indices never go stale.
    shuffle: Option<ShuffleOrder>,
    /// Playlist indices queued via `EnqueueIndex`, consumed front-first by
    /// the next advance; kept pointing at the same tracks across
    /// removals/reorders.
    queue: VecDeque<usize>,
    /// Set to cancel the waveform being computed for the current track.
    waveform_cancel: Option<Arc<AtomicBool>>,
    /// The output device asked for (`None`: the system default).
    output_request: Option<String>,
    /// The app's id for the playlist being held (see `SwitchPlaylist`).
    playlist_id: i64,
    /// The device actually playing.
    output_device: Option<String>,
}

fn run(command_rx: Receiver<EngineCommand>, event_tx: Sender<EngineEvent>, clock: PlaybackClock) {
    let (sink, output_device) = match CpalSink::default_device() {
        Ok(sink) => {
            let id = sink.device_id().map(str::to_string);
            (Box::new(sink) as Box<dyn OutputSink>, id)
        }
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
        playback_mode: PlaybackMode::Normal,
        shuffle: None,
        queue: VecDeque::new(),
        waveform_cancel: None,
        output_request: None,
        playlist_id: 1,
        output_device,
    };
    let output_checks = OutputChecks::new();

    loop {
        // While playing, sleep (still answering commands) until the output
        // has drained to half full or a spectrum frame is due, then top the
        // output up in one go -- a few wakeups a second rather than one
        // per chunk. Otherwise there's nothing to do until a command comes
        // or the output devices are to be checked.
        let timeout = match state.playback_state {
            PlaybackState::Playing => {
                let mut next = state.pipeline.time_until_refill();
                if let Some(spectrum_due) = state.pipeline.time_until_spectrum() {
                    next = next.min(spectrum_due);
                }
                Some(next.min(PLAYING_POLL_INTERVAL))
            }
            PlaybackState::Paused | PlaybackState::Stopped => None,
        };

        match wait(&command_rx, output_checks.due(), timeout) {
            Wake::Command(command) => {
                if !handle_command(&mut state, command, &event_tx) {
                    break;
                }
            }
            Wake::OutputCheck => {
                if let Some(target) = output_target(&state) {
                    if target != state.output_device {
                        switch_output(&mut state, &event_tx);
                    }
                }
            }
            Wake::Timeout => {}
            Wake::Closed => break,
        }

        if state.playback_state == PlaybackState::Playing {
            // The spectrum first: it's due at a set moment, and a refill
            // (a batch of decoding) would otherwise make it late.
            if let Some((bars, peaks)) = state.pipeline.audible_spectrum() {
                let _ = event_tx.send(EngineEvent::SpectrumUpdate { bars, peaks });
            }
            refill_output(&mut state, &command_rx, &event_tx);
        }
        publish_clock(&state, &clock);
    }
}

/// What woke the playback thread.
enum Wake {
    Command(EngineCommand),
    /// Time to check which output device to play on.
    OutputCheck,
    /// Only time passing: the output to top up, or a spectrum frame due.
    Timeout,
    /// The engine's handle is gone.
    Closed,
}

/// Sleeps until a command comes, an output check is due, or `timeout`
/// (if any) has passed.
fn wait(command_rx: &Receiver<EngineCommand>, output_checks: &Receiver<Instant>, timeout: Option<Duration>) -> Wake {
    use crossbeam_channel::select;
    match timeout {
        Some(timeout) => select! {
            recv(command_rx) -> command => command.map_or(Wake::Closed, Wake::Command),
            recv(output_checks) -> _ => Wake::OutputCheck,
            default(timeout) => Wake::Timeout,
        },
        None => select! {
            recv(command_rx) -> command => command.map_or(Wake::Closed, Wake::Command),
            recv(output_checks) -> _ => Wake::OutputCheck,
        },
    }
}

/// When to check which output device to play on: whenever the system
/// reports a change, where it does (Windows, see `DeviceWatch`), and every
/// `OUTPUT_CHECK_INTERVAL` elsewhere -- or if registering for the reports
/// fails.
struct OutputChecks {
    due: Receiver<Instant>,
    #[cfg(windows)]
    _watch: Option<crate::output::DeviceWatch>,
}

impl OutputChecks {
    fn new() -> Self {
        #[cfg(windows)]
        match crate::output::DeviceWatch::new() {
            Ok(watch) => {
                return Self {
                    due: watch.changes().clone(),
                    _watch: Some(watch),
                }
            }
            Err(e) => tracing::warn!("not told about output device changes, checking every {OUTPUT_CHECK_INTERVAL:?}: {e}"),
        }
        Self {
            due: crossbeam_channel::tick(OUTPUT_CHECK_INTERVAL),
            #[cfg(windows)]
            _watch: None,
        }
    }

    /// Gets a message whenever a check is due.
    fn due(&self) -> &Receiver<Instant> {
        &self.due
    }
}

/// Decodes until the output is full (or a command is waiting), going on
/// to the next track when one ends.
fn refill_output(state: &mut EngineState, command_rx: &Receiver<EngineCommand>, event_tx: &Sender<EngineEvent>) {
    for _ in 0..MAX_CHUNKS_PER_REFILL {
        if state.playback_state != PlaybackState::Playing
            || !state.pipeline.has_room_for_chunk()
            || !command_rx.is_empty()
        {
            return;
        }
        match state.pipeline.process_chunk() {
            Ok(0) => on_track_finished(state, event_tx),
            Ok(written) => state.samples_played += written as u64,
            Err(e) => {
                let _ = event_tx.send(EngineEvent::Error(e.to_string()));
                set_playback_state(state, PlaybackState::Stopped, event_tx);
                return;
            }
        }
    }
}

/// Tells the [`PlaybackClock`] what is audible right now.
fn publish_clock(state: &EngineState, clock: &PlaybackClock) {
    let channels = state.channels.max(1) as i64;
    let decoded = state.samples_played as i64;
    // What's been heard, not just decoded: the sink still holds the
    // difference. Right after a track change that's audio from the
    // previous track, so this is negative until the new one is audible.
    let audible = decoded - state.pipeline.queued_samples() as i64;
    let duration = state
        .playlist
        .current()
        .map(|t| t.metadata.duration)
        .unwrap_or_default();
    clock.publish(
        audible.div_euclid(channels),
        decoded / channels,
        state.sample_rate,
        state.playback_state == PlaybackState::Playing,
        duration,
    );
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
                // Respect shuffle's own (already-randomized) starting pick
                // instead of always starting a cold play at track 0.
                let start_index = match state.playback_mode {
                    PlaybackMode::Shuffle => state
                        .shuffle
                        .as_ref()
                        .and_then(ShuffleOrder::current)
                        .unwrap_or(0),
                    PlaybackMode::Normal | PlaybackMode::RepeatAll | PlaybackMode::RepeatOne => 0,
                };
                load_track(state, start_index, event_tx);
            }
            if state.playlist.current().is_some() {
                set_playback_state(state, PlaybackState::Playing, event_tx);
            }
        }
        EngineCommand::Pause => {
            if state.playback_state == PlaybackState::Playing {
                drop_queued_audio(state, true);
            }
            set_playback_state(state, PlaybackState::Paused, event_tx);
        }
        EngineCommand::Stop => {
            drop_queued_audio(state, false);
            set_playback_state(state, PlaybackState::Stopped, event_tx);
            state.samples_played = 0;
        }
        // Nothing loaded (e.g. clicking the empty waveform while stopped):
        // nothing to seek, and no error worth showing.
        EngineCommand::Seek(_) if state.playlist.current().is_none() => {}
        EngineCommand::Seek(pos) => {
            drop_queued_audio(state, false);
            if let Err(e) = state.pipeline.seek(pos) {
                let _ = event_tx.send(EngineEvent::Error(e.to_string()));
            } else {
                state.samples_played =
                    (pos.as_secs_f64() * state.sample_rate as f64 * state.channels as f64) as u64;
            }
        }
        EngineCommand::Next => {
            drop_queued_audio(state, false);
            advance_to_next_track(state, event_tx);
        }
        EngineCommand::Previous => {
            drop_queued_audio(state, false);
            // Shuffle has its own notion of "previous" (back one step in
            // its randomized order); RepeatAll wraps to the last track;
            // Normal/RepeatOne just step the index down and stop at 0.
            // Either way, an explicit Previous always navigates for real --
            // unlike a track ending naturally, RepeatOne doesn't apply here.
            let index = match state.playback_mode {
                PlaybackMode::Shuffle => state.shuffle.as_mut().and_then(ShuffleOrder::retreat),
                PlaybackMode::RepeatAll => {
                    let len = state.playlist.len();
                    (len > 0).then(|| match state.playlist.current_index() {
                        Some(0) | None => len - 1,
                        Some(i) => i - 1,
                    })
                }
                PlaybackMode::Normal | PlaybackMode::RepeatOne => Some(
                    state
                        .playlist
                        .current_index()
                        .and_then(|i| i.checked_sub(1))
                        .unwrap_or(0),
                ),
            };
            if let Some(index) = index {
                let was_playing = state.playback_state == PlaybackState::Playing;
                load_track(state, index, event_tx);
                if was_playing {
                    set_playback_state(state, PlaybackState::Playing, event_tx);
                }
            }
        }
        EngineCommand::SetVolume(volume) => {
            state.volume = volume.clamp(0.0, 1.0);
            state.pipeline.set_volume(state.volume);
        }
        EngineCommand::LoadPlaylist(paths) => load_playlist(state, paths, event_tx),
        EngineCommand::PlayLibrary { paths, start } => play_library(state, paths, start, event_tx),
        EngineCommand::AppendToPlaylist(paths) => append_to_playlist(state, paths, event_tx),
        EngineCommand::LoadFolder(dir) => {
            let paths = scan_audio_files(&dir);
            if paths.is_empty() {
                let _ = event_tx.send(EngineEvent::Error(format!(
                    "No audio files found in {}",
                    dir.display()
                )));
            } else {
                load_playlist(state, paths, event_tx);
            }
        }
        EngineCommand::PlayIndex(index) => play_index(state, index, event_tx),
        EngineCommand::RemoveIndex(index) => {
            if remove_index(state, index) {
                set_playback_state(state, PlaybackState::Stopped, event_tx);
                state.samples_played = 0;
            }
            rebuild_shuffle_order(state, state.playlist.current_index());
            send_playlist_update(state, event_tx);
        }
        EngineCommand::RemoveIndices(mut indices) => {
            // Highest first, so the remaining indices stay valid.
            indices.sort_unstable_by(|a, b| b.cmp(a));
            indices.dedup();
            let mut removed_current = false;
            for index in indices {
                removed_current |= remove_index(state, index);
            }
            if removed_current {
                set_playback_state(state, PlaybackState::Stopped, event_tx);
                state.samples_played = 0;
            }
            rebuild_shuffle_order(state, state.playlist.current_index());
            send_playlist_update(state, event_tx);
        }
        EngineCommand::SwitchPlaylist { id, paths } => switch_playlist(state, id, paths, event_tx),
        EngineCommand::ClearPlaylist => {
            state.playlist.clear();
            set_playback_state(state, PlaybackState::Stopped, event_tx);
            state.samples_played = 0;
            state.shuffle = None;
            state.queue.clear();
            send_playlist_update(state, event_tx);
        }
        EngineCommand::ReorderPlaylist(order) => {
            if state.playlist.reorder(&order) {
                for queued in state.queue.iter_mut() {
                    *queued = order
                        .iter()
                        .position(|&old| old == *queued)
                        .expect("reorder already validated the permutation");
                }
                rebuild_shuffle_order(state, state.playlist.current_index());
                send_playlist_update(state, event_tx);
            }
        }
        EngineCommand::RefreshMetadata => {
            let paths: Vec<_> = state
                .playlist
                .tracks()
                .iter()
                .map(|t| t.path.clone())
                .collect();
            let metadata = load_metadata_with_progress(&paths, event_tx);
            state.playlist.replace_metadata(metadata);
            send_playlist_update(state, event_tx);
        }
        EngineCommand::EnqueueIndex(index) => {
            if index < state.playlist.len() {
                state.queue.push_back(index);
            }
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
        EngineCommand::SetOutputDevice(id) => {
            state.output_request = id;
            switch_output(state, event_tx);
        }
        EngineCommand::SetEqualizer(settings) => {
            state.pipeline.set_equalizer(settings);
        }
        EngineCommand::SetPlaybackMode(mode) => {
            state.playback_mode = mode;
            if mode == PlaybackMode::Shuffle {
                rebuild_shuffle_order(state, state.playlist.current_index());
            } else {
                state.shuffle = None;
            }
        }
    }
    true
}

fn load_playlist(
    state: &mut EngineState,
    paths: Vec<std::path::PathBuf>,
    event_tx: &Sender<EngineEvent>,
) {
    replace_tracks(state, paths, event_tx);
    send_playlist_update(state, event_tx);
}

/// Loads `paths` as the media library's throwaway playlist (issue #37) and
/// plays from `start`: reported under [`crate::TEMP_PLAYLIST_ID`], so the
/// app shows it without saving it over a named playlist.
fn play_library(
    state: &mut EngineState,
    paths: Vec<std::path::PathBuf>,
    start: usize,
    event_tx: &Sender<EngineEvent>,
) {
    replace_tracks(state, paths, event_tx);
    state.playlist_id = crate::TEMP_PLAYLIST_ID;
    send_playlist_update(state, event_tx);
    play_index(state, start, event_tx);
}

/// The playlist becomes `paths` (their tags read), with nothing queued.
fn replace_tracks(state: &mut EngineState, paths: Vec<std::path::PathBuf>, event_tx: &Sender<EngineEvent>) {
    let metadata = load_metadata_with_progress(&paths, event_tx);
    let tracks: Vec<Track> = paths
        .into_iter()
        .zip(metadata)
        .map(|(path, metadata)| Track { path, metadata })
        .collect();
    state.playlist = Playlist::from_tracks(tracks);
    state.samples_played = 0;
    state.queue.clear();
    rebuild_shuffle_order(state, None);
}

fn play_index(state: &mut EngineState, index: usize, event_tx: &Sender<EngineEvent>) {
    drop_queued_audio(state, false);
    load_track(state, index, event_tx);
    set_playback_state(state, PlaybackState::Playing, event_tx);
    // Re-anchor the shuffle cycle on the track the user just picked by
    // hand, rather than leaving it mid-way through the old one.
    rebuild_shuffle_order(state, Some(index));
}

/// Takes `index` out of the playlist and the queue; `true` if it was the
/// track playing.
fn remove_index(state: &mut EngineState, index: usize) -> bool {
    let removed_current = state.playlist.current_index() == Some(index);
    state.playlist.remove(index);
    state.queue.retain(|&i| i != index);
    for queued in state.queue.iter_mut() {
        if *queued > index {
            *queued -= 1;
        }
    }
    removed_current
}

fn switch_playlist(
    state: &mut EngineState,
    id: i64,
    paths: Vec<std::path::PathBuf>,
    event_tx: &Sender<EngineEvent>,
) {
    let playing = state
        .playlist
        .current()
        .filter(|_| state.playback_state != PlaybackState::Stopped)
        .map(|track| track.path.clone());
    let metadata = load_metadata_with_progress(&paths, event_tx);
    let tracks: Vec<Track> = paths
        .into_iter()
        .zip(metadata)
        .map(|(path, metadata)| Track { path, metadata })
        .collect();
    // The decoder keeps playing what it has; if that song is in the new
    // playlist too, it's the current one there.
    let current = playing.and_then(|path| tracks.iter().position(|t| t.path == path));
    state.playlist = Playlist::from_tracks(tracks);
    if let Some(index) = current {
        state.playlist.set_current_index(index);
    }
    state.playlist_id = id;
    state.queue.clear();
    rebuild_shuffle_order(state, current);
    send_playlist_update(state, event_tx);
}

fn append_to_playlist(
    state: &mut EngineState,
    paths: Vec<std::path::PathBuf>,
    event_tx: &Sender<EngineEvent>,
) {
    if paths.is_empty() {
        return;
    }
    let metadata = load_metadata_with_progress(&paths, event_tx);
    let current = state.playlist.current_index();
    let mut tracks = state.playlist.tracks().to_vec();
    tracks.extend(
        paths
            .into_iter()
            .zip(metadata)
            .map(|(path, metadata)| Track { path, metadata }),
    );
    state.playlist = Playlist::from_tracks(tracks);
    if let Some(index) = current {
        state.playlist.set_current_index(index);
    }
    rebuild_shuffle_order(state, current);
    send_playlist_update(state, event_tx);
}

/// `load_metadata_parallel`, reporting `LoadProgress` at most every
/// `LOAD_PROGRESS_INTERVAL` (plus once at the end) so a big load doesn't
/// flood the UI's event queue.
fn load_metadata_with_progress(
    paths: &[std::path::PathBuf],
    event_tx: &Sender<EngineEvent>,
) -> Vec<crate::TrackMetadata> {
    let mut last_sent = Instant::now();
    load_metadata_parallel(paths, |done, total| {
        if done == total || last_sent.elapsed() >= LOAD_PROGRESS_INTERVAL {
            let _ = event_tx.send(EngineEvent::LoadProgress { done, total });
            last_sent = Instant::now();
        }
    })
}

/// Rebuilds the shuffle cycle for the playlist's current length -- a no-op
/// unless shuffle is the active mode. `anchor`, if given, is kept as the
/// cycle's first entry so rebuilding doesn't interrupt what's playing.
fn rebuild_shuffle_order(state: &mut EngineState, anchor: Option<usize>) {
    if state.playback_mode == PlaybackMode::Shuffle {
        state.shuffle = Some(ShuffleOrder::new(state.playlist.len(), anchor));
    }
}

fn send_playlist_update(state: &EngineState, event_tx: &Sender<EngineEvent>) {
    let _ = event_tx.send(EngineEvent::PlaylistUpdated {
        tracks: state.playlist.tracks().to_vec(),
        current_index: state.playlist.current_index(),
        playlist_id: state.playlist_id,
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
                spawn_waveform_computation(state, track.path.clone(), index, event_tx.clone());
            }
            let _ = event_tx.send(EngineEvent::TrackChanged {
                index,
                track: Box::new(track),
            });
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
///
/// Skipping quickly through tracks would otherwise pile up one full-file
/// decode per track (each on several cores, see `compute_peaks`), so the
/// previous track's computation is cancelled, and a new one only starts
/// once its track has stayed current for `WAVEFORM_START_DELAY`.
fn spawn_waveform_computation(
    state: &mut EngineState,
    path: std::path::PathBuf,
    index: usize,
    event_tx: Sender<EngineEvent>,
) {
    if let Some(previous) = state.waveform_cancel.take() {
        previous.store(true, Ordering::Relaxed);
    }
    let cancel = Arc::new(AtomicBool::new(false));
    state.waveform_cancel = Some(cancel.clone());
    std::thread::Builder::new()
        .name("openbar5000-waveform".into())
        .spawn(move || {
            crate::background::lower_thread_priority();
            std::thread::sleep(WAVEFORM_START_DELAY);
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            if let Ok(Some(channels)) =
                crate::waveform::compute_peaks_cancellable(&path, WAVEFORM_BUCKET_COUNT, &cancel)
            {
                let _ = event_tx.send(EngineEvent::WaveformReady { index, channels });
            }
        })
        .expect("failed to spawn waveform thread");
}

/// Called when a track reaches its end on its own (not via an explicit
/// Next/Previous command) -- the one place RepeatOne applies, replaying the
/// same track instead of moving on.
fn on_track_finished(state: &mut EngineState, event_tx: &Sender<EngineEvent>) {
    // A queued track always wins, even over RepeatOne -- the user asked for
    // it explicitly.
    if state.playback_mode == PlaybackMode::RepeatOne && state.queue.is_empty() {
        if let Some(index) = state.playlist.current_index() {
            load_track(state, index, event_tx);
            set_playback_state(state, PlaybackState::Playing, event_tx);
            return;
        }
    }
    advance_to_next_track(state, event_tx);
}

fn advance_to_next_track(state: &mut EngineState, event_tx: &Sender<EngineEvent>) {
    let next_index = match state.queue.pop_front() {
        Some(queued) => Some(queued),
        None => mode_next_index(state),
    };
    match next_index {
        Some(next_index) => {
            let was_playing = state.playback_state == PlaybackState::Playing;
            load_track(state, next_index, event_tx);
            if was_playing {
                set_playback_state(state, PlaybackState::Playing, event_tx);
            }
        }
        None => set_playback_state(state, PlaybackState::Stopped, event_tx),
    }
}

/// Where the active playback mode goes after the current track, ignoring
/// the queue.
fn mode_next_index(state: &mut EngineState) -> Option<usize> {
    match state.playback_mode {
        PlaybackMode::Shuffle => state.shuffle.as_mut().and_then(ShuffleOrder::advance),
        PlaybackMode::Normal | PlaybackMode::RepeatOne => {
            let next = state.playlist.current_index().map(|i| i + 1).unwrap_or(0);
            (next < state.playlist.len()).then_some(next)
        }
        PlaybackMode::RepeatAll => {
            let len = state.playlist.len();
            let next = state.playlist.current_index().map(|i| i + 1).unwrap_or(0);
            (len > 0).then_some(next % len)
        }
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

/// Drops the audio queued in the output but not heard yet, so a pause,
/// seek or track change takes effect at once instead of after it. With
/// `resume_here` the decoder steps back to what was last heard, so playing
/// on continues from exactly there.
fn drop_queued_audio(state: &mut EngineState, resume_here: bool) {
    let queued = state.pipeline.queued_samples() as u64;
    if queued == 0 {
        return;
    }
    let channels = state.channels.max(1) as u64;
    let audible = state.samples_played.saturating_sub(queued) / channels * channels;
    state.pipeline.flush();
    if resume_here && state.playlist.current().is_some() {
        let frames = audible / channels;
        let position = Duration::from_secs_f64(frames as f64 / state.sample_rate.max(1) as f64);
        if state.pipeline.seek(position).is_ok() {
            state.samples_played = audible;
        }
    } else {
        state.samples_played = audible;
    }
}

/// The device playback should be on right now: the chosen one if it's
/// connected, otherwise the system default. `None` when that can't be
/// told (no devices at all) -- leave things as they are.
fn output_target(state: &EngineState) -> Option<Option<String>> {
    if let Some(requested) = &state.output_request {
        // Looked up even while playing on it, to notice it being unplugged.
        if crate::output::output_devices().iter().any(|d| &d.id == requested) {
            return Some(Some(requested.clone()));
        }
    }
    crate::output::default_output_device().map(|device| Some(device.id))
}

/// Moves playback to the device `output_request` calls for, carrying on
/// from what was audible: the old device's queued audio is dropped, so the
/// decoder steps back to the heard position.
fn switch_output(state: &mut EngineState, event_tx: &Sender<EngineEvent>) {
    let sink = match CpalSink::open(state.output_request.as_deref()) {
        Ok(sink) => sink,
        Err(e) => {
            let _ = event_tx.send(EngineEvent::Error(e.to_string()));
            return;
        }
    };
    let channels = state.channels.max(1) as u64;
    let audible = state
        .samples_played
        .saturating_sub(state.pipeline.queued_samples() as u64)
        / channels
        * channels;
    let id = sink.device_id().map(str::to_string);
    let name = sink.device_name().to_string();
    if let Err(e) = state.pipeline.replace_sink(Box::new(sink)) {
        let _ = event_tx.send(EngineEvent::Error(e.to_string()));
        return;
    }
    tracing::info!("audio output: {name}");
    let fallback = state.output_request.is_some() && state.output_request != id;
    state.output_device = id;
    if state.playlist.current().is_some() && audible > 0 {
        let frames = audible / channels;
        let position = Duration::from_secs_f64(frames as f64 / state.sample_rate.max(1) as f64);
        if state.pipeline.seek(position).is_ok() {
            state.samples_played = audible;
        }
    }
    let _ = event_tx.send(EngineEvent::OutputDeviceChanged { name, fallback });
}

