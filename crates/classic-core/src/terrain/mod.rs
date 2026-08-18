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

pub mod fractal;
pub mod lunar;
pub mod material;
pub mod tileset;

pub use lunar::{generate_lunar, LandingZone, LunarParams, LunarStats, LunarTerrain};
pub use material::LunarMaterial;
pub use tileset::{build_default_lunar_tileset, build_lunar_tileset};
