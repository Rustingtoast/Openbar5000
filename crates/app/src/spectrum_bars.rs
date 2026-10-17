//! The spectrum's bars and peak markers, drawn here as one small image:
//! a pixel column per bar, as tall as the plot is in physical pixels,
//! which the spectrum view stretches over the plot (each bar's column
//! filling its note's column) with the gaps between bars drawn over it.
//!
//! As Slint elements -- a rectangle per bar and per peak -- they cost most
//! of a frame's work while playing: each is visited, set up and drawn as a
//! shape of its own on every frame, by Slint and then by Skia (on Windows,
//! close to half of the UI thread's time). An image is one draw, and this
//! one is small: copied and uploaded every frame, a full-size image of the
//! plot would cost as much as the rectangles did. (Not paths: Skia
//! rasterizes a path of many rectangles on the CPU and uploads it every
//! frame.)

use slint::{Color, Rgba8Pixel};
use std::ops::Range;

/// What the image is drawn for: the plot's height in physical pixels, how
/// many of those make a logical pixel, and the theme's colors.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plot {
    pub height: usize,
    pub scale: f32,
    /// A quiet bar.
    pub bar: Color,
    /// A bar at the top, and the peak markers.
    pub accent: Color,
}

const TRANSPARENT: Rgba8Pixel = Rgba8Pixel { r: 0, g: 0, b: 0, a: 0 };

/// Draws `bars`, then `peaks` (if any), into `pixels`: premultiplied RGBA,
/// row by row, `bars.len()` wide and `plot.height` tall, over a transparent
/// background. Each bar is at least a logical pixel tall, each peak a
/// logical pixel line in the accent. Values are the analyzer's `[0, 1]`
/// heights; a bar goes from `bar` to `accent` with the cube of its height,
/// so only bars near the top turn fully accent-colored, as in the
/// reference.
pub fn draw(pixels: &mut [Rgba8Pixel], plot: &Plot, bars: &[f32], peaks: &[f32]) {
    let width = bars.len();
    debug_assert_eq!(pixels.len(), width * plot.height);
    pixels.fill(TRANSPARENT);
    if plot.scale <= 0.0 {
        return;
    }
    // Heights in logical pixels, like the rest of the UI.
    let height = plot.height as f32 / plot.scale;
    let mut fill = |column: usize, top: f32, bottom: f32, color: Color| {
        let color = premultiplied(color);
        for row in rows(top, bottom, plot) {
            pixels[row * width + column] = color;
        }
    };
    for (i, &value) in bars.iter().enumerate() {
        let bar_height = (value.min(1.0) * height).max(1.0);
        let shade = value.clamp(0.0, 1.0).powi(3);
        fill(i, height - bar_height, height, plot.accent.mix(&plot.bar, shade));
    }
    for (i, &peak) in peaks.iter().take(width).enumerate() {
        let top = height - peak.min(1.0) * height;
        fill(i, top, top + 1.0, plot.accent);
    }
}

/// The rows from `top` to `bottom` (logical) cover: those whose centers lie
/// between them, as a GPU fills a rectangle.
fn rows(top: f32, bottom: f32, plot: &Plot) -> Range<usize> {
    let row = |at: f32| ((at * plot.scale - 0.5).ceil().max(0.0) as usize).min(plot.height);
    row(top)..row(bottom)
}

fn premultiplied(color: Color) -> Rgba8Pixel {
    let alpha = color.alpha() as u16;
    let scale = |channel: u8| ((channel as u16 * alpha + 127) / 255) as u8;
    Rgba8Pixel { r: scale(color.red()), g: scale(color.green()), b: scale(color.blue()), a: color.alpha() }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BAR: Color = Color::from_rgb_u8(0x30, 0x30, 0x30);
    const ACCENT: Color = Color::from_rgb_u8(0xff, 0x80, 0x00);

    fn plot(height: usize, scale: f32) -> Plot {
        Plot { height, scale, bar: BAR, accent: ACCENT }
    }

    fn render(plot: &Plot, bars: &[f32], peaks: &[f32]) -> Vec<Rgba8Pixel> {
        let mut pixels = vec![Rgba8Pixel { r: 1, g: 2, b: 3, a: 4 }; bars.len() * plot.height];
        draw(&mut pixels, plot, bars, peaks);
        pixels
    }

    /// One bar's column, top to bottom: `.` transparent, `#` drawn.
    fn column(pixels: &[Rgba8Pixel], width: usize, x: usize) -> String {
        pixels.iter().skip(x).step_by(width).map(|p| if p.a == 0 { '.' } else { '#' }).collect()
    }

    #[test]
    fn bars_rise_from_the_bottom_shaded_by_their_height() {
        let plot = plot(10, 1.0);
        let pixels = render(&plot, &[0.5, 0.0, 1.5], &[]);
        assert_eq!(column(&pixels, 3, 0), ".....#####");
        // A silent bar is still a pixel tall, in the plain bar color.
        assert_eq!(column(&pixels, 3, 1), ".........#");
        assert_eq!(pixels[9 * 3 + 1], premultiplied(BAR));
        assert_eq!(pixels[9 * 3], premultiplied(ACCENT.mix(&BAR, 0.125)));
        // A loud one is capped at the top, all in the accent.
        assert_eq!(column(&pixels, 3, 2), "##########");
        assert_eq!(pixels[2], premultiplied(ACCENT.mix(&BAR, 1.0)));
    }

    #[test]
    fn peaks_are_accent_lines_drawn_over_the_bars() {
        let plot = plot(100, 1.0);
        let pixels = render(&plot, &[0.5, 0.0], &[0.25, 2.0]);
        let bar = premultiplied(ACCENT.mix(&BAR, 0.125));
        assert_eq!((pixels[74 * 2], pixels[75 * 2], pixels[76 * 2]), (bar, premultiplied(ACCENT), bar));
        // Beyond the top: drawn at the top.
        assert_eq!(column(&pixels, 2, 1), format!("#{}#", ".".repeat(98)));
    }

    #[test]
    fn heights_and_lines_are_logical_pixels() {
        // Two physical pixels to a logical one: 10 logical pixels tall.
        let plot = plot(20, 2.0);
        let pixels = render(&plot, &[0.0, 0.5], &[0.5, 0.0]);
        assert_eq!(column(&pixels, 2, 0), "..........##......##");
        assert_eq!(column(&pixels, 2, 1), "..........##########");
    }

    #[test]
    fn colors_are_premultiplied() {
        let half = Color::from_argb_u8(128, 255, 100, 0);
        assert_eq!(premultiplied(half), Rgba8Pixel { r: 128, g: 50, b: 0, a: 128 });
    }

    #[test]
    fn nothing_to_draw_without_bars_or_room() {
        assert!(render(&plot(10, 1.0), &[], &[]).is_empty());
        assert!(render(&plot(0, 1.0), &[0.5], &[0.5]).is_empty());
    }
}
