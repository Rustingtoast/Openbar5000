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
];

/// The id of the one playlist this app currently exposes in the UI. The
/// schema already supports more (see `MIGRATIONS`); wiring up multiple
/// playlists is future UI work, not a storage change.
const DEFAULT_PLAYLIST_ID: i64 = 1;

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

    // --- playlist: ordered paths for the single default playlist --------

    pub fn load_playlist(&self) -> Vec<PathBuf> {
        let result = (|| -> rusqlite::Result<Vec<PathBuf>> {
            let mut stmt = self.conn.prepare(
                "SELECT path FROM playlist_tracks WHERE playlist_id = ?1 ORDER BY position",
            )?;
            let rows = stmt.query_map(params![DEFAULT_PLAYLIST_ID], |row| {
                row.get::<_, String>(0)
            })?;
            rows.map(|r| r.map(PathBuf::from)).collect()
        })();
        let paths = result.unwrap_or_else(|e| {
            tracing::warn!("failed to load playlist: {e}");
            Vec::new()
        });
        // Drop entries for files that have since moved or been deleted
        // rather than handing the engine paths it can't open.
        paths.into_iter().filter(|p| p.exists()).collect()
    }

    pub fn save_playlist(&self, paths: &[PathBuf]) {
        let result = (|| -> rusqlite::Result<()> {
            let tx = self.conn.unchecked_transaction()?;
            tx.execute(
                "DELETE FROM playlist_tracks WHERE playlist_id = ?1",
                params![DEFAULT_PLAYLIST_ID],
            )?;
            for (position, path) in paths.iter().enumerate() {
                tx.execute(
                    "INSERT INTO playlist_tracks (playlist_id, position, path) VALUES (?1, ?2, ?3)",
                    params![DEFAULT_PLAYLIST_ID, position as i64, path.to_string_lossy()],
                )?;
            }
            tx.commit()
        })();
        if let Err(e) = result {
            tracing::warn!("failed to save playlist: {e}");
        }
    }
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

        store.save_playlist(&[b.clone(), missing, a.clone()]);
        let loaded = store.load_playlist();

        std::fs::remove_dir_all(&dir).unwrap();

        assert_eq!(loaded, vec![b, a]);
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

        std::fs::remove_dir_all(&dir).unwrap();

        assert_eq!(store.get_setting("theme_id"), Some("dark".to_string()));
    }
}
