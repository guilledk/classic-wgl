//! Raised floors: axis-aligned tile rects whose ground stands at a fixed
//! height (`set_floor_rect`), for decks that sit above the terrain (a landing
//! pad).  Inside a rect, what stands on the ground (vehicle wheel contacts and
//! so the body height, pitch and roll, their dust, `sprite_fit_terrain`) takes
//! `max(terrain, z)`; the terrain itself, its rendering and pathfinding are
//! untouched, and every `frame_offset` altitude stays relative to the terrain
//! (the sprite model re-adds the terrain height).
//!
//! Rects are axis-aligned in tiles: the pad, the one user, is square and
//! tile-aligned, and a rotated deck can be covered by several rects.

use crate::Engine;

/// One raised floor: tiles `[x0, x1] × [y0, y1]` (edges included) at world
/// height `z` (metres, the terrain's unit).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FloorRect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub z: f32,
}

impl FloorRect {
    /// Whether tile point `(x, y)` lies on this floor.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x0 && x <= self.x1 && y >= self.y0 && y <= self.y1
    }
}

/// The ground under tile point `(x, y)` over `terrain`: the highest floor
/// containing it, else the terrain.
pub(crate) fn ground_height(floors: &[FloorRect], terrain: f32, x: f32, y: f32) -> f32 {
    floors.iter().filter(|f| f.contains(x, y)).fold(terrain, |h, f| h.max(f.z))
}

impl Engine {
    /// Set (or replace) raised floor `id`: tiles `[x0, x1] × [y0, y1]` (either
    /// corner order) whose ground stands at world height `z_m`.  `false` for
    /// a non-finite value.
    pub fn set_floor_rect(
        &mut self,
        id: i32,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        z_m: f32,
    ) -> bool {
        if ![x0, y0, x1, y1, z_m].iter().all(|v| v.is_finite()) {
            return false;
        }
        let rect =
            FloorRect { x0: x0.min(x1), y0: y0.min(y1), x1: x0.max(x1), y1: y0.max(y1), z: z_m };
        self.floor_rects.insert(id, rect);
        true
    }

    /// Remove raised floor `id`; `false` when there is none.
    pub fn clear_floor_rect(&mut self, id: i32) -> bool {
        self.floor_rects.remove(&id).is_some()
    }

    /// The raised floors, in id order.
    pub(crate) fn floor_list(&self) -> Vec<FloorRect> {
        self.floor_rects.values().copied().collect()
    }

    /// The ground height (world metres) at tile point `(x, y)`: the terrain,
    /// raised to any floor rect over it.  What things standing on the ground
    /// rest on; [`Engine::height_at`] stays the bare terrain.
    pub fn ground_at(&self, x: f32, y: f32) -> f32 {
        let terrain = self.height_at(x, y);
        if self.floor_rects.is_empty() {
            return terrain;
        }
        ground_height(&self.floor_list(), terrain, x, y)
    }
}
