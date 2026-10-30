//! The now-playing file for stream overlays (issue #2): what's playing,
//! kept up to date in a small JSON file that an OBS browser source polls.
//! It's the format foobar2000's foo_np_simple writes for the "nowplaying"
//! overlay -- the same fields, UTF-8 with a byte order mark -- so that
//! overlay works with Openbar5000 unchanged:
//!
//! ```json
//! {"nowplaying": {"playing": 1, "paused": 0, "albumartist": "...",
//!   "album": "...", "artist": "...", "title": "...", "tracknumber": 3,
//!   "length": 409, "elapsed": 235, "path": "/the/track's/folder"},
//!  "config": {"fadeout": 0}}
//! ```
//!
//! The file only changes about once a second while playing (the elapsed
//! seconds), and is written on a thread of its own, so a slow disk never
//! holds up the window.

use crate::settings::Settings;
use crate::store::Store;
use crossbeam_channel::{Receiver, Sender};
use engine::{PlaybackState, Track};
use serde::Serialize;
use slint::ComponentHandle;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;
use ui::SettingsWindow;

/// What `NowPlayingFile::update` is shown, for the file.
pub struct NowPlaying<'a> {
    pub track: Option<&'a Track>,
    pub state: PlaybackState,
    pub elapsed: Duration,
}

/// Keeps the file at the chosen path (if any) up to date.
pub struct NowPlayingFile {
    path: RefCell<Option<PathBuf>>,
    /// The contents last handed to the writer, so unchanged ones aren't.
    written: RefCell<Option<String>>,
    writer: Sender<(PathBuf, String)>,
}

impl NowPlayingFile {
    /// Set up as saved, and run from Preferences > Integrations.
    pub fn wire(preferences: &SettingsWindow, store: Rc<Store>, settings: Rc<RefCell<Settings>>) -> Rc<Self> {
        let file = Rc::new(Self::new(chosen_path(&settings.borrow())));
        preferences.set_now_playing_file_enabled(settings.borrow().now_playing_file_enabled);
        preferences.set_now_playing_file_path(settings.borrow().now_playing_file_path.as_str().into());

        let apply = {
            let file = file.clone();
            let preferences = preferences.as_weak();
            move |settings: &Settings| {
                settings.save(&store);
                file.set_path(chosen_path(settings));
                if let Some(preferences) = preferences.upgrade() {
                    preferences.set_now_playing_file_enabled(settings.now_playing_file_enabled);
                    preferences.set_now_playing_file_path(settings.now_playing_file_path.as_str().into());
                }
            }
        };
        // Choosing a file turns it on; turning it on without one asks for
        // one, and stays off if none is chosen.
        let choose = {
            let settings = settings.clone();
            let apply = apply.clone();
            move || {
                let current = PathBuf::from(&settings.borrow().now_playing_file_path);
                let mut dialog = rfd::FileDialog::new()
                    .set_title("Now playing file")
                    .add_filter("JSON", &["json"])
                    .set_file_name("nowplaying.json");
                if let Some(dir) = current.parent().filter(|dir| dir.is_dir()) {
                    dialog = dialog.set_directory(dir);
                }
                let chosen = dialog.save_file();
                let mut settings = settings.borrow_mut();
                if let Some(path) = chosen {
                    settings.now_playing_file_path = path.to_string_lossy().into_owned();
                    settings.now_playing_file_enabled = true;
                }
                apply(&settings);
            }
        };
        preferences.on_now_playing_file_choose_clicked(choose.clone());
        preferences.on_now_playing_file_toggled(move |enabled| {
            if enabled && settings.borrow().now_playing_file_path.is_empty() {
                choose();
                return;
            }
            settings.borrow_mut().now_playing_file_enabled = enabled;
            apply(&settings.borrow());
        });
        file
    }

    /// Writes to `path`, or nothing until `set_path` gives one.
    fn new(path: Option<PathBuf>) -> Self {
        let (writer, writes) = crossbeam_channel::unbounded();
        let spawned = std::thread::Builder::new()
            .name("openbar5000-nowplaying".into())
            .spawn(move || write_files(writes));
        if let Err(e) = spawned {
            tracing::warn!("no now-playing file: couldn't start its thread: {e}");
        }
        Self {
            path: RefCell::new(path),
            written: RefCell::new(None),
            writer,
        }
    }

    /// Where to write from now on; `None` stops. The next `update` writes
    /// the new file at once.
    fn set_path(&self, path: Option<PathBuf>) {
        *self.path.borrow_mut() = path;
        *self.written.borrow_mut() = None;
    }

    pub fn update(&self, now: &NowPlaying) {
        let Some(path) = self.path.borrow().clone() else { return };
        let contents = contents(now);
        if self.written.borrow().as_deref() == Some(contents.as_str()) {
            return;
        }
        *self.written.borrow_mut() = Some(contents.clone());
        let _ = self.writer.send((path, contents));
    }
}

/// Where `settings` say to write, if anywhere.
fn chosen_path(settings: &Settings) -> Option<PathBuf> {
    let path = &settings.now_playing_file_path;
    (settings.now_playing_file_enabled && !path.is_empty()).then(|| PathBuf::from(path))
}

/// The writer thread: writes each file handed to it, skipping to the newest
/// when several are waiting. Ends with the `NowPlayingFile`.
fn write_files(writes: Receiver<(PathBuf, String)>) {
    // Logged once per kind of failure, not once a second.
    let mut last_error = None;
    while let Ok(mut write) = writes.recv() {
        while let Ok(newer) = writes.try_recv() {
            write = newer;
        }
        let (path, contents) = write;
        match write_file(&path, &contents) {
            Ok(()) => last_error = None,
            Err(e) => {
                let error = e.to_string();
                if last_error.as_ref() != Some(&error) {
                    tracing::warn!("couldn't write the now-playing file {}: {error}", path.display());
                    last_error = Some(error);
                }
            }
        }
    }
}

/// Writes next to `path` and then moves it into place, so the overlay never
/// reads half a file. Where the move is refused (Windows, while the file is
/// open elsewhere), it's written in place instead.
fn write_file(path: &Path, contents: &str) -> std::io::Result<()> {
    let mut bytes = Vec::with_capacity(3 + contents.len());
    bytes.extend_from_slice(b"\xEF\xBB\xBF");
    bytes.extend_from_slice(contents.as_bytes());
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    std::fs::write(&temporary, &bytes)?;
    if std::fs::rename(&temporary, path).is_err() {
        let _ = std::fs::remove_file(&temporary);
        std::fs::write(path, &bytes)?;
    }
    Ok(())
}

#[derive(Serialize)]
struct File<'a> {
    nowplaying: Fields<'a>,
    config: Config,
}

/// In the overlay's order. Text missing a tag is empty, numbers are 0 --
/// what foobar2000's formatting string makes of them.
#[derive(Serialize, Default)]
struct Fields<'a> {
    /// 1 while a track is playing or paused.
    playing: u8,
    paused: u8,
    albumartist: &'a str,
    album: &'a str,
    artist: &'a str,
    title: &'a str,
    tracknumber: u32,
    /// Whole seconds.
    length: u64,
    elapsed: u64,
    /// The folder the track is in (where the overlay looks for folder.jpg).
    path: String,
}

#[derive(Serialize)]
struct Config {
    /// Seconds before the overlay fades out after a change; 0 keeps it up.
    fadeout: u32,
}

/// The file's contents for `now`.
fn contents(now: &NowPlaying) -> String {
    let track = now.track.filter(|_| now.state != PlaybackState::Stopped);
    let file_name = track
        .and_then(|t| t.path.file_stem())
        .map(|name| name.to_string_lossy())
        .unwrap_or_default();
    let nowplaying = match track {
        Some(track) => {
            let m = &track.metadata;
            // As foobar2000 reads them: an untagged artist or album artist
            // is the other one, an untagged title the file name.
            Fields {
                playing: 1,
                paused: u8::from(now.state == PlaybackState::Paused),
                albumartist: if m.album_artist.is_empty() { &m.artist } else { &m.album_artist },
                album: &m.album,
                artist: if m.artist.is_empty() { &m.album_artist } else { &m.artist },
                title: if m.title.is_empty() { &file_name } else { &m.title },
                tracknumber: m.track_number.unwrap_or(0),
                length: m.duration.as_secs_f64().round() as u64,
                elapsed: now.elapsed.as_secs(),
                path: track.path.parent().map(|dir| dir.to_string_lossy().into_owned()).unwrap_or_default(),
            }
        }
        None => Fields::default(),
    };
    let file = File { nowplaying, config: Config { fadeout: 0 } };
    serde_json::to_string(&file).expect("plain strings and numbers always serialize")
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::TrackMetadata;

    fn track() -> Track {
        Track {
            path: PathBuf::from("/music/Super8 & Tab/Black Is The New Yellow/03 Extended Mix.flac"),
            metadata: TrackMetadata {
                title: "Black Is The New Yellow (Simon Gregory Extended Mix)".into(),
                artist: "Super8 & Tab".into(),
                album: "Black Is The New Yellow (Simon Gregory Remix)".into(),
                track_number: Some(3),
                duration: Duration::from_millis(409_400),
                ..Default::default()
            },
        }
    }

    fn parsed(now: &NowPlaying) -> serde_json::Value {
        serde_json::from_str(&contents(now)).unwrap()
    }

    #[test]
    fn a_playing_track_in_the_overlays_format() {
        let track = track();
        let now = NowPlaying { track: Some(&track), state: PlaybackState::Playing, elapsed: Duration::from_millis(235_900) };
        assert_eq!(
            parsed(&now),
            serde_json::json!({
                "nowplaying": {
                    "playing": 1,
                    "paused": 0,
                    // No album artist tag: the artist, as in foobar2000.
                    "albumartist": "Super8 & Tab",
                    "album": "Black Is The New Yellow (Simon Gregory Remix)",
                    "artist": "Super8 & Tab",
                    "title": "Black Is The New Yellow (Simon Gregory Extended Mix)",
                    "tracknumber": 3,
                    "length": 409,
                    "elapsed": 235,
                    "path": "/music/Super8 & Tab/Black Is The New Yellow"
                },
                "config": { "fadeout": 0 }
            })
        );
    }

    #[test]
    fn paused_still_counts_as_playing() {
        let track = track();
        let now = NowPlaying { track: Some(&track), state: PlaybackState::Paused, elapsed: Duration::ZERO };
        let file = parsed(&now);
        assert_eq!(file["nowplaying"]["playing"], 1);
        assert_eq!(file["nowplaying"]["paused"], 1);
    }

    #[test]
    fn stopped_or_nothing_loaded_is_all_empty() {
        let track = track();
        for track in [Some(&track), None] {
            let file = parsed(&NowPlaying { track, state: PlaybackState::Stopped, elapsed: Duration::ZERO });
            assert_eq!(file["nowplaying"]["playing"], 0);
            assert_eq!(file["nowplaying"]["title"], "");
            assert_eq!(file["nowplaying"]["length"], 0);
        }
    }

    #[test]
    fn untagged_titles_are_the_file_name_and_quotes_are_escaped() {
        let mut track = track();
        track.metadata.title.clear();
        track.metadata.album = "Say \"Hi\"".into();
        let now = NowPlaying { track: Some(&track), state: PlaybackState::Playing, elapsed: Duration::ZERO };
        let file = parsed(&now);
        assert_eq!(file["nowplaying"]["title"], "03 Extended Mix");
        assert_eq!(file["nowplaying"]["album"], "Say \"Hi\"");
    }

    #[test]
    fn writes_utf8_with_a_byte_order_mark_and_updates_in_place() {
        let dir = std::env::temp_dir().join(format!("openbar5000-nowplaying-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("nowplaying.json");
        write_file(&path, "{\"a\":1}").unwrap();
        write_file(&path, "{\"a\":2}").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"\xEF\xBB\xBF{\"a\":2}");
        // Nothing left behind next to it.
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
