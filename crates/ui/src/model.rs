use slint::{Rgba8Pixel, SharedPixelBuffer};
use std::time::Duration;

/// Formats a duration as foobar2000-style `m:ss` (or `h:mm:ss` past an hour).
pub fn format_duration(duration: Duration) -> String {
    let total_secs = duration.as_secs();
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

const PLACEHOLDER_COVER_SIZE: u32 = 48;
const PLACEHOLDER_COVER_COLOR: [u8; 3] = [0x3a, 0x3a, 0x3a];

/// Wraps already-decoded RGBA8 cover pixels (the engine decodes and shrinks
/// art at load time, off the UI thread) in a displayable [`slint::Image`].
/// Falls back to [`placeholder_cover_image`] if the buffer doesn't match
/// the dimensions.
pub fn cover_image_from_rgba(width: u32, height: u32, rgba: &[u8]) -> slint::Image {
    if rgba.len() != (width as usize) * (height as usize) * 4 {
        return placeholder_cover_image();
    }
    slint::Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
        rgba, width, height,
    ))
}

/// A plain dark square, so the playlist's group headers always have a
/// consistent-sized thumbnail slot when a track has no art.
pub fn placeholder_cover_image() -> slint::Image {
    let mut buffer =
        SharedPixelBuffer::<Rgba8Pixel>::new(PLACEHOLDER_COVER_SIZE, PLACEHOLDER_COVER_SIZE);
    let [r, g, b] = PLACEHOLDER_COVER_COLOR;
    for pixel in buffer.make_mut_slice() {
        *pixel = Rgba8Pixel::new(r, g, b, 0xff);
    }
    slint::Image::from_rgba8(buffer)
}
