//! Procedural terrain generation + the noise primitives behind it.
//!
//! `#![no_std]` (with `alloc`) so it can compile into ROM guest `.wasm`
//! modules as well as the native host.  Everything here is a pure function of
//! `(seed, dims, params)` — no system clock, no GL — so output is reproducible
//! across targets and stable for golden traces.
//!
//! Shared by the host (which re-exports it as `classic_core::terrain` /
//! `classic_core::simplex_noise` and exposes the [`noise_fields`] bulk API to
//! guests) and by ROM guests (which link it to generate custom maps).
//!
//! - [`simplex_noise`] — seedable 2D/3D/4D simplex + a deterministic [`Random`].
//! - [`fractal`] — multi-octave combinators over `simplex_noise`.
//! - [`material`] — the lunar material table.
//! - [`lunar`] — the lunar surface generator.
//! - [`tileset`] — the matching procedurally painted tile sheet.
//! - [`noise_fields`] — bulk field-fill helpers behind the host SDK.
//! - [`types`] — the generic generated-terrain contract.

#![no_std]

extern crate alloc;

use alloc::format;
use alloc::string::ToString;

pub mod fractal;
pub mod lunar;
pub mod material;
pub mod noise_fields;
pub mod simplex_noise;
pub mod tileset;
pub mod types;

pub use lunar::{generate_lunar, LandingZone, LunarParams, LunarStats, LunarTerrain};
pub use material::LunarMaterial;
pub use tileset::{build_default_lunar_tileset, build_lunar_tileset};
pub use types::{GeneratedTerrain, Terrain, Tileset};

/// Generate a named terrain and its tileset.
///
/// This is the generic entry point ROM guests use via the `generate_terrain`
/// SDK import: `kind` selects a generator, `seed` re-rolls it.  Unknown kinds
/// return `None`.  Add a new generator by adding a match arm here (its
/// `Terrain`/`Tileset` contract is all the engine needs).
pub fn generate(kind: &str, seed: &str) -> Option<GeneratedTerrain> {
    match kind {
        "lunar" => {
            let params = LunarParams { seed: seed.to_string(), ..LunarParams::default() };
            let t = generate_lunar(&params);
            let (rgba, width, height) = build_lunar_tileset(
                &format!("{seed}:tileset"),
                32,
                tileset::DEFAULT_COLS,
                tileset::DEFAULT_ROWS,
            );
            Some(GeneratedTerrain {
                terrain: Terrain {
                    size_x: t.size_x,
                    size_y: t.size_y,
                    tiles: t.tiles,
                    heights: t.heights,
                    nav: t.nav,
                },
                tileset: Tileset { rgba, width, height },
                nav_slope_threshold: params.nav_max_slope,
            })
        }
        _ => None,
    }
}
