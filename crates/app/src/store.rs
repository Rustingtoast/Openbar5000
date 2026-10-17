use crate::media_library::LibraryTrack;
use crate::theme::{ThemeColors, ThemeEntry};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};

/// Ordered, numbered migrations applied against `PRAGMA user_version`. Each
/// entry is run exactly once, in order, the first time a database is opened
/// at a lower version -- the standard way to let the schema grow across
/// releases without losing existing users' data. Append new migrations
/// here; never edit an already-shipped one.
const MIGRATIONS: &[&str] = &[
    // v1: settings (flat key/value), themes (one row per custom theme), and
    // a single implicit "Default" playlist -- `playlist_id` is already a
    // real foreign key so supporting multiple named playlists later is a
    // UI-layer change, not a schema change.
    r#"
    CREATE TABLE settings (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );

    CREATE TABLE themes (
        id                 TEXT PRIMARY KEY,
        label              TEXT NOT NULL,
        bg_base            TEXT NOT NULL,
        bg_raised          TEXT NOT NULL,
        bg_sunken          TEXT NOT NULL,
        text_primary       TEXT NOT NULL,
        text_muted         TEXT NOT NULL,
        accent             TEXT NOT NULL,
        accent_text        TEXT NOT NULL,
        waveform_unplayed  TEXT NOT NULL,
        spectrum_bar       TEXT NOT NULL
    );

    CREATE TABLE playlists (
        id   INTEGER PRIMARY KEY AUTOINCREMENT,
        name TEXT NOT NULL
    );

    CREATE TABLE playlist_tracks (
        playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
        position    INTEGER NOT NULL,
        path        TEXT NOT NULL,
        PRIMARY KEY (playlist_id, position)
    );

    INSERT INTO playlists (id, name) VALUES (1, 'Default');
    "#,
    // v2: cached artist lookups for the BIO panel (see `artist_info`). `data`
    // is the JSON `ArtistInfo`, or NULL when the lookup found nothing --
    // cached too, so a miss isn't retried on every track.
    r#"
    CREATE TABLE artist_info (
        key        TEXT PRIMARY KEY,
        data       TEXT,
        photo      BLOB,
        fetched_at INTEGER NOT NULL
    );
    "#,
    // v3: the media library index (see `media_library`): one row per audio
    // file under the music folders, with the tags the LIB panel groups by.
    // `modified`/`size` tell a rescan which files need reading again.
    r#"
    CREATE TABLE library_tracks (
        path         TEXT PRIMARY KEY,
        modified     INTEGER NOT NULL,
        size         INTEGER NOT NULL,
        title        TEXT NOT NULL,
        artist       TEXT NOT NULL,
        album_artist TEXT NOT NULL,
        album        TEXT NOT NULL,
        track        INTEGER,
        disc         INTEGER,
        date         TEXT NOT NULL,
        genre        TEXT NOT NULL,
        duration_ms  INTEGER NOT NULL
    );
    "#,
    // v4: playback statistics for the Properties window (issue #34).
    // Times are Unix seconds.
    r#"
    CREATE TABLE play_stats (
        path         TEXT PRIMARY KEY,
        count        INTEGER NOT NULL,
        first_played INTEGER NOT NULL,
        last_played  INTEGER NOT NULL
    );
    "#,
];

/// The playlist every database starts with (migration v1).
pub const DEFAULT_PLAYLIST_ID: i64 = 1;

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open() -> rusqlite::Result<Self> {
        let path = db_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let conn = Connection::open(path)?;
        Self::from_connection(conn)
    }

    #[cfg(test)]
    pub(crate) fn open_in_memory() -> rusqlite::Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(conn: Connection) -> rusqlite::Result<Self> {
        conn.pragma_update(None, "foreign_keys", true)?;
        run_migrations(&conn)?;
        Ok(Self { conn })
    }

    // --- artist info cache ------------------------------------------------

    /// A cached lookup: `Some((data, photo))` if `key` was looked up before
    /// (`data` is `None` when nothing was found), `None` if never.
    pub fn artist_info(&self, key: &str) -> Option<(Option<String>, Option<Vec<u8>>)> {
        self.conn
            .query_row(
                "SELECT data, photo FROM artist_info WHERE key = ?1",
                params![key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .unwrap_or(None)
    }

    pub fn save_artist_info(&self, key: &str, data: Option<&str>, photo: Option<&[u8]>) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        if let Err(e) = self.conn.execute(
            "INSERT OR REPLACE INTO artist_info (key, data, photo, fetched_at) VALUES (?1, ?2, ?3, ?4)",
            params![key, data, photo, now],
        ) {
            tracing::warn!("failed to cache artist info: {e}");
        }
    }

    // --- media library ---------------------------------------------------

    pub fn library_tracks(&self) -> Vec<LibraryTrack> {
        let result = (|| -> rusqlite::Result<Vec<LibraryTrack>> {
            let mut stmt = self.conn.prepare(
                "SELECT path, modified, size, title, artist, album_artist, album, track, disc,
                        date, genre, duration_ms
                 FROM library_tracks",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(LibraryTrack {
                    path: PathBuf::from(row.get::<_, String>(0)?),
                    modified: row.get(1)?,
                    size: row.get::<_, i64>(2)? as u64,
                    tags: engine::LibraryTags {
                        title: row.get(3)?,
                        artist: row.get(4)?,
                        album_artist: row.get(5)?,
                        album: row.get(6)?,
                        track_number: row.get(7)?,
                        disc_number: row.get(8)?,
                        date: row.get(9)?,
                        genre: row.get(10)?,
                        duration: std::time::Duration::from_millis(row.get::<_, i64>(11)?.max(0) as u64),
                    },
                })
            })?;
            rows.collect()
        })();
        result.unwrap_or_else(|e| {
            tracing::warn!("failed to load the media library: {e}");
            Vec::new()
        })
    }

    /// Adds or updates `upserts` and drops `removed`, in one transaction.
    pub fn update_library(&self, upserts: &[LibraryTrack], removed: &[PathBuf]) {
        let result = (|| -> rusqlite::Result<()> {
            let tx = self.conn.unchecked_transaction()?;
            {
                let mut insert = tx.prepare(
                    "INSERT OR REPLACE INTO library_tracks (path, modified, size, title, artist,
                        album_artist, album, track, disc, date, genre, duration_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                )?;
                for track in upserts {
                    let tags = &track.tags;
                    insert.execute(params![
                        track.path.to_string_lossy(),
                        track.modified,
                        track.size as i64,
                        tags.title,
                        tags.artist,
                        tags.album_artist,
                        tags.album,
                        tags.track_number,
                        tags.disc_number,
                        tags.date,
                        tags.genre,
                        tags.duration.as_millis() as i64,
                    ])?;
                }
                let mut delete = tx.prepare("DELETE FROM library_tracks WHERE path = ?1")?;
                for path in removed {
                    delete.execute(params![path.to_string_lossy()])?;
                }
            }
            tx.commit()
        })();
        if let Err(e) = result {
            tracing::warn!("failed to update the media library: {e}");
        }
    }

    // --- settings: flat key/value --------------------------------------

    pub fn get_setting(&self, key: &str) -> Option<String> {
        self.conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
            .unwrap_or(None)
    }

    /// Upserts every pair in one transaction.
    pub fn set_settings(&self, pairs: &[(&str, String)]) {
        let result = (|| -> rusqlite::Result<()> {
            let tx = self.conn.unchecked_transaction()?;
            for (key, value) in pairs {
                tx.execute(
                    "INSERT INTO settings (key, value) VALUES (?1, ?2)
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    params![key, value],
                )?;
            }
            tx.commit()
        })();
        if let Err(e) = result {
            tracing::warn!("failed to save settings: {e}");
        }
    }

    // --- themes: one row per custom theme -------------------------------

    pub fn load_custom_themes(&self) -> Vec<ThemeEntry> {
        let result = (|| -> rusqlite::Result<Vec<ThemeEntry>> {
            let mut stmt = self.conn.prepare(
                "SELECT id, label, bg_base, bg_raised, bg_sunken, text_primary, text_muted, \
                        accent, accent_text, waveform_unplayed, spectrum_bar \
                 FROM themes ORDER BY rowid",
            )?;
            let rows = stmt.query_map([], |row| {
                let hex = |i: usize| -> rusqlite::Result<String> { row.get(i) };
                Ok(ThemeEntry {
                    id: row.get(0)?,
                    label: row.get(1)?,
                    colors: ThemeColors {
                        bg_base: parse_or_black(&hex(2)?),
                        bg_raised: parse_or_black(&hex(3)?),
                        bg_sunken: parse_or_black(&hex(4)?),
                        text_primary: parse_or_black(&hex(5)?),
                        text_muted: parse_or_black(&hex(6)?),
                        accent: parse_or_black(&hex(7)?),
                        accent_text: parse_or_black(&hex(8)?),
                        waveform_unplayed: parse_or_black(&hex(9)?),
                        spectrum_bar: parse_or_black(&hex(10)?),
                    },
                    is_custom: true,
                })
            })?;
            rows.collect()
        })();
        result.unwrap_or_else(|e| {
            tracing::warn!("failed to load custom themes: {e}");
            Vec::new()
        })
    }

    /// Replaces every custom theme row with `entries` (builtins are never
    /// passed in -- see [`crate::theme::ThemeRegistry`]).
    pub fn save_custom_themes(&self, entries: &[ThemeEntry]) {
        use crate::theme::color_to_hex;
        let result = (|| -> rusqlite::Result<()> {
            let tx = self.conn.unchecked_transaction()?;
            tx.execute("DELETE FROM themes", [])?;
            for entry in entries {
                let c = &entry.colors;
                tx.execute(
                    "INSERT INTO themes (id, label, bg_base, bg_raised, bg_sunken, text_primary, \
                                          text_muted, accent, accent_text, waveform_unplayed, spectrum_bar) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    params![
                        entry.id,
                        entry.label,
                        color_to_hex(c.bg_base),
                        color_to_hex(c.bg_raised),
                        color_to_hex(c.bg_sunken),
                        color_to_hex(c.text_primary),
                        color_to_hex(c.text_muted),
                        color_to_hex(c.accent),
                        color_to_hex(c.accent_text),
                        color_to_hex(c.waveform_unplayed),
                        color_to_hex(c.spectrum_bar),
                    ],
                )?;
            }
            tx.commit()
        })();
        if let Err(e) = result {
            tracing::warn!("failed to save custom themes: {e}");
        }
    }

    // --- playback statistics (issue #34) --------------------------------

    /// Counts a play of `path` at `when` (Unix seconds).
    pub fn record_play(&self, path: &Path, when: i64) {
        if let Err(e) = self.conn.execute(
            "INSERT INTO play_stats (path, count, first_played, last_played) VALUES (?1, 1, ?2, ?2)
             ON CONFLICT(path) DO UPDATE SET count = count + 1, last_played = excluded.last_played",
            params![path.to_string_lossy(), when],
        ) {
            tracing::warn!("failed to record a play: {e}");
        }
    }

    pub fn play_stats(&self, path: &Path) -> Option<PlayStats> {
        self.conn
            .query_row(
                "SELECT count, first_played, last_played FROM play_stats WHERE path = ?1",
                params![path.to_string_lossy()],
                |row| Ok(PlayStats { count: row.get::<_, i64>(0)? as u64, first_played: row.get(1)?, last_played: row.get(2)? }),
            )
            .optional()
            .unwrap_or(None)
    }

    // --- playlists: named, ordered lists of paths (issue #33) ----------

    /// Every playlist, oldest first, with how many tracks it holds.
    pub fn playlists(&self) -> Vec<PlaylistInfo> {
        let result = (|| -> rusqlite::Result<Vec<PlaylistInfo>> {
            let mut stmt = self.conn.prepare(
                "SELECT p.id, p.name, COUNT(t.path) FROM playlists p
                 LEFT JOIN playlist_tracks t ON t.playlist_id = p.id
                 GROUP BY p.id ORDER BY p.id",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(PlaylistInfo {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    count: row.get::<_, i64>(2)? as usize,
                })
            })?;
            rows.collect()
        })();
        let mut playlists = result.unwrap_or_else(|e| {
            tracing::warn!("failed to list playlists: {e}");
            Vec::new()
        });
        // There's always at least one.
        if playlists.is_empty() {
            let id = self.create_playlist("Default", &[]);
            playlists.push(PlaylistInfo { id, name: "Default".into(), count: 0 });
        }
        playlists
    }

    /// A new playlist holding `paths`; returns its id.
    pub fn create_playlist(&self, name: &str, paths: &[PathBuf]) -> i64 {
        if let Err(e) = self.conn.execute("INSERT INTO playlists (name) VALUES (?1)", params![name]) {
            tracing::warn!("failed to create playlist: {e}");
            return DEFAULT_PLAYLIST_ID;
        }
        let id = self.conn.last_insert_rowid();
        self.save_playlist(id, paths);
        id
    }

    pub fn rename_playlist(&self, id: i64, name: &str) {
        if let Err(e) = self.conn.execute("UPDATE playlists SET name = ?1 WHERE id = ?2", params![name, id]) {
            tracing::warn!("failed to rename playlist: {e}");
        }
    }

    /// Deletes the playlist and its tracks (`ON DELETE CASCADE`).
    pub fn delete_playlist(&self, id: i64) {
        if let Err(e) = self.conn.execute("DELETE FROM playlists WHERE id = ?1", params![id]) {
            tracing::warn!("failed to delete playlist: {e}");
        }
    }

    /// The playlist's paths, in order -- missing files included only when
    /// `keep_missing` (dropping them is right for playback, not for
    /// copying a playlist).
    pub fn playlist_paths(&self, id: i64, keep_missing: bool) -> Vec<PathBuf> {
        let result = (|| -> rusqlite::Result<Vec<PathBuf>> {
            let mut stmt = self.conn.prepare(
                "SELECT path FROM playlist_tracks WHERE playlist_id = ?1 ORDER BY position",
            )?;
            let rows = stmt.query_map(params![id], |row| row.get::<_, String>(0))?;
            rows.map(|r| r.map(PathBuf::from)).collect()
        })();
        let paths = result.unwrap_or_else(|e| {
            tracing::warn!("failed to load playlist: {e}");
            Vec::new()
        });
        // Drop entries for files that have since moved or been deleted
        // rather than handing the engine paths it can't open.
        if keep_missing {
            paths
        } else {
            paths.into_iter().filter(|p| p.exists()).collect()
        }
    }

    pub fn save_playlist(&self, id: i64, paths: &[PathBuf]) {
        let result = (|| -> rusqlite::Result<()> {
            let tx = self.conn.unchecked_transaction()?;
            tx.execute("DELETE FROM playlist_tracks WHERE playlist_id = ?1", params![id])?;
            {
                let mut insert = tx.prepare(
                    "INSERT INTO playlist_tracks (playlist_id, position, path) VALUES (?1, ?2, ?3)",
                )?;
                for (position, path) in paths.iter().enumerate() {
                    insert.execute(params![id, position as i64, path.to_string_lossy()])?;
                }
            }
            tx.commit()
        })();
        if let Err(e) = result {
            tracing::warn!("failed to save playlist: {e}");
        }
    }
}

/// How often and when a file was played (times are Unix seconds).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayStats {
    pub count: u64,
    pub first_played: i64,
    pub last_played: i64,
}

/// A playlist as the Playlist Manager lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaylistInfo {
    pub id: i64,
    pub name: String,
    pub count: usize,
}

fn parse_or_black(hex: &str) -> slint::Color {
    crate::theme::parse_hex_color(hex).unwrap_or(slint::Color::from_rgb_u8(0, 0, 0))
}

fn run_migrations(conn: &Connection) -> rusqlite::Result<()> {
    let current_version: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    for (i, migration) in MIGRATIONS.iter().enumerate() {
        let version = i as i32 + 1;
        if version <= current_version {
            continue;
        }
        conn.execute_batch(migration)?;
        conn.pragma_update(None, "user_version", version)?;
    }
    Ok(())
}

fn db_path() -> PathBuf {
    db_path_in(&crate::settings::config_dir())
}

fn db_path_in(dir: &Path) -> PathBuf {
    dir.join("openbar5000.db")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::parse_hex_color;

    #[test]
    fn settings_round_trip() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.get_setting("theme_id"), None);

        store.set_settings(&[
            ("theme_id", "dark".to_string()),
            ("show_waveform", "true".to_string()),
        ]);
        assert_eq!(store.get_setting("theme_id"), Some("dark".to_string()));
        assert_eq!(store.get_setting("show_waveform"), Some("true".to_string()));

        // Re-saving the same key updates it in place rather than erroring
        // or duplicating a row.
        store.set_settings(&[("theme_id", "light".to_string())]);
        assert_eq!(store.get_setting("theme_id"), Some("light".to_string()));
    }

    #[test]
    fn custom_themes_round_trip_and_full_replace() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.load_custom_themes().is_empty());

        let colors = ThemeColors {
            bg_base: parse_hex_color("#111111").unwrap(),
            bg_raised: parse_hex_color("#222222").unwrap(),
            bg_sunken: parse_hex_color("#000000").unwrap(),
            text_primary: parse_hex_color("#eeeeee").unwrap(),
            text_muted: parse_hex_color("#999999").unwrap(),
            accent: parse_hex_color("#c97a26").unwrap(),
            accent_text: parse_hex_color("#111111").unwrap(),
            waveform_unplayed: parse_hex_color("#4a4a4a").unwrap(),
            spectrum_bar: parse_hex_color("#3fb8e8").unwrap(),
        };
        let entry = ThemeEntry {
            id: "custom:ocean-deep".into(),
            label: "Ocean Deep".into(),
            colors,
            is_custom: true,
        };
        store.save_custom_themes(std::slice::from_ref(&entry));

        let loaded = store.load_custom_themes();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "custom:ocean-deep");
        assert_eq!(loaded[0].label, "Ocean Deep");
        assert_eq!(loaded[0].colors.accent, colors.accent);

        // Saving again with a shorter list fully replaces the table rather
        // than merging -- mirrors "every custom theme currently in memory"
        // semantics the registry relies on.
        store.save_custom_themes(&[]);
        assert!(store.load_custom_themes().is_empty());
    }

    #[test]
    fn playlist_round_trip_preserves_order_and_drops_missing_files() {
        let store = Store::open_in_memory().unwrap();
        let dir = std::env::temp_dir().join(format!("openbar5000-store-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.wav");
        let b = dir.join("b.wav");
        std::fs::write(&a, b"").unwrap();
        std::fs::write(&b, b"").unwrap();
        let missing = dir.join("missing.wav");

        store.save_playlist(DEFAULT_PLAYLIST_ID, &[b.clone(), missing.clone(), a.clone()]);
        let loaded = store.playlist_paths(DEFAULT_PLAYLIST_ID, false);
        let all = store.playlist_paths(DEFAULT_PLAYLIST_ID, true);

        std::fs::remove_dir_all(&dir).unwrap();

        assert_eq!(loaded, vec![b.clone(), a.clone()]);
        assert_eq!(all, vec![b, missing, a]);
    }

    #[test]
    fn plays_are_counted_with_first_and_last_times() {
        let store = Store::open_in_memory().unwrap();
        let song = Path::new("/m/song.flac");
        assert_eq!(store.play_stats(song), None);
        store.record_play(song, 100);
        store.record_play(song, 250);
        assert_eq!(store.play_stats(song), Some(PlayStats { count: 2, first_played: 100, last_played: 250 }));
    }

    #[test]
    fn playlists_can_be_created_renamed_and_deleted() {
        let store = Store::open_in_memory().unwrap();
        // Every database starts with the Default playlist.
        assert_eq!(store.playlists(), vec![PlaylistInfo { id: DEFAULT_PLAYLIST_ID, name: "Default".into(), count: 0 }]);

        let chill = store.create_playlist("Chill", &[PathBuf::from("/m/a.flac"), PathBuf::from("/m/b.flac")]);
        store.rename_playlist(chill, "Chill Evening");
        let listed = store.playlists();
        assert_eq!(listed.len(), 2);
        assert_eq!((listed[1].name.as_str(), listed[1].count), ("Chill Evening", 2));

        // Deleting takes its tracks with it; deleting the last one leaves a
        // fresh Default behind.
        store.delete_playlist(chill);
        store.delete_playlist(DEFAULT_PLAYLIST_ID);
        assert!(store.playlist_paths(chill, true).is_empty());
        let listed = store.playlists();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "Default");
    }

    #[test]
    fn migrations_are_idempotent_across_reopen() {
        let dir = std::env::temp_dir().join(format!(
            "openbar5000-store-migrate-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = db_path_in(&dir);

        {
            let conn = Connection::open(&path).unwrap();
            Store::from_connection(conn).unwrap();
        }
        // Reopening an already-migrated database must not re-run (and fail
        // on, e.g. duplicate `CREATE TABLE`) any migration.
        let conn = Connection::open(&path).unwrap();
        let store = Store::from_connection(conn).unwrap();
        store.set_settings(&[("theme_id", "dark".to_string())]);
        assert_eq!(store.get_setting("theme_id"), Some("dark".to_string()));

        // Closed first: Windows won't delete a file that's open.
        drop(store);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn artist_info_cache_round_trips_hits_and_misses() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.artist_info("armin|en"), None);
        store.save_artist_info("armin|en", Some("{\"name\":\"Armin\"}"), Some(&[1, 2, 3]));
        store.save_artist_info("nobody|en", None, None);
        assert_eq!(
            store.artist_info("armin|en"),
            Some((Some("{\"name\":\"Armin\"}".to_string()), Some(vec![1, 2, 3])))
        );
        assert_eq!(store.artist_info("nobody|en"), Some((None, None)));
    }
}
