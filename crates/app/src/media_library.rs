//! The media library (LIB panel): every audio file under the music folders
//! chosen in Preferences > Media Library, indexed in the database
//! (`library_tracks`) and shown as a tree -- artist > album > track by
//! default -- with search. Folders are scanned in the background on
//! startup and watched afterwards, so added, changed and deleted files show
//! up on their own; a rescan only reads the tags of files whose size or
//! modification time changed.

use crate::playlists::Playlists;
use crate::settings::Settings;
use crate::store::Store;
use crossbeam_channel::{Receiver, Sender};
use engine::LibraryTags;
use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, UNIX_EPOCH};
use ui::{format_duration, LibraryRow, MainWindow, SettingsWindow};

/// One indexed file.
#[derive(Debug, Clone, PartialEq)]
pub struct LibraryTrack {
    pub path: PathBuf,
    /// Modification time, milliseconds since the Unix epoch.
    pub modified: i64,
    pub size: u64,
    pub tags: LibraryTags,
}

// --- Scanning ----------------------------------------------------------------

/// `(modified, size)`: what a rescan compares to decide whether a file
/// changed.
fn file_stamp(path: &Path) -> Option<(i64, u64)> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as i64);
    Some((modified, metadata.len()))
}

#[derive(Debug, Default)]
pub struct ScanResult {
    /// New or changed files, with fresh tags.
    pub upserts: Vec<LibraryTrack>,
    /// Indexed files that are gone (or no longer under a music folder).
    pub removed: Vec<PathBuf>,
}

/// Brings an index holding `known` (path -> stamp) up to date with what's
/// on disk under `folders`. `None` if cancelled.
pub fn scan(
    folders: &[PathBuf],
    known: &HashMap<PathBuf, (i64, u64)>,
    cancel: &AtomicBool,
    mut progress: impl FnMut(usize, usize),
) -> Option<ScanResult> {
    let mut found = HashSet::new();
    let mut changed = Vec::new();
    for folder in folders {
        for path in engine::scan_audio_files(folder) {
            if cancel.load(Ordering::Relaxed) {
                return None;
            }
            if !found.insert(path.clone()) {
                continue; // nested music folders
            }
            if let Some(stamp) = file_stamp(&path) {
                if known.get(&path) != Some(&stamp) {
                    changed.push((path, stamp));
                }
            }
        }
    }
    let removed = known.keys().filter(|path| !found.contains(*path)).cloned().collect();

    // Reading tags is the slow part; spread it over the cores.
    let total = changed.len();
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get()).min(total.max(1));
    let next = std::sync::atomic::AtomicUsize::new(0);
    let (done_tx, done_rx) = crossbeam_channel::unbounded();
    let mut upserts = Vec::with_capacity(total);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            let done_tx = done_tx.clone();
            let (next, changed) = (&next, &changed);
            scope.spawn(move || {
                engine::lower_thread_priority();
                loop {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some((path, (modified, size))) = changed.get(i) else { break };
                    let track = LibraryTrack {
                        path: path.clone(),
                        modified: *modified,
                        size: *size,
                        tags: engine::read_library_tags(path),
                    };
                    if done_tx.send(track).is_err() {
                        break;
                    }
                }
            });
        }
        drop(done_tx);
        for track in done_rx.iter() {
            upserts.push(track);
            progress(upserts.len(), total);
        }
    });
    if cancel.load(Ordering::Relaxed) {
        return None;
    }
    Some(ScanResult { upserts, removed })
}

// --- The tree ----------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grouping {
    ArtistAlbum,
    Album,
    Genre,
    Year,
    Folder,
}

pub const GROUPINGS: [(Grouping, &str, &str); 5] = [
    (Grouping::ArtistAlbum, "artist-album", "Artist / Album"),
    (Grouping::Album, "album", "Album"),
    (Grouping::Genre, "genre", "Genre"),
    (Grouping::Year, "year", "Year"),
    (Grouping::Folder, "folder", "Folder"),
];

impl Grouping {
    pub fn from_id(id: &str) -> Self {
        GROUPINGS
            .iter()
            .find(|(_, gid, _)| *gid == id)
            .map_or(Grouping::ArtistAlbum, |(g, _, _)| *g)
    }

    fn index(self) -> usize {
        GROUPINGS.iter().position(|(g, _, _)| *g == self).unwrap_or(0)
    }
}

fn or_unknown(value: &str, unknown: &str) -> String {
    if value.trim().is_empty() {
        unknown.to_string()
    } else {
        value.trim().to_string()
    }
}

/// The year from a tagged date ("2019", "2019-11-15"), if it has one.
fn year_of(date: &str) -> Option<&str> {
    let year = date.get(..4)?;
    year.chars().all(|c| c.is_ascii_digit()).then_some(year)
}

fn album_artist(tags: &LibraryTags) -> String {
    let artist = if tags.album_artist.trim().is_empty() { &tags.artist } else { &tags.album_artist };
    or_unknown(artist, "Unknown artist")
}

fn album_label(tags: &LibraryTags) -> String {
    let album = or_unknown(&tags.album, "Unknown album");
    match year_of(&tags.date) {
        Some(year) => format!("{album} ({year})"),
        None => album,
    }
}

/// The folder a track is in, relative to (and named after) the music
/// folder it's under.
fn folder_label(path: &Path, roots: &[PathBuf]) -> String {
    let parent = path.parent().unwrap_or(path);
    roots
        .iter()
        .filter(|root| parent.starts_with(root))
        .max_by_key(|root| root.components().count())
        .map(|root| {
            let name = root.file_name().map_or_else(|| root.to_string_lossy(), |n| n.to_string_lossy());
            match parent.strip_prefix(root) {
                Ok(rest) if !rest.as_os_str().is_empty() => format!("{name}/{}", rest.to_string_lossy()),
                _ => name.into_owned(),
            }
        })
        .unwrap_or_else(|| parent.to_string_lossy().into_owned())
}

/// The groups a track sits in, outermost first.
fn groups_of(track: &LibraryTrack, grouping: Grouping, roots: &[PathBuf]) -> Vec<String> {
    let tags = &track.tags;
    let artist_album = || format!("{} \u{2013} {}", album_artist(tags), album_label(tags));
    match grouping {
        Grouping::ArtistAlbum => vec![album_artist(tags), album_label(tags)],
        Grouping::Album => vec![format!("{} \u{2013} {}", album_label(tags), album_artist(tags))],
        Grouping::Genre => vec![or_unknown(&tags.genre, "Unknown genre"), artist_album()],
        Grouping::Year => vec![
            year_of(&tags.date).map_or_else(|| "Unknown year".to_string(), str::to_string),
            artist_album(),
        ],
        Grouping::Folder => vec![folder_label(&track.path, roots)],
    }
}

/// Every whitespace-separated word of `query` appears in the track's
/// title, artist, album, genre, date or file name (case-insensitive).
fn matches(track: &LibraryTrack, words: &[String]) -> bool {
    if words.is_empty() {
        return true;
    }
    let tags = &track.tags;
    let haystack = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}",
        tags.title,
        tags.artist,
        tags.album_artist,
        tags.album,
        tags.genre,
        tags.date,
        track.path.file_name().unwrap_or_default().to_string_lossy()
    )
    .to_lowercase();
    words.iter().all(|word| haystack.contains(word.as_str()))
}

fn group_key(groups: &[String]) -> String {
    groups.join("\u{1f}")
}

/// What a visible row stands for.
#[derive(Debug, Clone, PartialEq)]
pub enum RowTarget {
    /// A group, by key (its groups joined) and depth.
    Group { key: String, depth: usize },
    /// A track, by position in `View::order`.
    Track(usize),
}

/// The library sorted and grouped one way, filtered by a search.
#[derive(Debug, Default)]
pub struct View {
    /// Matching tracks in display order, with their groups.
    pub order: Vec<(usize, Vec<String>)>,
    counts: HashMap<String, usize>,
}

impl View {
    pub fn build(tracks: &[LibraryTrack], grouping: Grouping, roots: &[PathBuf], query: &str) -> Self {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        let mut order: Vec<(usize, Vec<String>)> = tracks
            .iter()
            .enumerate()
            .filter(|(_, track)| matches(track, &words))
            .map(|(i, track)| (i, groups_of(track, grouping, roots)))
            .collect();
        let sort_key = |(i, groups): &(usize, Vec<String>)| {
            let tags = &tracks[*i].tags;
            (
                groups.iter().map(|g| g.to_lowercase()).collect::<Vec<_>>(),
                tags.disc_number.unwrap_or(0),
                tags.track_number.unwrap_or(u32::MAX),
                tags.title.to_lowercase(),
                tracks[*i].path.clone(),
            )
        };
        order.sort_by_cached_key(sort_key);
        let mut counts = HashMap::new();
        for (_, groups) in &order {
            for depth in 0..groups.len() {
                *counts.entry(group_key(&groups[..=depth])).or_insert(0) += 1;
            }
        }
        Self { order, counts }
    }

    /// The visible rows: groups, and the tracks of expanded groups (all of
    /// them when `expand_all`, e.g. while searching).
    pub fn rows(&self, expanded: &HashSet<String>, expand_all: bool) -> Vec<RowTarget> {
        let is_open = |key: &str| expand_all || expanded.contains(key);
        let mut rows = Vec::new();
        let mut previous: &[String] = &[];
        for (position, (_, groups)) in self.order.iter().enumerate() {
            let first_new = groups
                .iter()
                .zip(previous)
                .take_while(|(a, b)| a == b)
                .count();
            for depth in first_new..groups.len() {
                // Shown only if every enclosing group is open.
                if (0..depth).all(|d| is_open(&group_key(&groups[..=d]))) {
                    rows.push(RowTarget::Group {
                        key: group_key(&groups[..=depth]),
                        depth,
                    });
                }
            }
            if (0..groups.len()).all(|d| is_open(&group_key(&groups[..=d]))) {
                rows.push(RowTarget::Track(position));
            }
            previous = groups;
        }
        rows
    }

    /// Positions (in `order`) of the tracks in the group `key`.
    pub fn group_tracks(&self, key: &str) -> Vec<usize> {
        let prefix = format!("{key}\u{1f}");
        self.order
            .iter()
            .enumerate()
            .filter(|(_, (_, groups))| {
                let own = group_key(groups);
                own == key || own.starts_with(&prefix)
            })
            .map(|(position, _)| position)
            .collect()
    }

    pub fn count(&self, key: &str) -> usize {
        self.counts.get(key).copied().unwrap_or(0)
    }
}

/// `folder` for display, with the home directory shortened to `~`.
fn display_folder(folder: &Path) -> String {
    match std::env::var_os("HOME").map(PathBuf::from) {
        Some(home) if folder.starts_with(&home) && folder != home => format!(
            "~/{}",
            folder.strip_prefix(&home).unwrap_or(folder).to_string_lossy()
        ),
        _ => folder.to_string_lossy().into_owned(),
    }
}

// --- Wiring ------------------------------------------------------------------

enum ScanMessage {
    Progress { done: usize, total: usize },
    Done(Option<ScanResult>),
}

/// How long the folders must be quiet after a change before rescanning --
/// copying an album in fires an event per file.
const WATCH_SETTLE: Duration = Duration::from_secs(2);

/// The Media Library tab's index in the Preferences `TabWidget`.
const MEDIA_LIBRARY_TAB: i32 = 4;

pub struct MediaLibrary {
    main: slint::Weak<MainWindow>,
    preferences: slint::Weak<SettingsWindow>,
    /// Where tracks are played and added (see `Playlists::play_library`).
    playlists: Rc<Playlists>,
    store: Rc<Store>,
    settings: Rc<RefCell<Settings>>,
    tracks: RefCell<Vec<LibraryTrack>>,
    grouping: Cell<Grouping>,
    query: RefCell<String>,
    expanded: RefCell<HashSet<String>>,
    view: RefCell<View>,
    targets: RefCell<Vec<RowTarget>>,
    rows: Rc<VecModel<LibraryRow>>,
    scan_cancel: RefCell<Option<Arc<AtomicBool>>>,
    scan_messages: RefCell<Option<Receiver<ScanMessage>>>,
    scan_progress: Cell<Option<(usize, usize)>>,
    /// A rescan asked for while one was running.
    rescan_pending: Cell<bool>,
    watcher: RefCell<Option<notify::RecommendedWatcher>>,
    changes: (Sender<()>, Receiver<()>),
    changed_at: Cell<Option<Instant>>,
    timer: Timer,
}

impl MediaLibrary {
    pub fn wire(
        main: &MainWindow,
        preferences: &SettingsWindow,
        playlists: Rc<Playlists>,
        store: Rc<Store>,
        settings: Rc<RefCell<Settings>>,
    ) -> Rc<Self> {
        let grouping = Grouping::from_id(&settings.borrow().library_grouping);
        let library = Rc::new(Self {
            main: main.as_weak(),
            preferences: preferences.as_weak(),
            playlists,
            tracks: RefCell::new(store.library_tracks()),
            store,
            settings,
            grouping: Cell::new(grouping),
            query: RefCell::new(String::new()),
            expanded: RefCell::new(HashSet::new()),
            view: RefCell::new(View::default()),
            targets: RefCell::new(Vec::new()),
            rows: Rc::new(VecModel::default()),
            scan_cancel: RefCell::new(None),
            scan_messages: RefCell::new(None),
            scan_progress: Cell::new(None),
            rescan_pending: Cell::new(false),
            watcher: RefCell::new(None),
            changes: crossbeam_channel::unbounded(),
            changed_at: Cell::new(None),
            timer: Timer::default(),
        });

        main.set_library_rows(ModelRc::from(library.rows.clone()));
        main.set_library_groupings(ModelRc::from(Rc::new(VecModel::from(
            GROUPINGS.iter().map(|(_, _, label)| SharedString::from(*label)).collect::<Vec<_>>(),
        ))));
        main.set_library_grouping_index(grouping.index() as i32);

        macro_rules! on {
            ($window:ident . $callback:ident, |$lib:ident $(, $arg:ident)*| $body:expr) => {{
                let weak = Rc::downgrade(&library);
                $window.$callback(move |$($arg),*| {
                    if let Some($lib) = weak.upgrade() {
                        $body;
                    }
                });
            }};
        }
        on!(main.on_library_grouping_changed, |lib, index| lib.set_grouping(index));
        on!(main.on_library_search_edited, |lib, text| lib.search(&text));
        on!(main.on_library_row_clicked, |lib, row| lib.select(row));
        on!(main.on_library_row_toggled, |lib, row| lib.toggle(row));
        on!(main.on_library_row_activated, |lib, row| lib.play(row));
        on!(main.on_library_row_add_clicked, |lib, row| lib.add_to_playlist(row));
        on!(main.on_library_collapse_all_clicked, |lib| {
            lib.expanded.borrow_mut().clear();
            lib.refresh_rows();
        });
        on!(main.on_library_rescan_clicked, |lib| lib.start_scan());
        on!(main.on_library_add_folder_clicked, |lib| lib.add_folder());
        on!(main.on_library_folders_clicked, |lib| lib.open_preferences());
        on!(preferences.on_library_add_folder_clicked, |lib| lib.add_folder());
        on!(preferences.on_library_remove_folder_clicked, |lib, index| lib.remove_folder(index));
        on!(preferences.on_library_rescan_clicked, |lib| lib.start_scan());

        {
            let weak = Rc::downgrade(&library);
            library.timer.start(TimerMode::Repeated, Duration::from_millis(250), move || {
                if let Some(library) = weak.upgrade() {
                    library.poll();
                }
            });
        }

        library.folders_changed();
        library.rebuild_view();
        library
    }

    fn folders(&self) -> Vec<PathBuf> {
        self.settings.borrow().library_folders.clone()
    }

    /// Re-watches, rescans and shows the (new) folder list.
    fn folders_changed(&self) {
        let folders = self.folders();
        if let Some(preferences) = self.preferences.upgrade() {
            preferences.set_library_folders(ModelRc::from(Rc::new(VecModel::from(
                folders
                    .iter()
                    .map(|folder| SharedString::from(display_folder(folder)))
                    .collect::<Vec<_>>(),
            ))));
        }
        if let Some(main) = self.main.upgrade() {
            main.set_library_has_folders(!folders.is_empty());
        }
        self.watch(&folders);
        self.start_scan();
    }

    fn watch(&self, folders: &[PathBuf]) {
        use notify::Watcher;
        *self.watcher.borrow_mut() = None;
        if folders.is_empty() {
            return;
        }
        let tx = self.changes.0.clone();
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            if event.is_ok_and(|event| !event.kind.is_access()) {
                let _ = tx.send(());
            }
        });
        match watcher {
            Ok(mut watcher) => {
                for folder in folders {
                    if let Err(e) = watcher.watch(folder, notify::RecursiveMode::Recursive) {
                        tracing::warn!("can't watch {}: {e}", folder.display());
                    }
                }
                *self.watcher.borrow_mut() = Some(watcher);
            }
            Err(e) => tracing::warn!("can't watch the music folders: {e}"),
        }
    }

    fn start_scan(&self) {
        if self.scan_messages.borrow().is_some() {
            self.rescan_pending.set(true);
            return;
        }
        let folders = self.folders();
        let known: HashMap<PathBuf, (i64, u64)> = self
            .tracks
            .borrow()
            .iter()
            .map(|track| (track.path.clone(), (track.modified, track.size)))
            .collect();
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = crossbeam_channel::unbounded();
        {
            let cancel = cancel.clone();
            let spawned = std::thread::Builder::new()
                .name("library-scan".into())
                .spawn(move || {
                    let mut last = Instant::now();
                    let result = scan(&folders, &known, &cancel, |done, total| {
                        if done == total || last.elapsed() >= Duration::from_millis(200) {
                            let _ = tx.send(ScanMessage::Progress { done, total });
                            last = Instant::now();
                        }
                    });
                    let _ = tx.send(ScanMessage::Done(result));
                });
            if let Err(e) = spawned {
                tracing::warn!("couldn't start the library scan: {e}");
                return;
            }
        }
        *self.scan_cancel.borrow_mut() = Some(cancel);
        *self.scan_messages.borrow_mut() = Some(rx);
        self.scan_progress.set(Some((0, 0)));
        self.show_status();
    }

    fn poll(&self) {
        // Folder changes: rescan once things have been quiet for a moment.
        let mut changed = false;
        while self.changes.1.try_recv().is_ok() {
            changed = true;
        }
        if changed {
            self.changed_at.set(Some(Instant::now()));
        }
        if self.changed_at.get().is_some_and(|at| at.elapsed() >= WATCH_SETTLE) {
            self.changed_at.set(None);
            self.start_scan();
        }

        let mut finished = None;
        if let Some(rx) = self.scan_messages.borrow().as_ref() {
            for message in rx.try_iter() {
                match message {
                    ScanMessage::Progress { done, total } => self.scan_progress.set(Some((done, total))),
                    ScanMessage::Done(result) => finished = Some(result),
                }
            }
        } else {
            return;
        }
        match finished {
            Some(result) => {
                *self.scan_messages.borrow_mut() = None;
                *self.scan_cancel.borrow_mut() = None;
                self.scan_progress.set(None);
                if let Some(result) = result {
                    self.apply(result);
                }
                if self.rescan_pending.replace(false) {
                    self.start_scan();
                } else {
                    self.show_status();
                }
            }
            None => self.show_status(),
        }
    }

    fn apply(&self, result: ScanResult) {
        if result.upserts.is_empty() && result.removed.is_empty() {
            return;
        }
        self.store.update_library(&result.upserts, &result.removed);
        {
            let mut tracks = self.tracks.borrow_mut();
            let removed: HashSet<&PathBuf> = result.removed.iter().collect();
            let updated: HashSet<&PathBuf> = result.upserts.iter().map(|t| &t.path).collect();
            tracks.retain(|track| !removed.contains(&track.path) && !updated.contains(&track.path));
            tracks.extend(result.upserts);
        }
        self.rebuild_view();
    }

    fn show_status(&self) {
        let tracks = self.tracks.borrow().len();
        let folders = self.folders().len();
        let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
        let mut status = format!("{} in {}", plural(tracks, "track"), plural(folders, "folder"));
        if let Some((done, total)) = self.scan_progress.get() {
            status = if total == 0 {
                format!("{status} \u{2014} scanning\u{2026}")
            } else {
                format!("{status} \u{2014} reading tags {done}/{total}\u{2026}")
            };
        }
        let shown = self.view.borrow().order.len();
        let main_status = if self.query.borrow().trim().is_empty() || shown == tracks {
            status.clone()
        } else {
            format!("{} found \u{00b7} {status}", plural(shown, "track"))
        };
        if let Some(main) = self.main.upgrade() {
            main.set_library_status(main_status.into());
        }
        if let Some(preferences) = self.preferences.upgrade() {
            preferences.set_library_status(status.into());
        }
    }

    fn rebuild_view(&self) {
        *self.view.borrow_mut() = View::build(
            &self.tracks.borrow(),
            self.grouping.get(),
            &self.folders(),
            &self.query.borrow(),
        );
        self.refresh_rows();
        self.show_status();
    }

    fn refresh_rows(&self) {
        let view = self.view.borrow();
        let tracks = self.tracks.borrow();
        let searching = !self.query.borrow().trim().is_empty();
        let targets = view.rows(&self.expanded.borrow(), searching);
        let rows: Vec<LibraryRow> = targets
            .iter()
            .map(|target| match target {
                RowTarget::Group { key, depth } => {
                    let count = view.count(key);
                    LibraryRow {
                        is_group: true,
                        depth: *depth as i32,
                        text: key.rsplit('\u{1f}').next().unwrap_or_default().into(),
                        detail: count.to_string().into(),
                        expanded: searching || self.expanded.borrow().contains(key),
                        number: SharedString::new(),
                    }
                }
                RowTarget::Track(position) => {
                    let (index, groups) = &view.order[*position];
                    let tags = &tracks[*index].tags;
                    // Name the performer when it isn't the album's artist.
                    let text = if self.grouping.get() == Grouping::ArtistAlbum
                        && !tags.artist.is_empty()
                        && tags.artist != album_artist(tags)
                        || self.grouping.get() == Grouping::Folder && !tags.artist.is_empty()
                    {
                        format!("{} \u{00b7} {}", tags.title, tags.artist)
                    } else {
                        tags.title.clone()
                    };
                    LibraryRow {
                        is_group: false,
                        depth: groups.len() as i32,
                        text: text.into(),
                        detail: format_duration(tags.duration).into(),
                        expanded: false,
                        number: tags.track_number.map(|n| n.to_string()).unwrap_or_default().into(),
                    }
                }
            })
            .collect();
        drop(view);
        drop(tracks);
        self.rows.set_vec(rows);
        *self.targets.borrow_mut() = targets;
        if let Some(main) = self.main.upgrade() {
            main.set_library_selected(-1);
        }
    }

    fn set_grouping(&self, index: i32) {
        let Some((grouping, id, _)) = usize::try_from(index).ok().and_then(|i| GROUPINGS.get(i)) else {
            return;
        };
        if *grouping == self.grouping.get() {
            return;
        }
        self.grouping.set(*grouping);
        self.expanded.borrow_mut().clear();
        self.settings.borrow_mut().library_grouping = id.to_string();
        self.settings.borrow().save(&self.store);
        self.rebuild_view();
    }

    fn search(&self, text: &str) {
        if *self.query.borrow() == text {
            return;
        }
        *self.query.borrow_mut() = text.to_string();
        self.rebuild_view();
    }

    fn target(&self, row: i32) -> Option<RowTarget> {
        usize::try_from(row).ok().and_then(|row| self.targets.borrow().get(row).cloned())
    }

    fn select(&self, row: i32) {
        if let Some(main) = self.main.upgrade() {
            main.set_library_selected(row);
        }
    }

    fn toggle(&self, row: i32) {
        let Some(RowTarget::Group { key, .. }) = self.target(row) else { return };
        if !self.query.borrow().trim().is_empty() {
            return; // everything is open while searching
        }
        {
            let mut expanded = self.expanded.borrow_mut();
            if !expanded.remove(&key) {
                expanded.insert(key);
            }
        }
        self.refresh_rows();
        self.select(row);
    }

    /// The paths a row stands for, and which of them to start playing: a
    /// group's tracks from the first, or a track's album (innermost group)
    /// from that track.
    fn paths_for(&self, row: i32) -> Option<(Vec<PathBuf>, usize)> {
        let view = self.view.borrow();
        let tracks = self.tracks.borrow();
        let (positions, start) = match self.target(row)? {
            RowTarget::Group { key, .. } => (view.group_tracks(&key), 0),
            RowTarget::Track(position) => {
                let positions = view.group_tracks(&group_key(&view.order[position].1));
                let start = positions.iter().position(|&p| p == position).unwrap_or(0);
                (positions, start)
            }
        };
        let paths = positions.iter().map(|&p| tracks[view.order[p].0].path.clone()).collect();
        Some((paths, start))
    }

    fn play(&self, row: i32) {
        let Some((paths, start)) = self.paths_for(row) else { return };
        if paths.is_empty() {
            return;
        }
        // Not into the playlist shown, which this would replace for good
        // (issue #37).
        self.playlists.play_library(paths, start);
    }

    fn add_to_playlist(&self, row: i32) {
        let Some(paths) = (match self.target(row) {
            // Just the track, not its whole album.
            Some(RowTarget::Track(position)) => {
                let view = self.view.borrow();
                Some(vec![self.tracks.borrow()[view.order[position].0].path.clone()])
            }
            Some(RowTarget::Group { .. }) => self.paths_for(row).map(|(paths, _)| paths),
            None => None,
        }) else {
            return;
        };
        self.playlists.add_to_active(paths);
    }

    fn add_folder(&self) {
        let Some(folder) = rfd::FileDialog::new().set_title("Add music folder").pick_folder() else {
            return;
        };
        {
            let mut settings = self.settings.borrow_mut();
            if settings.library_folders.contains(&folder) {
                return;
            }
            settings.library_folders.push(folder);
            settings.save(&self.store);
        }
        self.folders_changed();
    }

    fn remove_folder(&self, index: i32) {
        {
            let mut settings = self.settings.borrow_mut();
            let Some(index) = usize::try_from(index).ok().filter(|&i| i < settings.library_folders.len()) else {
                return;
            };
            settings.library_folders.remove(index);
            settings.save(&self.store);
        }
        // A running scan would re-add the removed folder's tracks.
        if let Some(cancel) = self.scan_cancel.borrow().as_ref() {
            cancel.store(true, Ordering::Relaxed);
        }
        self.folders_changed();
    }

    /// Opens Preferences on the Media Library tab.
    fn open_preferences(&self) {
        if let Some(preferences) = self.preferences.upgrade() {
            preferences.set_current_tab(MEDIA_LIBRARY_TAB);
            let _ = preferences.show();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(path: &str, artist: &str, album: &str, number: u32, title: &str) -> LibraryTrack {
        LibraryTrack {
            path: PathBuf::from(path),
            modified: 0,
            size: 0,
            tags: LibraryTags {
                title: title.into(),
                artist: artist.into(),
                album: album.into(),
                track_number: Some(number),
                date: "2019-05-01".into(),
                genre: "Trance".into(),
                ..Default::default()
            },
        }
    }

    fn library() -> Vec<LibraryTrack> {
        vec![
            track("/m/b/2.flac", "Above & Beyond", "Common Ground", 2, "Northern Soul"),
            track("/m/b/1.flac", "Above & Beyond", "Common Ground", 1, "Common Ground"),
            track("/m/a/1.mp3", "Arty", "Glorious", 1, "Glorious"),
        ]
    }

    #[test]
    fn groups_artists_then_albums_with_tracks_in_order() {
        let tracks = library();
        let view = View::build(&tracks, Grouping::ArtistAlbum, &[], "");
        // All collapsed: just the artists.
        assert_eq!(
            view.rows(&HashSet::new(), false),
            [
                RowTarget::Group { key: "Above & Beyond".into(), depth: 0 },
                RowTarget::Group { key: "Arty".into(), depth: 0 },
            ]
        );
        assert_eq!(view.count("Above & Beyond"), 2);
        // Expanded: album, then tracks by number.
        let expanded: HashSet<String> =
            ["Above & Beyond".to_string(), "Above & Beyond\u{1f}Common Ground (2019)".to_string()].into();
        let rows = view.rows(&expanded, false);
        assert_eq!(rows.len(), 5);
        let titles: Vec<_> = rows
            .iter()
            .filter_map(|row| match row {
                RowTarget::Track(p) => Some(tracks[view.order[*p].0].tags.title.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(titles, ["Common Ground", "Northern Soul"]);
    }

    #[test]
    fn search_matches_every_word_and_opens_everything() {
        let tracks = library();
        let view = View::build(&tracks, Grouping::ArtistAlbum, &[], "beyond SOUL");
        assert_eq!(view.order.len(), 1);
        // Artist, album, track.
        assert_eq!(view.rows(&HashSet::new(), true).len(), 3);
        assert!(View::build(&tracks, Grouping::ArtistAlbum, &[], "nothing").order.is_empty());
    }

    #[test]
    fn a_group_holds_its_tracks_in_display_order() {
        let tracks = library();
        let view = View::build(&tracks, Grouping::Genre, &[], "");
        let positions = view.group_tracks("Trance");
        assert_eq!(positions.len(), 3);
        let album = view.group_tracks("Trance\u{1f}Arty \u{2013} Glorious (2019)");
        assert_eq!(album.len(), 1);
        assert_eq!(tracks[view.order[album[0]].0].tags.title, "Glorious");
    }

    #[test]
    fn folders_are_named_relative_to_their_music_folder() {
        let roots = [PathBuf::from("/home/me/Music")];
        assert_eq!(folder_label(Path::new("/home/me/Music/A/B/x.flac"), &roots), "Music/A/B");
        assert_eq!(folder_label(Path::new("/home/me/Music/x.flac"), &roots), "Music");
    }

    #[test]
    fn scanning_finds_new_changed_and_deleted_files() {
        let dir = std::env::temp_dir().join(format!("openbar-library-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../engine/tests/fixtures/test_tone.wav");
        let a = dir.join("a.wav");
        let b = dir.join("sub/b.wav");
        std::fs::copy(&fixture, &a).unwrap();
        std::fs::copy(&fixture, &b).unwrap();
        let never = AtomicBool::new(false);
        let folders = [dir.clone()];

        let first = scan(&folders, &HashMap::new(), &never, |_, _| {}).unwrap();
        assert_eq!(first.upserts.len(), 2);
        assert!(first.upserts.iter().all(|t| t.tags.duration > Duration::ZERO));

        // Nothing changed: nothing read.
        let known: HashMap<_, _> = first.upserts.iter().map(|t| (t.path.clone(), (t.modified, t.size))).collect();
        let again = scan(&folders, &known, &never, |_, _| {}).unwrap();
        assert!(again.upserts.is_empty() && again.removed.is_empty());

        // A deleted file is dropped; an indexed file outside the folders too.
        std::fs::remove_file(&b).unwrap();
        let mut known = known;
        known.insert(PathBuf::from("/elsewhere/c.flac"), (0, 0));
        let after = scan(&folders, &known, &never, |_, _| {}).unwrap();
        let mut removed = after.removed.clone();
        removed.sort();
        // Sorted the same way: on Windows, `C:\...` comes before `/...`.
        let mut expected = vec![PathBuf::from("/elsewhere/c.flac"), b];
        expected.sort();
        assert_eq!(removed, expected);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
