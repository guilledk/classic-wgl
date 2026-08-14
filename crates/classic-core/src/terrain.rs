//! Re-export of the [`classic_terrain`] crate (see that crate's docs).
//!
//! The terrain generator and noise primitives moved into the `#![no_std]`
//! `classic-terrain` crate so ROM guests can link them; this module keeps
//! `classic_core::terrain::*` working for the host engine and demo.

pub use classic_terrain::fractal;
pub use classic_terrain::lunar;
pub use classic_terrain::material;
pub use classic_terrain::noise_fields;
pub use classic_terrain::tileset;
pub use classic_terrain::types;

pub use classic_terrain::{
    build_default_lunar_tileset, build_lunar_tileset, generate, generate_lunar, GeneratedTerrain,
    LandingZone, LunarMaterial, LunarParams, LunarStats, LunarTerrain, Terrain, Tileset,
};
