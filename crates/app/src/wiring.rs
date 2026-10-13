use crate::platform;
use crate::playlist_rows::build_grouped_rows;
use crate::settings::Settings;
use crate::store::Store;
use crate::theme::{self, ThemeColors, ThemeRegistry};
use engine::{AudioInfo, EngineCommand, EngineEvent, PlaybackEngine, PlaybackState, Track};
use slint::{ComponentHandle, Model, ModelRc, Timer, TimerMode, VecModel};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;
use ui::{
    format_duration, AboutWindow, MainWindow, PlaylistRow, PropertiesWindow, SettingsWindow,
    ThemeEditorWindow, ThemeListEntry,
};

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
    pub theme_editor_window: ThemeEditorWindow,
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
pub fn wire(window: &MainWindow, engine: Rc<PlaybackEngine>, store: Rc<Store>) -> UiHandles {
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

    let restored_paths = store.load_playlist();
    if !restored_paths.is_empty() {
        engine.send(EngineCommand::LoadPlaylist(restored_paths));
        window.set_status_text("Restoring playlist\u{2026}".into());
    }

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

    let settings_window = SettingsWindow::new().expect("failed to create settings window");
    let theme_editor_window =
        ThemeEditorWindow::new().expect("failed to create theme editor window");

    // --- Theme setup -----------------------------------------------------
    let settings = Rc::new(RefCell::new(Settings::load(&store)));
    let theme_registry = Rc::new(RefCell::new(ThemeRegistry::load(&store)));
    // The theme being built in the (initially hidden) editor window, reset
    // each time "New Theme..." is clicked.
    let draft_colors: Rc<RefCell<ThemeColors>> = Rc::new(RefCell::new(
        theme_registry
            .borrow()
            .find("dark")
            .expect("dark builtin theme must exist")
            .colors,
    ));
    let draft_name: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));

    let initial_theme_id = settings.borrow().theme_id.clone();
    let initial_colors = {
        let registry = theme_registry.borrow();
        registry
            .find(&initial_theme_id)
            .or_else(|| registry.find("dark"))
            .map(|entry| entry.colors)
            .expect("dark builtin theme must exist")
    };
    theme::apply_theme(
        window,
        &properties_window,
        &about_window,
        &settings_window,
        &theme_editor_window,
        &initial_colors,
    );
    refresh_theme_list(
        &settings_window,
        &theme_registry.borrow(),
        &initial_theme_id,
    );

    window.set_show_waveform(settings.borrow().show_waveform);
    window.set_show_spectrum(settings.borrow().show_spectrum);
    engine.send(EngineCommand::SetWaveformEnabled(
        settings.borrow().show_waveform,
    ));
    engine.send(EngineCommand::SetSpectrumEnabled(
        settings.borrow().show_spectrum,
    ));
    settings_window.set_show_waveform(settings.borrow().show_waveform);
    settings_window.set_show_spectrum(settings.borrow().show_spectrum);

    engine.send(EngineCommand::SetSpectrumConfig(settings.borrow().spectrum));
    {
        let spectrum = settings.borrow().spectrum;
        settings_window.set_spectrum_channels(spectrum_channel_label(spectrum.channel_mode).into());
        settings_window.set_spectrum_min_db(spectrum.min_db as i32);
        settings_window.set_spectrum_max_db(spectrum.max_db as i32);
        settings_window.set_spectrum_fft_size(spectrum.fft_size.to_string().into());
        settings_window.set_spectrum_peaks_enabled(spectrum.peaks_enabled);
        settings_window.set_spectrum_peak_hold_ms(spectrum.peak_hold_ms as i32);
        settings_window.set_spectrum_gravity(spectrum.gravity_db_per_s2 as i32);
        settings_window.set_spectrum_interval_ms(spectrum.interval_ms as i32);
    }

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
        let store = store.clone();
        let window_weak = window.as_weak();
        let engine = engine.clone();
        settings_window.on_waveform_toggled(move |enabled| {
            settings.borrow_mut().show_waveform = enabled;
            settings.borrow().save(&store);
            if let Some(window) = window_weak.upgrade() {
                window.set_show_waveform(enabled);
            }
            engine.send(EngineCommand::SetWaveformEnabled(enabled));
        });
    }
    {
        let settings = settings.clone();
        let store = store.clone();
        let window_weak = window.as_weak();
        let engine = engine.clone();
        settings_window.on_spectrum_toggled(move |enabled| {
            settings.borrow_mut().show_spectrum = enabled;
            settings.borrow().save(&store);
            if let Some(window) = window_weak.upgrade() {
                window.set_show_spectrum(enabled);
            }
            engine.send(EngineCommand::SetSpectrumEnabled(enabled));
        });
    }

    {
        let settings = settings.clone();
        let store = store.clone();
        let engine = engine.clone();
        settings_window.on_spectrum_channels_changed(move |value| {
            settings.borrow_mut().spectrum.channel_mode = spectrum_channel_from_label(&value);
            settings.borrow().save(&store);
            engine.send(EngineCommand::SetSpectrumConfig(settings.borrow().spectrum));
        });
    }
    {
        let settings = settings.clone();
        let store = store.clone();
        let engine = engine.clone();
        settings_window.on_spectrum_min_db_changed(move |v| {
            settings.borrow_mut().spectrum.min_db = v as f32;
            settings.borrow().save(&store);
            engine.send(EngineCommand::SetSpectrumConfig(settings.borrow().spectrum));
        });
    }
    {
        let settings = settings.clone();
        let store = store.clone();
        let engine = engine.clone();
        settings_window.on_spectrum_max_db_changed(move |v| {
            settings.borrow_mut().spectrum.max_db = v as f32;
            settings.borrow().save(&store);
            engine.send(EngineCommand::SetSpectrumConfig(settings.borrow().spectrum));
        });
    }
    {
        let settings = settings.clone();
        let store = store.clone();
        let engine = engine.clone();
        settings_window.on_spectrum_fft_size_changed(move |value| {
            let Ok(size) = value.parse::<usize>() else {
                return;
            };
            settings.borrow_mut().spectrum.fft_size = size;
            settings.borrow().save(&store);
            engine.send(EngineCommand::SetSpectrumConfig(settings.borrow().spectrum));
        });
    }
    {
        let settings = settings.clone();
        let store = store.clone();
        let settings_window_weak = settings_window.as_weak();
        let engine = engine.clone();
        settings_window.on_spectrum_peaks_toggled(move |enabled| {
            settings.borrow_mut().spectrum.peaks_enabled = enabled;
            settings.borrow().save(&store);
            if let Some(window) = settings_window_weak.upgrade() {
                window.set_spectrum_peaks_enabled(enabled);
            }
            engine.send(EngineCommand::SetSpectrumConfig(settings.borrow().spectrum));
        });
    }
    {
        let settings = settings.clone();
        let store = store.clone();
        let engine = engine.clone();
        settings_window.on_spectrum_peak_hold_changed(move |v| {
            settings.borrow_mut().spectrum.peak_hold_ms = v as f32;
            settings.borrow().save(&store);
            engine.send(EngineCommand::SetSpectrumConfig(settings.borrow().spectrum));
        });
    }
    {
        let settings = settings.clone();
        let store = store.clone();
        let engine = engine.clone();
        settings_window.on_spectrum_gravity_changed(move |v| {
            settings.borrow_mut().spectrum.gravity_db_per_s2 = v as f32;
            settings.borrow().save(&store);
            engine.send(EngineCommand::SetSpectrumConfig(settings.borrow().spectrum));
        });
    }
    {
        let settings = settings.clone();
        let store = store.clone();
        let engine = engine.clone();
        settings_window.on_spectrum_interval_changed(move |v| {
            settings.borrow_mut().spectrum.interval_ms = v as u32;
            settings.borrow().save(&store);
            engine.send(EngineCommand::SetSpectrumConfig(settings.borrow().spectrum));
        });
    }

    {
        let theme_registry = theme_registry.clone();
        let settings = settings.clone();
        let store = store.clone();
        let window_weak = window.as_weak();
        let properties_window_weak = properties_window.as_weak();
        let about_window_weak = about_window.as_weak();
        let settings_window_weak = settings_window.as_weak();
        let theme_editor_window_weak = theme_editor_window.as_weak();
        settings_window.on_theme_row_clicked(move |id| {
            let id = id.to_string();
            let Some(colors) = theme_registry.borrow().find(&id).map(|e| e.colors) else {
                return;
            };
            settings.borrow_mut().theme_id = id.clone();
            settings.borrow().save(&store);

            let (
                Some(window),
                Some(properties_window),
                Some(about_window),
                Some(settings_window),
                Some(editor),
            ) = (
                window_weak.upgrade(),
                properties_window_weak.upgrade(),
                about_window_weak.upgrade(),
                settings_window_weak.upgrade(),
                theme_editor_window_weak.upgrade(),
            )
            else {
                return;
            };
            theme::apply_theme(
                &window,
                &properties_window,
                &about_window,
                &settings_window,
                &editor,
                &colors,
            );
            refresh_theme_list(&settings_window, &theme_registry.borrow(), &id);
        });
    }
    {
        let theme_registry = theme_registry.clone();
        let settings = settings.clone();
        let draft_colors = draft_colors.clone();
        let draft_name = draft_name.clone();
        let theme_editor_window_weak = theme_editor_window.as_weak();
        settings_window.on_new_theme_clicked(move || {
            let current_id = settings.borrow().theme_id.clone();
            let colors = {
                let registry = theme_registry.borrow();
                registry
                    .find(&current_id)
                    .or_else(|| registry.find("dark"))
                    .map(|entry| entry.colors)
                    .expect("dark builtin theme must exist")
            };
            *draft_colors.borrow_mut() = colors;
            *draft_name.borrow_mut() = String::new();

            let Some(editor) = theme_editor_window_weak.upgrade() else {
                return;
            };
            editor.set_theme_name("".into());
            set_editor_fields(&editor, &colors);
            editor.show().expect("failed to show theme editor window");
        });
    }
    {
        let draft_name = draft_name.clone();
        theme_editor_window.on_name_edited(move |value| {
            *draft_name.borrow_mut() = value.to_string();
        });
    }
    {
        let draft_colors = draft_colors.clone();
        let theme_editor_window_weak = theme_editor_window.as_weak();
        theme_editor_window.on_field_edited(move |key, value| {
            let Some(color) = theme::parse_hex_color(&value) else {
                return;
            };
            draft_colors.borrow_mut().set(&key, color);
            if let Some(editor) = theme_editor_window_weak.upgrade() {
                set_editor_swatch(&editor, &key, color);
            }
        });
    }
    {
        let theme_editor_window_weak = theme_editor_window.as_weak();
        theme_editor_window.on_cancel_clicked(move || {
            if let Some(editor) = theme_editor_window_weak.upgrade() {
                editor.hide().expect("failed to hide theme editor window");
            }
        });
    }
    {
        let theme_registry = theme_registry.clone();
        let settings = settings.clone();
        let store = store.clone();
        let draft_colors = draft_colors.clone();
        let draft_name = draft_name.clone();
        let window_weak = window.as_weak();
        let properties_window_weak = properties_window.as_weak();
        let about_window_weak = about_window.as_weak();
        let settings_window_weak = settings_window.as_weak();
        let theme_editor_window_weak = theme_editor_window.as_weak();
        theme_editor_window.on_save_clicked(move || {
            let name = draft_name.borrow().trim().to_string();
            if name.is_empty() {
                return;
            }
            let colors = *draft_colors.borrow();
            let new_id = theme_registry.borrow_mut().add_custom(&store, &name, colors);
            settings.borrow_mut().theme_id = new_id.clone();
            settings.borrow().save(&store);

            let (
                Some(window),
                Some(properties_window),
                Some(about_window),
                Some(settings_window),
                Some(editor),
            ) = (
                window_weak.upgrade(),
                properties_window_weak.upgrade(),
                about_window_weak.upgrade(),
                settings_window_weak.upgrade(),
                theme_editor_window_weak.upgrade(),
            )
            else {
                return;
            };
            theme::apply_theme(
                &window,
                &properties_window,
                &about_window,
                &settings_window,
                &editor,
                &colors,
            );
            refresh_theme_list(&settings_window, &theme_registry.borrow(), &new_id);
            editor.hide().expect("failed to hide theme editor window");
        });
    }
    {
        let theme_registry = theme_registry.clone();
        let settings = settings.clone();
        let store = store.clone();
        let window_weak = window.as_weak();
        let properties_window_weak = properties_window.as_weak();
        let about_window_weak = about_window.as_weak();
        let settings_window_weak = settings_window.as_weak();
        let theme_editor_window_weak = theme_editor_window.as_weak();
        settings_window.on_delete_theme_clicked(move || {
            let current_id = settings.borrow().theme_id.clone();
            let is_custom = theme_registry
                .borrow()
                .find(&current_id)
                .map(|e| e.is_custom)
                .unwrap_or(false);
            if !is_custom {
                return;
            }
            theme_registry.borrow_mut().remove_custom(&store, &current_id);

            let fallback_id = "dark".to_string();
            let colors = theme_registry
                .borrow()
                .find(&fallback_id)
                .map(|entry| entry.colors)
                .expect("dark builtin theme must exist");
            settings.borrow_mut().theme_id = fallback_id.clone();
            settings.borrow().save(&store);

            let (
                Some(window),
                Some(properties_window),
                Some(about_window),
                Some(settings_window),
                Some(editor),
            ) = (
                window_weak.upgrade(),
                properties_window_weak.upgrade(),
                about_window_weak.upgrade(),
                settings_window_weak.upgrade(),
                theme_editor_window_weak.upgrade(),
            )
            else {
                return;
            };
            theme::apply_theme(
                &window,
                &properties_window,
                &about_window,
                &settings_window,
                &editor,
                &colors,
            );
            refresh_theme_list(&settings_window, &theme_registry.borrow(), &fallback_id);
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
                &store,
                event,
            );
        }
    });

    UiHandles {
        event_poll_timer,
        properties_window,
        about_window,
        settings_window,
        theme_editor_window,
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

fn refresh_theme_list(
    settings_window: &SettingsWindow,
    registry: &ThemeRegistry,
    current_id: &str,
) {
    let entries: Vec<ThemeListEntry> = registry
        .entries
        .iter()
        .map(|entry| ThemeListEntry {
            id: entry.id.clone().into(),
            label: entry.label.clone().into(),
            swatch: entry.colors.accent,
            is_current: entry.id == current_id,
            is_custom: entry.is_custom,
        })
        .collect();
    settings_window.set_themes(ModelRc::from(Rc::new(VecModel::from(entries))));
    settings_window.set_current_theme_is_custom(
        registry
            .find(current_id)
            .map(|e| e.is_custom)
            .unwrap_or(false),
    );
}

fn set_editor_fields(editor: &ThemeEditorWindow, colors: &ThemeColors) {
    for (key, _) in ThemeColors::FIELDS {
        let color = colors.get(key).expect("field key must be valid");
        set_editor_swatch(editor, key, color);
        set_editor_value(editor, key, theme::color_to_hex(color));
    }
}

fn set_editor_swatch(editor: &ThemeEditorWindow, key: &str, color: slint::Color) {
    match key {
        "bg_base" => editor.set_swatch_bg_base(color),
        "bg_raised" => editor.set_swatch_bg_raised(color),
        "bg_sunken" => editor.set_swatch_bg_sunken(color),
        "text_primary" => editor.set_swatch_text_primary(color),
        "text_muted" => editor.set_swatch_text_muted(color),
        "accent" => editor.set_swatch_accent(color),
        "accent_text" => editor.set_swatch_accent_text(color),
        "waveform_unplayed" => editor.set_swatch_waveform_unplayed(color),
        "spectrum_bar" => editor.set_swatch_spectrum_bar(color),
        _ => {}
    }
}

fn set_editor_value(editor: &ThemeEditorWindow, key: &str, value: String) {
    match key {
        "bg_base" => editor.set_value_bg_base(value.into()),
        "bg_raised" => editor.set_value_bg_raised(value.into()),
        "bg_sunken" => editor.set_value_bg_sunken(value.into()),
        "text_primary" => editor.set_value_text_primary(value.into()),
        "text_muted" => editor.set_value_text_muted(value.into()),
        "accent" => editor.set_value_accent(value.into()),
        "accent_text" => editor.set_value_accent_text(value.into()),
        "waveform_unplayed" => editor.set_value_waveform_unplayed(value.into()),
        "spectrum_bar" => editor.set_value_spectrum_bar(value.into()),
        _ => {}
    }
}

fn apply_event(
    window: &MainWindow,
    playlist_model: &Rc<VecModel<PlaylistRow>>,
    playlist_view_state: &Rc<RefCell<PlaylistViewState>>,
    current_track: &Rc<RefCell<Option<Track>>>,
    store: &Store,
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
            store.save_playlist(&tracks.iter().map(|t| t.path.clone()).collect::<Vec<_>>());
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
        EngineEvent::SpectrumUpdate { bars, peaks } => {
            window.set_spectrum_bars(ModelRc::from(Rc::new(VecModel::from(bars))));
            window.set_spectrum_peaks(ModelRc::from(Rc::new(VecModel::from(peaks))));
        }
    }
}

fn spectrum_channel_label(mode: engine::SpectrumChannelMode) -> &'static str {
    match mode {
        engine::SpectrumChannelMode::All => "All Channels",
        engine::SpectrumChannelMode::Left => "Left",
        engine::SpectrumChannelMode::Right => "Right",
    }
}

fn spectrum_channel_from_label(label: &str) -> engine::SpectrumChannelMode {
    match label {
        "Left" => engine::SpectrumChannelMode::Left,
        "Right" => engine::SpectrumChannelMode::Right,
        _ => engine::SpectrumChannelMode::All,
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
