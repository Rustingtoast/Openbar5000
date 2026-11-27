//! The icon chooser's preview (`LogoPreviewWindow`, from Preferences >
//! Appearance > Icon): the chosen icon big, and every version of it --
//! regular, the holidays', each month's pride flag's -- any of which can be
//! kept for good.

use crate::holiday;
use crate::logo_files::{LogoIcon, VERSIONS};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::rc::Rc;
use ui::{LogoPreviewWindow, LogoVersion};

/// The first versions are regular and the holidays; the rest the flags.
const HOLIDAY_VERSIONS: usize = 6;

/// `svg` as an image; an empty one if it doesn't load (a test checks they
/// all do).
pub fn image(svg: &str) -> slint::Image {
    slint::Image::load_from_svg_data(svg.as_bytes()).unwrap_or_default()
}

fn version(icon: &LogoIcon, key: &str) -> LogoVersion {
    LogoVersion {
        key: key.into(),
        label: holiday::version_label(key).into(),
        image: image(holiday::logo_svg(icon, key)),
    }
}

fn model(versions: Vec<LogoVersion>) -> ModelRc<LogoVersion> {
    ModelRc::from(Rc::new(VecModel::from(versions)))
}

/// Opens (or refreshes) the preview for `icon`, `pinned` ("" or a
/// version) marked; shows the pinned version big, or the regular one.
pub fn show(window: &LogoPreviewWindow, icon: &LogoIcon, pinned: &str) {
    let versions: Vec<LogoVersion> = VERSIONS.iter().map(|key| version(icon, key)).collect();
    let chosen = versions
        .iter()
        .find(|v| v.key == pinned)
        .unwrap_or(&versions[0])
        .clone();
    window.set_icon_name(icon.name.into());
    window.set_holidays(model(versions[..HOLIDAY_VERSIONS].to_vec()));
    window.set_flags(model(versions[HOLIDAY_VERSIONS..].to_vec()));
    window.set_chosen(chosen);
    set_pinned(window, pinned);
    if let Err(e) = window.show() {
        tracing::warn!("couldn't show the icon preview: {e}");
    }
    window.invoke_focus_keys();
}

/// Marks `pinned` ("" for none) as the version shown always.
pub fn set_pinned(window: &LogoPreviewWindow, pinned: &str) {
    window.set_pinned(pinned.into());
    window.set_pinned_label(if pinned.is_empty() { String::new() } else { holiday::version_label(pinned) }.into());
}
