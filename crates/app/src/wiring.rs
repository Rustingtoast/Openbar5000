use crate::platform;
use crate::playlist_rows::build_grouped_rows;
use crate::settings::Settings;
use engine::{AudioInfo, EngineCommand, EngineEvent, PlaybackEngine, PlaybackState, Track};
use slint::{ComponentHandle, Model, ModelRc, Timer, TimerMode, VecModel};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;
use ui::{format_duration, AboutWindow, MainWindow, PlaylistRow, PropertiesWindow, SettingsWindow};

const POLL_INTERVAL: Duration = Duration::from_millis(150);

/// Resources the caller must keep alive for as long as the window's event
/// loop runs: the timer that drains [`EngineEvent`]s, and the (initially
/// hidden) secondary dialogs it may show on request. None of these fields
/// are read again after `wire()` returns -- they only need to not be dropped.
#[allow(dead_code)]
pub struct UiHandles {
    pub event_poll_timer: Timer,
    pub properties_window: PropertiesWindow,
    pub about_window: AboutWindow,
    pub settings_window: SettingsWindow,
}

/// Shared UI-thread state kept in sync with the engine's playlist, so any
/// row-targeted action (play/properties/remove/open-folder/search) or
/// re-render (after a search edit) can work from the full, current track
/// list without round-tripping through the engine.
struct PlaylistViewState {
    tracks: Vec<Track>,
    current_index: Option<usize>,
    search_query: String,
}

/// Wires every UI callback to an [`EngineCommand`] and starts the timer that
/// drains [`EngineEvent`]s back into window properties. The UI thread never
/// touches the engine's internals directly -- only this channel boundary.
pub fn wire(window: &MainWindow, engine: Rc<PlaybackEngine>) -> UiHandles {
    let playlist_model = Rc::new(VecModel::<PlaylistRow>::default());
    window.set_playlist_rows(ModelRc::from(playlist_model.clone()));

    let playlist_view_state = Rc::new(RefCell::new(PlaylistViewState {
        tracks: Vec::new(),
        current_index: None,
        search_query: String::new(),
    }));
    // The currently loaded track, if any -- used by the top-menu
    // "Properties" item, which (unlike the playlist's right-click one) isn't
    // tied to a specific row.
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

    let about_window = AboutWindow::new().expect("failed to create about window");
    {
        let about_window_weak = about_window.as_weak();
        about_window.on_close_clicked(move || {
            if let Some(window) = about_window_weak.upgrade() {
                window.hide().expect("failed to hide about window");
            }
        });
    }
    {
        let about_window_weak = about_window.as_weak();
        window.on_about_clicked(move || {
            if let Some(window) = about_window_weak.upgrade() {
                window.show().expect("failed to show about window");
            }
        });
    }

    let settings = Rc::new(RefCell::new(Settings::load()));
    window.set_show_waveform(settings.borrow().show_waveform);
    window.set_show_spectrum(settings.borrow().show_spectrum);
    engine.send(EngineCommand::SetWaveformEnabled(
        settings.borrow().show_waveform,
    ));
    engine.send(EngineCommand::SetSpectrumEnabled(
        settings.borrow().show_spectrum,
    ));

    let settings_window = SettingsWindow::new().expect("failed to create settings window");
    settings_window.set_show_waveform(settings.borrow().show_waveform);
    settings_window.set_show_spectrum(settings.borrow().show_spectrum);
    {
        let settings_window_weak = settings_window.as_weak();
        settings_window.on_close_clicked(move || {
            if let Some(window) = settings_window_weak.upgrade() {
                window.hide().expect("failed to hide settings window");
            }
        });
    }
    {
        let settings_window_weak = settings_window.as_weak();
        window.on_settings_clicked(move || {
            if let Some(window) = settings_window_weak.upgrade() {
                window.show().expect("failed to show settings window");
            }
        });
    }
    {
        let settings = settings.clone();
        let window_weak = window.as_weak();
        let engine = engine.clone();
        settings_window.on_waveform_toggled(move |enabled| {
            settings.borrow_mut().show_waveform = enabled;
            settings.borrow().save();
            if let Some(window) = window_weak.upgrade() {
                window.set_show_waveform(enabled);
            }
            engine.send(EngineCommand::SetWaveformEnabled(enabled));
        });
    }
    {
        let settings = settings.clone();
        let window_weak = window.as_weak();
        let engine = engine.clone();
        settings_window.on_spectrum_toggled(move |enabled| {
            settings.borrow_mut().show_spectrum = enabled;
            settings.borrow().save();
            if let Some(window) = window_weak.upgrade() {
                window.set_show_spectrum(enabled);
            }
            engine.send(EngineCommand::SetSpectrumEnabled(enabled));
        });
    }

    window.on_exit_clicked(|| {
        slint::quit_event_loop().expect("failed to quit event loop");
    });

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
        let engine = engine.clone();
        window.on_clear_playlist_clicked(move || engine.send(EngineCommand::ClearPlaylist));
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
        let playlist_view_state = playlist_view_state.clone();
        let properties_window_weak = properties_window.as_weak();
        window.on_row_properties_clicked(move |index| {
            let Some(track) = playlist_view_state
                .borrow()
                .tracks
                .get(index as usize)
                .cloned()
            else {
                return;
            };
            let Some(properties_window) = properties_window_weak.upgrade() else {
                return;
            };
            show_properties(&properties_window, &track);
        });
    }

    {
        let playlist_view_state = playlist_view_state.clone();
        window.on_row_open_folder_clicked(move |index| {
            if let Some(track) = playlist_view_state.borrow().tracks.get(index as usize) {
                platform::reveal_in_file_manager(&track.path);
            }
        });
    }

    {
        let engine = engine.clone();
        window.on_row_remove_clicked(move |index| {
            engine.send(EngineCommand::RemoveIndex(index as usize));
        });
    }

    {
        let playlist_model = playlist_model.clone();
        let playlist_view_state = playlist_view_state.clone();
        window.on_search_edited(move |query| {
            playlist_view_state.borrow_mut().search_query = query.to_string();
            refresh_playlist_rows(&playlist_model, &playlist_view_state);
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
        window.on_menu_play_clicked(move || engine.send(EngineCommand::Play));
    }
    {
        let engine = engine.clone();
        window.on_menu_pause_clicked(move || engine.send(EngineCommand::Pause));
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
            apply_event(
                &window,
                &playlist_model,
                &playlist_view_state,
                &current_track,
                event,
            );
        }
    });

    UiHandles {
        event_poll_timer,
        properties_window,
        about_window,
        settings_window,
    }
}

fn refresh_playlist_rows(
    playlist_model: &Rc<VecModel<PlaylistRow>>,
    playlist_view_state: &Rc<RefCell<PlaylistViewState>>,
) {
    let state = playlist_view_state.borrow();
    let rows = build_grouped_rows(&state.tracks, state.current_index, &state.search_query);
    playlist_model.set_vec(rows);
}

fn apply_event(
    window: &MainWindow,
    playlist_model: &Rc<VecModel<PlaylistRow>>,
    playlist_view_state: &Rc<RefCell<PlaylistViewState>>,
    current_track: &Rc<RefCell<Option<Track>>>,
    event: EngineEvent,
) {
    match event {
        EngineEvent::TrackChanged { index, track } => {
            window.set_duration_seconds(track.metadata.duration.as_secs_f32());
            window.set_duration_text(format_duration(track.metadata.duration).into());
            window.set_status_text(
                format!(
                    "{} \u{2014} {}",
                    track.metadata.artist, track.metadata.title
                )
                .into(),
            );
            window.set_waveform_peaks(ModelRc::from(Rc::new(VecModel::<f32>::default())));
            window.set_waveform_played_bucket_index(0);
            *current_track.borrow_mut() = Some(track);
            playlist_view_state.borrow_mut().current_index = Some(index);
            refresh_playlist_rows(playlist_model, playlist_view_state);
        }
        EngineEvent::PositionUpdate { position, duration } => {
            window.set_position_seconds(position.as_secs_f32());
            window.set_position_text(format_duration(position).into());
            if duration.as_secs_f32() > 0.0 {
                window.set_duration_seconds(duration.as_secs_f32());
                window.set_duration_text(format_duration(duration).into());
            }
            let peak_count = window.get_waveform_peaks().row_count() as i32;
            if peak_count > 0 && duration.as_secs_f32() > 0.0 {
                let fraction = position.as_secs_f32() / duration.as_secs_f32();
                let played = ((fraction * peak_count as f32) as i32).clamp(0, peak_count - 1);
                window.set_waveform_played_bucket_index(played);
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
        EngineEvent::PlaylistUpdated {
            tracks,
            current_index,
        } => {
            let track_count = tracks.len();
            *current_track.borrow_mut() = current_index.and_then(|i| tracks.get(i).cloned());
            {
                let mut state = playlist_view_state.borrow_mut();
                state.tracks = tracks;
                state.current_index = current_index;
            }
            refresh_playlist_rows(playlist_model, playlist_view_state);
            window.set_status_text(format!("{track_count} track(s) loaded").into());
        }
        EngineEvent::WaveformReady { index, peaks } => {
            if playlist_view_state.borrow().current_index == Some(index) {
                window.set_waveform_peaks(ModelRc::from(Rc::new(VecModel::from(peaks))));
                window.set_waveform_played_bucket_index(0);
            }
        }
        EngineEvent::SpectrumUpdate { bars } => {
            window.set_spectrum_bars(ModelRc::from(Rc::new(VecModel::from(bars))));
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
