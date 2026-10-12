use engine::{AudioInfo, EngineCommand, EngineEvent, PlaybackEngine, PlaybackState, Track};
use slint::{ComponentHandle, ModelRc, Timer, TimerMode, VecModel};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;
use ui::{format_duration, playlist_row, MainWindow, PlaylistRow, PropertiesWindow};

const POLL_INTERVAL: Duration = Duration::from_millis(150);

/// Resources the caller must keep alive for as long as the window's event
/// loop runs: the timer that drains [`EngineEvent`]s, and the (initially
/// hidden) properties dialog it may show on request. Neither field is read
/// again after `wire()` returns -- they only need to not be dropped.
#[allow(dead_code)]
pub struct UiHandles {
    pub event_poll_timer: Timer,
    pub properties_window: PropertiesWindow,
}

/// Wires every UI callback to an [`EngineCommand`] and starts the timer that
/// drains [`EngineEvent`]s back into window properties. The UI thread never
/// touches the engine's internals directly -- only this channel boundary.
pub fn wire(window: &MainWindow, engine: Rc<PlaybackEngine>) -> UiHandles {
    let playlist_model = Rc::new(VecModel::<PlaylistRow>::default());
    window.set_playlist_rows(ModelRc::from(playlist_model.clone()));

    // Tracks the currently loaded track so "Properties" can look up its path
    // and tags on demand, independent of playback state.
    let current_track: Rc<RefCell<Option<Track>>> = Rc::new(RefCell::new(None));

    let properties_window = PropertiesWindow::new().expect("failed to create properties window");
    {
        let properties_window_weak = properties_window.as_weak();
        properties_window.on_close_clicked(move || {
            if let Some(window) = properties_window_weak.upgrade() {
                window.hide().expect("failed to hide properties window");
            }
        });
    }

    {
        let window_weak = window.as_weak();
        let engine = engine.clone();
        window.on_open_files_clicked(move || {
            let Some(paths) = rfd::FileDialog::new()
                .add_filter("Audio", engine::AUDIO_EXTENSIONS)
                .pick_files()
            else {
                return;
            };
            engine.send(EngineCommand::LoadPlaylist(paths));
            if let Some(window) = window_weak.upgrade() {
                window.set_status_text("Loading playlist\u{2026}".into());
            }
        });
    }

    {
        let window_weak = window.as_weak();
        let engine = engine.clone();
        window.on_open_folder_clicked(move || {
            let Some(dir) = rfd::FileDialog::new().pick_folder() else {
                return;
            };
            let paths = engine::scan_audio_files(&dir);
            if paths.is_empty() {
                if let Some(window) = window_weak.upgrade() {
                    window.set_status_text(
                        format!("No audio files found in {}", dir.display()).into(),
                    );
                }
                return;
            }
            engine.send(EngineCommand::LoadPlaylist(paths));
            if let Some(window) = window_weak.upgrade() {
                window.set_status_text("Loading playlist\u{2026}".into());
            }
        });
    }

    {
        let current_track = current_track.clone();
        let properties_window_weak = properties_window.as_weak();
        window.on_properties_clicked(move || {
            let Some(track) = current_track.borrow().clone() else {
                return;
            };
            let Some(properties_window) = properties_window_weak.upgrade() else {
                return;
            };
            show_properties(&properties_window, &track);
        });
    }

    {
        let window_weak = window.as_weak();
        let engine = engine.clone();
        window.on_play_pause_clicked(move || {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            if window.get_is_playing() {
                engine.send(EngineCommand::Pause);
            } else {
                engine.send(EngineCommand::Play);
            }
        });
    }

    {
        let engine = engine.clone();
        window.on_stop_clicked(move || engine.send(EngineCommand::Stop));
    }
    {
        let engine = engine.clone();
        window.on_next_clicked(move || engine.send(EngineCommand::Next));
    }
    {
        let engine = engine.clone();
        window.on_previous_clicked(move || engine.send(EngineCommand::Previous));
    }
    {
        let engine = engine.clone();
        window.on_seek_requested(move |position_seconds| {
            engine.send(EngineCommand::Seek(Duration::from_secs_f32(
                position_seconds.max(0.0),
            )));
        });
    }
    {
        let engine = engine.clone();
        window.on_volume_changed(move |volume| engine.send(EngineCommand::SetVolume(volume)));
    }
    {
        let engine = engine.clone();
        window.on_row_clicked(move |index| engine.send(EngineCommand::PlayIndex(index as usize)));
    }

    let event_poll_timer = Timer::default();
    let window_weak = window.as_weak();
    event_poll_timer.start(TimerMode::Repeated, POLL_INTERVAL, move || {
        let Some(window) = window_weak.upgrade() else {
            return;
        };
        while let Some(event) = engine.try_recv_event() {
            apply_event(&window, &playlist_model, &current_track, event);
        }
    });

    UiHandles {
        event_poll_timer,
        properties_window,
    }
}

fn apply_event(
    window: &MainWindow,
    playlist_model: &Rc<VecModel<PlaylistRow>>,
    current_track: &Rc<RefCell<Option<Track>>>,
    event: EngineEvent,
) {
    match event {
        EngineEvent::TrackChanged { index, track } => {
            window.set_current_track_index(index as i32);
            window.set_duration_seconds(track.metadata.duration.as_secs_f32());
            window.set_duration_text(format_duration(track.metadata.duration).into());
            window.set_status_text(
                format!(
                    "{} \u{2014} {}",
                    track.metadata.artist, track.metadata.title
                )
                .into(),
            );
            *current_track.borrow_mut() = Some(track);
        }
        EngineEvent::PositionUpdate { position, duration } => {
            window.set_position_seconds(position.as_secs_f32());
            window.set_position_text(format_duration(position).into());
            if duration.as_secs_f32() > 0.0 {
                window.set_duration_seconds(duration.as_secs_f32());
                window.set_duration_text(format_duration(duration).into());
            }
        }
        EngineEvent::PlaybackStateChanged(state) => {
            window.set_is_playing(state == PlaybackState::Playing);
        }
        EngineEvent::Error(message) => {
            window.set_status_text(format!("Error: {message}").into());
        }
        EngineEvent::Underrun => {
            tracing::warn!("audio output underrun");
        }
        EngineEvent::PlaylistLoaded { tracks } => {
            let rows: Vec<PlaylistRow> = tracks
                .iter()
                .enumerate()
                .map(|(i, track)| {
                    let track_number = track
                        .metadata
                        .track_number
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| (i + 1).to_string());
                    playlist_row(
                        &track_number,
                        &track.metadata.artist,
                        &track.metadata.title,
                        &track.metadata.album,
                        track.metadata.duration,
                    )
                })
                .collect();
            let track_count = tracks.len();
            playlist_model.set_vec(rows);
            window.set_current_track_index(-1);
            *current_track.borrow_mut() = None;
            window.set_status_text(format!("{track_count} track(s) loaded").into());
        }
    }
}

fn show_properties(properties_window: &PropertiesWindow, track: &Track) {
    properties_window.set_file_path(track.path.display().to_string().into());
    properties_window.set_prop_title(track.metadata.title.clone().into());
    properties_window.set_prop_artist(track.metadata.artist.clone().into());
    properties_window.set_prop_album(track.metadata.album.clone().into());
    properties_window.set_prop_track_number(
        track
            .metadata
            .track_number
            .map(|n| n.to_string())
            .unwrap_or_default()
            .into(),
    );
    properties_window.set_prop_duration(format_duration(track.metadata.duration).into());

    match engine::probe_audio_info(&track.path) {
        Ok(AudioInfo {
            sample_rate,
            channels,
            bits_per_sample,
            ..
        }) => {
            properties_window.set_prop_sample_rate(format!("{sample_rate} Hz").into());
            properties_window.set_prop_channels(channels.to_string().into());
            properties_window.set_prop_bits_per_sample(bits_per_sample.to_string().into());
        }
        Err(e) => {
            properties_window.set_prop_sample_rate("unknown".into());
            properties_window.set_prop_channels("unknown".into());
            properties_window.set_prop_bits_per_sample("unknown".into());
            tracing::warn!("failed to probe audio info for properties view: {e}");
        }
    }

    properties_window
        .show()
        .expect("failed to show properties window");
}
