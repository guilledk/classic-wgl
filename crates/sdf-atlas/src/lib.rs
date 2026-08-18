//! SDF (signed-distance-field) font atlas generator.
//!
//! Port of `scripts/make-font-atlas.mjs`.  Given TTF bytes it rasterises every
//! glyph in a charset, computes a signed-distance field (Felzenszwalb separable
//! EDT), packs the cells into a power-of-two atlas, and returns a grayscale
//! atlas PNG plus snake_case glyph-metrics JSON matching the engine's
//! [`SdfFontMetrics`] schema.

mod charset;

use anyhow::{Context, Result};
use fontdue::{Font, FontSettings};

pub use charset::full_charset;

/// Glyph-size / layout constants, matching `make-font-atlas.mjs`.
const GLYPH_SIZE: f32 = 64.0;
const PAD: i32 = 2;
const FONT_CELL_SIZE: f32 = GLYPH_SIZE * 0.4; // 25.6
/// Space advance in cell px — matches `make-font-atlas.mjs` (`fontSize * 0.28`).
const SPACE_ADVANCE: f32 = FONT_CELL_SIZE * 0.28; // 7.168

/// Atlas generation options.
#[derive(Clone, Debug)]
pub struct Options {
    /// Font family name (recorded in the metrics JSON).
    pub family: String,
    /// Supersampling factor for rasterisation (default 12).
    pub supersample: u32,
    /// SDF spread in cell pixels (default 4).
    pub spread: u32,
    /// Maximum atlas width/height (default 4096).
    pub max_size: u32,
}

impl Default for Options {
    fn default() -> Self {
        Self { family: "DejaVuSans".into(), supersample: 12, spread: 4, max_size: 4096 }
    }
}

/// A generated atlas: grayscale PNG bytes + snake_case metrics JSON.
pub struct Atlas {
    pub png: Vec<u8>,
    pub metrics_json: String,
}

/// One packed glyph cell, pre-blit.
struct Cell {
    ch: char,
    x_advance: f32,
    w: i32,
    h: i32,
    atlas_x: i32,
    atlas_y: i32,
    sdf: Vec<u8>, // cell_w * cell_h
}

/// Generate the atlas + metrics for the default (full) charset.
pub fn generate(font_bytes: &[u8], opts: &Options) -> Result<Atlas> {
    let font = Font::from_bytes(font_bytes, FontSettings::default())
        .map_err(|e| anyhow::anyhow!("parse font: {e}"))?;

    let ss = opts.supersample as i32;
    let spread = opts.spread as i32;
    let px = FONT_CELL_SIZE * ss as f32;
    let baseline = FONT_CELL_SIZE * 0.78; // 19.968
    let line_height = FONT_CELL_SIZE * 1.3; // 33.28

    let mut cells = Vec::new();
    for ch in full_charset() {
        if font.lookup_glyph_index(ch) == 0 {
            continue; // .notdef — no real outline
        }
        let (metrics, bitmap) = font.rasterize(ch, px);

        let ink_w = (metrics.bounds.width / ss as f32).ceil().max(1.0) as i32;
        let ink_h = (metrics.bounds.height / ss as f32).ceil().max(1.0) as i32;
        let cell_w = ink_w + 2 * spread + 2 * PAD;
        let cell_h = ink_h + 2 * spread + 2 * PAD;

        let sdf = build_sdf_cell(&metrics, &bitmap, cell_w, cell_h, ss, spread, baseline);
        if !sdf.iter().any(|&b| b != 128) {
            continue; // fully blank (no variation)
        }

        let x_advance = glyph_advance(ch, metrics.advance_width, ss as f32);

        cells.push(Cell { ch, x_advance, w: cell_w, h: cell_h, atlas_x: 0, atlas_y: 0, sdf });
    }

    if cells.is_empty() {
        anyhow::bail!("no glyphs rendered from the charset");
    }

    let atlas_size = pack(&mut cells, PAD, opts.max_size as i32)?;

    // Blit cells into a grayscale atlas.
    let mut atlas = vec![0u8; (atlas_size * atlas_size) as usize];
    for cell in &cells {
        for row in 0..cell.h {
            let src = (row * cell.w) as usize;
            let dst = ((cell.atlas_y + row) * atlas_size + cell.atlas_x) as usize;
            atlas[dst..dst + cell.w as usize]
                .copy_from_slice(&cell.sdf[src..src + cell.w as usize]);
        }
    }

    let png = encode_grayscale_png(&atlas, atlas_size, atlas_size)?;
    let metrics_json = metrics_json(opts, baseline, line_height, atlas_size, &cells)?;

    Ok(Atlas { png, metrics_json })
}

/// Advance width (cell px) for a glyph.  Space uses the fixed [`SPACE_ADVANCE`]
/// (matching the legacy atlas), other glyphs use the font's advance scaled down
/// from supersampled px.
fn glyph_advance(ch: char, advance_px: f32, ss: f32) -> f32 {
    if ch == ' ' {
        SPACE_ADVANCE
    } else {
        advance_px / ss
    }
}

/// Build a single glyph's SDF cell (cell_w × cell_h) from fontdue's rasterized
/// bitmap, following `make-font-atlas.mjs::renderGlyphSDF`.
fn build_sdf_cell(
    metrics: &fontdue::Metrics,
    bitmap: &[u8],
    cell_w: i32,
    cell_h: i32,
    ss: i32,
    spread: i32,
    baseline: f32,
) -> Vec<u8> {
    let src_w = (cell_w * ss) as usize;
    let src_h = (cell_h * ss) as usize;

    // Pen (baseline-left) position in the high-res cell, in cell-px scaled by ss.
    let pen_x = (PAD + spread) as f32 * ss as f32;
    let pen_y = (PAD + spread) as f32 + baseline;

    let bw = metrics.width;
    let bh = metrics.height;
    let top = metrics.ymin + bh as i32; // top edge of the bitmap in font-space (y-up)

    let mut inside = vec![0u8; src_w * src_h];
    for cy in 0..src_h {
        for cx in 0..src_w {
            // cell (y-down) -> font space (y-up, origin at pen)
            let fx = cx as f32 - pen_x;
            let fy = pen_y * ss as f32 - cy as f32;
            let bx = (fx - metrics.xmin as f32).floor() as i32;
            let by = (top as f32 - 1.0 - fy).floor() as i32;
            let cov = if bx >= 0 && by >= 0 && (bx as usize) < bw && (by as usize) < bh {
                bitmap[(by as usize) * bw + bx as usize]
            } else {
                0
            };
            inside[cy * src_w + cx] = if cov > 128 { 1 } else { 0 };
        }
    }

    let outside: Vec<u8> = inside.iter().map(|&v| 1 - v).collect();
    let mut buf_a = vec![0.0f64; src_w * src_h];
    let mut buf_b = vec![0.0f64; src_w * src_h];
    edt2d(&outside, src_w, src_h, &mut buf_a);
    edt2d(&inside, src_w, src_h, &mut buf_b);

    let max_dist = (spread as f64) * (ss as f64);
    let half = (ss / 2) as usize;
    let mut cell = vec![0u8; (cell_w * cell_h) as usize];
    for cy in 0..cell_h as usize {
        for cx in 0..cell_w as usize {
            let p = (cy * ss as usize + half) * src_w + (cx * ss as usize + half);
            let sd = if inside[p] == 1 { buf_a[p].sqrt() } else { -(buf_b[p].sqrt()) };
            let norm = (sd / max_dist).clamp(-1.0, 1.0);
            cell[cy * cell_w as usize + cx] = (128.0 + norm * 127.0).round() as u8;
        }
    }
    cell
}

/// Felzenszwalb separable squared-distance transform (port of `edt2d`).
/// `mask` marks the source set (1 = source); `out[p]` = squared distance from
/// `p` to the nearest source pixel.
fn edt2d(mask: &[u8], w: usize, h: usize, out: &mut [f64]) {
    const INF: f64 = 1e20; // matches the mjs — finite, so no `inf - inf = NaN`
    let m = w.max(h);
    let mut f = vec![0.0f64; m];
    let mut d = vec![0.0f64; m];
    let mut v = vec![0i32; m];
    let mut z = vec![0.0f64; m + 1];

    // Rows
    for x in 0..w {
        for y in 0..h {
            f[y] = if mask[y * w + x] != 0 { 0.0 } else { INF };
        }
        dt1d(&f, h, &mut d, &mut v, &mut z);
        for y in 0..h {
            out[y * w + x] = d[y];
        }
    }
    // Columns
    for y in 0..h {
        for x in 0..w {
            f[x] = out[y * w + x];
        }
        dt1d(&f, w, &mut d, &mut v, &mut z);
        for x in 0..w {
            out[y * w + x] = d[x];
        }
    }
}

#[allow(clippy::needless_range_loop)] // index-heavy numerical port of the mjs EDT
fn dt1d(f: &[f64], n: usize, d: &mut [f64], v: &mut [i32], z: &mut [f64]) {
    let mut k = 0usize;
    v[0] = 0;
    z[0] = f64::NEG_INFINITY;
    z[1] = f64::INFINITY;
    for q in 1..n {
        let qf = q as f64;
        let mut s = {
            let vk = v[k] as usize;
            (f[q] + qf * qf - (f[vk] + (vk as f64) * (vk as f64))) / (2.0 * (qf - vk as f64))
        };
        while s <= z[k] {
            k -= 1;
            let vk = v[k] as usize;
            s = (f[q] + qf * qf - (f[vk] + (vk as f64) * (vk as f64))) / (2.0 * (qf - vk as f64));
        }
        k += 1;
        v[k] = q as i32;
        z[k] = s;
        z[k + 1] = f64::INFINITY;
    }
    k = 0;
    for q in 0..n {
        while z[k + 1] < q as f64 {
            k += 1;
        }
        d[q] = (q as f64 - v[k] as f64).powi(2) + f[v[k] as usize];
    }
}

fn nearest_pow2(n: i32) -> i32 {
    let mut v = 1;
    while v < n {
        v *= 2;
    }
    v
}

/// Greedy shelf packing (port of `packAtlas`).  Assigns `atlas_x`/`atlas_y`.
fn pack(cells: &mut [Cell], pad: i32, max_size: i32) -> Result<i32> {
    cells.sort_by(|a, b| b.h.cmp(&a.h).then_with(|| b.w.cmp(&a.w)));

    let total_area: i64 =
        cells.iter().map(|c| (c.w as i64 + pad as i64) * (c.h as i64 + pad as i64)).sum();
    let mut size = nearest_pow2(((total_area as f64).sqrt() * 1.3).ceil() as i32);

    loop {
        let mut x = pad;
        let mut y = pad;
        let mut row_h = 0;
        let mut ok = true;
        for c in cells.iter_mut() {
            if x + c.w + pad > size {
                y += row_h + pad;
                x = pad;
                row_h = 0;
            }
            if y + c.h + pad > size {
                ok = false;
                break;
            }
            c.atlas_x = x;
            c.atlas_y = y;
            x += c.w + pad;
            row_h = row_h.max(c.h);
        }
        if ok {
            return Ok(size);
        }
        if size >= max_size {
            anyhow::bail!(
                "atlas packer could not fit {} glyphs at max size {max_size}",
                cells.len()
            );
        }
        size *= 2;
    }
}

fn encode_grayscale_png(bytes: &[u8], w: i32, h: i32) -> Result<Vec<u8>> {
    let img = image::GrayImage::from_raw(w as u32, h as u32, bytes.to_vec())
        .context("build grayscale image")?;
    let mut out = Vec::new();
    image::DynamicImage::ImageLuma8(img)
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .context("encode PNG")?;
    Ok(out)
}

#[derive(serde::Serialize)]
struct GlyphOut {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    x_offset: f32,
    y_offset: f32,
    x_advance: f32,
}

#[derive(serde::Serialize)]
struct MetricsOut {
    name: String,
    family: String,
    atlas_size: [f32; 2],
    glyph_size: f32,
    spread: f32,
    baseline: f32,
    line_height: f32,
    glyphs: std::collections::BTreeMap<String, GlyphOut>,
}

fn metrics_json(
    opts: &Options,
    baseline: f32,
    line_height: f32,
    atlas_size: i32,
    cells: &[Cell],
) -> Result<String> {
    let x_offset = -(PAD + opts.spread as i32) as f32;
    let y_offset = -(PAD + opts.spread as i32) as f32 - baseline;

    let mut glyphs = std::collections::BTreeMap::new();
    for c in cells {
        glyphs.insert(
            c.ch.to_string(),
            GlyphOut {
                x: c.atlas_x as f32,
                y: c.atlas_y as f32,
                w: c.w as f32,
                h: c.h as f32,
                x_offset,
                y_offset,
                x_advance: c.x_advance,
            },
        );
    }

    let name = opts.family.to_lowercase().replace([' ', '-'], "-");
    let m = MetricsOut {
        name,
        family: opts.family.clone(),
        atlas_size: [atlas_size as f32, atlas_size as f32],
        glyph_size: GLYPH_SIZE,
        spread: opts.spread as f32,
        baseline,
        line_height,
        glyphs,
    };
    serde_json::to_string_pretty(&m).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn edt_distances_are_correct() {
        // A single source pixel in the centre of a 5x5 grid.
        let mut mask = vec![0u8; 25];
        mask[2 * 5 + 2] = 1;
        let mut out = vec![0.0f64; 25];
        edt2d(&mask, 5, 5, &mut out);
        // Centre = 0; orthogonal neighbour = 1; diagonal = 2.
        assert_eq!(out[2 * 5 + 2], 0.0);
        assert_eq!(out[2 * 5 + 3], 1.0);
        assert_eq!(out[3 * 5 + 3], 2.0);
    }

    #[test]
    fn nearest_pow2_rounds_up() {
        assert_eq!(nearest_pow2(1), 1);
        assert_eq!(nearest_pow2(100), 128);
        assert_eq!(nearest_pow2(300), 512);
    }

    #[test]
    fn full_charset_is_deduped() {
        let cs = full_charset();
        let unique: HashSet<char> = cs.iter().copied().collect();
        assert_eq!(cs.len(), unique.len());
        assert!(cs.contains(&'A'));
        assert!(cs.contains(&' '));
        assert!(cs.contains(&'α')); // greek
    }

    #[test]
    fn space_uses_fixed_advance() {
        // Space must not fall back to the engine's `glyph_size * 0.5` (32) —
        // a missing space glyph visibly widens space-heavy text.
        assert_eq!(glyph_advance(' ', 100.0, 12.0), SPACE_ADVANCE);
        assert_eq!(glyph_advance(' ', 0.0, 12.0), SPACE_ADVANCE);
        assert_eq!(SPACE_ADVANCE, 7.168);
        assert_eq!(SPACE_ADVANCE, FONT_CELL_SIZE * 0.28);
    }

    #[test]
    fn non_space_uses_scaled_advance() {
        assert_eq!(glyph_advance('A', 210.0, 12.0), 17.5);
        assert_eq!(glyph_advance('\u{00a0}', 100.0, 12.0), 100.0 / 12.0);
    }
}
