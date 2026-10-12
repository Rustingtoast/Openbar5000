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

/// Decodes embedded cover-art bytes (JPEG/PNG, as read from the file's tags)
/// into a displayable [`slint::Image`]. Falls back to a plain dark square
/// when there's no art, or it fails to decode, so the playlist's group
/// headers always have a consistent-sized thumbnail slot.
pub fn decode_cover_image(bytes: Option<&[u8]>) -> slint::Image {
    let Some(bytes) = bytes else {
        return placeholder_cover_image();
    };
    let Ok(decoded) = image::load_from_memory(bytes) else {
        return placeholder_cover_image();
    };

    let rgba = decoded.to_rgba8();
    let (width, height) = rgba.dimensions();
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(width, height);
    for (dst, src) in buffer.make_mut_slice().iter_mut().zip(rgba.pixels()) {
        *dst = Rgba8Pixel::new(src[0], src[1], src[2], src[3]);
    }
    slint::Image::from_rgba8(buffer)
}

fn placeholder_cover_image() -> slint::Image {
    let mut buffer =
        SharedPixelBuffer::<Rgba8Pixel>::new(PLACEHOLDER_COVER_SIZE, PLACEHOLDER_COVER_SIZE);
    let [r, g, b] = PLACEHOLDER_COVER_COLOR;
    for pixel in buffer.make_mut_slice() {
        *pixel = Rgba8Pixel::new(r, g, b, 0xff);
    }
    slint::Image::from_rgba8(buffer)
}
