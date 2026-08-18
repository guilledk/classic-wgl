//! Procedural lunar tileset generator.
//!
//! Paints an RGBA tile sheet in memory from the [`MATERIALS`] table, so the
//! terrain generator and its texture can never drift apart and no binary asset
//! has to be committed to the (separate, private) `assets/` submodule.
//!
//! Register the result with `Gfx::add_texture_rgba8`.
//!
//! # Seamlessness
//!
//! The tilemap fragment shader samples each map tile with `fract()`, so
//! wherever two tiles of the same id meet, the left edge of the cell abuts its
//! own right edge.  Every noise lookup here therefore goes through
//! [`tiling_fbm_2d`], which is periodic by construction, and microcraters use
//! toroidal distance.  Non-periodic noise would show a hard grid of seams
//! across every flat region.

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use crate::material::{material_for_tile_id, tile_count, MaterialSpec, MATERIALS};
use classic_terrain::fractal::tiling_fbm_2d;
use classic_terrain::simplex_noise::{Random, SimplexNoise};

/// Default tileset geometry: an 8x8 grid of 32px cells, i.e. 256x256, which
/// matches `tilePixelSize [32, 32]` in the scene description.
pub const DEFAULT_TILE_PX: u32 = 32;
pub const DEFAULT_COLS: u32 = 8;
pub const DEFAULT_ROWS: u32 = 8;

/// Build the lunar tileset.  Returns `(rgba, width, height)`.
///
/// Panics if the material table does not fit in `cols * rows` cells (id `0` is
/// reserved and never painted with material data).
pub fn build_lunar_tileset(seed: &str, tile_px: u32, cols: u32, rows: u32) -> (Vec<u8>, u32, u32) {
    assert!(
        tile_count() < cols * rows,
        "lunar material table ({} tiles + reserved id 0) does not fit in a {cols}x{rows} tileset",
        tile_count()
    );

    let w = cols * tile_px;
    let h = rows * tile_px;
    let mut rgba = vec![0u8; (w * h * 4) as usize];

    // Fill every cell (including unused ones) with a neutral regolith base so
    // an out-of-range tile id degrades into plain ground rather than garbage.
    let fallback = MATERIALS[2];
    for row in 0..rows {
        for col in 0..cols {
            paint_cell(&mut rgba, w, tile_px, col, row, &fallback, seed, 0, u32::MAX);
        }
    }

    for spec in MATERIALS {
        for variant in 0..spec.variants {
            let id = crate::material::tile_id(spec.material, variant);
            let col = id % cols;
            let row = id / cols;
            paint_cell(&mut rgba, w, tile_px, col, row, spec, seed, variant, id);
        }
    }

    (rgba, w, h)
}

/// Convenience wrapper using the default 256x256 / 32px geometry.
pub fn build_default_lunar_tileset(seed: &str) -> (Vec<u8>, u32, u32) {
    build_lunar_tileset(seed, DEFAULT_TILE_PX, DEFAULT_COLS, DEFAULT_ROWS)
}

/// The `tileSetSize` uniform value (tiles per row / column) for a tileset of
/// the given pixel geometry.
pub fn tile_set_size(tile_px: u32, w: u32, h: u32) -> [f32; 2] {
    [w as f32 / tile_px as f32, h as f32 / tile_px as f32]
}

#[allow(clippy::too_many_arguments)]
fn paint_cell(
    rgba: &mut [u8],
    img_w: u32,
    tile_px: u32,
    col: u32,
    row: u32,
    spec: &MaterialSpec,
    seed: &str,
    variant: u32,
    id: u32,
) {
    let noise = SimplexNoise::new(&format!("{seed}:tex:{id}:{variant}"));
    let period = tile_px as f64;
    let tau = core::f64::consts::TAU;
    // `tiling_noise_2d` sweeps an arc of 2*pi*radius per period, so dividing
    // by tau makes `radius` read directly as "cycles across one tile".
    let fine_r = spec.speckle_freq / tau;
    let coarse_r = 1.0 / tau;

    let craterlets = make_craterlets(spec, tile_px, seed, id);

    for py in 0..tile_px {
        for px in 0..tile_px {
            let u = px as f64;
            let v = py as f64;

            // Two scales: fine grain plus a broad mottle so large same-tile
            // regions do not read as a uniform wash.
            let fine = tiling_fbm_2d(&noise, u, v, period, 3, fine_r);
            let coarse = tiling_fbm_2d(&noise, u + 11.0, v - 7.0, period, 2, coarse_r);
            let mut shade = fine as f32 * spec.speckle + coarse as f32 * spec.speckle * 0.6;

            shade += craterlet_shade(&craterlets, px as f32, py as f32, tile_px as f32);

            let x = col * tile_px + px;
            let y = row * tile_px + py;
            let o = ((y * img_w + x) * 4) as usize;
            for c in 0..3 {
                rgba[o + c] = (spec.albedo[c] as f32 + shade).clamp(0.0, 255.0) as u8;
            }
            rgba[o + 3] = 255;
        }
    }
}

#[derive(Clone, Copy)]
struct Craterlet {
    x: f32,
    y: f32,
    r: f32,
    depth: f32,
}

fn make_craterlets(spec: &MaterialSpec, tile_px: u32, seed: &str, id: u32) -> Vec<Craterlet> {
    let mut rng = Random::from_seed_str(&format!("{seed}:craterlet:{id}"));
    let mut out = Vec::with_capacity(spec.craterlets as usize);
    for _ in 0..spec.craterlets {
        out.push(Craterlet {
            x: rng.next_f64() as f32 * tile_px as f32,
            y: rng.next_f64() as f32 * tile_px as f32,
            r: 1.2 + rng.next_f64() as f32 * (tile_px as f32 * 0.10),
            depth: 0.45 + rng.next_f64() as f32 * 0.55,
        });
    }
    out
}

/// Shading contribution of the microcraters at one pixel: a dark pit with a
/// bright rim, evaluated with toroidal distance so craters near a cell edge
/// wrap around instead of being clipped.
fn craterlet_shade(craterlets: &[Craterlet], px: f32, py: f32, tile_px: f32) -> f32 {
    let mut acc = 0f32;
    for c in craterlets {
        let mut dx = (px - c.x).abs();
        let mut dy = (py - c.y).abs();
        if dx > tile_px * 0.5 {
            dx = tile_px - dx;
        }
        if dy > tile_px * 0.5 {
            dy = tile_px - dy;
        }
        let d = libm::sqrtf(dx * dx + dy * dy);
        if d < c.r {
            // Pit: darkest at the centre.
            acc -= 26.0 * c.depth * (1.0 - d / c.r);
        } else if d < c.r * 1.45 {
            // Rim: freshly excavated material is brighter than its
            // surroundings, which is what makes small craters legible.
            let t = (d - c.r) / (c.r * 0.45);
            acc += 20.0 * c.depth * (1.0 - t);
        }
    }
    acc
}

/// Look up which material a generated tile id maps to.  Re-exported here so
/// callers of the tileset do not have to reach into the material module.
pub fn tile_material(id: u32) -> Option<crate::material::LunarMaterial> {
    material_for_tile_id(id).map(|(m, _)| m)
}
