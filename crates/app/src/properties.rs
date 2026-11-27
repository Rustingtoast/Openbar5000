//! The Properties window (issue #34): edit the tags of one or more tracks,
//! add/replace/remove their pictures, and see the first one's file details
//! and playback statistics. Fields the selected tracks disagree on show
//! "‹multiple values›" and are only written if edited. Saving runs in the
//! background (see `engine::edit_tags`), then the playlist re-reads its
//! tags. The Vinyl tab picks the look of the song's record on the VNL panel
//! (see `crate::vinyl_style`), saved with the rest.

use crate::color_picker::ColorPicker;
use crate::store::Store;
use crate::theme::parse_hex_color;
use crate::vinyl_style::{self, Design, Label, VinylStyle, PRESETS};
use crossbeam_channel::{Receiver, Sender};
use engine::{EngineCommand, PictureKind, PlaybackEngine, TagEdit, TagField};
use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;
use ui::{cover_image_from_rgba, format_duration, ArtworkRow, DetailRow, PropertiesWindow, PropertyField, VinylSwatch};

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
/// Per file: what went wrong (if anything), and -- when the album or its
/// artist was edited -- the album its record's look is now kept for
/// (`playlist_rows::album_key_from`, from the tags written), worked out
/// with the save rather than by reading the files again on the UI thread.
type SaveOutcome = Vec<(PathBuf, Result<(), String>, Option<String>)>;

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
    /// The tracks open (path, the albums on its record as `vinyl_style`
    /// keeps them), and their record's look and whether it's the album's:
    /// as opened, and now.
    vinyl_tracks: RefCell<Vec<VinylTrack>>,
    vinyl_original: RefCell<(VinylStyle, bool)>,
    vinyl: RefCell<(VinylStyle, bool)>,
    /// Called after a new look is saved, to show it on the VNL panel.
    vinyl_saved: RefCell<Option<Box<dyn Fn()>>>,
    /// The first track's own album, whose look "Same vinyl for the whole
    /// album" shows.
    vinyl_album: RefCell<String>,
    /// The album's look, while a save renames the album (or its artist):
    /// kept for the new name too once the tags are written, or the record
    /// would lose it.
    renamed_look: RefCell<Option<VinylStyle>>,
    /// For the vinyl's and label's "any colour" chip.
    color_picker: Rc<ColorPicker>,
    /// The texture image being read in the background, counted so one
    /// read for tracks no longer open is dropped.
    texture_request: std::cell::Cell<i32>,
}

/// A track open in the window, for its record's look: its file, its own
/// album and all the albums on its record (see `PropertiesTrack`).
struct VinylTrack {
    path: PathBuf,
    album: String,
    record_albums: Vec<String>,
}

/// The Vinyl tab's place among the window's tabs.
const VINYL_TAB: i32 = 3;

/// Which of the record's colours a swatch or field sets.
#[derive(Clone, Copy)]
enum Part {
    Vinyl,
    Label,
}

/// One track for the Properties window: its file and title, and for its
/// record's look (`playlist_rows::vinyl_style_key`), its own album and all
/// the albums on its record -- more than one when smart grouping gathered
/// several taggings into one.
pub struct PropertiesTrack {
    pub path: PathBuf,
    pub title: String,
    pub vinyl_album: String,
    pub vinyl_record_albums: Vec<String>,
}

impl PropertiesEditor {
    pub fn wire(
        window: &PropertiesWindow,
        engine: Rc<PlaybackEngine>,
        store: Rc<Store>,
        color_picker: Rc<ColorPicker>,
    ) -> Rc<Self> {
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
            vinyl_tracks: RefCell::new(Vec::new()),
            vinyl_original: RefCell::new((VinylStyle::default(), true)),
            vinyl: RefCell::new((VinylStyle::default(), true)),
            vinyl_saved: RefCell::new(None),
            vinyl_album: RefCell::new(String::new()),
            renamed_look: RefCell::new(None),
            color_picker,
            texture_request: std::cell::Cell::new(0),
        });
        window.set_artwork(ModelRc::from(editor.artwork_model.clone()));

        let swatches: Vec<VinylSwatch> = PRESETS
            .iter()
            .filter_map(|(name, hex)| {
                Some(VinylSwatch { name: (*name).into(), hex: (*hex).into(), color: parse_hex_color(hex)? })
            })
            .collect();
        let (top, bottom) = swatches.split_at(swatches.len().div_ceil(2));
        window.set_vinyl_swatches_top(ModelRc::from(Rc::new(VecModel::from(top.to_vec()))));
        window.set_vinyl_swatches_bottom(ModelRc::from(Rc::new(VecModel::from(bottom.to_vec()))));
        window.set_vinyl_design_labels(ModelRc::from(Rc::new(VecModel::from(
            Design::ALL.iter().map(|design| SharedString::from(design.label())).collect::<Vec<_>>(),
        ))));

        let weak = Rc::downgrade(&editor);
        window.on_vinyl_synced_toggled(move |synced| {
            if let Some(editor) = weak.upgrade() {
                editor.vinyl_synced_toggled(synced);
            }
        });
        let weak = Rc::downgrade(&editor);
        window.on_vinyl_color_chosen(move |hex| {
            if let Some(editor) = weak.upgrade() {
                editor.vinyl_color_edited(Part::Vinyl, &hex, true);
            }
        });
        let weak = Rc::downgrade(&editor);
        window.on_vinyl_hex_edited(move |hex| {
            if let Some(editor) = weak.upgrade() {
                editor.vinyl_color_edited(Part::Vinyl, &hex, false);
            }
        });
        let weak = Rc::downgrade(&editor);
        window.on_vinyl_label_color_chosen(move |hex| {
            if let Some(editor) = weak.upgrade() {
                editor.vinyl_color_edited(Part::Label, &hex, true);
            }
        });
        let weak = Rc::downgrade(&editor);
        window.on_vinyl_label_hex_edited(move |hex| {
            if let Some(editor) = weak.upgrade() {
                editor.vinyl_color_edited(Part::Label, &hex, false);
            }
        });
        let weak = Rc::downgrade(&editor);
        window.on_vinyl_color_reset(move || {
            if let Some(editor) = weak.upgrade() {
                editor.edit_vinyl(|style| style.color = vinyl_style::default_color());
            }
        });
        let weak = Rc::downgrade(&editor);
        window.on_vinyl_label_reset(move || {
            if let Some(editor) = weak.upgrade() {
                editor.edit_vinyl(|style| style.set_label(Label::Accent));
            }
        });
        let weak = Rc::downgrade(&editor);
        window.on_vinyl_label_match_toggled(move |on| {
            if let Some(editor) = weak.upgrade() {
                editor.edit_vinyl(|style| {
                    if on {
                        style.match_vinyl();
                    } else {
                        style.set_label(Label::Accent);
                    }
                });
            }
        });
        let weak = Rc::downgrade(&editor);
        window.on_vinyl_label_clear_toggled(move |on| {
            if let Some(editor) = weak.upgrade() {
                let slider = editor.window.get_vinyl_label_clarity();
                editor.edit_vinyl(|style| {
                    // Matching see-through vinyl: as see-through as that.
                    let clarity = match style.label {
                        Label::Match => style.vinyl_clarity().unwrap_or(slider),
                        _ => slider,
                    };
                    style.label_clear = on.then_some(clarity);
                });
            }
        });
        let weak = Rc::downgrade(&editor);
        window.on_vinyl_label_clarity_changed(move |clarity| {
            if let Some(editor) = weak.upgrade() {
                editor.edit_vinyl(|style| style.label_clear = Some(clarity));
            }
        });
        let weak = Rc::downgrade(&editor);
        window.on_vinyl_clarity_changed(move |clarity| {
            if let Some(editor) = weak.upgrade() {
                editor.edit_vinyl(|style| style.edit_vinyl(|vinyl| vinyl.clarity = clarity));
            }
        });
        let weak = Rc::downgrade(&editor);
        window.on_vinyl_texture_choose_clicked(move || {
            if let Some(editor) = weak.upgrade() {
                editor.choose_texture();
            }
        });
        let weak = Rc::downgrade(&editor);
        window.on_vinyl_texture_imported(move |request, name, error| {
            if let Some(editor) = weak.upgrade() {
                editor.texture_imported(request, &name, &error);
            }
        });
        // Closed by its title bar's button: the colour picker with it.
        let weak = Rc::downgrade(&editor);
        window.window().on_close_requested(move || {
            if let Some(editor) = weak.upgrade() {
                editor.color_picker.close();
            }
            slint::CloseRequestResponse::HideWindow
        });
        let weak = Rc::downgrade(&editor);
        window.on_vinyl_pick_clicked(move || {
            if let Some(editor) = weak.upgrade() {
                editor.pick_vinyl_color(Part::Vinyl);
            }
        });
        let weak = Rc::downgrade(&editor);
        window.on_vinyl_label_pick_clicked(move || {
            if let Some(editor) = weak.upgrade() {
                editor.pick_vinyl_color(Part::Label);
            }
        });
        let weak = Rc::downgrade(&editor);
        window.on_vinyl_label_art_toggled(move |on| {
            if let Some(editor) = weak.upgrade() {
                editor.edit_vinyl(|style| style.label_art = on);
            }
        });
        let weak = Rc::downgrade(&editor);
        window.on_vinyl_design_chosen(move |index| {
            if let Some(editor) = weak.upgrade() {
                if let Some(&design) = usize::try_from(index).ok().and_then(|i| Design::ALL.get(i)) {
                    editor.vinyl_design_chosen(design);
                }
            }
        });

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
                editor.hide();
            }
        });
        editor
    }

    /// `hook` runs after a new record look is saved.
    pub fn on_vinyl_saved(&self, hook: impl Fn() + 'static) {
        *self.vinyl_saved.borrow_mut() = Some(Box::new(hook));
    }

    /// Opens the window for `tracks` on its Vinyl tab ("Customize
    /// Vinyl…").
    pub fn open_vinyl(&self, tracks: Vec<PropertiesTrack>) {
        self.open(tracks);
        self.window.set_current_tab(VINYL_TAB);
    }

    /// Whether the renderer can turn things, for the Vinyl tab's preview.
    pub fn set_vinyl_can_turn(&self, can_turn: bool) {
        self.window.set_vinyl_can_turn(can_turn);
    }

    /// Opens the window for `tracks`, on its first tab.
    pub fn open(&self, tracks: Vec<PropertiesTrack>) {
        // A colour being picked, or a texture being read, was for the
        // tracks open before.
        self.color_picker.close();
        self.texture_request
            .set(self.texture_request.get().wrapping_add(1));
        self.window.set_current_tab(0);
        let Some(first) = tracks.first() else { return };
        let (first, first_title) = (first.path.clone(), first.title.clone());
        // The record's look as the first track has it.
        let look = vinyl_style::style_for(&self.store, &first, &tracks[0].vinyl_album);
        *self.vinyl_original.borrow_mut() = look.clone();
        *self.vinyl.borrow_mut() = look;
        *self.vinyl_tracks.borrow_mut() = tracks
            .iter()
            .map(|t| VinylTrack {
                path: t.path.clone(),
                album: t.vinyl_album.clone(),
                record_albums: t.vinyl_record_albums.clone(),
            })
            .collect();
        *self.vinyl_album.borrow_mut() = tracks[0].vinyl_album.clone();
        self.window.set_vinyl_record_albums(tracks[0].vinyl_record_albums.len().max(1) as i32);
        self.show_vinyl();
        self.window.set_vinyl_synced(self.vinyl.borrow().1);
        let paths: Vec<PathBuf> = tracks.into_iter().map(|t| t.path).collect();
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
        self.window.set_vinyl_album(
            original
                .iter()
                .find(|(field, _)| *field == TagField::Album)
                .and_then(|(_, value)| value.clone())
                .unwrap_or_default()
                .into(),
        );
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
        // The record's label in the Vinyl tab's preview.
        let front = rows.first().filter(|row| row.present);
        self.window.set_vinyl_has_art(front.is_some());
        self.window.set_vinyl_art(front.map(|row| row.picture.clone()).unwrap_or_default());
        self.artwork_model.set_vec(rows);
    }

    /// Shows the record's look as chosen so far.
    fn show_vinyl(&self) {
        let (style, _) = self.vinyl.borrow().clone();
        self.window.set_vinyl_color(style.color);
        self.window.set_vinyl_contrast(style.contrast());
        self.window.set_vinyl_design(style.design.id().into());
        let index = Design::ALL.iter().position(|d| *d == style.design).unwrap_or_default();
        self.window.set_vinyl_design_index(index as i32);
        // Not while it's being typed: that would move the cursor.
        if parse_hex_color(&self.window.get_vinyl_hex()) != Some(style.color) {
            self.window.set_vinyl_hex(style.hex().into());
        }
        self.window.set_vinyl_color_changed(style.color != vinyl_style::default_color());
        self.window.set_vinyl_label_kind(style.label.kind().into());
        self.window.set_vinyl_label_match(style.label == Label::Match);
        self.window.set_vinyl_label_color(style.label.color().unwrap_or_default());
        match style.label.color() {
            Some(label) if parse_hex_color(&self.window.get_vinyl_label_hex()) != Some(label) => {
                self.window.set_vinyl_label_hex(crate::theme::color_to_hex(label).into());
            }
            Some(_) => {}
            // The accent, the vinyl's or clear: no colour of its own.
            None => self.window.set_vinyl_label_hex(SharedString::new()),
        }
        self.window.set_vinyl_label_art(style.label_art);
        self.window.set_vinyl_clarity(style.clarity);
        self.window.set_vinyl_texture(style.texture_image());
        self.window.set_vinyl_has_texture(style.texture.is_some());
        let custom = |color: slint::Color| {
            !PRESETS
                .iter()
                .any(|(_, hex)| parse_hex_color(hex) == Some(color))
        };
        self.window.set_vinyl_color_custom(custom(style.color));
        self.window
            .set_vinyl_label_custom(style.label.color().is_some_and(custom));
        self.window
            .set_vinyl_label_clear(style.label_clear.is_some());
        // Off, the slider keeps where it was, for when it's on again.
        if let Some(clarity) = style.label_clear {
            self.window.set_vinyl_label_clarity(clarity);
        }
    }

    /// Changes the look being chosen, and shows it.
    /// A design picked: each see-through one starting from its own
    /// default -- a texture solid, translucent half clear -- and a texture
    /// without an image asking for one.
    fn vinyl_design_chosen(&self, design: Design) {
        let before = self.vinyl.borrow().0.design;
        if design == before {
            return;
        }
        self.edit_vinyl(|style| {
            style.edit_vinyl(|vinyl| {
                vinyl.design = design;
                match design {
                    Design::Texture => vinyl.clarity = 0.0,
                    Design::Translucent => vinyl.clarity = vinyl_style::DEFAULT_CLARITY,
                    _ => {}
                }
            })
        });
        if design == Design::Texture && self.vinyl.borrow().0.texture.is_none() {
            self.choose_texture();
        }
    }

    /// "Choose Image…": an image of one's own for the texture design.
    fn choose_texture(&self) {
        let Some(file) = crate::platform::file_dialog_over(self.window.window())
            .set_title("Choose an image for the record")
            .add_filter("Images", &["jpg", "jpeg", "png"])
            .pick_file()
        else {
            return;
        };
        // Read in the background: a big photo takes a while to decode.
        let request = self.texture_request.get().wrapping_add(1);
        self.texture_request.set(request);
        self.window.set_status("Reading the image\u{2026}".into());
        let window = self.window.as_weak();
        crate::platform::spawn("openbar5000-texture", move || {
            let (name, error) = match vinyl_style::import_texture(&file) {
                Ok(name) => (name, String::new()),
                Err(e) => (String::new(), e),
            };
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(window) = window.upgrade() {
                    window.invoke_vinyl_texture_imported(request, name.into(), error.into());
                }
            });
        });
    }

    /// The image `choose_texture` read: the record's texture, if it's for
    /// the tracks still open.
    fn texture_imported(&self, request: i32, name: &str, error: &str) {
        if request != self.texture_request.get() {
            return;
        }
        if !error.is_empty() {
            self.window.set_status(error.into());
            return;
        }
        self.window.set_status(SharedString::new());
        let name = name.to_string();
        self.edit_vinyl(|style| {
            style.edit_vinyl(|vinyl| {
                vinyl.design = Design::Texture;
                vinyl.texture = Some(name);
            })
        });
    }

    fn hide(&self) {
        self.color_picker.close();
        let _ = self.window.hide();
    }

    /// The rainbow chip: the colour picker, on the vinyl's colour or the
    /// label's (the accent, unless it has its own).
    fn pick_vinyl_color(self: &Rc<Self>, part: Part) {
        let style = self.vinyl.borrow().0.clone();
        let (subject, color) = match part {
            Part::Vinyl => ("Vinyl color", style.color),
            Part::Label => (
                "Label color",
                style
                    .label
                    .color()
                    .unwrap_or_else(|| self.window.global::<ui::Theme>().get_accent()),
            ),
        };
        let weak = Rc::downgrade(self);
        self.color_picker.open(subject, color, move |color| {
            if let Some(editor) = weak.upgrade() {
                editor.vinyl_color_edited(part, &crate::theme::color_to_hex(color), true);
            }
        });
    }

    fn edit_vinyl(&self, change: impl FnOnce(&mut VinylStyle)) {
        change(&mut self.vinyl.borrow_mut().0);
        self.show_vinyl();
        self.update_dirty();
    }

    /// "Same vinyl for the whole album" ticked: the album's look is shown,
    /// so OK keeps it (unless it's changed here) rather than giving the
    /// whole album this song's own. Unticked: the song's own look again, if
    /// it had one -- otherwise the album's, to start from.
    fn vinyl_synced_toggled(&self, synced: bool) {
        let original = self.vinyl_original.borrow().clone();
        let look = if synced {
            vinyl_style::album_look(&self.store, &self.vinyl_album.borrow())
        } else if !original.1 {
            original.0
        } else {
            self.vinyl.borrow().0.clone()
        };
        *self.vinyl.borrow_mut() = (look, synced);
        self.show_vinyl();
        self.update_dirty();
    }

    /// A swatch picked (`swatch`), or the custom color typed: taken once
    /// it's a whole "#rrggbb".
    fn vinyl_color_edited(&self, part: Part, hex: &str, swatch: bool) {
        let Some(color) = parse_hex_color(hex) else { return };
        if swatch {
            match part {
                Part::Vinyl => self.window.set_vinyl_hex(hex.into()),
                Part::Label => self.window.set_vinyl_label_hex(hex.into()),
            }
        }
        self.edit_vinyl(|style| match part {
            Part::Vinyl => style.color = color,
            Part::Label => style.set_label(Label::Color(color)),
        });
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
        self.window.set_dirty(
            !self.edits.borrow().is_empty() || !self.picture_edits.borrow().is_empty() || self.vinyl_changed(),
        );
    }

    fn vinyl_changed(&self) -> bool {
        *self.vinyl.borrow() != *self.vinyl_original.borrow()
    }

    /// Stores the record's new look (no files are written for it).
    fn save_vinyl(&self) {
        if !self.vinyl_changed() {
            return;
        }
        let (style, synced) = self.vinyl.borrow().clone();
        let tracks: Vec<(PathBuf, Vec<String>)> =
            self.vinyl_tracks.borrow().iter().map(|t| (t.path.clone(), t.record_albums.clone())).collect();
        vinyl_style::save(&self.store, &tracks, &style, synced);
        *self.vinyl_original.borrow_mut() = (style, synced);
        if let Some(hook) = self.vinyl_saved.borrow().as_ref() {
            hook();
        }
    }

    fn set_picture_edit(&self, kind: PictureKind, data: Option<Vec<u8>>) {
        let mut edits = self.picture_edits.borrow_mut();
        edits.retain(|(k, _)| *k != kind);
        edits.push((kind, data));
    }

    fn add_picture(&self, index: i32) {
        let Some(&kind) = usize::try_from(index).ok().and_then(|i| PictureKind::ALL.get(i)) else { return };
        let Some(file) = crate::platform::file_dialog_over(self.window.window())
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
        let vinyl_changed = self.vinyl_changed();
        self.save_vinyl();
        let edit = TagEdit {
            fields: self.edits.borrow().iter().map(|(f, v)| (*f, v.clone())).collect(),
            pictures: self.picture_edits.borrow().clone(),
        };
        if edit.is_empty() {
            self.update_dirty();
            if close_after {
                self.hide();
            } else if vinyl_changed {
                self.window.set_status("Saved.".into());
            }
            return;
        }
        // Renaming the album: its look goes along (see `poll_save`).
        let renames_album = edit
            .fields
            .iter()
            .any(|(field, _)| matches!(field, TagField::Album | TagField::AlbumArtist | TagField::Artist));
        *self.renamed_look.borrow_mut() =
            (renames_album && self.vinyl.borrow().1).then(|| self.vinyl.borrow().0.clone());
        let paths = self.paths.borrow().clone();
        let count = paths.len();
        self.window.set_busy(true);
        self.window.set_status(
            if count == 1 { "Saving\u{2026}".to_string() } else { format!("Saving {count} files\u{2026}") }.into(),
        );
        self.close_after_save.set(close_after);
        let tx = self.saves.0.clone();
        let started = crate::platform::spawn("openbar5000-properties", move || {
            let outcome = paths.into_iter().map(|path| {
                let result = engine::edit_tags(&path, &edit);
                let album = (renames_album && result.is_ok()).then(|| written_album(&path)).flatten();
                (path, result, album)
            });
            let _ = tx.send(outcome.collect());
        });
        if !started {
            self.close_after_save.set(false);
            self.window.set_status("Couldn\u{2019}t save just now: the system is short of resources. Try again.".into());
            return;
        }
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
            .filter_map(|(path, result, _)| {
                result.as_ref().err().map(|e| {
                    format!("{}: {e}", path.file_name().unwrap_or_default().to_string_lossy())
                })
            })
            .collect();
        // The album renamed: its look kept for the new name too, and the
        // window going on with the new name (a later look saved under it).
        let renamed_look = self.renamed_look.borrow_mut().take();
        let mut renamed = false;
        for (path, _, album) in &outcome {
            let Some(album) = album else { continue };
            renamed = true;
            if let Some(look) = &renamed_look {
                vinyl_style::save_album_look(&self.store, album, look);
            }
            let mut tracks = self.vinyl_tracks.borrow_mut();
            for (i, track) in tracks.iter_mut().enumerate().filter(|(_, t)| &t.path == path) {
                for key in track.record_albums.iter_mut().filter(|key| **key == track.album) {
                    key.clone_from(album);
                }
                track.album.clone_from(album);
                if i == 0 {
                    self.vinyl_album.borrow_mut().clone_from(album);
                }
            }
        }
        if renamed && renamed_look.is_some() {
            if let Some(hook) = self.vinyl_saved.borrow().as_ref() {
                hook();
            }
        }
        // Show the new tags and covers in the playlist.
        self.engine.send(EngineCommand::RefreshMetadata);
        if failures.is_empty() {
            self.edits.borrow_mut().clear();
            self.picture_edits.borrow_mut().clear();
            self.update_dirty();
            if self.close_after_save.get() {
                self.hide();
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

/// The album `path`'s record look is kept for now, from its tags as just
/// written (`playlist_rows::album_key_from`); `None` if they can't be read.
fn written_album(path: &std::path::Path) -> Option<String> {
    let fields = engine::read_tag_fields(path).ok()?;
    let tag = |wanted: TagField| fields.iter().find(|(field, _)| *field == wanted).map_or("", |(_, value)| value.as_str());
    Some(crate::playlist_rows::album_key_from(tag(TagField::AlbumArtist), tag(TagField::Artist), tag(TagField::Album), path))
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
