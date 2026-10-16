//! The spectrum's bars and peak markers as one flat list of rectangles,
//! laid out here in the plot's pixels. Drawn per note -- a column element
//! holding a conditional bar and peak, each working out its own geometry
//! and shade -- they cost most of a frame's work while playing; plain
//! rectangles are the cheapest thing to draw. (Not paths: Skia rasterizes
//! a path of many rectangles on the CPU and uploads it every frame.)

/// One rectangle in the plot: `x`/`y` from its top-left, in pixels.
/// `shade` runs from 0 (the plain bar color) to 1 (the accent).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpectrumRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub shade: f32,
}

/// Lays out `bars`, then `peaks` (if any), across a `width` x `height`
/// plot: one column per bar, each bar inset 1px on both sides and at least
/// 1px tall, each peak a 1px accent line. Values are the analyzer's
/// `[0, 1]` heights; a bar's shade is the cube of its height, so only bars
/// near the top turn fully accent-colored, as in the reference.
pub fn spectrum_rects(bars: &[f32], peaks: &[f32], width: f32, height: f32) -> Vec<SpectrumRect> {
    if bars.is_empty() || width <= 0.0 || height <= 0.0 {
        return Vec::new();
    }
    let column = width / bars.len() as f32;
    let bar_width = column - 2.0;
    if bar_width <= 0.0 {
        return Vec::new();
    }
    let bars = bars.iter().enumerate().map(|(i, &value)| {
        let bar_height = (value.min(1.0) * height).max(1.0);
        SpectrumRect {
            x: i as f32 * column + 1.0,
            y: height - bar_height,
            width: bar_width,
            height: bar_height,
            shade: value.clamp(0.0, 1.0).powi(3),
        }
    });
    let peaks = peaks.iter().take(bars.len()).enumerate().map(|(i, &peak)| SpectrumRect {
        x: i as f32 * column + 1.0,
        y: height - peak.min(1.0) * height,
        width: bar_width,
        height: 1.0,
        shade: 1.0,
    });
    bars.chain(peaks).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, width: f32, height: f32, shade: f32) -> SpectrumRect {
        SpectrumRect { x, y, width, height, shade }
    }

    #[test]
    fn bars_fill_their_column_less_a_pixel_each_side_from_the_bottom() {
        let rects = spectrum_rects(&[0.5, 0.0], &[], 20.0, 100.0);
        // The silent bar is still 1px tall, in the plain bar color.
        assert_eq!(rects, [rect(1.0, 50.0, 8.0, 50.0, 0.125), rect(11.0, 99.0, 8.0, 1.0, 0.0)]);
    }

    #[test]
    fn loud_bars_are_capped_at_the_top_in_the_accent() {
        let rects = spectrum_rects(&[1.5], &[], 10.0, 10.0);
        assert_eq!(rects, [rect(1.0, 0.0, 8.0, 10.0, 1.0)]);
    }

    #[test]
    fn peaks_follow_the_bars_as_one_pixel_accent_lines() {
        let rects = spectrum_rects(&[0.0, 0.0], &[0.75, 2.0], 20.0, 100.0);
        assert_eq!(&rects[2..], [rect(1.0, 25.0, 8.0, 1.0, 1.0), rect(11.0, 0.0, 8.0, 1.0, 1.0)]);
    }

    #[test]
    fn nothing_to_draw_without_room_or_bars() {
        assert!(spectrum_rects(&[], &[], 100.0, 100.0).is_empty());
        assert!(spectrum_rects(&[0.5], &[0.5], 0.0, 100.0).is_empty());
        // Columns 2px wide leave no bar between the insets.
        assert!(spectrum_rects(&[0.5, 0.5], &[0.5, 0.5], 4.0, 100.0).is_empty());
    }
}
