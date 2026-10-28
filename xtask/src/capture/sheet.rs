//! JPEG contact sheets for `cargo xtask capture`: every captured frame scaled
//! into one labelled grid, at most `max_width` pixels wide, so a review reads
//! one small image instead of N full-size PNGs.

use std::path::{Path, PathBuf};

use anyhow::Context;
use image::{imageops, Rgb, RgbImage};

/// Gap between tiles and around the sheet, in pixels.
const PAD: u32 = 4;
/// Height of the label strip above each tile.
const LABEL_H: u32 = 18;
/// Glyph scale of the built-in 5x7 label font.
const SCALE: u32 = 2;
const BG: Rgb<u8> = Rgb([30, 30, 34]);
const FG: Rgb<u8> = Rgb([230, 230, 230]);

/// One sheet tile: the image to place and the label printed above it.
pub struct Tile {
    pub path: PathBuf,
    pub label: String,
}

/// Columns for `n` tiles when the caller did not ask for a count: a near-square
/// grid, capped at four so a tile stays legible at the default width.
pub fn default_cols(n: usize) -> u32 {
    match n {
        0..=1 => 1,
        2..=4 => 2,
        5..=9 => 3,
        _ => 4,
    }
}

/// Write `tiles` as a JPEG grid at `out`, `max_width` pixels wide; returns the
/// sheet size.
pub fn write_sheet(
    tiles: &[Tile],
    out: &Path,
    max_width: u32,
    cols: Option<u32>,
) -> anyhow::Result<(u32, u32)> {
    anyhow::ensure!(!tiles.is_empty(), "no frames to put on a contact sheet");
    let cols = cols.unwrap_or_else(|| default_cols(tiles.len())).clamp(1, tiles.len() as u32);
    let rows = (tiles.len() as u32).div_ceil(cols);
    let first =
        image::open(&tiles[0].path).with_context(|| format!("open {}", tiles[0].path.display()))?;
    // Never upscale: a lone 1280 px frame stays 1280 px.
    let tile_w =
        (max_width.saturating_sub(PAD * (cols + 1)) / cols).clamp(16, first.width().max(16));
    let tile_h = (tile_w as u64 * first.height() as u64 / first.width().max(1) as u64) as u32;
    let sheet_w = PAD + cols * (tile_w + PAD);
    let sheet_h = PAD + rows * (LABEL_H + tile_h + PAD);
    let mut sheet = RgbImage::from_pixel(sheet_w, sheet_h, BG);
    for (i, t) in tiles.iter().enumerate() {
        let img = image::open(&t.path).with_context(|| format!("open {}", t.path.display()))?;
        let small =
            imageops::resize(&img.to_rgb8(), tile_w, tile_h, imageops::FilterType::Triangle);
        let (c, r) = (i as u32 % cols, i as u32 / cols);
        let x = PAD + c * (tile_w + PAD);
        let y = PAD + r * (LABEL_H + tile_h + PAD);
        draw_text(&mut sheet, x + 2, y + 2, &t.label, tile_w);
        imageops::replace(&mut sheet, &small, x as i64, (y + LABEL_H) as i64);
    }
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let file = std::fs::File::create(out).with_context(|| format!("create {}", out.display()))?;
    let mut enc =
        image::codecs::jpeg::JpegEncoder::new_with_quality(std::io::BufWriter::new(file), 85);
    enc.encode_image(&sheet).with_context(|| format!("encode {}", out.display()))?;
    Ok((sheet_w, sheet_h))
}

/// Draw `text` (upper-cased, clipped to `max_w`) with the 5x7 font.
fn draw_text(img: &mut RgbImage, x: u32, y: u32, text: &str, max_w: u32) {
    let advance = 6 * SCALE;
    for (i, ch) in text.chars().enumerate() {
        let gx = x + i as u32 * advance;
        if gx + advance > x + max_w {
            break;
        }
        for (row, bits) in glyph(ch).iter().enumerate() {
            for col in 0..5u32 {
                if bits & (0x10 >> col) == 0 {
                    continue;
                }
                for dy in 0..SCALE {
                    for dx in 0..SCALE {
                        let (px, py) = (gx + col * SCALE + dx, y + row as u32 * SCALE + dy);
                        if px < img.width() && py < img.height() {
                            img.put_pixel(px, py, FG);
                        }
                    }
                }
            }
        }
    }
}

/// A 5x7 bitmap glyph (one byte per row, bit 4 = leftmost column).  Letters
/// render upper-case; anything unknown renders as `?`.
fn glyph(c: char) -> [u8; 7] {
    match c.to_ascii_uppercase() {
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        '3' => [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        '6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        'D' => [0x1C, 0x12, 0x11, 0x11, 0x11, 0x12, 0x1C],
        'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        'G' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F],
        'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'I' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        'J' => [0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0C],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x11, 0x19, 0x15, 0x13, 0x11, 0x11],
        'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        'Q' => [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D],
        'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0A],
        'X' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
        'Y' => [0x11, 0x11, 0x11, 0x0A, 0x04, 0x04, 0x04],
        'Z' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F],
        ' ' => [0; 7],
        '_' => [0, 0, 0, 0, 0, 0, 0x1F],
        '-' => [0, 0, 0, 0x1F, 0, 0, 0],
        '.' => [0, 0, 0, 0, 0, 0x0C, 0x0C],
        ':' => [0, 0x0C, 0x0C, 0, 0x0C, 0x0C, 0],
        '/' => [0x01, 0x01, 0x02, 0x04, 0x08, 0x10, 0x10],
        '=' => [0, 0, 0x1F, 0, 0x1F, 0, 0],
        _ => [0x0E, 0x11, 0x01, 0x02, 0x04, 0, 0x04],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_is_near_square_and_capped() {
        assert_eq!(default_cols(1), 1);
        assert_eq!(default_cols(4), 2);
        assert_eq!(default_cols(9), 3);
        assert_eq!(default_cols(40), 4);
    }

    #[test]
    fn sheet_fits_the_width_budget() {
        let dir = std::env::temp_dir().join(format!("xtask-sheet-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tiles: Vec<Tile> = (0..10)
            .map(|i| {
                let path = dir.join(format!("frame_{i:05}.png"));
                RgbImage::from_pixel(1280, 720, Rgb([i as u8 * 20, 0, 0])).save(&path).unwrap();
                Tile { path, label: format!("f{i}") }
            })
            .collect();
        let out = dir.join("sheet.jpg");
        let (w, h) = write_sheet(&tiles, &out, 1400, None).unwrap();
        assert!(w <= 1400, "sheet is {w} px wide");
        assert_eq!(image::open(&out).unwrap().width(), w);
        // 10 tiles at 4 columns: 3 rows of 16:9 tiles.
        let tile_w = (1400 - PAD * 5) / 4;
        assert_eq!(h, PAD + 3 * (LABEL_H + tile_w * 720 / 1280 + PAD));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn every_label_character_has_a_glyph() {
        let unknown = glyph('\u{1}');
        for c in "0123456789abcdefghijklmnopqrstuvwxyz_-.:/=".chars() {
            assert_ne!(glyph(c), unknown, "{c:?} falls back to `?`");
        }
    }
}
