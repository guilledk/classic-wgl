//! Procedural terrain generation.
//!
//! Pure, GL-free, allocation-light and fully deterministic: everything here is
//! a function of a seed string, so results are reproducible across platforms
//! (including `wasm32`) and stable for golden traces.
//!
//! - [`fractal`] — multi-octave combinators over `simplex_noise`.
//! - [`material`] — the lunar material table shared by generator and tileset.
//! - [`lunar`] — the lunar surface generator.
//! - [`tileset`] — the matching procedurally painted tile sheet.
//! - [`types`] — the generic generated-terrain contract.

pub mod fractal;
pub mod lunar;
pub mod material;
pub mod noise_fields;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_lunar_produces_valid_terrain() {
        let gen = generate("lunar", "apollo").unwrap();
        let (sx, sy) = (gen.terrain.size_x as usize, gen.terrain.size_y as usize);
        assert_eq!(gen.terrain.tiles.len(), sx * sy);
        assert_eq!(gen.terrain.heights.len(), (sx + 1) * (sy + 1));
        assert_eq!(gen.terrain.nav.len(), sx * sy);
        assert!(gen.tileset.width > 0 && gen.tileset.height > 0);
        assert!(!gen.tileset.rgba.is_empty());
        assert!(gen.nav_slope_threshold > 0.0);
    }

    #[test]
    fn generate_unknown_kind_returns_none() {
        assert!(generate("bogus", "x").is_none());
    }
}
