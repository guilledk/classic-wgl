//! SDF font glyph-buffer builder.
//!
//! Port of `SdfText::_buildGlyphBuffer()` from `src/classic/sdfText.ts:189-311`.

use crate::components::TextJustify;
use crate::types::SdfFontMetrics;

/// One interleaved vertex pushed to the GPU buffer.
/// Layout: `{local_x, local_y, tex_u, tex_v}` — stride = 16 bytes.
/// `local_x`/`local_y` are in [0..1] space (normalized by `text_width`/`text_height`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct SdfGlyphVertex {
    pub local_x: f32,
    pub local_y: f32,
    pub tex_u: f32,
    pub tex_v: f32,
}

unsafe impl bytemuck::Zeroable for SdfGlyphVertex {}
unsafe impl bytemuck::Pod for SdfGlyphVertex {}

const VERTS_PER_GLYPH: usize = 6;

/// The result of building a glyph buffer from a text string.
pub struct SdfGlyphBuffer {
    /// Interleaved `SdfGlyphVertex` array (length = `glyph_count * 6`).
    pub vertices: Vec<SdfGlyphVertex>,
    /// Total text bounding-box width (world units, after scale).
    pub text_width: f32,
    /// Total text bounding-box height (world units, after scale).
    pub text_height: f32,
    /// Number of vertices in `vertices`.
    pub vertex_count: usize,
}

/// Advance width for a character (pixels × scale).
fn advance_for(metrics: &SdfFontMetrics, ch: char, scale: f32) -> f32 {
    let ch_str = ch.to_string();
    if let Some(g) = metrics.glyphs.get(&ch_str) {
        return g.x_advance * scale;
    }
    if ch == ' ' {
        return metrics.glyphs.get(" ").map(|g| g.x_advance).unwrap_or(metrics.glyph_size * 0.5)
            * scale;
    }
    if ch == '\t' {
        return metrics
            .glyphs
            .get(" ")
            .map(|g| g.x_advance * 4.0)
            .unwrap_or(metrics.glyph_size * 2.0)
            * scale;
    }
    metrics.glyph_size * 0.5 * scale
}

/// Build an SDF glyph vertex buffer from a text string.
///
/// `layout_width` is the column width used for justify (center/right).
/// Pass `0` to use the maximal line width.
pub fn build_sdf_glyph_buffer(
    metrics: &SdfFontMetrics,
    text: &str,
    scale: f32,
    justify: TextJustify,
    layout_width: f32,
) -> SdfGlyphBuffer {
    let atlas_w = metrics.atlas_size[0];
    let atlas_h = metrics.atlas_size[1];
    let max_h = metrics.line_height * scale;
    let margin = ((metrics.spread + 2.0) + 2.0) * scale;

    // Phase 1 — per-line layout
    struct PlacedGlyph {
        char: char,
        x: f32,
        y: i32, // line index
        adv: f32,
    }

    let mut per_line: Vec<PlacedGlyph> = Vec::new();
    let mut line_index: i32 = 0;
    let mut max_width: f32 = 0.0;

    for line in text.split('\n') {
        let mut line_x: f32 = 0.0;
        for ch in line.chars() {
            let adv = advance_for(metrics, ch, scale);
            let glyph = metrics.glyphs.get(&ch.to_string());
            if let Some(g) = glyph {
                per_line.push(PlacedGlyph {
                    char: ch,
                    x: line_x + g.x_offset * scale,
                    y: line_index,
                    adv,
                });
            }
            line_x += adv;
        }
        if line_x > max_width {
            max_width = line_x;
        }
        line_index += 1;
    }

    // Phase 2 — justify
    if justify != TextJustify::Left {
        let column_w = if layout_width > 0.0 { layout_width } else { max_width };
        let mut line_widths: std::collections::HashMap<i32, f32> = std::collections::HashMap::new();
        for pg in &per_line {
            *line_widths.entry(pg.y).or_insert(0.0) += pg.adv;
        }
        for pg in &mut per_line {
            let lw = line_widths.get(&pg.y).copied().unwrap_or(1.0).max(1.0);
            let extra = column_w - lw;
            if justify == TextJustify::Center {
                pg.x += extra / 2.0;
            } else if justify == TextJustify::Right {
                pg.x += extra;
            }
        }
    }

    let text_w =
        if justify != TextJustify::Left && layout_width > 0.0 { layout_width } else { max_width };
    let mut text_h = max_h * (line_index.max(1) as f32);

    // Phase 3 — compute glyph-extent height
    let mut glyph_extent_min = f32::INFINITY;
    let mut glyph_extent_max = f32::NEG_INFINITY;
    for pg in &per_line {
        let g = match metrics.glyphs.get(&pg.char.to_string()) {
            Some(g) => g,
            None => continue,
        };
        let gy = metrics.baseline * scale
            + g.y_offset * scale
            + pg.y as f32 * metrics.line_height * scale;
        if gy < glyph_extent_min {
            glyph_extent_min = gy;
        }
        if gy + g.h * scale > glyph_extent_max {
            glyph_extent_max = gy + g.h * scale;
        }
    }
    if glyph_extent_min < glyph_extent_max {
        // Pad text-height so the visual centre of the glyph row
        // coincides with the element's geometric centre at ch/2.
        text_h = (glyph_extent_min + glyph_extent_max).max(1.0);
    }

    let _layout_h = (text_h - 2.0 * margin).max(1.0);

    // Phase 4 — build interleaved vertex buffer
    let total_verts = per_line.len() * VERTS_PER_GLYPH;
    let mut verts: Vec<SdfGlyphVertex> = Vec::with_capacity(total_verts);

    let tw = text_w.max(1.0);
    let th = text_h.max(1.0);

    for pg in &per_line {
        let g = match metrics.glyphs.get(&pg.char.to_string()) {
            Some(g) => g,
            None => continue,
        };
        let gx = pg.x;
        let gy = metrics.baseline * scale
            + g.y_offset * scale
            + pg.y as f32 * metrics.line_height * scale;
        let gw = g.w * scale;
        let gh = g.h * scale;

        let lx0 = gx / tw;
        let lx1 = (gx + gw) / tw;
        let ly0 = gy / th;
        let ly1 = (gy + gh) / th;

        let ux0 = g.x / atlas_w;
        let ux1 = (g.x + g.w) / atlas_w;
        let uy0 = g.y / atlas_h;
        let uy1 = (g.y + g.h) / atlas_h;

        // Triangle 0: (lx0,ly0)→(lx1,ly0)→(lx1,ly1)
        verts.push(SdfGlyphVertex { local_x: lx0, local_y: ly0, tex_u: ux0, tex_v: uy0 });
        verts.push(SdfGlyphVertex { local_x: lx1, local_y: ly0, tex_u: ux1, tex_v: uy0 });
        verts.push(SdfGlyphVertex { local_x: lx1, local_y: ly1, tex_u: ux1, tex_v: uy1 });

        // Triangle 1: (lx0,ly0)→(lx1,ly1)→(lx0,ly1)
        verts.push(SdfGlyphVertex { local_x: lx0, local_y: ly0, tex_u: ux0, tex_v: uy0 });
        verts.push(SdfGlyphVertex { local_x: lx1, local_y: ly1, tex_u: ux1, tex_v: uy1 });
        verts.push(SdfGlyphVertex { local_x: lx0, local_y: ly1, tex_u: ux0, tex_v: uy1 });
    }

    SdfGlyphBuffer {
        vertex_count: verts.len(),
        vertices: verts,
        text_width: text_w,
        text_height: text_h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_metrics() -> SdfFontMetrics {
        SdfFontMetrics {
            name: "test".into(),
            family: "test".into(),
            atlas_size: [512.0, 512.0],
            glyph_size: 64.0,
            spread: 4.0,
            baseline: 20.0,
            line_height: 33.28,
            glyphs: [
                (
                    "A".into(),
                    crate::types::GlyphMetrics {
                        x: 10.0,
                        y: 20.0,
                        w: 30.0,
                        h: 40.0,
                        x_offset: -2.0,
                        y_offset: -18.0,
                        x_advance: 26.0,
                    },
                ),
                (
                    "B".into(),
                    crate::types::GlyphMetrics {
                        x: 50.0,
                        y: 20.0,
                        w: 28.0,
                        h: 40.0,
                        x_offset: -2.0,
                        y_offset: -18.0,
                        x_advance: 24.0,
                    },
                ),
            ]
            .into_iter()
            .collect(),
        }
    }

    #[test]
    fn builds_two_glyphs() {
        let m = sample_metrics();
        let buf = build_sdf_glyph_buffer(&m, "AB", 1.0, TextJustify::Left, 0.0);
        assert_eq!(buf.vertex_count, 12); // 2 glyphs × 6 verts
        assert!(buf.text_width > 0.0);
        assert!(buf.text_height > 0.0);
    }

    #[test]
    fn empty_string_returns_zero_verts() {
        let m = sample_metrics();
        let buf = build_sdf_glyph_buffer(&m, "", 1.0, TextJustify::Left, 0.0);
        assert_eq!(buf.vertex_count, 0);
    }

    #[test]
    fn missing_glyphs_skipped() {
        let m = sample_metrics();
        let buf = build_sdf_glyph_buffer(&m, "AXB", 1.0, TextJustify::Left, 0.0);
        // 'X' has no metrics → skipped, only 'A' and 'B' → 12 verts
        assert_eq!(buf.vertex_count, 12);
    }
}
