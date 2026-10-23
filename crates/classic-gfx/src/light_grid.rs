//! Screen-tile light culling ("forward+ lite") for the lit fragment shaders.
//!
//! Every lit fragment used to loop over every light in the `LightBlock` UBO.
//! Instead, once per frame the CPU projects each light's sphere to screen
//! space (the iso camera is orthographic, so the projection is affine and a
//! sphere maps to exactly a filled ellipse), bins it into the
//! [`LIGHT_TILE_PX`]-pixel tiles of the main target it touches, and uploads
//! the per-tile light lists as one `R32UI` texture.  `sheet.frag`, `iso_tilemap.frag` and
//! `mesh.frag` look up their tile from `gl_FragCoord` (the `LIGHT GRID` block,
//! `evaluateLightsTiled`) and loop over that list only.
//!
//! ## Layout (declared once, here; the shader block mirrors these constants)
//!
//! The texture is a flat array of `u32` words, [`LIGHT_GRID_ROW`] per row
//! (word `k` lives at texel `(k % ROW, k / ROW)`):
//!
//! ```text
//! word t, t < tiles_x * tiles_y : tile header  offset << COUNT_BITS | count
//!                                (tile t = ty * tiles_x + tx, ty = 0 at the
//!                                 bottom, like gl_FragCoord)
//! word offset ..                : the tile's light indices, one byte each,
//!                                four per word, low byte first, ascending
//! ```
//!
//! `count == LIGHT_GRID_OVERFLOW` means "no list": the tile has more than
//! [`LIGHT_TILE_CAP`] lights (or the texture ran out of rows) and the shader
//! runs the full loop there.  Lights are never dropped.
//!
//! ## Why it is exact
//!
//! A fragment's lighting position `vLightPos` is the world point its screen
//! position was rasterised from (terrain, models and iso sprites all project
//! `world` through the same `clip_from_world`), so a light whose sphere
//! reaches a fragment has that fragment's screen position inside the
//! sphere's projected rect.  Lists keep UBO order, so the per-fragment sum
//! adds the same non-zero terms in the same order as the full loop; the
//! lights a list omits contribute exactly 0 (the radius early-out of the
//! shared lighting block).
//!
//! The grid is in the **main target's** pixels.  The model pass renders into a
//! smaller target; its fragments map back through
//! [`LightGridGpu::map_for_target`], which scales `gl_FragCoord` by the target
//! ratio before dividing by the tile size.

use classic_core::components::Light;
use glam::{Mat4, Vec3, Vec4};
use glow::HasContext;

use crate::MAX_LIGHTS;

/// Edge of a screen tile in main-target pixels.
pub const LIGHT_TILE_PX: u32 = 32;

/// Most lights one tile lists; a tile reached by more falls back to the full
/// loop (it is dense anyway).  Bounds the per-frame upload to
/// `tiles * (1 + LIGHT_TILE_CAP / 4)` words.
pub const LIGHT_TILE_CAP: usize = 64;

/// Words per texture row (`LIGHT_GRID_ROW` in the shader).
pub const LIGHT_GRID_ROW: u32 = 1024;

/// `log2(LIGHT_GRID_ROW)` (`LIGHT_GRID_ROW_SHIFT` in the shader).
pub const LIGHT_GRID_ROW_SHIFT: u32 = 10;

/// Low bits of a tile header holding the count (`LIGHT_GRID_COUNT_BITS`).
pub const LIGHT_GRID_COUNT_BITS: u32 = 9;

/// The "no list, run the full loop" count (`LIGHT_GRID_OVERFLOW`).
pub const LIGHT_GRID_OVERFLOW: u32 = (1 << LIGHT_GRID_COUNT_BITS) - 1;

/// Texture unit the grid binds to (sprites 0-2, shadow 3, model composite 4-5,
/// particles 6, bloom 7-8, smoke noise 9, half-res composite 10).
pub const LIGHT_GRID_UNIT: u32 = 11;

/// Extra margin around every projected light rect, in main-target pixels:
/// absorbs float error between the CPU projection and the rasteriser.
const LIGHT_RECT_MARGIN_PX: f32 = 1.0;

// Light indices are stored as bytes.
const _: () = assert!(MAX_LIGHTS <= 256);
// A capped count must stay below the overflow sentinel.
const _: () = assert!(LIGHT_TILE_CAP < LIGHT_GRID_OVERFLOW as usize);
const _: () = assert!(1 << LIGHT_GRID_ROW_SHIFT == LIGHT_GRID_ROW);

/// `world (metres) -> clip`, exactly as `iso_tilemap.vert` / `mesh.vert` /
/// `direct_tex.vert` rasterise: `view = world_matrix · world`, screen pixels
/// `(view.x · ppm, −view.y · ppm)`, then `projection · camera`.
pub fn clip_from_world(projection: &Mat4, camera: &Mat4, world_matrix: &Mat4, ppm: f32) -> Mat4 {
    let to_screen = Mat4::from_cols(
        Vec4::new(ppm, 0.0, 0.0, 0.0),
        Vec4::new(0.0, -ppm, 0.0, 0.0),
        Vec4::ZERO,
        Vec4::W,
    );
    *projection * *camera * to_screen * *world_matrix
}

/// The affine `world -> main-target pixel` map the grid bins in.
#[derive(Clone, Copy, Debug)]
pub struct LightGridView {
    /// Rows `x`, `y` of `world -> pixel` (`xyz` linear part, `w` offset);
    /// pixel `y` grows upward like `gl_FragCoord`.
    rows: [Vec4; 2],
    width: u32,
    height: u32,
}

impl LightGridView {
    /// `None` unless `clip_from_world` is affine (an orthographic camera: the
    /// clip `w` row is `(0, 0, 0, 1)`) and the target is non-empty, in which
    /// case the grid stays off and every fragment runs the full loop.
    pub fn new(clip_from_world: &Mat4, width: u32, height: u32) -> Option<Self> {
        let w_row = clip_from_world.row(3);
        let affine = w_row.truncate().abs().max_element() <= 1e-6 && (w_row.w - 1.0).abs() <= 1e-6;
        if !affine || width == 0 || height == 0 || !clip_from_world.is_finite() {
            return None;
        }
        // window = (ndc * 0.5 + 0.5) * size
        let row = |r: Vec4, size: u32| {
            let s = 0.5 * size as f32;
            Vec4::new(r.x * s, r.y * s, r.z * s, r.w * s + s)
        };
        Some(Self {
            rows: [row(clip_from_world.row(0), width), row(clip_from_world.row(1), height)],
            width,
            height,
        })
    }

    /// The main-target pixel a world point rasterises at.
    pub fn pixel(&self, world: Vec3) -> [f32; 2] {
        let p = world.extend(1.0);
        [self.rows[0].dot(p), self.rows[1].dot(p)]
    }

    /// Grid size in tiles.
    pub fn tiles(&self) -> (u32, u32) {
        (self.width.div_ceil(LIGHT_TILE_PX), self.height.div_ceil(LIGHT_TILE_PX))
    }

    /// The inclusive tile rect `[x0, y0, x1, y1]` a light can reach, clipped to
    /// the grid; `None` when it is entirely off screen.  A light without a
    /// finite positive radius reaches everywhere.
    pub fn tile_rect(&self, light: &Light) -> Option<[u32; 4]> {
        let (tx, ty) = self.tiles();
        let all = Some([0, 0, tx - 1, ty - 1]);
        let r = light.radius;
        if !(r > 0.0 && r.is_finite()) {
            return all;
        }
        let c = self.pixel(light.position);
        let mut out = [0u32; 4];
        for axis in 0..2 {
            // An affine map sends a ball of radius r to an ellipse whose
            // bounding half-extent on this axis is r * |linear row|.
            let half = r * self.rows[axis].truncate().length() + LIGHT_RECT_MARGIN_PX;
            let (lo, hi) = (c[axis] - half, c[axis] + half);
            let tiles = if axis == 0 { tx } else { ty };
            let size = (tiles * LIGHT_TILE_PX) as f32;
            if !(hi >= 0.0 && lo < size) {
                return None; // off screen (or NaN)
            }
            let t = LIGHT_TILE_PX as f32;
            out[axis] = (lo.max(0.0) / t) as u32;
            out[axis + 2] = ((hi / t) as u32).min(tiles - 1);
        }
        Some(out)
    }

    /// The light's projected ball: an affine map sends the ball of radius
    /// `r` about `position` to exactly the filled ellipse `(x − c)ᵀ (A Aᵀ)⁻¹
    /// (x − c) ≤ r²` (`A` = the linear part of `world -> pixel`), and every
    /// fragment the light reaches rasterises inside it.  `None` for a light
    /// that reaches everywhere or a degenerate map (the rect alone decides).
    pub fn footprint(&self, light: &Light) -> Option<Footprint> {
        let r = light.radius as f64;
        if !(r > 0.0 && r.is_finite()) {
            return None;
        }
        let [a, b] = self.rows.map(|row| row.truncate().as_dvec3());
        // M = A Aᵀ (2x2, symmetric) and its inverse.
        let (m00, m01, m11) = (a.dot(a), a.dot(b), b.dot(b));
        let det = m00 * m11 - m01 * m01;
        if !det.is_finite() || det <= 1e-12 * m00 * m11 {
            return None;
        }
        let c = self.pixel(light.position);
        Some(Footprint {
            c: [c[0] as f64, c[1] as f64],
            inv: [m11 / det, -m01 / det, m00 / det],
            r2: r * r,
        })
    }
}

/// A light's screen footprint ([`LightGridView::footprint`]), in main-target
/// pixels.
#[derive(Clone, Copy, Debug)]
pub struct Footprint {
    c: [f64; 2],
    /// `(A Aᵀ)⁻¹` as `[xx, xy, yy]`.
    inv: [f64; 3],
    r2: f64,
}

impl Footprint {
    /// `(x − c)ᵀ (A Aᵀ)⁻¹ (x − c)` for an offset `(dx, dy)` from the centre.
    fn q(&self, dx: f64, dy: f64) -> f64 {
        let [xx, xy, yy] = self.inv;
        xx * dx * dx + 2.0 * xy * dx * dy + yy * dy * dy
    }

    /// Whether the ellipse touches tile `(tx, ty)`, grown by the rect test's
    /// pixel margin: the minimum of the (convex) quadratic over the box is
    /// 0 with the centre inside, else attained on one of its four edges.
    pub fn touches_tile(&self, tx: u32, ty: u32) -> bool {
        let t = LIGHT_TILE_PX as f64;
        let m = LIGHT_RECT_MARGIN_PX as f64;
        let (x0, x1) = (tx as f64 * t - m - self.c[0], (tx + 1) as f64 * t + m - self.c[0]);
        let (y0, y1) = (ty as f64 * t - m - self.c[1], (ty + 1) as f64 * t + m - self.c[1]);
        if x0 <= 0.0 && 0.0 <= x1 && y0 <= 0.0 && 0.0 <= y1 {
            return true;
        }
        let [xx, xy, yy] = self.inv;
        // On an edge x = const the minimiser is y = −xy·x / yy, clamped to
        // the edge (and symmetrically for y = const).
        let edge_x = |x: f64| self.q(x, (-xy * x / yy).clamp(y0, y1));
        let edge_y = |y: f64| self.q((-xy * y / xx).clamp(x0, x1), y);
        let min = edge_x(x0).min(edge_x(x1)).min(edge_y(y0)).min(edge_y(y1));
        min <= self.r2
    }

    /// The tiles of row `ty` the ellipse touches (by [`Self::touches_tile`]'s
    /// rule), as an inclusive `[x0, x1]` within `0..tiles_x`: the ellipse's
    /// x-extent over the row's (margin-grown) band, in closed form.  A box
    /// meets a convex set iff their x-ranges over the box's band overlap.
    pub fn row_span(&self, ty: u32, tiles_x: u32) -> Option<(u32, u32)> {
        let t = LIGHT_TILE_PX as f64;
        let m = LIGHT_RECT_MARGIN_PX as f64;
        let [xx, xy, yy] = self.inv;
        // Half-extents: x_max = r·sqrt(M00), y_max = r·sqrt(M11), with
        // M = inv⁻¹ = [yy, −xy; −xy, xx] / det(inv).
        let det = xx * yy - xy * xy;
        let (mx, my) = (yy / det, xx / det);
        let (rx, ry) = ((self.r2 * mx).sqrt(), (self.r2 * my).sqrt());
        // The band, relative to the centre, clipped to the ellipse's y-range.
        let y0 = (ty as f64 * t - m - self.c[1]).max(-ry);
        let y1 = ((ty + 1) as f64 * t + m - self.c[1]).min(ry);
        if y0 > y1 {
            return None;
        }
        // The section at height y: x = (−xy·y ± sqrt(xy²y² − xx(yy·y² − r²))) / xx.
        let section = |y: f64| {
            let disc = (xy * xy * y * y - xx * (yy * y * y - self.r2)).max(0.0);
            let (mid, half) = (-xy * y / xx, disc.sqrt() / xx);
            (mid - half, mid + half)
        };
        let (lo0, hi0) = section(y0);
        let (lo1, hi1) = section(y1);
        let (mut lo, mut hi) = (lo0.min(lo1), hi0.max(hi1));
        // The extreme points (where the tangent is vertical) sit at
        // y = −xy·x / yy for x = ±rx: inside the band, they bound it.
        let y_at = |x: f64| -xy * x / yy;
        if (y0..=y1).contains(&y_at(rx)) {
            hi = rx;
        }
        if (y0..=y1).contains(&y_at(-rx)) {
            lo = -rx;
        }
        // Tiles whose grown x-range meets [lo, hi]; a hair of slack keeps
        // round-off on the conservative side.
        let eps = 1e-6;
        // Tile x meets it iff x·t − m <= hi and (x + 1)·t + m >= lo.
        let x_lo = ((lo + self.c[0] - m - eps) / t).ceil() - 1.0;
        let x_hi = ((hi + self.c[0] + m + eps) / t).floor();
        let last = tiles_x as f64 - 1.0;
        let (a, b) = (x_lo.max(0.0), x_hi.min(last));
        (a <= b).then_some((a as u32, b as u32))
    }
}

/// One frame's binned light lists, in the texture's word layout.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LightGrid {
    pub tiles_x: u32,
    pub tiles_y: u32,
    /// Headers (`tiles_x * tiles_y` words) then the packed index lists.
    pub words: Vec<u32>,
}

impl LightGrid {
    /// Bin the first [`MAX_LIGHTS`] of `lights` (the ones `pack_lights`
    /// uploads, same indices) into `view`'s tiles.  `max_words` caps the
    /// texture: a tile whose list no longer fits overflows to the full loop.
    pub fn build(lights: &[Light], view: &LightGridView, max_words: usize) -> Self {
        let (tiles_x, tiles_y) = view.tiles();
        let n_tiles = (tiles_x * tiles_y) as usize;
        let lights = &lights[..lights.len().min(MAX_LIGHTS)];
        // Every light's tiles, in light (UBO) order: the tiles of its rect
        // that its projected ball (an exact ellipse) touches.
        let mut hits: Vec<u32> = Vec::new();
        let mut spans: Vec<(usize, usize)> = Vec::with_capacity(lights.len());
        for light in lights {
            let start = hits.len();
            if let Some(r) = view.tile_rect(light) {
                let fp = view.footprint(light);
                for y in r[1]..=r[3] {
                    let (x0, x1) = match &fp {
                        Some(fp) => match fp.row_span(y, tiles_x) {
                            Some((a, b)) => (a.max(r[0]), b.min(r[2])),
                            None => continue,
                        },
                        None => (r[0], r[2]),
                    };
                    hits.extend((x0..=x1).map(|x| y * tiles_x + x));
                }
            }
            spans.push((start, hits.len()));
        }

        // Pass 1: count per tile.
        let mut counts = vec![0u32; n_tiles];
        for &t in &hits {
            counts[t as usize] += 1;
        }

        // Headers: place each list, or mark the tile overflowed.
        let mut words = vec![0u32; n_tiles];
        let mut cursor = vec![0usize; n_tiles]; // next byte slot, in bytes
        let mut next = n_tiles;
        for (t, &count) in counts.iter().enumerate() {
            let len = (count as usize).div_ceil(4);
            if count as usize > LIGHT_TILE_CAP || next + len > max_words {
                words[t] = LIGHT_GRID_OVERFLOW;
                continue;
            }
            words[t] = ((next as u32) << LIGHT_GRID_COUNT_BITS) | count;
            cursor[t] = next * 4;
            next += len;
        }
        words.resize(next, 0);

        // Pass 2: write indices in UBO order, so every list is ascending.
        for (i, &(start, end)) in spans.iter().enumerate() {
            for &t in &hits[start..end] {
                let t = t as usize;
                if words[t] & LIGHT_GRID_OVERFLOW == LIGHT_GRID_OVERFLOW {
                    continue;
                }
                let slot = cursor[t];
                words[slot / 4] |= (i as u32) << ((slot % 4) * 8);
                cursor[t] = slot + 1;
            }
        }
        Self { tiles_x, tiles_y, words }
    }

    /// Profiler counters (`CLASSIC_PERF`): `grid_tiles`, `grid_lit_tiles`
    /// (tiles with a list of at least one light), `grid_entries` (sum of the
    /// list lengths), `grid_max` (longest list) and `grid_overflow` (tiles
    /// on the full loop).
    pub fn count_stats(&self) {
        let n = (self.tiles_x * self.tiles_y) as usize;
        let (mut lit, mut entries, mut max, mut overflow) = (0u64, 0u64, 0u64, 0u64);
        for &h in &self.words[..n] {
            let count = (h & LIGHT_GRID_OVERFLOW) as u64;
            if count == LIGHT_GRID_OVERFLOW as u64 {
                overflow += 1;
            } else if count > 0 {
                lit += 1;
                entries += count;
                max = max.max(count);
            }
        }
        crate::perf::count("grid_tiles", n as u64);
        crate::perf::count("grid_lit_tiles", lit);
        crate::perf::count("grid_entries", entries);
        crate::perf::count("grid_max", max);
        crate::perf::count("grid_overflow", overflow);
    }

    /// CPU reference of the shader lookup: the light indices tile
    /// `(tx, ty)` evaluates, or `None` when it runs the full loop.
    pub fn tile_lights(&self, tx: u32, ty: u32) -> Option<Vec<usize>> {
        let header = self.words[(ty * self.tiles_x + tx) as usize];
        let count = header & LIGHT_GRID_OVERFLOW;
        if count == LIGHT_GRID_OVERFLOW {
            return None;
        }
        let base = (header >> LIGHT_GRID_COUNT_BITS) as usize;
        Some(
            (0..count as usize)
                .map(|j| ((self.words[base + j / 4] >> ((j % 4) * 8)) & 0xFF) as usize)
                .collect(),
        )
    }
}

/// The GPU side: the `R32UI` grid texture plus the state the lit draws bind.
pub struct LightGridGpu {
    /// A ring of [`crate::light::FRAME_RING`] textures, one written per
    /// upload, so a frame never rewrites the grid an earlier frame may still
    /// be sampling; `cur` is the one bound for this frame's draws.
    textures: [glow::Texture; crate::light::FRAME_RING],
    /// Allocated rows of each texture.
    rows: [u32; crate::light::FRAME_RING],
    cur: usize,
    max_rows: u32,
    /// `None` while the grid is off (no lights, a non-orthographic camera,
    /// or no build since the last light upload): the shaders run the full loop.
    tiles: Option<(u32, u32)>,
    /// The main target size the grid was binned for.
    main: (u32, u32),
    /// The `clip_from_world` it was binned with: a draw whose own transform
    /// differs (another camera) runs the full loop instead.
    clip: Mat4,
}

impl LightGridGpu {
    /// Create the grid texture (one row, all zero) and leave it bound to
    /// [`LIGHT_GRID_UNIT`], so the `usampler2D` always samples an integer
    /// texture (WebGL rejects a draw whose sampler type mismatches its unit).
    pub fn new(gl: &glow::Context) -> Self {
        let textures = std::array::from_fn(|_| {
            unsafe { gl.create_texture() }.expect("create light grid texture")
        });
        let max_rows = unsafe { gl.get_parameter_i32(glow::MAX_TEXTURE_SIZE) }.max(2048) as u32;
        let mut grid = Self {
            textures,
            rows: [0; crate::light::FRAME_RING],
            cur: 0,
            max_rows,
            tiles: None,
            main: (1, 1),
            clip: Mat4::ZERO,
        };
        // Allocate every ring texture (the last one stays bound).
        for i in (0..crate::light::FRAME_RING).rev() {
            grid.cur = i;
            grid.store(gl, &[0]);
            grid.set_params(gl);
        }
        unsafe { gl.active_texture(glow::TEXTURE0) };
        grid
    }

    /// Integer-texture sampling state of the texture bound on
    /// [`LIGHT_GRID_UNIT`] (set once per ring texture).
    fn set_params(&self, gl: &glow::Context) {
        unsafe {
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::NEAREST as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::NEAREST as i32);
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );
        }
    }

    /// Most words the texture can hold.
    pub fn max_words(&self) -> usize {
        (self.max_rows * LIGHT_GRID_ROW) as usize
    }

    /// Turn the grid off until the next [`LightGridGpu::upload`]: called when
    /// the light UBO changes, so a stale grid never indexes new lights.
    pub fn invalidate(&mut self) {
        self.tiles = None;
    }

    /// Upload a frame's grid (or turn it off with `None`).
    pub fn upload(&mut self, gl: &glow::Context, grid: Option<(&LightGrid, (u32, u32), Mat4)>) {
        let Some((grid, main, clip)) = grid else {
            self.tiles = None;
            return;
        };
        self.cur = (self.cur + 1) % crate::light::FRAME_RING;
        self.store(gl, &grid.words);
        unsafe { gl.active_texture(glow::TEXTURE0) };
        self.tiles = Some((grid.tiles_x, grid.tiles_y));
        self.main = main;
        self.clip = clip;
    }

    /// Write `words` (padded to whole rows) into the texture, growing it when
    /// needed; leaves it bound on [`LIGHT_GRID_UNIT`].
    fn store(&mut self, gl: &glow::Context, words: &[u32]) {
        let rows = (words.len() as u32).div_ceil(LIGHT_GRID_ROW).max(1);
        let mut padded;
        let data = if words.len() as u32 == rows * LIGHT_GRID_ROW {
            words
        } else {
            padded = Vec::with_capacity((rows * LIGHT_GRID_ROW) as usize);
            padded.extend_from_slice(words);
            padded.resize((rows * LIGHT_GRID_ROW) as usize, 0);
            &padded[..]
        };
        let bytes: &[u8] = bytemuck::cast_slice(data);
        unsafe {
            gl.active_texture(glow::TEXTURE0 + LIGHT_GRID_UNIT);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.textures[self.cur]));
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
            if rows > self.rows[self.cur] {
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::R32UI as i32,
                    LIGHT_GRID_ROW as i32,
                    rows as i32,
                    0,
                    glow::RED_INTEGER,
                    glow::UNSIGNED_INT,
                    glow::PixelUnpackData::Slice(Some(bytes)),
                );
                self.rows[self.cur] = rows;
            } else {
                gl.tex_sub_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    0,
                    0,
                    LIGHT_GRID_ROW as i32,
                    rows as i32,
                    glow::RED_INTEGER,
                    glow::UNSIGNED_INT,
                    glow::PixelUnpackData::Slice(Some(bytes)),
                );
            }
        }
    }

    /// The `light_grid_map` uniform for a draw with transform `clip` into a
    /// `target`-sized framebuffer: `gl_FragCoord` -> main-target pixel -> tile,
    /// plus the grid size.  `None` (full loop) while the grid is off or when
    /// the draw does not rasterise with the transform the grid was binned for.
    pub fn map_for(&self, clip: &Mat4, target: (u32, u32)) -> Option<[f32; 4]> {
        let tiles = self.tiles?;
        (*clip == self.clip).then(|| grid_map(self.main, target, tiles))
    }

    /// Bind the grid texture on [`LIGHT_GRID_UNIT`] (it stays there; rebinding
    /// per draw keeps it robust to code that touches that unit).
    pub fn bind(&self, gl: &glow::Context) {
        unsafe {
            gl.active_texture(glow::TEXTURE0 + LIGHT_GRID_UNIT);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.textures[self.cur]));
            gl.active_texture(glow::TEXTURE0);
        }
    }
}

/// `light_grid_map` for a `target`-sized framebuffer over a grid binned for a
/// `main`-sized target: `xy` scales `gl_FragCoord` to main pixels then tiles,
/// `zw` is the grid size.
pub fn grid_map(main: (u32, u32), target: (u32, u32), tiles: (u32, u32)) -> [f32; 4] {
    let t = LIGHT_TILE_PX as f32;
    let sx = main.0 as f32 / target.0.max(1) as f32 / t;
    let sy = main.1 as f32 / target.1.max(1) as f32 / t;
    [sx, sy, tiles.0 as f32, tiles.1 as f32]
}

#[cfg(test)]
#[path = "light_grid_tests.rs"]
mod tests;
