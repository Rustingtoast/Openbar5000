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

/// `svg` drawn once into a `size`-pixel-square bitmap. For the logo every
/// window shows: an SVG image is drawn again each time it's on screen, and
/// the logos' painterly filters make that a few milliseconds at a time --
/// a hitch in the turning record whenever a menu opened over the logo or a
/// window opened. A bitmap is just scaled. (`image`'s SVG if drawing fails.)
pub fn raster(svg: &str, size: u32) -> slint::Image {
    let drawn = (|| {
        let tree = resvg::usvg::Tree::from_data(svg.as_bytes(), &resvg::usvg::Options::default()).ok()?;
        let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size)?;
        let scale = size as f32 / tree.size().width().max(tree.size().height());
        resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
        let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(pixmap.data(), size, size);
        Some(slint::Image::from_rgba8_premultiplied(buffer))
    })();
    drawn.unwrap_or_else(|| {
        tracing::warn!("couldn't draw the logo into a bitmap; showing it as SVG");
        image(svg)
    })
}

/// The size the logo is drawn at: plenty for its largest use (About, the
/// window icon on a 2x screen).
pub const LOGO_SIZE: u32 = 256;

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

/// Opens the preview for `icon` (see `fill`), or brings it to the front.
pub fn show(window: &LogoPreviewWindow, icon: &LogoIcon, pinned: &str) {
    fill(window, icon, pinned);
    // Already open (behind Preferences, say): to the front.
    crate::platform::show_in_front(window.window());
    window.invoke_focus_keys();
}

/// Fills the preview with `icon`'s versions, `pinned` ("" or a version)
/// marked, the pinned version big (or the regular one).
pub fn fill(window: &LogoPreviewWindow, icon: &LogoIcon, pinned: &str) {
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
}

/// Marks `pinned` ("" for none) as the version shown always.
pub fn set_pinned(window: &LogoPreviewWindow, pinned: &str) {
    window.set_pinned(pinned.into());
    window.set_pinned_label(if pinned.is_empty() { String::new() } else { holiday::version_label(pinned) }.into());
}
