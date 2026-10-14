use crate::discord_presence::{
    NowPlaying, PresenceClient, PresenceCommand, PresenceEvent, DEFAULT_CLIENT_ID,
};
use crate::layout::{drop_at, drop_preview, Axis, DragStart, Drop, Layout, PanelKind, Placement, Side};
use crate::artist_info::{self, ArtistInfo, Candidate, Outcome, WIKIPEDIA_LANGUAGES};
use crate::equalizer::Equalizer;
use crate::lyrics::Lyrics;
use crate::media_keys::{MediaAction, MediaKeys};
use crate::platform;
use crate::playlist_rows::{
    all_group_keys, build_grouped_rows, group_key_of, sort_order, CoverCache, PlaylistGrouping,
    PlaylistSort,
};
use crate::settings::{
    note_from_name, note_name, normalize_fps, valid_note_range, Settings, A4_MAX_HZ, A4_MIN_HZ,
    HIGHEST_NOTE, LOWEST_NOTE,
};
use crate::store::Store;
use crate::theme::{self, ThemeColors, ThemeRegistry};
use engine::{
    AudioInfo, EngineCommand, EngineEvent, PlaybackEngine, PlaybackMode, PlaybackState, Track,
};
use slint::{ComponentHandle, Model, ModelRc, Timer, TimerMode, VecModel};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use ui::{
    cover_image_from_rgba, format_duration, AboutWindow, ArtistCandidate, DropPreview, InfoRow, LayoutDivider, LyricLine, SpectrumNote, TabStripSlot, MainWindow, PanelChoice, PanelSlot, PlaylistRow, PropertiesWindow, SettingsWindow,
    LyricsEditorWindow, ThemeEditorWindow, ThemeListEntry,
};

/// The workspace version from `Cargo.toml` -- the one number the build
/// scripts also stamp into the macOS/Windows/Linux packages.
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// How often engine events are drained -- which, since the spectrum and
/// seekbar only change when they are, is also their redraw rate.
fn poll_interval(fps: u32) -> Duration {
    Duration::from_millis(1000 / normalize_fps(fps) as u64)
}

fn fps_label(fps: u32) -> String {
    format!("{} fps", normalize_fps(fps))
}

/// Resources the caller must keep alive for as long as the window's event
/// loop runs: the timer that drains [`EngineEvent`]s, and the (initially
/// hidden) secondary dialogs it may show on request. None of these fields
/// are read again after `wire()` returns -- they only need to not be dropped.
#[allow(dead_code)]
pub struct UiHandles {
    pub event_poll_timer: Rc<Timer>,
    pub properties_window: PropertiesWindow,
    pub about_window: AboutWindow,
    pub settings_window: SettingsWindow,
    pub theme_editor_window: ThemeEditorWindow,
    pub lyrics_editor_window: LyricsEditorWindow,
    pub equalizer: Rc<Equalizer>,
}

/// Shared UI-thread state kept in sync with the engine's playlist, so any
/// row-targeted action (play/properties/remove/open-folder/search) or
/// re-render (after a search edit) can work from the full, current track
/// list without round-tripping through the engine.
struct PlaylistViewState {
    tracks: Vec<Track>,
    current_index: Option<usize>,
    search_query: String,
    grouping: PlaylistGrouping,
    /// Group keys (see `PlaylistGrouping`) the user collapsed; reset when
    /// the grouping changes, since the keys mean something else then.
    collapsed: HashSet<String>,
    covers: CoverCache,
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
        grouping: PlaylistGrouping::default(),
        collapsed: HashSet::new(),
        covers: CoverCache::default(),
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

    window.set_app_version(APP_VERSION.into());
    let about_window = AboutWindow::new().expect("failed to create about window");
    about_window.set_app_version(APP_VERSION.into());
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
    let lyrics_editor_window =
        LyricsEditorWindow::new().expect("failed to create lyrics editor window");

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
    let themed_windows = theme::ThemedWindows::new(
        window,
        &properties_window,
        &about_window,
        &settings_window,
        &theme_editor_window,
        &lyrics_editor_window,
    );
    themed_windows.apply(&initial_colors);
    refresh_theme_list(
        &settings_window,
        &theme_registry.borrow(),
        &initial_theme_id,
    );

    playlist_view_state.borrow_mut().grouping = settings.borrow().playlist_grouping;
    window.set_playlist_grouping(settings.borrow().playlist_grouping.as_str().into());

    let equalizer = Equalizer::wire(
        window,
        &settings_window,
        engine.clone(),
        store.clone(),
        settings.clone(),
    );

    let track_panels = Rc::new(TrackPanels::new(window, store.clone(), settings.clone()));
    {
        let track_panels = track_panels.clone();
        let window_weak = window.as_weak();
        window.on_artist_lookup_clicked(move || {
            if let Some(window) = window_weak.upgrade() {
                track_panels.look_up_artist(&window, false);
            }
        });
    }
    {
        let track_panels = track_panels.clone();
        let window_weak = window.as_weak();
        window.on_artist_candidate_clicked(move |index| {
            if let Some(window) = window_weak.upgrade() {
                track_panels.choose_artist(&window, index as usize);
            }
        });
    }
    {
        let track_panels = track_panels.clone();
        let window_weak = window.as_weak();
        window.on_artist_none_of_these_clicked(move || {
            if let Some(window) = window_weak.upgrade() {
                track_panels.reject_candidates(&window);
            }
        });
    }
    {
        let track_panels = track_panels.clone();
        window.on_artist_link_clicked(move || {
            if let Some(link) = track_panels.artist_link.borrow().as_deref() {
                platform::open_url(link);
            }
        });
    }
    let lyrics_editor = Rc::new(LyricsEditor {
        window: lyrics_editor_window.clone_strong(),
        path: RefCell::new(None),
        original: RefCell::new(String::new()),
        close_after_save: Cell::new(false),
        saves: crossbeam_channel::unbounded(),
    });
    {
        let lyrics_editor = lyrics_editor.clone();
        let track_panels = track_panels.clone();
        let current_track = current_track.clone();
        window.on_edit_lyrics_clicked(move || {
            let (Some(path), Some(track)) = (
                track_panels.track_path.borrow().clone(),
                current_track.borrow().clone(),
            ) else {
                return;
            };
            let text = track_panels.lyrics_text.borrow().clone().unwrap_or_default();
            lyrics_editor.open(path, &track.metadata.title, &text);
        });
    }
    {
        let editor = lyrics_editor_window.as_weak();
        let window_weak = window.as_weak();
        lyrics_editor_window.on_sync_clicked(move || {
            let (Some(editor), Some(window)) = (editor.upgrade(), window_weak.upgrade()) else {
                return;
            };
            let position = Duration::from_secs_f32(window.get_position_seconds().max(0.0));
            let (text, cursor) =
                crate::lyrics::sync_line(&editor.get_text(), editor.get_cursor().max(0) as usize, position);
            editor.set_text(text.into());
            editor.invoke_place_cursor(cursor as i32);
        });
    }
    {
        let editor = lyrics_editor_window.as_weak();
        lyrics_editor_window.on_remove_timestamp_clicked(move || {
            let Some(editor) = editor.upgrade() else {
                return;
            };
            let (text, cursor) =
                crate::lyrics::remove_timestamp(&editor.get_text(), editor.get_cursor().max(0) as usize);
            editor.set_text(text.into());
            editor.invoke_place_cursor(cursor as i32);
        });
    }
    for (forward, step) in [(false, -5.0f32), (true, 5.0)] {
        let window_weak = window.as_weak();
        let seek = move || {
            if let Some(window) = window_weak.upgrade() {
                let target = (window.get_position_seconds() + step).clamp(0.0, window.get_duration_seconds().max(0.0));
                window.invoke_seek_requested(target);
            }
        };
        if forward {
            lyrics_editor_window.on_seek_forward_clicked(seek);
        } else {
            lyrics_editor_window.on_seek_back_clicked(seek);
        }
    }
    {
        let window_weak = window.as_weak();
        lyrics_editor_window.on_play_pause_clicked(move || {
            if let Some(window) = window_weak.upgrade() {
                window.invoke_play_pause_clicked();
            }
        });
    }
    {
        let lyrics_editor = lyrics_editor.clone();
        lyrics_editor_window.on_undo_clicked(move || {
            let original = lyrics_editor.original.borrow().clone();
            lyrics_editor.window.set_text(original.into());
            lyrics_editor.window.set_status("Changes undone.".into());
        });
    }
    {
        let lyrics_editor = lyrics_editor.clone();
        lyrics_editor_window.on_apply_clicked(move || lyrics_editor.save(false));
    }
    {
        let lyrics_editor = lyrics_editor.clone();
        lyrics_editor_window.on_ok_clicked(move || lyrics_editor.save(true));
    }
    {
        let editor = lyrics_editor_window.as_weak();
        lyrics_editor_window.on_cancel_clicked(move || {
            if let Some(editor) = editor.upgrade() {
                let _ = editor.hide();
            }
        });
    }

    settings_window.set_artist_wikipedia(settings.borrow().artist_wikipedia);
    settings_window.set_wikipedia_languages(ModelRc::from(Rc::new(VecModel::from(
        WIKIPEDIA_LANGUAGES
            .iter()
            .map(|(_, label)| slint::SharedString::from(*label))
            .collect::<Vec<_>>(),
    ))));
    settings_window.set_wikipedia_language_index(
        WIKIPEDIA_LANGUAGES
            .iter()
            .position(|(code, _)| *code == settings.borrow().artist_wikipedia_language)
            .unwrap_or(0) as i32,
    );
    {
        let settings = settings.clone();
        let store = store.clone();
        let track_panels = track_panels.clone();
        let window_weak = window.as_weak();
        settings_window.on_artist_wikipedia_toggled(move |enabled| {
            settings.borrow_mut().artist_wikipedia = enabled;
            settings.borrow().save(&store);
            if let Some(window) = window_weak.upgrade() {
                track_panels.show_artist(&window);
            }
        });
    }
    {
        let settings = settings.clone();
        let store = store.clone();
        let track_panels = track_panels.clone();
        let window_weak = window.as_weak();
        settings_window.on_wikipedia_language_changed(move |index| {
            let Some((code, _)) = WIKIPEDIA_LANGUAGES.get(index as usize) else {
                return;
            };
            settings.borrow_mut().artist_wikipedia_language = code.to_string();
            settings.borrow().save(&store);
            if let Some(window) = window_weak.upgrade() {
                track_panels.show_artist(&window);
            }
        });
    }
    wire_layout(window, &engine, &settings, &store, &track_panels);

    let initial_playback_mode = settings.borrow().playback_mode;
    sync_playback_mode_window(window, initial_playback_mode);
    engine.send(EngineCommand::SetPlaybackMode(initial_playback_mode));

    engine.send(EngineCommand::SetSpectrumConfig(settings.borrow().spectrum));
    {
        let spectrum = settings.borrow().spectrum;
        window.set_spectrum_notes(ModelRc::from(Rc::new(VecModel::from(spectrum_notes(
            spectrum.first_note,
            spectrum.last_note,
        )))));
        settings_window.set_note_names(ModelRc::from(Rc::new(VecModel::from(
            (LOWEST_NOTE..=HIGHEST_NOTE)
                .map(|note| slint::SharedString::from(note_name(note)))
                .collect::<Vec<_>>(),
        ))));
        settings_window.set_spectrum_first_note_index((spectrum.first_note - LOWEST_NOTE) as i32);
        settings_window.set_spectrum_last_note_index((spectrum.last_note - LOWEST_NOTE) as i32);
        settings_window.set_spectrum_a4_hz(spectrum.a4_hz.round() as i32);
    }
    window.set_spectrum_min_db(settings.borrow().spectrum.min_db as i32);
    window.set_spectrum_max_db(settings.borrow().spectrum.max_db as i32);
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

    // --- Discord Rich Presence setup --------------------------------------
    let presence = Rc::new(PresenceClient::spawn(&settings.borrow()));
    settings_window.set_discord_presence_enabled(settings.borrow().discord_presence_enabled);
    settings_window.set_discord_client_id(settings.borrow().discord_client_id.clone().into());
    settings_window.set_discord_status_text(
        presence_status_label(settings.borrow().discord_presence_enabled, false).into(),
    );

    {
        let settings = settings.clone();
        let store = store.clone();
        let presence = presence.clone();
        let settings_window_weak = settings_window.as_weak();
        settings_window.on_discord_presence_toggled(move |enabled| {
            // First time this is turned on with no Client ID set yet, fall
            // back to Openbar5000's own shared application so it works with
            // zero setup -- still a plain editable field afterward.
            if enabled
                && settings.borrow().discord_client_id.is_empty()
                && !DEFAULT_CLIENT_ID.is_empty()
            {
                settings.borrow_mut().discord_client_id = DEFAULT_CLIENT_ID.to_string();
                if let Some(window) = settings_window_weak.upgrade() {
                    window.set_discord_client_id(DEFAULT_CLIENT_ID.into());
                }
            }

            settings.borrow_mut().discord_presence_enabled = enabled;
            settings.borrow().save(&store);
            presence.send(PresenceCommand::Configure {
                enabled,
                client_id: settings.borrow().discord_client_id.clone(),
            });
            if let Some(window) = settings_window_weak.upgrade() {
                window.set_discord_status_text(presence_status_label(enabled, false).into());
            }
        });
    }
    {
        let settings = settings.clone();
        let store = store.clone();
        let presence = presence.clone();
        settings_window.on_discord_client_id_edited(move |client_id| {
            let client_id = client_id.to_string();
            settings.borrow_mut().discord_client_id = client_id.clone();
            settings.borrow().save(&store);
            presence.send(PresenceCommand::Configure {
                enabled: settings.borrow().discord_presence_enabled,
                client_id,
            });
        });
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
    // Created here (started at the end of `wire`) so the frame-rate setting
    // can retune it.
    let event_poll_timer = Rc::new(Timer::default());
    settings_window.set_visualization_fps(fps_label(settings.borrow().visualization_fps).into());
    {
        let settings = settings.clone();
        let store = store.clone();
        let event_poll_timer = event_poll_timer.clone();
        settings_window.on_visualization_fps_changed(move |label| {
            let fps = normalize_fps(label.trim_end_matches(" fps").parse().unwrap_or(60));
            settings.borrow_mut().visualization_fps = fps;
            settings.borrow().save(&store);
            event_poll_timer.set_interval(poll_interval(fps));
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
        let window_weak = window.as_weak();
        settings_window.on_spectrum_min_db_changed(move |v| {
            if let Some(window) = window_weak.upgrade() {
                window.set_spectrum_min_db(v);
            }
            settings.borrow_mut().spectrum.min_db = v as f32;
            settings.borrow().save(&store);
            engine.send(EngineCommand::SetSpectrumConfig(settings.borrow().spectrum));
        });
    }
    {
        let settings = settings.clone();
        let store = store.clone();
        let engine = engine.clone();
        let window_weak = window.as_weak();
        settings_window.on_spectrum_max_db_changed(move |v| {
            if let Some(window) = window_weak.upgrade() {
                window.set_spectrum_max_db(v);
            }
            settings.borrow_mut().spectrum.max_db = v as f32;
            settings.borrow().save(&store);
            engine.send(EngineCommand::SetSpectrumConfig(settings.borrow().spectrum));
        });
    }
    {
        let settings = settings.clone();
        let store = store.clone();
        let engine = engine.clone();
        let window_weak = window.as_weak();
        let settings_window_weak = settings_window.as_weak();
        settings_window.on_spectrum_note_range_changed(move |first, last| {
            let (Some(mut first), Some(mut last)) = (note_from_name(&first), note_from_name(&last))
            else {
                return;
            };
            // Keep at least two notes, nudging whichever end can move.
            if first >= last {
                if first < HIGHEST_NOTE {
                    last = first + 1;
                } else {
                    first = last - 1;
                }
            }
            if !valid_note_range(first, last) {
                return;
            }
            settings.borrow_mut().spectrum.first_note = first;
            settings.borrow_mut().spectrum.last_note = last;
            settings.borrow().save(&store);
            engine.send(EngineCommand::SetSpectrumConfig(settings.borrow().spectrum));
            if let Some(window) = window_weak.upgrade() {
                window.set_spectrum_notes(ModelRc::from(Rc::new(VecModel::from(spectrum_notes(first, last)))));
            }
            if let Some(settings_window) = settings_window_weak.upgrade() {
                settings_window.set_spectrum_first_note_index((first - LOWEST_NOTE) as i32);
                settings_window.set_spectrum_last_note_index((last - LOWEST_NOTE) as i32);
            }
        });
    }
    {
        let settings = settings.clone();
        let store = store.clone();
        let engine = engine.clone();
        settings_window.on_spectrum_a4_changed(move |hz| {
            settings.borrow_mut().spectrum.a4_hz = (hz as f32).clamp(A4_MIN_HZ, A4_MAX_HZ);
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
        let themed_windows = themed_windows.clone();
        let settings_window_weak = settings_window.as_weak();
        settings_window.on_theme_row_clicked(move |id| {
            let id = id.to_string();
            let Some(colors) = theme_registry.borrow().find(&id).map(|e| e.colors) else {
                return;
            };
            settings.borrow_mut().theme_id = id.clone();
            settings.borrow().save(&store);

            themed_windows.apply(&colors);
            let Some(settings_window) = settings_window_weak.upgrade()
            else {
                return;
            };
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
        let themed_windows = themed_windows.clone();
        let settings_window_weak = settings_window.as_weak();
        let theme_editor_window_weak = theme_editor_window.as_weak();
        theme_editor_window.on_save_clicked(move || {
            let name = draft_name.borrow().trim().to_string();
            if name.is_empty() {
                return;
            }
            let colors = *draft_colors.borrow();
            let new_id = theme_registry
                .borrow_mut()
                .add_custom(&store, &name, colors);
            settings.borrow_mut().theme_id = new_id.clone();
            settings.borrow().save(&store);

            themed_windows.apply(&colors);
            let (Some(settings_window), Some(editor)) =
                (settings_window_weak.upgrade(), theme_editor_window_weak.upgrade())
            else {
                return;
            };
            refresh_theme_list(&settings_window, &theme_registry.borrow(), &new_id);
            editor.hide().expect("failed to hide theme editor window");
        });
    }
    {
        let theme_registry = theme_registry.clone();
        let settings = settings.clone();
        let store = store.clone();
        let themed_windows = themed_windows.clone();
        let settings_window_weak = settings_window.as_weak();
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
            theme_registry
                .borrow_mut()
                .remove_custom(&store, &current_id);

            let fallback_id = "dark".to_string();
            let colors = theme_registry
                .borrow()
                .find(&fallback_id)
                .map(|entry| entry.colors)
                .expect("dark builtin theme must exist");
            settings.borrow_mut().theme_id = fallback_id.clone();
            settings.borrow().save(&store);

            themed_windows.apply(&colors);
            let Some(settings_window) = settings_window_weak.upgrade()
            else {
                return;
            };
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
            // Scanned on the engine thread: a big (or network) folder would
            // otherwise freeze the window while it's walked.
            engine.send(EngineCommand::LoadFolder(dir));
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

    // Created on first use and then kept alive: on Linux/X11 the copied
    // text is only served while the `Clipboard` that set it still exists.
    let clipboard: Rc<RefCell<Option<arboard::Clipboard>>> = Rc::new(RefCell::new(None));
    {
        let playlist_view_state = playlist_view_state.clone();
        let clipboard = clipboard.clone();
        window.on_row_copy_clicked(move |index| {
            if let Some(track) = playlist_view_state.borrow().tracks.get(index as usize) {
                copy_to_clipboard(&clipboard, track.path.display().to_string());
            }
        });
    }
    {
        let playlist_view_state = playlist_view_state.clone();
        let clipboard = clipboard.clone();
        let engine = engine.clone();
        window.on_row_cut_clicked(move |index| {
            let Some(path) = playlist_view_state
                .borrow()
                .tracks
                .get(index as usize)
                .map(|t| t.path.display().to_string())
            else {
                return;
            };
            if copy_to_clipboard(&clipboard, path) {
                engine.send(EngineCommand::RemoveIndex(index as usize));
            }
        });
    }
    {
        let playlist_view_state = playlist_view_state.clone();
        let engine = engine.clone();
        let window_weak = window.as_weak();
        window.on_row_enqueue_clicked(move |index| {
            let Some(title) = playlist_view_state
                .borrow()
                .tracks
                .get(index as usize)
                .map(|t| t.metadata.title.clone())
            else {
                return;
            };
            engine.send(EngineCommand::EnqueueIndex(index as usize));
            if let Some(window) = window_weak.upgrade() {
                window.set_status_text(format!("Queued: {title}").into());
            }
        });
    }
    {
        let playlist_model = playlist_model.clone();
        let playlist_view_state = playlist_view_state.clone();
        window.on_group_toggle_clicked(move |key| {
            {
                let mut state = playlist_view_state.borrow_mut();
                let key = key.to_string();
                if !state.collapsed.remove(&key) {
                    state.collapsed.insert(key);
                }
            }
            refresh_playlist_rows(&playlist_model, &playlist_view_state);
        });
    }
    {
        let playlist_model = playlist_model.clone();
        let playlist_view_state = playlist_view_state.clone();
        window.on_collapse_all_clicked(move || {
            {
                let mut state = playlist_view_state.borrow_mut();
                state.collapsed = all_group_keys(&state.tracks, state.grouping);
            }
            refresh_playlist_rows(&playlist_model, &playlist_view_state);
        });
    }
    {
        let playlist_model = playlist_model.clone();
        let playlist_view_state = playlist_view_state.clone();
        window.on_expand_all_clicked(move || {
            playlist_view_state.borrow_mut().collapsed.clear();
            refresh_playlist_rows(&playlist_model, &playlist_view_state);
        });
    }
    {
        let playlist_model = playlist_model.clone();
        let playlist_view_state = playlist_view_state.clone();
        let settings = settings.clone();
        let store = store.clone();
        let window_weak = window.as_weak();
        window.on_grouping_clicked(move |id| {
            let grouping = PlaylistGrouping::from_str(&id);
            {
                let mut state = playlist_view_state.borrow_mut();
                state.grouping = grouping;
                state.collapsed.clear();
            }
            settings.borrow_mut().playlist_grouping = grouping;
            settings.borrow().save(&store);
            if let Some(window) = window_weak.upgrade() {
                window.set_playlist_grouping(grouping.as_str().into());
            }
            refresh_playlist_rows(&playlist_model, &playlist_view_state);
        });
    }
    {
        let playlist_view_state = playlist_view_state.clone();
        let engine = engine.clone();
        window.on_sort_clicked(move |id| {
            let Some(sort) = PlaylistSort::from_str(&id) else {
                return;
            };
            let order = sort_order(&playlist_view_state.borrow().tracks, sort);
            engine.send(EngineCommand::ReorderPlaylist(order));
        });
    }
    {
        let playlist_model = playlist_model.clone();
        let playlist_view_state = playlist_view_state.clone();
        let window_weak = window.as_weak();
        window.on_show_now_playing_clicked(move || {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            {
                let mut state = playlist_view_state.borrow_mut();
                let Some(current) = state.current_index else {
                    return;
                };
                if let Some(key) = group_key_of(&state.tracks, current, state.grouping) {
                    state.collapsed.remove(&key);
                }
            }
            refresh_playlist_rows(&playlist_model, &playlist_view_state);
            if let Some(y) = current_row_offset(&playlist_model) {
                window.invoke_scroll_playlist_to(y);
            }
        });
    }
    {
        let engine = engine.clone();
        window.on_refresh_playlist_clicked(move || engine.send(EngineCommand::RefreshMetadata));
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
        let presence = presence.clone();
        let current_track = current_track.clone();
        let window_weak = window.as_weak();
        window.on_seek_requested(move |position_seconds| {
            let position = Duration::from_secs_f32(position_seconds.max(0.0));
            engine.send(EngineCommand::Seek(position));
            if let (Some(track), Some(window)) =
                (current_track.borrow().clone(), window_weak.upgrade())
            {
                send_presence_for_track(&presence, &track, position, window.get_is_playing());
            }
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
    {
        let settings = settings.clone();
        let store = store.clone();
        let engine = engine.clone();
        let window_weak = window.as_weak();
        window.on_shuffle_clicked(move || {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            // Mutually exclusive with Repeat: turning Shuffle on always
            // turns Repeat off, and clicking it again just goes back to
            // Normal.
            let new_mode = if settings.borrow().playback_mode == PlaybackMode::Shuffle {
                PlaybackMode::Normal
            } else {
                PlaybackMode::Shuffle
            };
            settings.borrow_mut().playback_mode = new_mode;
            settings.borrow().save(&store);
            sync_playback_mode_window(&window, new_mode);
            engine.send(EngineCommand::SetPlaybackMode(new_mode));
        });
    }
    {
        let settings = settings.clone();
        let store = store.clone();
        let engine = engine.clone();
        let window_weak = window.as_weak();
        window.on_loop_clicked(move || {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            // Three-state cycle, same as Spotify/fb2k/etc.: off -> repeat
            // the whole queue -> repeat just this one track -> off. Any
            // other mode (Normal or Shuffle) starts the cycle over at
            // RepeatAll.
            let new_mode = match settings.borrow().playback_mode {
                PlaybackMode::RepeatAll => PlaybackMode::RepeatOne,
                PlaybackMode::RepeatOne => PlaybackMode::Normal,
                PlaybackMode::Normal | PlaybackMode::Shuffle => PlaybackMode::RepeatAll,
            };
            settings.borrow_mut().playback_mode = new_mode;
            settings.borrow().save(&store);
            sync_playback_mode_window(&window, new_mode);
            engine.send(EngineCommand::SetPlaybackMode(new_mode));
        });
    }

    // Kept for the window's lifetime and updated row by row: handing Slint a
    // fresh model every frame would make it rebuild every bar element.
    let spectrum_bars = Rc::new(VecModel::<f32>::default());
    let spectrum_peaks = Rc::new(VecModel::<f32>::default());
    window.set_spectrum_bars(ModelRc::from(spectrum_bars.clone()));
    window.set_spectrum_peaks(ModelRc::from(spectrum_peaks.clone()));

    // OS media keys (issue #17). Registered once the event loop runs: on
    // Windows it needs the native window, which only exists once shown.
    let media_keys: Rc<RefCell<Option<MediaKeys>>> = Rc::new(RefCell::new(None));
    {
        let media_keys = media_keys.clone();
        let window_weak = window.as_weak();
        Timer::single_shot(Duration::ZERO, move || {
            if let Some(window) = window_weak.upgrade() {
                *media_keys.borrow_mut() = MediaKeys::new(native_window_handle(&window));
            }
        });
    }

    let window_weak = window.as_weak();
    let settings_window_weak = settings_window.as_weak();
    let initial_poll_interval = poll_interval(settings.borrow().visualization_fps);
    event_poll_timer.start(TimerMode::Repeated, initial_poll_interval, move || {
        let Some(window) = window_weak.upgrade() else {
            return;
        };
        // Position and spectrum updates can arrive several per tick; only
        // the newest of each is worth drawing.
        let mut latest_position = None;
        let mut latest_spectrum = None;
        while let Some(event) = engine.try_recv_event() {
            match event {
                EngineEvent::PositionUpdate { .. } => latest_position = Some(event),
                EngineEvent::SpectrumUpdate { bars, peaks } => latest_spectrum = Some((bars, peaks)),
                event => {
                    if matches!(event, EngineEvent::TrackChanged { .. }) {
                        // Belongs to the previous track.
                        latest_position = None;
                    }
                    if let Some(media) = media_keys.borrow_mut().as_mut() {
                        match &event {
                            EngineEvent::TrackChanged { track, .. } => media.set_track(track),
                            EngineEvent::PlaybackStateChanged(state) => media.set_state(
                                *state,
                                Duration::from_secs_f32(window.get_position_seconds()),
                            ),
                            _ => {}
                        }
                    }
                    apply_event(
                        &window,
                        &playlist_model,
                        &playlist_view_state,
                        &current_track,
                        &store,
                        &presence,
                        &track_panels,
                        event,
                    );
                }
            }
        }
        if let Some(event) = latest_position {
            apply_event(
                &window,
                &playlist_model,
                &playlist_view_state,
                &current_track,
                &store,
                &presence,
                &track_panels,
                event,
            );
        }
        if let Some((bars, peaks)) = latest_spectrum {
            sync_model(&spectrum_bars, bars);
            sync_model(&spectrum_peaks, peaks);
        }
        track_panels.poll(&window);
        lyrics_editor.poll(&window, &track_panels);
        let actions: Vec<MediaAction> = media_keys
            .borrow()
            .as_ref()
            .map(|media| std::iter::from_fn(|| media.try_recv()).collect())
            .unwrap_or_default();
        for action in actions {
            apply_media_action(&window, action);
        }
        while let Some(event) = presence.try_recv_event() {
            if let Some(settings_window) = settings_window_weak.upgrade() {
                let enabled = settings.borrow().discord_presence_enabled;
                let connected = event == PresenceEvent::Connected;
                settings_window
                    .set_discord_status_text(presence_status_label(enabled, connected).into());
            }
        }
    });

    UiHandles {
        event_poll_timer,
        properties_window,
        about_window,
        settings_window,
        theme_editor_window,
        lyrics_editor_window,
        equalizer,
    }
}

/// UI-thread state for the panel layout (see `crate::layout`).
struct LayoutState {
    /// Size of the panel area, as last reported by Slint.
    area: (f32, f32),
    /// The current layout placed into `area` -- slot/divider indices from
    /// Slint index into this.
    placement: Placement,
    /// Active divider drag and the pointer position it started at.
    drag: Option<(DragStart, f32)>,
    editing: bool,
    panel_drag: Option<PanelDrag>,
    tab_drag: Option<TabDrag>,
    /// What the engine was last told, so it's only told about changes.
    waveform_enabled: Option<bool>,
    spectrum_enabled: Option<bool>,
}

/// How long a track must stay current before the open BIO panel looks its
/// artist up on its own.
const AUTO_LOOKUP_DELAY: Duration = Duration::from_millis(700);

/// Lyrics read from a track's file: (path, lyrics).
type LyricsResult = (PathBuf, Option<String>);

/// A finished artist lookup: (cache key, artist, result).
type ArtistResult = (String, String, Result<Outcome, String>);

/// State behind the panels that describe the current track: album art,
/// the Info table, lyrics and artist information.
struct TrackPanels {
    album_art: AlbumArtLoader,
    lyrics: RefCell<Option<Lyrics>>,
    lyric_line: Cell<i32>,
    /// The playing track's file, and its lyrics exactly as stored (what
    /// the editor opens).
    track_path: RefCell<Option<PathBuf>>,
    lyrics_text: RefCell<Option<String>>,
    lyrics_results: (crossbeam_channel::Sender<LyricsResult>, crossbeam_channel::Receiver<LyricsResult>),
    store: Rc<Store>,
    settings: Rc<RefCell<Settings>>,
    /// The playing track's artist, if it has one, and its title (which
    /// tells namesakes apart in a lookup).
    artist: RefCell<Option<String>>,
    title: RefCell<String>,
    /// Namesakes offered in the BIO panel for the current artist.
    candidates: RefCell<Vec<Candidate>>,
    /// A namesake list waiting for the user's choice, by cache key -- so
    /// re-opening the tab shows it again without searching again.
    pending_choice: RefCell<Option<(String, Vec<Candidate>)>>,
    /// Whether the BIO panel is on screen: artists are only looked up
    /// online while it is.
    biography_visible: Cell<bool>,
    /// Cache key of the lookup in flight, so it isn't started twice.
    lookup_in_flight: RefCell<Option<String>>,
    /// Bumped when the artist changes; an automatic lookup still waiting
    /// out `AUTO_LOOKUP_DELAY` for an older artist then gives up.
    artist_generation: Arc<AtomicU64>,
    /// Where the BIO panel's "Read more" link goes.
    artist_link: RefCell<Option<String>>,
    artist_results: (crossbeam_channel::Sender<ArtistResult>, crossbeam_channel::Receiver<ArtistResult>),
}

impl TrackPanels {
    fn new(window: &MainWindow, store: Rc<Store>, settings: Rc<RefCell<Settings>>) -> Self {
        Self {
            album_art: AlbumArtLoader::new(window),
            lyrics: RefCell::new(None),
            lyric_line: Cell::new(-1),
            track_path: RefCell::new(None),
            lyrics_text: RefCell::new(None),
            lyrics_results: crossbeam_channel::unbounded(),
            store,
            settings,
            artist: RefCell::new(None),
            title: RefCell::new(String::new()),
            candidates: RefCell::new(Vec::new()),
            pending_choice: RefCell::new(None),
            biography_visible: Cell::new(false),
            lookup_in_flight: RefCell::new(None),
            artist_generation: Arc::new(AtomicU64::new(0)),
            artist_link: RefCell::new(None),
            artist_results: crossbeam_channel::unbounded(),
        }
    }

    fn track_changed(&self, window: &MainWindow, track: &Track) {
        window.set_now_playing_text(now_playing_text(track).into());
        window.set_track_info(ModelRc::from(Rc::new(VecModel::from(track_info_rows(track)))));
        self.album_art.show_track(Some(track.path.clone()));
        let artist = track.metadata.artist.trim();
        let artist = (!artist.is_empty()).then(|| artist.to_string());
        self.artist_changed(artist.as_deref());
        *self.artist.borrow_mut() = artist;
        *self.title.borrow_mut() = track.metadata.title.clone();
        self.show_artist(window);
        // Read the file's own lyrics in the background (its tags include
        // the cover art, which can be large); `poll` shows them.
        *self.track_path.borrow_mut() = Some(track.path.clone());
        *self.lyrics_text.borrow_mut() = None;
        self.set_lyrics(window, None, "");
        window.set_lyrics_editable(true);
        let path = track.path.clone();
        let results = self.lyrics_results.0.clone();
        std::thread::Builder::new()
            .name("openbar5000-lyrics".into())
            .spawn(move || {
                let lyrics = engine::read_lyrics(&path);
                let _ = results.send((path, lyrics));
            })
            .expect("failed to spawn lyrics thread");
    }

    fn stopped(&self, window: &MainWindow) {
        window.set_now_playing_text("".into());
        window.set_track_info(ModelRc::default());
        self.album_art.show_track(None);
        *self.track_path.borrow_mut() = None;
        *self.lyrics_text.borrow_mut() = None;
        window.set_lyrics_editable(false);
        self.artist_changed(None);
        *self.artist.borrow_mut() = None;
        self.show_artist(window);
        self.set_lyrics(window, None, "Nothing playing");
    }

    /// Shows `lyrics` in the Lyrics panel, or `status` when there are none.
    /// The entry point for lyric sources.
    fn set_lyrics(&self, window: &MainWindow, lyrics: Option<Lyrics>, status: &str) {
        let lines: Vec<LyricLine> = lyrics
            .iter()
            .flat_map(|lyrics| &lyrics.lines)
            .map(|line| LyricLine {
                text: line.text.clone().into(),
            })
            .collect();
        window.set_lyrics_synced(lyrics.as_ref().is_some_and(Lyrics::is_synced));
        window.set_lyric_lines(ModelRc::from(Rc::new(VecModel::from(lines))));
        window.set_lyric_status(status.into());
        window.set_lyric_active_line(-1);
        self.lyric_line.set(-1);
        *self.lyrics.borrow_mut() = lyrics;
    }

    /// A new track's artist: unless it's the same one (next song of an
    /// album), lookups still pending for the previous artist are dropped.
    fn artist_changed(&self, artist: Option<&str>) {
        let options = self.settings.borrow().artist_lookup();
        let key = artist.map(|artist| options.cache_key(artist));
        if self.lookup_in_flight.borrow().as_ref() != key.as_ref() {
            *self.lookup_in_flight.borrow_mut() = None;
            self.artist_generation.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Called whenever the layout is placed: opening the BIO panel (e.g.
    /// its tab) looks up the playing artist if that hasn't happened yet.
    fn set_biography_visible(&self, window: &MainWindow, visible: bool) {
        if self.biography_visible.replace(visible) != visible && visible {
            self.show_artist(window);
        }
    }

    /// Shows the current artist in the BIO panel: from the cache if it was
    /// looked up before (with the current Preferences); otherwise, if the
    /// panel is on screen, looks it up online -- and not while it isn't.
    fn show_artist(&self, window: &MainWindow) {
        *self.artist_link.borrow_mut() = None;
        self.candidates.borrow_mut().clear();
        let Some(artist) = self.artist.borrow().clone() else {
            window.set_artist_state("nothing".into());
            return;
        };
        window.set_artist_name(artist.clone().into());
        let options = self.settings.borrow().artist_lookup();
        let key = options.cache_key(&artist);
        let pending = self
            .pending_choice
            .borrow()
            .as_ref()
            .filter(|(pending_key, _)| *pending_key == key)
            .map(|(_, candidates)| candidates.clone());
        if let Some(candidates) = pending {
            self.show_candidates(window, candidates);
            return;
        }
        match self.store.artist_info(&key) {
            Some((Some(json), photo)) => match serde_json::from_str::<ArtistInfo>(&json) {
                Ok(info) => self.show_artist_info(window, &info, photo.as_deref()),
                Err(_) => self.show_artist_idle(window, &artist),
            },
            Some((None, _)) => {
                window.set_artist_message(
                    format!("No information found on {}.", options.services()).into(),
                );
                window.set_artist_state("not-found".into());
            }
            None if self.biography_visible.get() => self.look_up_artist(window, true),
            None => self.show_artist_idle(window, &artist),
        }
    }

    fn show_artist_idle(&self, window: &MainWindow, artist: &str) {
        let services = self.settings.borrow().artist_lookup().services();
        window.set_artist_message(
            format!("\u{201c}{artist}\u{201d} is looked up on {services} while this panel is open.")
                .into(),
        );
        window.set_artist_state("idle".into());
    }

    fn show_artist_info(&self, window: &MainWindow, info: &ArtistInfo, photo: Option<&[u8]>) {
        // The credit as written in the tags, unless MusicBrainz knows the
        // artist's own spelling.
        if !info.name.is_empty() {
            window.set_artist_name(info.name.clone().into());
        }
        window.set_artist_meta(info.meta_line().into());
        window.set_artist_genres(info.genres.join(", ").into());
        window.set_artist_summary(info.summary.clone().into());
        let image = photo.and_then(decode_photo);
        window.set_artist_has_photo(image.is_some());
        let (label, credit) = match (info.from_wikipedia, image.is_some()) {
            (true, true) => (
                "Read more on Wikipedia",
                "Biography and photo from Wikipedia (CC BY-SA 4.0 and the photo's own license). Artist data from MusicBrainz.",
            ),
            (true, false) => (
                "Read more on Wikipedia",
                "Biography from Wikipedia, under CC BY-SA 4.0. Artist data from MusicBrainz.",
            ),
            (false, _) => ("Open on MusicBrainz", "Artist data from MusicBrainz."),
        };
        window.set_artist_link_label(label.into());
        window.set_artist_credit(credit.into());
        window.set_artist_photo(image.unwrap_or_default());
        *self.artist_link.borrow_mut() = Some(info.link.clone());
        window.set_artist_state("found".into());
    }

    /// The BIO panel's button: looks the current artist up in the
    /// background; the result arrives through `poll`.
    /// `automatic` (the panel opened, or the track changed with it open)
    /// waits `AUTO_LOOKUP_DELAY` first and gives up if the artist changed
    /// meanwhile -- so skipping through tracks with the BIO tab open
    /// doesn't fire a lookup per track. A click starts right away.
    fn look_up_artist(&self, window: &MainWindow, automatic: bool) {
        let Some(artist) = self.artist.borrow().clone() else {
            return;
        };
        let options = self.settings.borrow().artist_lookup();
        let key = options.cache_key(&artist);
        window.set_artist_state("loading".into());
        if self.lookup_in_flight.borrow().as_deref() == Some(key.as_str()) {
            return;
        }
        *self.lookup_in_flight.borrow_mut() = Some(key.clone());
        let title = self.title.borrow().clone();
        let results = self.artist_results.0.clone();
        let generation = self.artist_generation.clone();
        let started_for = generation.load(Ordering::SeqCst);
        std::thread::Builder::new()
            .name("openbar5000-artist-info".into())
            .spawn(move || {
                if automatic {
                    std::thread::sleep(AUTO_LOOKUP_DELAY);
                    if generation.load(Ordering::SeqCst) != started_for {
                        return;
                    }
                }
                let result = artist_info::look_up(&artist, &title, &options);
                let _ = results.send((key, artist, result));
            })
            .expect("failed to spawn artist lookup thread");
    }

    /// The user picked namesake `index` from the BIO panel's list.
    fn choose_artist(&self, window: &MainWindow, index: usize) {
        let (Some(artist), Some(candidate)) = (
            self.artist.borrow().clone(),
            self.candidates.borrow().get(index).cloned(),
        ) else {
            return;
        };
        let options = self.settings.borrow().artist_lookup();
        let key = options.cache_key(&artist);
        window.set_artist_state("loading".into());
        *self.pending_choice.borrow_mut() = None;
        *self.lookup_in_flight.borrow_mut() = Some(key.clone());
        let results = self.artist_results.0.clone();
        std::thread::Builder::new()
            .name("openbar5000-artist-info".into())
            .spawn(move || {
                let result = artist_info::look_up_id(&candidate.id, &options);
                let _ = results.send((key, artist, result));
            })
            .expect("failed to spawn artist lookup thread");
    }

    /// "None of these": remember that this name has no known artist.
    fn reject_candidates(&self, window: &MainWindow) {
        let Some(artist) = self.artist.borrow().clone() else {
            return;
        };
        let key = self.settings.borrow().artist_lookup().cache_key(&artist);
        self.store.save_artist_info(&key, None, None);
        *self.pending_choice.borrow_mut() = None;
        self.show_artist(window);
    }

    fn show_candidates(&self, window: &MainWindow, candidates: Vec<Candidate>) {
        window.set_artist_candidates(ModelRc::from(Rc::new(VecModel::from(
            candidates
                .iter()
                .map(|c| ArtistCandidate {
                    name: c.name.clone().into(),
                    detail: c.detail.clone().into(),
                })
                .collect::<Vec<_>>(),
        ))));
        *self.candidates.borrow_mut() = candidates;
        window.set_artist_state("choose".into());
    }

    /// Shows `text` as the playing track's lyrics if `path` is still that
    /// track -- after reading them, or after the editor saved new ones.
    fn lyrics_loaded(&self, window: &MainWindow, path: &std::path::Path, text: Option<String>) {
        if self.track_path.borrow().as_deref() != Some(path) {
            return;
        }
        let lyrics = text.as_deref().map(Lyrics::parse).filter(|l| !l.lines.is_empty());
        self.set_lyrics(window, lyrics, "No Lyrics");
        *self.lyrics_text.borrow_mut() = text;
    }

    /// Takes finished lookups: caches them, and shows them if that artist
    /// is still the one playing.
    fn poll(&self, window: &MainWindow) {
        while let Ok((path, text)) = self.lyrics_results.1.try_recv() {
            self.lyrics_loaded(window, &path, text);
        }
        while let Ok((key, artist, result)) = self.artist_results.1.try_recv() {
            if self.lookup_in_flight.borrow().as_deref() == Some(key.as_str()) {
                *self.lookup_in_flight.borrow_mut() = None;
            }
            if let Ok(Outcome::Ambiguous(candidates)) = &result {
                *self.pending_choice.borrow_mut() = Some((key.clone(), candidates.clone()));
            }
            match &result {
                Ok(Outcome::Found(info, photo)) => {
                    let json = serde_json::to_string(info).expect("artist info serializes");
                    self.store.save_artist_info(&key, Some(&json), photo.as_deref());
                }
                Ok(Outcome::NotFound) => self.store.save_artist_info(&key, None, None),
                // Not cached: the user's choice is (once made), and an error
                // is tried again on the next click.
                Ok(Outcome::Ambiguous(_)) | Err(_) => {}
            }
            let current = self.artist.borrow().clone();
            if current.as_deref() != Some(artist.as_str()) {
                continue;
            }
            match result {
                Err(e) => {
                    window.set_artist_message(format!("Couldn't look up the artist: {e}").into());
                    window.set_artist_state("error".into());
                }
                Ok(Outcome::Ambiguous(candidates)) => self.show_candidates(window, candidates),
                Ok(_) => self.show_artist(window),
            }
        }
    }

    /// Moves the lyrics highlight to the line being sung.
    fn position_changed(&self, window: &MainWindow, position: Duration) {
        let line = self
            .lyrics
            .borrow()
            .as_ref()
            .and_then(|lyrics| lyrics.active_line(position))
            .map_or(-1, |i| i as i32);
        if self.lyric_line.replace(line) != line {
            window.set_lyric_active_line(line);
        }
    }
}

/// Decodes an artist photo (JPEG/PNG bytes) for display.
fn decode_photo(bytes: &[u8]) -> Option<slint::Image> {
    let rgba = image::load_from_memory(bytes).ok()?.into_rgba8();
    let (width, height) = rgba.dimensions();
    Some(cover_image_from_rgba(width, height, rgba.as_raw()))
}

/// The Info panel's rows for `track`, leaving out what isn't known.
fn track_info_rows(track: &Track) -> Vec<InfoRow> {
    let m = &track.metadata;
    let quality = match (m.sample_rate, m.bit_depth) {
        (Some(rate), Some(bits)) => format!("{:.1} kHz, {bits} bit", rate as f64 / 1000.0),
        (Some(rate), None) => format!("{:.1} kHz", rate as f64 / 1000.0),
        _ => String::new(),
    };
    let size = if m.file_size > 0 {
        format!("{:.1} MB", m.file_size as f64 / 1_000_000.0)
    } else {
        String::new()
    };
    // The order issue #6 lists them in, then technical extras.
    [
        ("Title", m.title.clone()),
        ("Album", m.album.clone()),
        ("Track", m.track_number.map(|n| n.to_string()).unwrap_or_default()),
        ("Disc", m.disc_number.map(|n| n.to_string()).unwrap_or_default()),
        ("Artist", m.artist.clone()),
        ("Composer", m.composer.clone()),
        ("Year", m.date.clone()),
        ("Format", m.format_label.clone()),
        ("Length", format_duration(m.duration)),
        ("Size", size),
        ("BPM", m.bpm.map(|bpm| bpm.to_string()).unwrap_or_default()),
        ("Genre", m.genre.clone()),
        ("Bitrate", m.bitrate_kbps.map(|k| format!("{k} kbps")).unwrap_or_default()),
        ("Sample rate", quality),
        ("File", track.path.display().to_string()),
    ]
    .into_iter()
    .filter(|(_, value)| !value.is_empty())
    .map(|(label, value)| InfoRow {
        label: label.into(),
        value: value.into(),
    })
    .collect()
}

/// Runs an OS media-key action through the same callbacks as the
/// on-screen controls, so both behave identically.
fn apply_media_action(window: &MainWindow, action: MediaAction) {
    match action {
        MediaAction::Play => window.invoke_menu_play_clicked(),
        MediaAction::Pause => window.invoke_menu_pause_clicked(),
        MediaAction::TogglePlayPause => window.invoke_play_pause_clicked(),
        MediaAction::Stop => window.invoke_stop_clicked(),
        MediaAction::Next => window.invoke_next_clicked(),
        MediaAction::Previous => window.invoke_previous_clicked(),
        MediaAction::SeekBy(seconds) => {
            let duration = window.get_duration_seconds().max(0.0);
            let target = (window.get_position_seconds() as f64 + seconds).clamp(0.0, duration as f64);
            window.invoke_seek_requested(target as f32);
        }
        MediaAction::SeekTo(position) => window.invoke_seek_requested(position.as_secs_f32()),
        MediaAction::SetVolume(volume) => {
            window.set_volume(volume);
            window.invoke_volume_changed(volume);
        }
        MediaAction::Raise => {
            let _ = window.show();
        }
        MediaAction::Quit => window.invoke_exit_clicked(),
    }
}

/// The main window's native handle where the OS media controls need one
/// (Windows' SMTC); `None` elsewhere.
#[cfg(windows)]
fn native_window_handle(window: &MainWindow) -> Option<*mut std::ffi::c_void> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match window.window().window_handle().window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as *mut std::ffi::c_void),
        _ => None,
    }
}

#[cfg(not(windows))]
fn native_window_handle(_window: &MainWindow) -> Option<*mut std::ffi::c_void> {
    None
}

/// A finished lyrics save: (file, the text saved, result).
type SaveResult = (PathBuf, String, Result<(), String>);

/// The lyrics editor window's state (issue #6): which file it edits --
/// fixed when it opens, even if playback moves on -- and the text it
/// opened with, for "Undo changes".
struct LyricsEditor {
    window: LyricsEditorWindow,
    path: RefCell<Option<PathBuf>>,
    original: RefCell<String>,
    close_after_save: Cell<bool>,
    saves: (
        crossbeam_channel::Sender<SaveResult>,
        crossbeam_channel::Receiver<SaveResult>,
    ),
}

impl LyricsEditor {
    fn open(&self, path: PathBuf, title: &str, text: &str) {
        *self.path.borrow_mut() = Some(path);
        *self.original.borrow_mut() = text.to_string();
        self.window.set_track_title(title.into());
        self.window.set_text(text.into());
        self.window.set_status("".into());
        self.window.set_busy(false);
        let _ = self.window.show();
        self.window.invoke_place_cursor(0);
    }

    /// Saves the text into the file on a background thread (it may copy a
    /// large file); the result arrives in `poll`.
    fn save(&self, close_after: bool) {
        let Some(path) = self.path.borrow().clone() else {
            return;
        };
        let text = self.window.get_text().to_string();
        self.close_after_save.set(close_after);
        self.window.set_busy(true);
        self.window.set_status("Saving\u{2026}".into());
        let saves = self.saves.0.clone();
        std::thread::Builder::new()
            .name("openbar5000-lyrics-save".into())
            .spawn(move || {
                let result = engine::write_lyrics(&path, &text);
                let _ = saves.send((path, text, result));
            })
            .expect("failed to spawn lyrics save thread");
    }

    /// Finished saves, plus the live playback time while it's open.
    fn poll(&self, main: &MainWindow, track_panels: &TrackPanels) {
        if self.window.window().is_visible() {
            self.window.set_playback_time(main.get_position_text());
            self.window.set_is_playing(main.get_is_playing());
        }
        while let Ok((path, text, result)) = self.saves.1.try_recv() {
            self.window.set_busy(false);
            match result {
                Ok(()) => {
                    *self.original.borrow_mut() = text.clone();
                    self.window.set_status("Saved to the file.".into());
                    let saved = (!text.trim().is_empty()).then_some(text);
                    track_panels.lyrics_loaded(main, &path, saved);
                    if self.close_after_save.get() {
                        let _ = self.window.hide();
                    }
                }
                Err(e) => self.window.set_status(format!("Couldn't save: {e}").into()),
            }
        }
    }
}

/// Longest edge (px) the album-art panel's cover is decoded at -- sharp at
/// any sensible panel size without holding a full-resolution scan.
const ALBUM_ART_MAX_SIZE: u32 = 600;

/// Loads the current track's cover for the album-art panel (issue #10) on
/// a background thread, and only while that panel is in the layout.
struct AlbumArtLoader {
    window: slint::Weak<MainWindow>,
    /// Bumped on every request; a load that finishes after a newer request
    /// (the track changed again meanwhile) is dropped.
    generation: Arc<AtomicU64>,
    enabled: Cell<bool>,
    current: RefCell<Option<PathBuf>>,
}

impl AlbumArtLoader {
    fn new(window: &MainWindow) -> Self {
        Self {
            window: window.as_weak(),
            generation: Arc::new(AtomicU64::new(0)),
            enabled: Cell::new(false),
            current: RefCell::new(None),
        }
    }

    /// The track now playing (`None` once stopped).
    fn show_track(&self, path: Option<PathBuf>) {
        *self.current.borrow_mut() = path;
        self.reload();
    }

    /// Whether the album-art panel exists; adding it mid-track loads the
    /// cover right away.
    fn set_enabled(&self, enabled: bool) {
        if self.enabled.replace(enabled) != enabled {
            self.reload();
        }
    }

    fn reload(&self) {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let path = self.current.borrow().clone();
        let Some(path) = path.filter(|_| self.enabled.get()) else {
            if let Some(window) = self.window.upgrade() {
                window.set_has_album_art(false);
                window.set_album_art(slint::Image::default());
            }
            return;
        };
        let window = self.window.clone();
        let latest = self.generation.clone();
        std::thread::Builder::new()
            .name("openbar5000-album-art".into())
            .spawn(move || {
                // Already outdated (skipping through tracks): don't decode.
                if latest.load(Ordering::SeqCst) != generation {
                    return;
                }
                let art = engine::load_cover_art(&path, ALBUM_ART_MAX_SIZE);
                let _ = window.upgrade_in_event_loop(move |window| {
                    if latest.load(Ordering::SeqCst) != generation {
                        return;
                    }
                    match art {
                        Some(art) => {
                            window.set_album_art(cover_image_from_rgba(art.width, art.height, &art.rgba));
                            window.set_has_album_art(true);
                        }
                        None => window.set_has_album_art(false),
                    }
                });
            })
            .expect("failed to spawn album art thread");
    }
}

/// The closed outline of one channel's waveform as SVG path commands, in a
/// `bucket count` x [-1, 1] space (y down, so peaks are negated): along the
/// top edge (maxima) left to right, then back along the bottom (minima) --
/// the stroked shape the reference UI draws per channel.
fn waveform_outline_path(channel: &engine::WaveformChannel) -> String {
    use std::fmt::Write;
    let mut path = String::with_capacity(channel.max.len() * 24);
    for (i, max) in channel.max.iter().enumerate() {
        let _ = write!(path, "{} {} {:.3} ", if i == 0 { "M" } else { "L" }, i, -max);
    }
    for (i, min) in channel.min.iter().enumerate().rev() {
        let _ = write!(path, "L {} {:.3} ", i, -min);
    }
    if !path.is_empty() {
        path.push('Z');
    }
    path
}

/// Labels for the spectrum's bars, one per note from `first` to `last`
/// ("F#0" ... "A8" by default).
fn spectrum_notes(first: u8, last: u8) -> Vec<SpectrumNote> {
    (first..=last)
        .map(|note| {
            let name = note_name(note);
            SpectrumNote {
                sharp: name.contains('#'),
                octave_start: note % 12 == 0,
                name: name.into(),
            }
        })
        .collect()
}

/// The playlist panel's summary line: "23 tracks, Length: 1:21:07".
fn playlist_summary(tracks: &[Track]) -> String {
    let total: Duration = tracks.iter().map(|t| t.metadata.duration).sum();
    let noun = if tracks.len() == 1 { "track" } else { "tracks" };
    format!("{} {noun}, Length: {}", tracks.len(), format_duration(total))
}

/// The now-playing line (issue #10): "01. Title (6:43) • Artist • Album",
/// leaving out whatever the tags don't have.
fn now_playing_text(track: &Track) -> String {
    let m = &track.metadata;
    let mut text = match m.track_number {
        Some(n) => format!("{n:02}. {}", m.title),
        None => m.title.clone(),
    };
    if !m.duration.is_zero() {
        text.push_str(&format!(" ({})", format_duration(m.duration)));
    }
    for part in [&m.artist, &m.album] {
        if !part.is_empty() {
            text.push_str(" \u{2022} ");
            text.push_str(part);
        }
    }
    text
}

/// A panel being dragged in edit mode.
struct PanelDrag {
    /// Slot index (into `LayoutState::placement`) of the dragged panel.
    from: usize,
    pressed_at: (f32, f32),
    /// Past the small threshold that tells a drag from a click.
    active: bool,
    /// Slot index and drop under the pointer right now.
    target: Option<(usize, Drop)>,
}

/// A tab being pressed or dragged.
struct TabDrag {
    /// Index into `LayoutState::placement.tab_strips`.
    strip: usize,
    tab: usize,
    pressed_at: (f32, f32),
    active: bool,
    target: Option<TabDropTarget>,
}

enum TabDropTarget {
    /// Within its own strip: move the tab so it lands before tab `to`.
    Reorder(usize),
    /// Out onto a panel (edit mode only), like dragging a panel.
    Panel(usize, Drop),
}

/// Side-by-side dividers between panels at least this tall get a visible,
/// draggable separator outside edit mode (e.g. tabs | playlist), while slim
/// strips like the now-playing line stay clean.
const SEPARATOR_MIN_HEIGHT: f32 = 60.0;

/// The panel under (`x`, `y`) to drop onto, ignoring slot `exclude` (the
/// one being dragged), with the drop and its on-screen preview.
fn panel_drop_target(
    placement: &Placement,
    x: f32,
    y: f32,
    exclude: Option<usize>,
) -> Option<(usize, Drop, DropPreview)> {
    let (index, slot) = placement.slots.iter().enumerate().find(|(_, slot)| {
        let r = &slot.rect;
        x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
    })?;
    if Some(index) == exclude {
        return None;
    }
    let drop = drop_at(&slot.rect, x, y);
    let rect = drop_preview(&slot.rect, drop);
    Some((
        index,
        drop,
        DropPreview {
            visible: true,
            label: drop_label(drop, slot.kind).into(),
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
        },
    ))
}

/// How far (px) the pointer must move before a press becomes a drag.
const PANEL_DRAG_THRESHOLD: f32 = 6.0;

fn drop_label(drop: Drop, target: PanelKind) -> String {
    let target = target.label();
    match drop {
        Drop::Swap => format!("Swap with {target}"),
        Drop::Beside(Side::Left) => format!("Place left of {target}"),
        Drop::Beside(Side::Right) => format!("Place right of {target}"),
        Drop::Beside(Side::Top) => format!("Place above {target}"),
        Drop::Beside(Side::Bottom) => format!("Place below {target}"),
    }
}

/// Connects the layout menus, panel-area resizes and divider drags. The
/// layout itself lives in `settings` (it's saved with them).
fn wire_layout(
    window: &MainWindow,
    engine: &Rc<PlaybackEngine>,
    settings: &Rc<RefCell<Settings>>,
    store: &Rc<Store>,
    track_panels: &Rc<TrackPanels>,
) {
    let slots = Rc::new(VecModel::<PanelSlot>::default());
    let dividers = Rc::new(VecModel::<LayoutDivider>::default());
    window.set_panel_slots(ModelRc::from(slots.clone()));
    window.set_layout_dividers(ModelRc::from(dividers.clone()));
    window.set_panel_choices(ModelRc::from(Rc::new(VecModel::from(
        PanelKind::ALL
            .iter()
            .map(|kind| PanelChoice {
                id: kind.id().into(),
                label: kind.label().into(),
            })
            .collect::<Vec<_>>(),
    ))));
    let state = Rc::new(RefCell::new(LayoutState {
        area: (window.get_panel_area_width(), window.get_panel_area_height()),
        placement: Placement::default(),
        drag: None,
        editing: false,
        panel_drag: None,
        tab_drag: None,
        waveform_enabled: None,
        spectrum_enabled: None,
    }));

    // Re-places the layout and pushes it to Slint (in place, so panels
    // aren't rebuilt on every resize), and switches the engine's waveform /
    // spectrum work on or off to match which panels exist.
    let relayout = {
        let state = state.clone();
        let settings = settings.clone();
        let engine = engine.clone();
        let track_panels = track_panels.clone();
        let window_weak = window.as_weak();
        Rc::new(move || {
            let layout = settings.borrow().layout.clone();
            let mut state = state.borrow_mut();
            let (width, height) = state.area;
            state.placement = layout.place(width, height);
            sync_model(
                &slots,
                state
                    .placement
                    .slots
                    .iter()
                    .map(|slot| {
                        let moves = layout.sibling_moves(&slot.path);
                        PanelSlot {
                            kind: slot.kind.id().into(),
                            label: slot.kind.label().into(),
                            x: slot.rect.x,
                            y: slot.rect.y,
                            width: slot.rect.width,
                            height: slot.rect.height,
                            move_side_by_side: moves.axis == Some(Axis::Horizontal),
                            can_move_back: moves.can_move_back,
                            can_move_forward: moves.can_move_forward,
                        }
                    })
                    .collect(),
            );
            sync_model(
                &dividers,
                state
                    .placement
                    .dividers
                    .iter()
                    .map(|divider| LayoutDivider {
                        side_by_side: divider.axis == Axis::Horizontal,
                        separator: divider.axis == Axis::Horizontal
                            && divider.rect.height >= SEPARATOR_MIN_HEIGHT,
                        x: divider.rect.x,
                        y: divider.rect.y,
                        width: divider.rect.width,
                        height: divider.rect.height,
                    })
                    .collect(),
            );
            if let Some(window) = window_weak.upgrade() {
                window.set_tab_strips(ModelRc::from(Rc::new(VecModel::from(
                    state
                        .placement
                        .tab_strips
                        .iter()
                        .map(|strip| TabStripSlot {
                            labels: ModelRc::from(Rc::new(VecModel::from(
                                strip
                                    .labels
                                    .iter()
                                    .map(|label| slint::SharedString::from(*label))
                                    .collect::<Vec<_>>(),
                            ))),
                            active: strip.active as i32,
                            x: strip.rect.x,
                            y: strip.rect.y,
                            width: strip.rect.width,
                            height: strip.rect.height,
                        })
                        .collect::<Vec<_>>(),
                ))));
            }
            // Only panels actually on screen do work -- a panel in a hidden
            // tab costs nothing until its tab is shown.
            let visible = |kind| state.placement.slots.iter().any(|slot| slot.kind == kind);
            let (waveform, spectrum, album_art, biography) = (
                visible(PanelKind::Waveform),
                visible(PanelKind::Spectrum),
                visible(PanelKind::AlbumArt),
                visible(PanelKind::Biography),
            );
            if state.waveform_enabled != Some(waveform) {
                state.waveform_enabled = Some(waveform);
                engine.send(EngineCommand::SetWaveformEnabled(waveform));
            }
            track_panels.album_art.set_enabled(album_art);
            if let Some(window) = window_weak.upgrade() {
                track_panels.set_biography_visible(&window, biography);
            }
            if state.spectrum_enabled != Some(spectrum) {
                state.spectrum_enabled = Some(spectrum);
                engine.send(EngineCommand::SetSpectrumEnabled(spectrum));
            }
        })
    };
    relayout();

    // Applies a structural edit to the panel at `slot_index`, then saves.
    let edit_slot = {
        let state = state.clone();
        let settings = settings.clone();
        let store = store.clone();
        let relayout = relayout.clone();
        Rc::new(move |slot_index: i32, edit: &dyn Fn(&mut Layout, &[usize])| {
            let Some(path) = state
                .borrow()
                .placement
                .slots
                .get(slot_index as usize)
                .map(|slot| slot.path.clone())
            else {
                return;
            };
            edit(&mut settings.borrow_mut().layout, &path);
            settings.borrow().save(&store);
            relayout();
        })
    };

    {
        let state = state.clone();
        let relayout = relayout.clone();
        window.on_panel_area_resized(move |width, height| {
            state.borrow_mut().area = (width, height);
            relayout();
        });
    }
    {
        let state = state.clone();
        let window_weak = window.as_weak();
        window.on_layout_edit_toggled(move || {
            let editing = {
                let mut state = state.borrow_mut();
                state.editing = !state.editing;
                state.drag = None;
                state.panel_drag = None;
                state.editing
            };
            if let Some(window) = window_weak.upgrade() {
                window.set_drop_preview(DropPreview::default());
                window.set_layout_editing(editing);
                window.set_status_text(
                    if editing {
                        "Editing layout: drag the dividers to resize, right-click a panel to change it."
                    } else {
                        "Layout saved."
                    }
                    .into(),
                );
            }
        });
    }
    {
        let settings = settings.clone();
        let store = store.clone();
        let relayout = relayout.clone();
        window.on_layout_reset_clicked(move || {
            settings.borrow_mut().layout = Layout::default_with(true, true);
            settings.borrow().save(&store);
            relayout();
        });
    }
    {
        let edit_slot = edit_slot.clone();
        window.on_panel_replace_clicked(move |slot_index, kind_id| {
            if let Some(kind) = PanelKind::from_id(&kind_id) {
                edit_slot(slot_index, &|layout, path| layout.replace(path, kind));
            }
        });
    }
    {
        let edit_slot = edit_slot.clone();
        window.on_panel_split_clicked(move |slot_index, side_by_side| {
            let axis = if side_by_side { Axis::Horizontal } else { Axis::Vertical };
            edit_slot(slot_index, &|layout, path| layout.split(path, axis));
        });
    }
    {
        let edit_slot = edit_slot.clone();
        window.on_panel_add_tab_clicked(move |slot_index| {
            edit_slot(slot_index, &|layout, path| layout.add_tab(path));
        });
    }
    {
        let state = state.clone();
        let settings = settings.clone();
        let store = store.clone();
        let relayout = relayout.clone();
        let window_weak = window.as_weak();
        window.on_tab_drag_released(move || {
            let Some(drag) = state.borrow_mut().tab_drag.take() else {
                return;
            };
            if let Some(window) = window_weak.upgrade() {
                window.set_drop_preview(DropPreview::default());
            }
            let (strip_path, target_path) = {
                let state = state.borrow();
                let Some(strip) = state.placement.tab_strips.get(drag.strip) else {
                    return;
                };
                let target_path = match &drag.target {
                    Some(TabDropTarget::Panel(slot, _)) => {
                        state.placement.slots.get(*slot).map(|slot| slot.path.clone())
                    }
                    _ => None,
                };
                (strip.path.clone(), target_path)
            };
            {
                let layout = &mut settings.borrow_mut().layout;
                match (drag.active, &drag.target) {
                    // A press without a drag is a click: show that tab.
                    (false, _) => layout.set_active_tab(&strip_path, drag.tab),
                    (true, Some(TabDropTarget::Reorder(to))) => {
                        layout.move_tab(&strip_path, drag.tab, *to)
                    }
                    (true, Some(TabDropTarget::Panel(_, drop))) => {
                        let Some(onto) = target_path else {
                            return;
                        };
                        let mut from = strip_path.clone();
                        from.push(drag.tab);
                        layout.drop_panel(&from, &onto, *drop);
                    }
                    (true, None) => return,
                }
            }
            settings.borrow().save(&store);
            relayout();
        });
    }
    {
        let state = state.clone();
        window.on_tab_drag_pressed(move |strip, tab, x, y| {
            state.borrow_mut().tab_drag = Some(TabDrag {
                strip: strip as usize,
                tab: tab as usize,
                pressed_at: (x, y),
                active: false,
                target: None,
            });
        });
    }
    {
        let state = state.clone();
        let window_weak = window.as_weak();
        window.on_tab_drag_moved(move |x, y| {
            let preview = {
                let mut state = state.borrow_mut();
                let state = &mut *state;
                let Some(drag) = state.tab_drag.as_mut() else {
                    return;
                };
                let (px, py) = drag.pressed_at;
                if !drag.active && (x - px).hypot(y - py) < PANEL_DRAG_THRESHOLD {
                    return;
                }
                drag.active = true;
                let Some(strip) = state.placement.tab_strips.get(drag.strip) else {
                    return;
                };
                let r = &strip.rect;
                let in_strip = x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height;
                if in_strip {
                    // Reorder: insert at the tab boundary nearest the pointer.
                    let count = strip.labels.len().max(1);
                    let tab_width = r.width / count as f32;
                    let to = (((x - r.x) / tab_width + 0.5).floor() as usize).min(count);
                    if to == drag.tab || to == drag.tab + 1 {
                        drag.target = None;
                        DropPreview::default()
                    } else {
                        drag.target = Some(TabDropTarget::Reorder(to));
                        DropPreview {
                            visible: true,
                            label: "".into(),
                            x: r.x + to as f32 * tab_width - 1.5,
                            y: r.y,
                            width: 3.0,
                            height: r.height,
                        }
                    }
                } else if state.editing {
                    // Out of the strip onto a panel, like a panel drag. The
                    // tab's own panel (if showing) isn't a target.
                    let mut own = strip.path.clone();
                    own.push(drag.tab);
                    let exclude = state.placement.slots.iter().position(|slot| slot.path == own);
                    match panel_drop_target(&state.placement, x, y, exclude) {
                        Some((index, drop, preview)) => {
                            drag.target = Some(TabDropTarget::Panel(index, drop));
                            preview
                        }
                        None => {
                            drag.target = None;
                            DropPreview::default()
                        }
                    }
                } else {
                    drag.target = None;
                    DropPreview::default()
                }
            };
            if let Some(window) = window_weak.upgrade() {
                window.set_drop_preview(preview);
            }
        });
    }
    {
        let edit_slot = edit_slot.clone();
        window.on_panel_move_clicked(move |slot_index, forward| {
            edit_slot(slot_index, &|layout, path| layout.shift(path, forward));
        });
    }
    {
        let state = state.clone();
        window.on_panel_drag_pressed(move |slot_index, x, y| {
            state.borrow_mut().panel_drag = Some(PanelDrag {
                from: slot_index as usize,
                pressed_at: (x, y),
                active: false,
                target: None,
            });
        });
    }
    {
        let state = state.clone();
        let window_weak = window.as_weak();
        window.on_panel_drag_moved(move |x, y| {
            let preview = {
                let mut state = state.borrow_mut();
                let state = &mut *state;
                let Some(drag) = state.panel_drag.as_mut() else {
                    return;
                };
                let (px, py) = drag.pressed_at;
                if !drag.active && (x - px).hypot(y - py) < PANEL_DRAG_THRESHOLD {
                    return;
                }
                drag.active = true;
                match panel_drop_target(&state.placement, x, y, Some(drag.from)) {
                    Some((index, drop, preview)) => {
                        drag.target = Some((index, drop));
                        preview
                    }
                    // Over itself or outside every panel: nothing to drop on.
                    None => {
                        drag.target = None;
                        DropPreview::default()
                    }
                }
            };
            if let Some(window) = window_weak.upgrade() {
                window.set_drop_preview(preview);
            }
        });
    }
    {
        let state = state.clone();
        let settings = settings.clone();
        let store = store.clone();
        let relayout = relayout.clone();
        let window_weak = window.as_weak();
        window.on_panel_drag_released(move || {
            let Some(drag) = state.borrow_mut().panel_drag.take() else {
                return;
            };
            if let Some(window) = window_weak.upgrade() {
                window.set_drop_preview(DropPreview::default());
            }
            let Some((target, drop)) = drag.target.filter(|_| drag.active) else {
                return;
            };
            let (from, onto, moved) = {
                let state = state.borrow();
                let (Some(from), Some(onto)) =
                    (state.placement.slots.get(drag.from), state.placement.slots.get(target))
                else {
                    return;
                };
                (from.path.clone(), onto.path.clone(), from.kind)
            };
            settings.borrow_mut().layout.drop_panel(&from, &onto, drop);
            settings.borrow().save(&store);
            relayout();
            if let Some(window) = window_weak.upgrade() {
                window.set_status_text(format!("Moved {}.", moved.label()).into());
            }
        });
    }
    {
        let edit_slot = edit_slot.clone();
        window.on_panel_remove_clicked(move |slot_index| {
            edit_slot(slot_index, &|layout, path| layout.remove(path));
        });
    }
    {
        let state = state.clone();
        window.on_divider_pressed(move |divider_index, pointer| {
            let mut state = state.borrow_mut();
            let start = state
                .placement
                .dividers
                .get(divider_index as usize)
                .and_then(|divider| DragStart::new(divider, &state.placement));
            state.drag = start.map(|start| (start, pointer));
        });
    }
    {
        let state = state.clone();
        let settings = settings.clone();
        let relayout = relayout.clone();
        window.on_divider_moved(move |pointer| {
            let Some((start, pressed_at)) = state.borrow().drag.clone() else {
                return;
            };
            settings.borrow_mut().layout.drag(&start, pointer - pressed_at);
            relayout();
        });
    }
    {
        let state = state.clone();
        let settings = settings.clone();
        let store = store.clone();
        window.on_divider_released(move || {
            if state.borrow_mut().drag.take().is_some() {
                settings.borrow().save(&store);
            }
        });
    }
}

/// Makes `model` hold `rows`, updating changed rows in place when the
/// length is unchanged so a repeater bound to it keeps its elements (and
/// their state, like the playlist's scroll position).
fn sync_model<T: Clone + PartialEq + 'static>(model: &VecModel<T>, rows: Vec<T>) {
    if model.row_count() != rows.len() {
        model.set_vec(rows);
        return;
    }
    for (i, row) in rows.into_iter().enumerate() {
        if model.row_data(i).as_ref() != Some(&row) {
            model.set_row_data(i, row);
        }
    }
}

fn refresh_playlist_rows(
    playlist_model: &Rc<VecModel<PlaylistRow>>,
    playlist_view_state: &Rc<RefCell<PlaylistViewState>>,
) {
    let mut state = playlist_view_state.borrow_mut();
    let state = &mut *state;
    let rows = build_grouped_rows(
        &state.tracks,
        state.current_index,
        &state.search_query,
        state.grouping,
        &state.collapsed,
        &mut state.covers,
    );
    playlist_model.set_vec(rows);
}

/// Pixel offset of the now-playing track row from the top of the playlist.
/// Row heights must match `playlist_view.slint` (64px headers, 22px tracks).
fn current_row_offset(playlist_model: &VecModel<PlaylistRow>) -> Option<f32> {
    let mut y = 0.0;
    for row in playlist_model.iter() {
        if row.is_current && !row.is_header {
            return Some(y);
        }
        y += if row.is_header { 64.0 } else { 22.0 };
    }
    None
}

/// Puts `text` on the system clipboard, returning whether it worked.
fn copy_to_clipboard(clipboard: &RefCell<Option<arboard::Clipboard>>, text: String) -> bool {
    let mut clipboard = clipboard.borrow_mut();
    if clipboard.is_none() {
        match arboard::Clipboard::new() {
            Ok(c) => *clipboard = Some(c),
            Err(e) => {
                tracing::warn!("clipboard unavailable: {e}");
                return false;
            }
        }
    }
    let result = clipboard
        .as_mut()
        .expect("initialized above")
        .set_text(text);
    if let Err(e) = &result {
        tracing::warn!("failed to copy to clipboard: {e}");
    }
    result.is_ok()
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

#[allow(clippy::too_many_arguments)]
fn apply_event(
    window: &MainWindow,
    playlist_model: &Rc<VecModel<PlaylistRow>>,
    playlist_view_state: &Rc<RefCell<PlaylistViewState>>,
    current_track: &Rc<RefCell<Option<Track>>>,
    store: &Store,
    presence: &PresenceClient,
    track_panels: &TrackPanels,
    event: EngineEvent,
) {
    match event {
        EngineEvent::TrackChanged { index, track } => {
            track_panels.track_changed(window, &track);
            window.set_duration_seconds(track.metadata.duration.as_secs_f32());
            window.set_duration_text(format_duration(track.metadata.duration).into());
            // "Artist - Title - Album", as in the reference status bar.
            window.set_status_text(
                [&track.metadata.artist, &track.metadata.title, &track.metadata.album]
                    .into_iter()
                    .filter(|part| !part.is_empty())
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join(" - ")
                    .into(),
            );
            // Reset right away rather than waiting for the next
            // `PositionUpdate` -- otherwise a RepeatOne/RepeatAll loop back
            // onto the same track briefly keeps showing the old end position.
            window.set_position_seconds(0.0);
            window.set_position_text(format_duration(Duration::ZERO).into());
            window.set_waveform_paths(ModelRc::default());
            // Not just `PlaybackStateChanged`: advancing to the next track
            // while already playing doesn't re-fire that event (the state
            // didn't change), so this is the only reliable place to update
            // presence for a plain track-to-track advance.
            send_presence_for_track(presence, &track, Duration::ZERO, window.get_is_playing());
            *current_track.borrow_mut() = Some(*track);
            playlist_view_state.borrow_mut().current_index = Some(index);
            refresh_playlist_rows(playlist_model, playlist_view_state);
        }
        EngineEvent::PositionUpdate { position, duration } => {
            track_panels.position_changed(window, position);
            window.set_position_seconds(position.as_secs_f32());
            window.set_position_text(format_duration(position).into());
            if duration.as_secs_f32() > 0.0 {
                window.set_duration_seconds(duration.as_secs_f32());
                window.set_duration_text(format_duration(duration).into());
            }
        }
        EngineEvent::PlaybackStateChanged(state) => {
            window.set_is_playing(state == PlaybackState::Playing);
            match state {
                PlaybackState::Stopped => {
                    presence.send(PresenceCommand::Cleared);
                    track_panels.stopped(window);
                }
                PlaybackState::Playing | PlaybackState::Paused => {
                    if let Some(track) = current_track.borrow().clone() {
                        let elapsed = Duration::from_secs_f32(window.get_position_seconds());
                        send_presence_for_track(
                            presence,
                            &track,
                            elapsed,
                            state == PlaybackState::Playing,
                        );
                    }
                }
            }
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
                // Covers may have changed (new playlist, Refresh playlist).
                state.covers.clear();
            }
            refresh_playlist_rows(playlist_model, playlist_view_state);
            window.set_status_text(format!("{track_count} track(s) loaded").into());
            window.set_playlist_summary(playlist_summary(&playlist_view_state.borrow().tracks).into());
        }
        EngineEvent::LoadProgress { done, total } => {
            window.set_status_text(format!("Loading playlist\u{2026} {done}/{total}").into());
        }
        EngineEvent::WaveformReady { index, channels } => {
            if playlist_view_state.borrow().current_index == Some(index) {
                let paths: Vec<slint::SharedString> =
                    channels.iter().map(|c| waveform_outline_path(c).into()).collect();
                let bucket_count = channels.first().map_or(1, |c| c.max.len().max(1));
                window.set_waveform_bucket_count(bucket_count as i32);
                window.set_waveform_paths(ModelRc::from(Rc::new(VecModel::from(paths))));
            }
        }
        // Coalesced and applied by the poll timer instead (see `wire`).
        EngineEvent::SpectrumUpdate { .. } => {}
    }
}



fn send_presence_for_track(
    presence: &PresenceClient,
    track: &Track,
    elapsed: Duration,
    is_playing: bool,
) {
    presence.send(PresenceCommand::NowPlaying(NowPlaying {
        title: track.metadata.title.clone(),
        artist: track.metadata.artist.clone(),
        album: track.metadata.album.clone(),
        elapsed,
        is_playing,
    }));
}

/// Syncs the transport bar's Shuffle/Repeat button state to `mode` --
/// `loop-active` lights up the button for either repeat sub-state,
/// `repeat-one-active` additionally selects the "1"-badged icon.
fn sync_playback_mode_window(window: &MainWindow, mode: PlaybackMode) {
    window.set_shuffle_active(mode == PlaybackMode::Shuffle);
    window.set_loop_active(matches!(
        mode,
        PlaybackMode::RepeatAll | PlaybackMode::RepeatOne
    ));
    window.set_repeat_one_active(mode == PlaybackMode::RepeatOne);
}

fn presence_status_label(enabled: bool, connected: bool) -> &'static str {
    if !enabled {
        "Disabled"
    } else if connected {
        "Connected"
    } else {
        "Not connected"
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

#[cfg(test)]
mod tests {
    use super::*;
    use engine::TrackMetadata;

    #[test]
    fn now_playing_text_matches_issue_10_and_skips_missing_tags() {
        let track = |number, artist: &str, album: &str| Track {
            path: PathBuf::from("/m/x.flac"),
            metadata: TrackMetadata {
                title: "Helpless [Mix Cut]".into(),
                artist: artist.into(),
                album: album.into(),
                track_number: number,
                duration: Duration::from_secs(403),
                ..Default::default()
            },
        };
        assert_eq!(
            now_playing_text(&track(Some(1), "Alexander Popov & Myon", "Trance World, Vol. 16")),
            "01. Helpless [Mix Cut] (6:43) \u{2022} Alexander Popov & Myon \u{2022} Trance World, Vol. 16"
        );
        assert_eq!(now_playing_text(&track(None, "", "")), "Helpless [Mix Cut] (6:43)");
    }

    #[test]
    fn spectrum_notes_span_f_sharp_0_to_a8() {
        let notes = spectrum_notes(18, 117);
        assert_eq!(notes.first().unwrap().name, "F#0");
        assert_eq!(notes.last().unwrap().name, "A8");
        let c4 = notes.iter().find(|n| n.name == "C4").unwrap();
        assert!(c4.octave_start && !c4.sharp);
        assert!(notes.iter().find(|n| n.name == "A#3").unwrap().sharp);
    }

    #[test]
    fn waveform_outline_goes_along_the_top_then_back_along_the_bottom() {
        let channel = engine::WaveformChannel {
            max: vec![0.5, 1.0],
            min: vec![-0.25, -1.0],
        };
        assert_eq!(
            waveform_outline_path(&channel),
            "M 0 -0.500 L 1 -1.000 L 1 1.000 L 0 0.250 Z"
        );
    }
}
