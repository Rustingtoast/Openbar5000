//! The Properties window (issue #34): edit the tags of one or more tracks,
//! add/replace/remove their pictures, and see the first one's file details
//! and playback statistics. Fields the selected tracks disagree on show
//! "‹multiple values›" and are only written if edited. Saving runs in the
//! background (see `engine::edit_tags`), then the playlist re-reads its
//! tags.

use crate::store::Store;
use crossbeam_channel::{Receiver, Sender};
use engine::{EngineCommand, PictureKind, PlaybackEngine, TagEdit, TagField};
use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;
use ui::{cover_image_from_rgba, format_duration, ArtworkRow, DetailRow, PropertiesWindow, PropertyField};

/// "57.4 MB (60 283 134 bytes)".
fn format_size(bytes: u64) -> String {
    let grouped = bytes
        .to_string()
        .as_bytes()
        .rchunks(3)
        .rev()
        .map(|chunk| std::str::from_utf8(chunk).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\u{2009}");
    let size = if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    };
    format!("{size} ({grouped} bytes)")
}

/// "2023-02-21 20:06:46", local time.
fn format_time(unix_seconds: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_opt(unix_seconds, 0)
        .single()
        .map(|time| time.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_default()
}

/// "7:37.486".
fn format_precise(duration: Duration) -> String {
    format!("{}.{:03}", format_duration(duration), duration.subsec_millis())
}

/// The value shown for a field across `values`: the shared one, or `None`
/// when they differ.
fn common_value(values: &[String]) -> Option<String> {
    let first = values.first()?;
    values.iter().all(|v| v == first).then(|| first.clone())
}

/// A finished save: per file, what went wrong (if anything).
type SaveOutcome = Vec<(PathBuf, Result<(), String>)>;

pub struct PropertiesEditor {
    window: PropertiesWindow,
    engine: Rc<PlaybackEngine>,
    store: Rc<Store>,
    paths: RefCell<Vec<PathBuf>>,
    /// Per field, the shared value when it opened (`None`: they differ).
    original: RefCell<Vec<(TagField, Option<String>)>>,
    edits: RefCell<HashMap<TagField, String>>,
    /// Pictures to set (`Some`) or remove (`None`), by kind.
    picture_edits: RefCell<Vec<(PictureKind, Option<Vec<u8>>)>>,
    artwork_model: Rc<VecModel<ArtworkRow>>,
    saves: (Sender<SaveOutcome>, Receiver<SaveOutcome>),
    close_after_save: std::cell::Cell<bool>,
    poll: Timer,
}

impl PropertiesEditor {
    pub fn wire(window: &PropertiesWindow, engine: Rc<PlaybackEngine>, store: Rc<Store>) -> Rc<Self> {
        let editor = Rc::new(Self {
            window: window.clone_strong(),
            engine,
            store,
            paths: RefCell::new(Vec::new()),
            original: RefCell::new(Vec::new()),
            edits: RefCell::new(HashMap::new()),
            picture_edits: RefCell::new(Vec::new()),
            artwork_model: Rc::new(VecModel::default()),
            saves: crossbeam_channel::unbounded(),
            close_after_save: std::cell::Cell::new(false),
            poll: Timer::default(),
        });
        window.set_artwork(ModelRc::from(editor.artwork_model.clone()));

        let weak = Rc::downgrade(&editor);
        window.on_field_edited(move |index, text| {
            if let Some(editor) = weak.upgrade() {
                editor.field_edited(index, &text);
            }
        });
        let weak = Rc::downgrade(&editor);
        window.on_artwork_add(move |index| {
            if let Some(editor) = weak.upgrade() {
                editor.add_picture(index);
            }
        });
        let weak = Rc::downgrade(&editor);
        window.on_artwork_remove(move |index| {
            if let Some(editor) = weak.upgrade() {
                editor.remove_picture(index);
            }
        });
        let weak = Rc::downgrade(&editor);
        window.on_apply_clicked(move || {
            if let Some(editor) = weak.upgrade() {
                editor.save(false);
            }
        });
        let weak = Rc::downgrade(&editor);
        window.on_ok_clicked(move || {
            if let Some(editor) = weak.upgrade() {
                editor.save(true);
            }
        });
        let weak = Rc::downgrade(&editor);
        window.on_cancel_clicked(move || {
            if let Some(editor) = weak.upgrade() {
                let _ = editor.window.hide();
            }
        });
        editor
    }

    /// Opens the window for `tracks` (paths and titles).
    pub fn open(&self, tracks: Vec<(PathBuf, String)>) {
        let Some((first, first_title)) = tracks.first().cloned() else { return };
        let paths: Vec<PathBuf> = tracks.into_iter().map(|(path, _)| path).collect();
        self.window.set_subject(
            if paths.len() == 1 { first_title } else { format!("{} tracks", paths.len()) }.into(),
        );

        // Metadata: the value every track shares, or "multiple".
        let per_file: Vec<Vec<(TagField, String)>> =
            paths.iter().filter_map(|path| engine::read_tag_fields(path).ok()).collect();
        let original: Vec<(TagField, Option<String>)> = TagField::ALL
            .iter()
            .map(|&field| {
                let values: Vec<String> = per_file
                    .iter()
                    .map(|fields| fields.iter().find(|(f, _)| *f == field).map(|(_, v)| v.clone()).unwrap_or_default())
                    .collect();
                (field, common_value(&values))
            })
            .collect();
        self.window.set_fields(ModelRc::from(Rc::new(VecModel::from(
            original
                .iter()
                .map(|(field, value)| PropertyField {
                    label: field.label().into(),
                    value: value.clone().unwrap_or_default().into(),
                    multiple: value.is_none(),
                })
                .collect::<Vec<_>>(),
        ))));
        *self.original.borrow_mut() = original;
        self.edits.borrow_mut().clear();
        self.picture_edits.borrow_mut().clear();
        *self.paths.borrow_mut() = paths.clone();

        self.show_details(&first, paths.len());
        self.show_pictures(&first);
        let status = if per_file.len() < paths.len() {
            "Some files' tags couldn't be read.".to_string()
        } else if paths.len() > 1 {
            format!("Editing {} tracks; details and artwork show the first.", paths.len())
        } else {
            String::new()
        };
        self.window.set_status(status.into());
        self.window.set_dirty(false);
        self.window.set_busy(false);
        let _ = self.window.show();
    }

    fn show_details(&self, path: &std::path::Path, count: usize) {
        let details = engine::read_details(path);
        let row = |label: &str, value: String| DetailRow { label: label.into(), value: value.into(), is_section: false };
        let section = |label: &str| DetailRow { label: label.into(), value: SharedString::new(), is_section: true };
        let mut rows = vec![
            section(if count > 1 { "Location (first track)" } else { "Location" }),
            row("File name", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
            row("Folder name", path.parent().map(|p| p.display().to_string()).unwrap_or_default()),
            row("File path", path.display().to_string()),
            row("File size", format_size(details.size)),
            row(
                "Last modified",
                details
                    .modified
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| format_time(d.as_secs() as i64))
                    .unwrap_or_default(),
            ),
            section("General"),
            row(
                "Duration",
                match details.samples() {
                    Some(samples) => format!("{} ({samples} samples)", format_precise(details.duration)),
                    None => format_precise(details.duration),
                },
            ),
        ];
        let optional = |value: Option<String>| value.unwrap_or_else(|| "\u{2014}".into());
        rows.push(row("Sample rate", optional(details.sample_rate.map(|r| format!("{r} Hz")))));
        rows.push(row("Channels", optional(details.channels.map(|c| c.to_string()))));
        rows.push(row("Bits per sample", optional(details.bits_per_sample.map(|b| b.to_string()))));
        rows.push(row("Bitrate", optional(details.bitrate_kbps.map(|b| format!("{b} kbps")))));
        rows.push(row("Codec", details.codec.clone()));
        rows.push(row("Encoding", if details.lossless { "lossless" } else { "lossy" }.into()));
        rows.push(row("Tool", if details.tool.is_empty() { "\u{2014}".into() } else { details.tool.clone() }));
        rows.push(section("Playback Statistics"));
        match self.store.play_stats(path) {
            Some(stats) => {
                rows.push(row("Played", format!("{} time{}", stats.count, if stats.count == 1 { "" } else { "s" })));
                rows.push(row("First played", format_time(stats.first_played)));
                rows.push(row("Last played", format_time(stats.last_played)));
            }
            None => rows.push(row("Played", "never".into())),
        }
        self.window.set_details(ModelRc::from(Rc::new(VecModel::from(rows))));
    }

    fn show_pictures(&self, path: &std::path::Path) {
        let pictures = engine::read_pictures(path);
        let rows: Vec<ArtworkRow> = PictureKind::ALL
            .iter()
            .map(|&kind| match pictures.iter().find(|p| p.kind == kind) {
                Some(info) => ArtworkRow {
                    label: kind.label().into(),
                    present: true,
                    picture: info
                        .thumbnail
                        .as_ref()
                        .map(|t| cover_image_from_rgba(t.width, t.height, &t.rgba))
                        .unwrap_or_default(),
                    info: picture_info(info.bytes, info.width, info.height, &info.format).into(),
                },
                None => absent(kind),
            })
            .collect();
        self.artwork_model.set_vec(rows);
    }

    fn field_edited(&self, index: i32, text: &str) {
        let Some((field, original)) = usize::try_from(index).ok().and_then(|i| self.original.borrow().get(i).cloned())
        else {
            return;
        };
        let mut edits = self.edits.borrow_mut();
        // Back to what it was (a "multiple" field left empty included):
        // nothing to write.
        if original.as_deref() == Some(text) || (original.is_none() && text.is_empty()) {
            edits.remove(&field);
        } else {
            edits.insert(field, text.to_string());
        }
        drop(edits);
        self.update_dirty();
    }

    fn update_dirty(&self) {
        self.window
            .set_dirty(!self.edits.borrow().is_empty() || !self.picture_edits.borrow().is_empty());
    }

    fn set_picture_edit(&self, kind: PictureKind, data: Option<Vec<u8>>) {
        let mut edits = self.picture_edits.borrow_mut();
        edits.retain(|(k, _)| *k != kind);
        edits.push((kind, data));
    }

    fn add_picture(&self, index: i32) {
        let Some(&kind) = usize::try_from(index).ok().and_then(|i| PictureKind::ALL.get(i)) else { return };
        let Some(file) = rfd::FileDialog::new()
            .set_title(format!("Choose the {} picture", kind.label().to_lowercase()))
            .add_filter("Images", &["jpg", "jpeg", "png", "gif", "bmp", "webp"])
            .pick_file()
        else {
            return;
        };
        let data = match std::fs::read(&file) {
            Ok(data) => data,
            Err(e) => {
                self.window.set_status(format!("Couldn\u{2019}t read the picture: {e}").into());
                return;
            }
        };
        let decoded = match image::load_from_memory(&data) {
            Ok(decoded) => decoded,
            Err(_) => {
                self.window.set_status("That file isn\u{2019}t a picture Openbar5000 can read.".into());
                return;
            }
        };
        let thumbnail = decoded.thumbnail(160, 160).into_rgba8();
        let format = image::guess_format(&data).map(|f| format!("{f:?}").to_uppercase()).unwrap_or_default();
        self.artwork_model.set_row_data(
            index as usize,
            ArtworkRow {
                label: kind.label().into(),
                present: true,
                picture: cover_image_from_rgba(thumbnail.width(), thumbnail.height(), thumbnail.as_raw()),
                info: format!("{} (not saved yet)", picture_info(data.len(), decoded.width(), decoded.height(), &format)).into(),
            },
        );
        self.set_picture_edit(kind, Some(data));
        self.window.set_status(SharedString::new());
        self.update_dirty();
    }

    fn remove_picture(&self, index: i32) {
        let Some(&kind) = usize::try_from(index).ok().and_then(|i| PictureKind::ALL.get(i)) else { return };
        self.artwork_model.set_row_data(index as usize, absent(kind));
        self.set_picture_edit(kind, None);
        self.update_dirty();
    }

    fn save(self: &Rc<Self>, close_after: bool) {
        let edit = TagEdit {
            fields: self.edits.borrow().iter().map(|(f, v)| (*f, v.clone())).collect(),
            pictures: self.picture_edits.borrow().clone(),
        };
        if edit.is_empty() {
            if close_after {
                let _ = self.window.hide();
            }
            return;
        }
        let paths = self.paths.borrow().clone();
        let count = paths.len();
        self.window.set_busy(true);
        self.window.set_status(
            if count == 1 { "Saving\u{2026}".to_string() } else { format!("Saving {count} files\u{2026}") }.into(),
        );
        self.close_after_save.set(close_after);
        let tx = self.saves.0.clone();
        std::thread::Builder::new()
            .name("openbar5000-properties".into())
            .spawn(move || {
                let outcome = paths.into_iter().map(|path| {
                    let result = engine::edit_tags(&path, &edit);
                    (path, result)
                });
                let _ = tx.send(outcome.collect());
            })
            .expect("failed to spawn the tag writer");
        let weak = Rc::downgrade(self);
        self.poll.start(TimerMode::Repeated, Duration::from_millis(100), move || {
            if let Some(editor) = weak.upgrade() {
                editor.poll_save();
            }
        });
    }

    fn poll_save(&self) {
        let Ok(outcome) = self.saves.1.try_recv() else { return };
        self.poll.stop();
        self.window.set_busy(false);
        let failures: Vec<String> = outcome
            .iter()
            .filter_map(|(path, result)| {
                result.as_ref().err().map(|e| {
                    format!("{}: {e}", path.file_name().unwrap_or_default().to_string_lossy())
                })
            })
            .collect();
        // Show the new tags and covers in the playlist.
        self.engine.send(EngineCommand::RefreshMetadata);
        if failures.is_empty() {
            self.edits.borrow_mut().clear();
            self.picture_edits.borrow_mut().clear();
            self.update_dirty();
            if self.close_after_save.get() {
                let _ = self.window.hide();
            } else {
                self.window.set_status("Saved.".into());
                // Reload, so what's shown is what's in the files now.
                let first = self.paths.borrow().first().cloned();
                if let Some(first) = first {
                    self.show_pictures(&first);
                    self.show_details(&first, self.paths.borrow().len());
                }
            }
        } else {
            self.window.set_status(format!("Couldn\u{2019}t save {}", failures.join("; ")).into());
        }
    }
}

fn absent(kind: PictureKind) -> ArtworkRow {
    ArtworkRow {
        label: kind.label().into(),
        present: false,
        picture: slint::Image::default(),
        info: "\u{00ab}not present\u{00bb}".into(),
    }
}

/// "13.1 KB · 500×500 · JPEG".
fn picture_info(bytes: usize, width: u32, height: u32, format: &str) -> String {
    let size = if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    };
    format!("{size} \u{00b7} {width}\u{00d7}{height} \u{00b7} {format}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_values_format_like_the_reference() {
        assert_eq!(format_size(60_283_134), "57.5 MB (60\u{2009}283\u{2009}134 bytes)");
        assert_eq!(format_precise(Duration::from_millis(457_486)), "7:37.486");
        assert_eq!(picture_info(13_414, 500, 500, "JPEG"), "13.1 KB \u{00b7} 500\u{00d7}500 \u{00b7} JPEG");
        assert_eq!(common_value(&["a".into(), "a".into()]), Some("a".into()));
        assert_eq!(common_value(&["a".into(), "b".into()]), None);
    }
}
