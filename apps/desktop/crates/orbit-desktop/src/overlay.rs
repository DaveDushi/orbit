//! The Windows taskbar overlay: a red circle with the unread number (1 to 9, then "9+"), or a
//! small red dot for a flag. Drawn at run time at the taskbar's small-icon size (16 px × the
//! display scale), on whole pixels, so it stays sharp at 125 % and 150 %.

use crate::bridge::Badge;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mark {
    /// `setAppBadge()` without a number.
    Dot,
    /// "1" to "9", or "9+".
    Count(String),
}

/// What the overlay shows for a badge: nothing at zero.
pub fn mark(badge: Badge) -> Option<Mark> {
    match badge {
        Badge::Count(0) => None,
        Badge::Count(n) if n > 9 => Some(Mark::Count("9+".into())),
        Badge::Count(n) => Some(Mark::Count(n.to_string())),
        Badge::Flag => Some(Mark::Dot),
    }
}

/// The overlay's pixels (RGBA, `size` × `size`).
pub fn rgba(mark: &Mark, size: u32) -> Vec<u8> {
    let mut rgba = vec![0; (size * size * 4) as usize];
    let half = size as f32 / 2.0;
    match mark {
        Mark::Dot => {
            circle(&mut rgba, size, half * 0.5 + 1.0, WHITE);
            circle(&mut rgba, size, half * 0.5, RED);
        }
        Mark::Count(text) => {
            // a white ring keeps the circle apart from the pink W and from a dark taskbar
            circle(&mut rgba, size, half, WHITE);
            circle(&mut rgba, size, half - (size / 16).max(1) as f32, RED);
            draw_text(&mut rgba, size, text);
        }
    }
    rgba
}

const RED: [u8; 3] = [220, 38, 38];
const WHITE: [u8; 3] = [255, 255, 255];

/// A centred, anti-aliased disc of radius `r`, over what is already there.
fn circle(rgba: &mut [u8], size: u32, r: f32, color: [u8; 3]) {
    let c = size as f32 / 2.0;
    for y in 0..size {
        for x in 0..size {
            let d = ((x as f32 + 0.5 - c).powi(2) + (y as f32 + 0.5 - c).powi(2)).sqrt();
            let cover = (r - d + 0.5).clamp(0.0, 1.0);
            if cover > 0.0 {
                let i = ((y * size + x) * 4) as usize;
                for (k, v) in color.into_iter().enumerate() {
                    rgba[i + k] = (f32::from(v) * cover + f32::from(rgba[i + k]) * (1.0 - cover)) as u8;
                }
                rgba[i + 3] = (255.0 * cover + f32::from(rgba[i + 3]) * (1.0 - cover)) as u8;
            }
        }
    }
}

/// 3 × 5 glyphs, one row per entry, the high bit on the left.
fn glyph(c: char) -> [u8; 5] {
    match c {
        '0' => [0b111, 0b101, 0b101, 0b101, 0b111],
        '1' => [0b010, 0b110, 0b010, 0b010, 0b111],
        '2' => [0b111, 0b001, 0b111, 0b100, 0b111],
        '3' => [0b111, 0b001, 0b111, 0b001, 0b111],
        '4' => [0b101, 0b101, 0b111, 0b001, 0b001],
        '5' => [0b111, 0b100, 0b111, 0b001, 0b111],
        '6' => [0b111, 0b100, 0b111, 0b101, 0b111],
        '7' => [0b111, 0b001, 0b010, 0b010, 0b010],
        '8' => [0b111, 0b101, 0b111, 0b101, 0b111],
        '9' => [0b111, 0b101, 0b111, 0b001, 0b111],
        '+' => [0b000, 0b010, 0b111, 0b010, 0b000],
        _ => [0; 5],
    }
}

/// White text on whole-pixel cells: one glyph uses square cells, two use narrower ones to fit.
fn draw_text(rgba: &mut [u8], size: u32, text: &str) {
    let n = text.chars().count() as u32;
    let (cell_w, cell_h) = match (size / 8).max(1) {
        // "9+" is 7 cells wide: at most 60 % of the circle, and not much taller than wide
        cell if n > 1 => ((size * 6 / 70).max(1), cell.min((size * 6 / 70).max(1) + 1)),
        cell => (cell, cell),
    };
    let width = (4 * n - 1) * cell_w;
    let height = 5 * cell_h;
    let (x0, y0) = (size.saturating_sub(width) / 2, size.saturating_sub(height) / 2);
    for (k, c) in text.chars().enumerate() {
        for (row, bits) in glyph(c).into_iter().enumerate() {
            for col in 0..3 {
                if bits & (0b100 >> col) == 0 {
                    continue;
                }
                let x = x0 + (k as u32 * 4 + col) * cell_w;
                let y = y0 + row as u32 * cell_h;
                for py in y..(y + cell_h).min(size) {
                    for px in x..(x + cell_w).min(size) {
                        let i = ((py * size + px) * 4) as usize;
                        rgba[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count(text: &str) -> Option<Mark> {
        Some(Mark::Count(text.into()))
    }

    #[test]
    fn the_overlay_shows_one_to_nine_then_nine_plus() {
        assert_eq!(mark(Badge::Count(0)), None);
        assert_eq!(mark(Badge::Count(1)), count("1"));
        assert_eq!(mark(Badge::Count(9)), count("9"));
        assert_eq!(mark(Badge::Count(10)), count("9+"));
        assert_eq!(mark(Badge::Count(u32::MAX)), count("9+"));
        assert_eq!(mark(Badge::Flag), Some(Mark::Dot));
    }

    fn pixel(rgba: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * size + x) * 4) as usize;
        rgba[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn the_number_is_white_in_a_red_circle_with_clear_corners() {
        for size in [16, 20, 24, 32] {
            for text in ["1", "5", "8", "9+"] {
                let rgba = rgba(&Mark::Count(text.into()), size);
                assert_eq!(rgba.len(), (size * size * 4) as usize);
                assert_eq!(pixel(&rgba, size, 0, 0)[3], 0, "{text} at {size}: a round badge");
                assert_eq!(pixel(&rgba, size, size - 1, size - 1)[3], 0);
                let px: Vec<[u8; 4]> = rgba.chunks(4).map(|p| p.try_into().unwrap()).collect();
                let white = px.iter().filter(|p| **p == [255, 255, 255, 255]).count();
                let red = px.iter().filter(|p| **p == [220, 38, 38, 255]).count();
                assert!(white > 0 && red > white, "{text} at {size}: {white} white, {red} red");
            }
        }
        // the digits differ, so they are drawn
        assert_ne!(rgba(&Mark::Count("1".into()), 16), rgba(&Mark::Count("8".into()), 16));
    }

    #[test]
    fn a_flag_is_a_small_dot() {
        let size = 16;
        let rgba = rgba(&Mark::Dot, size);
        assert_eq!(pixel(&rgba, size, 8, 8), [220, 38, 38, 255]);
        assert_eq!(pixel(&rgba, size, 1, 8)[3], 0, "smaller than the number's circle");
    }

    /// Writes the overlays and a mock of the taskbar button to `$ORBIT_OVERLAY_OUT`:
    /// `ORBIT_OVERLAY_OUT=/tmp/shots cargo test -p orbit-desktop overlay -- --ignored`.
    #[test]
    #[ignore = "writes PNG files for a visual check"]
    fn write_pngs() {
        use image::{RgbaImage, imageops};
        let out = std::path::PathBuf::from(std::env::var("ORBIT_OVERLAY_OUT").expect("ORBIT_OVERLAY_OUT"));
        std::fs::create_dir_all(&out).unwrap();
        let marks = [("1", count("1")), ("5", count("5")), ("9", count("9")), ("9plus", count("9+")), ("dot", Some(Mark::Dot))];
        // and all of them on one grey sheet, 8× bigger: one row per mark, one column per size
        let mut sheet = RgbaImage::from_pixel(4 * 272, marks.len() as u32 * 272, image::Rgba([128, 128, 128, 255]));
        for (row, (name, m)) in marks.iter().enumerate() {
            for (col, size) in [16, 20, 24, 32].into_iter().enumerate() {
                let img = RgbaImage::from_raw(size, size, rgba(m.as_ref().unwrap(), size)).unwrap();
                img.save(out.join(format!("overlay-{name}-{size}.png"))).unwrap();
                let big = imageops::resize(&img, size * 8, size * 8, imageops::FilterType::Nearest);
                imageops::overlay(&mut sheet, &big, col as i64 * 272 + 8, row as i64 * 272 + 8);
            }
        }
        sheet.save(out.join("overlay-sheet-8x.png")).unwrap();
        // the taskbar at 150 %: a 48 px tall dark bar, 36 px buttons, a 24 px icon and the 24 px
        // overlay on its bottom right (as Windows draws it), then 4× bigger for the eye
        let icon = image::load_from_memory(include_bytes!("../assets/orbit-64.png")).unwrap().into_rgba8();
        let icon = imageops::resize(&icon, 36, 36, imageops::FilterType::Lanczos3);
        let shown = [("none", None), ("5", count("5")), ("9plus", count("9+")), ("dot", Some(Mark::Dot))];
        let mut bar = RgbaImage::from_pixel(72 * shown.len() as u32, 72, image::Rgba([32, 32, 32, 255]));
        for (k, (_, m)) in shown.iter().enumerate() {
            let x = k as i64 * 72 + 18;
            imageops::overlay(&mut bar, &icon, x, 18);
            if let Some(m) = m {
                let o = RgbaImage::from_raw(24, 24, rgba(m, 24)).unwrap();
                imageops::overlay(&mut bar, &o, x + 36 - 20, 18 + 36 - 20);
            }
        }
        let big = imageops::resize(&bar, bar.width() * 4, bar.height() * 4, imageops::FilterType::Nearest);
        big.save(out.join("taskbar-mock-150.png")).unwrap();
    }
}
