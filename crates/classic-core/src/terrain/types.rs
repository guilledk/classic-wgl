//! The generic generated-terrain contract.
//!
//! Any terrain generator produces a [`Terrain`] (tile/heights/nav grids) plus a
//! procedurally painted [`Tileset`]; [`GeneratedTerrain`] bundles them for the
//! engine's install/regenerate prefabs.  The `lunar` generator (see
//! [`crate::terrain::generate`]) is the first implementation.

/// A generated terrain's grids.
///
/// - `tiles` is `size_x * size_y`, material ids always `>= 1`.
/// - `heights` is the *vertex* grid, `(size_x + 1) * (size_y + 1)`.
/// - `nav` is `size_x * size_y`, `1` = walkable, `0` = blocked.
#[derive(Clone, Debug)]
pub struct Terrain {
    pub size_x: i32,
    pub size_y: i32,
    pub tiles: Vec<u32>,
    pub heights: Vec<f32>,
    pub nav: Vec<u32>,
}

/// A procedurally painted tile sheet (RGBA pixels).
#[derive(Clone, Debug)]
pub struct Tileset {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// A generated terrain plus its tileset, ready to install into the engine.
#[derive(Clone, Debug)]
pub struct GeneratedTerrain {
    pub terrain: Terrain,
    pub tileset: Tileset,
    /// Slope above which `sync_nav_heights` marks a tile impassable — a
    /// per-generator property the engine records when installing.
    pub nav_slope_threshold: f32,
}
