//! A* pathfinding over a 2D grid of walkable/blocked cells, plus the
//! footprint-, slope- and jump-aware vehicle search.
//!
//! `#![no_std]` (with `alloc`) so the same code compiles into the host
//! (`classic-core` re-exports this as `pathfinder`), the native worker thread,
//! and the web `pathfinder.wasm` worker module — a single source of truth for
//! native + web routes.  Transcendental functions use `libm` so results are
//! reproducible across targets (the same rationale as `classic-terrain`).
//!
//! Port of the algorithm from `src/classic/pathfinder.ts` (web worker).  The
//! nav mesh is a flat `[i32]` where `1` = walkable, `0` = blocked.

#![no_std]

extern crate alloc;

use alloc::collections::BinaryHeap;
use alloc::vec;
use alloc::vec::Vec;

/// Ordered float for priority queue keys.
#[derive(Copy, Clone, PartialEq)]
struct Key {
    cost: f32,
    cell: (i32, i32),
}

impl Eq for Key {}
impl PartialOrd for Key {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Key {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        // Reverse: lower cost = higher priority.
        other.cost.total_cmp(&self.cost).then_with(|| self.cell.cmp(&other.cell))
    }
}

/// A single cell coordinate on the nav grid.
pub type GridCell = (i32, i32);

/// Core 8-directional A* over a `size_x × size_y` grid.  `step_cost(current,
/// neighbor)` returns `Some(cost)` when the move is allowed and `None` when
/// blocked.  Neighbours are the 8 surrounding cells (bounds-checked here).
///
/// Returns the full path (including `from` and `to`) or `None` when no route
/// exists.  The heuristic is admissible for 8-directional grids with cardinal
/// cost 1.0 and diagonal cost √2; callers that add a cost multiplier (e.g. a
/// jump penalty) must keep it ≥ 1.0 so the heuristic stays admissible.
fn a_star<F>(
    size_x: i32,
    size_y: i32,
    from: GridCell,
    to: GridCell,
    mut step_cost: F,
) -> Option<Vec<GridCell>>
where
    F: FnMut(GridCell, GridCell) -> Option<f32>,
{
    let from = (from.0.clamp(0, size_x - 1), from.1.clamp(0, size_y - 1));
    let to = (to.0.clamp(0, size_x - 1), to.1.clamp(0, size_y - 1));

    let flatten = |x: i32, y: i32| -> usize { (x + y * size_x) as usize };

    if from == to {
        return Some(vec![from, to]);
    }

    let total = (size_x * size_y) as usize;
    let inf: f32 = f32::MAX;

    let mut g_cost = vec![inf; total];
    let mut f_cost = vec![inf; total];
    let mut came_from: Vec<Option<usize>> = vec![None; total];
    let mut in_open = vec![false; total];

    let from_idx = flatten(from.0, from.1);

    g_cost[from_idx] = 0.0;
    f_cost[from_idx] = heuristic(from, to);

    let mut open = BinaryHeap::new();
    open.push(Key { cost: f_cost[from_idx], cell: from });

    let neighbours: [(i32, i32); 8] =
        [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];

    while let Some(Key { cell: current, .. }) = open.pop() {
        let cur_idx = flatten(current.0, current.1);

        if current == to {
            return Some(reconstruct_path(&came_from, &g_cost, &f_cost, from, to, size_x));
        }

        in_open[cur_idx] = false;

        for &(dx, dy) in &neighbours {
            let nx = current.0 + dx;
            let ny = current.1 + dy;

            if nx < 0 || nx >= size_x || ny < 0 || ny >= size_y {
                continue;
            }
            let n_idx = flatten(nx, ny);
            let Some(step_cost) = step_cost(current, (nx, ny)) else { continue };

            let tentative_g = g_cost[cur_idx].min(inf) + step_cost;

            if tentative_g < g_cost[n_idx] {
                came_from[n_idx] = Some(cur_idx);
                g_cost[n_idx] = tentative_g;
                f_cost[n_idx] = tentative_g + heuristic((nx, ny), to);

                if !in_open[n_idx] {
                    open.push(Key { cost: f_cost[n_idx], cell: (nx, ny) });
                    in_open[n_idx] = true;
                }
            }
        }
    }

    None
}

/// Run 8-directional A* from `from` to `to` on a grid of `size_x × size_y`
/// cells.  `nav_data[i]` must be 1 (walkable) or 0 (blocked).
///
/// Returns the full path (including `from` and `to`) as vec of integer
/// cell coordinates, or `None` if no path exists.
pub fn find_path(
    nav_data: &[i32],
    size_x: i32,
    size_y: i32,
    from: GridCell,
    to: GridCell,
) -> Option<Vec<GridCell>> {
    a_star(size_x, size_y, from, to, |current, neighbour| {
        let idx = (neighbour.0 + neighbour.1 * size_x) as usize;
        if nav_data[idx] == 0 {
            return None;
        }
        Some(if neighbour.0 != current.0 && neighbour.1 != current.1 {
            libm::sqrtf(2.0)
        } else {
            1.0
        })
    })
}

/// Erode a walkability grid by a footprint of integer tile offsets: a cell is
/// passable in the output only when every footprint offset, relative to that
/// cell, is walkable in the input.
///
/// This is the standard "obstacle inflation" pass for multi-tile agents — the
/// agent is treated as a point on the eroded grid, so the existing 1×1 A* runs
/// unchanged.  `footprint` is a slice of `(dx, dy)` offsets from the anchor
/// cell; a 1×1 agent uses `&[(0, 0)]`, a 3×3 vehicle uses all 9 offsets in
/// `-1..=1 × -1..=1`.
pub fn erode_for_footprint(
    nav_data: &[i32],
    size_x: i32,
    size_y: i32,
    footprint: &[GridCell],
) -> Vec<i32> {
    let total = (size_x * size_y) as usize;
    let mut eroded = vec![1_i32; total];

    for y in 0..size_y {
        for x in 0..size_x {
            let mut passable = true;
            for &(ox, oy) in footprint {
                let cx = x + ox;
                let cy = y + oy;
                if cx < 0 || cx >= size_x || cy < 0 || cy >= size_y {
                    passable = false;
                    break;
                }
                let idx = (cx + cy * size_x) as usize;
                if nav_data[idx] == 0 {
                    passable = false;
                    break;
                }
            }
            eroded[(x + y * size_x) as usize] = if passable { 1 } else { 0 };
        }
    }

    eroded
}

/// Run footprint-aware A*: erode `nav_data` by `footprint` (see
/// [`erode_for_footprint`]) then run the standard 1×1 search on the result.
///
/// The `from` cell is exempted from the erosion check so an already-placed
/// vehicle can path *out* of a spot whose footprint barely overlaps an
/// obstacle; the goal must still be reachable with the full footprint.
pub fn find_path_for_footprint(
    nav_data: &[i32],
    size_x: i32,
    size_y: i32,
    from: GridCell,
    to: GridCell,
    footprint: &[GridCell],
) -> Option<Vec<GridCell>> {
    let mut eroded = erode_for_footprint(nav_data, size_x, size_y, footprint);
    // Relax the start: the agent is already there, so only neighbour cells are
    // held to the footprint rule during expansion.
    let from_idx = (from.0.clamp(0, size_x - 1) + from.1.clamp(0, size_y - 1) * size_x) as usize;
    if let Some(cell) = eroded.get_mut(from_idx) {
        *cell = 1;
    }
    find_path(&eroded, size_x, size_y, from, to)
}

/// Bilinear interpolation of height data at an iso-space position (px, py).
///
/// `heights` has shape `(size_x + 1) × (size_y + 1)` (one sample per edge vertex).
pub fn bilinear_height(heights: &[f32], size_x: i32, size_y: i32, px: f32, py: f32) -> f32 {
    // A generated map has no height grid until its guest uploads one, so
    // callers that sample terrain every frame must tolerate an empty (or
    // not-yet-committed) grid.  Treat it as flat (height 0) rather than
    // indexing out of bounds.
    if heights.len() != (size_x as usize + 1) * (size_y as usize + 1) {
        return 0.0;
    }

    let ftx = libm::floorf(px) as i32;
    let fty = libm::floorf(py) as i32;
    let fx = px - ftx as f32;
    let fy = py - fty as f32;

    let at = |tx: i32, ty: i32| -> f32 {
        let tx = tx.clamp(0, size_x) as usize;
        let ty = ty.clamp(0, size_y) as usize;
        heights[ty * (size_x as usize + 1) + tx]
    };

    let h_nw = at(ftx, fty);
    let h_ne = at(ftx + 1, fty);
    let h_sw = at(ftx, fty + 1);
    let h_se = at(ftx + 1, fty + 1);

    h_nw + (h_ne - h_nw) * fx + (h_sw - h_nw) * fy + (h_nw - h_ne - h_sw + h_se) * fx * fy
}

/// Derive a vehicle-specific slope-feasibility walkability grid from a vertex
/// height grid: `1` where the vehicle can stand (i.e. *some* heading keeps its
/// pitch/roll within limits), `0` otherwise.
///
/// Unlike a per-tile gradient (which flags a flat tile sitting next to a cliff
/// because one corner is on the cliff top), this samples the terrain at the
/// vehicle's wheel positions — `±wheelbase/2` front-rear and `±track/2`
/// left-right — in each of the 8 headings, using the same bilinear surface the
/// simulation drives on.  A tile is walkable when at least one heading keeps
/// `|pitch| ≤ max_pitch` and `|roll| ≤ max_roll`.
#[allow(clippy::too_many_arguments)]
pub fn derive_vehicle_slope_nav(
    heights: &[f32],
    size_x: i32,
    size_y: i32,
    height_scale: f32,
    tile_scale: f32,
    wheelbase_px: f32,
    track_px: f32,
    max_pitch: f32,
    max_roll: f32,
) -> Vec<i32> {
    let mut out = vec![0i32; (size_x * size_y) as usize];
    if wheelbase_px <= 0.0 || track_px <= 0.0 {
        return out;
    }
    let half_wb = (wheelbase_px / tile_scale.max(1e-6)) * 0.5;
    let half_tr = (track_px / tile_scale.max(1e-6)) * 0.5;
    let max_pitch_tan = libm::tanf(max_pitch);
    let max_roll_tan = libm::tanf(max_roll);

    for y in 0..size_y {
        for x in 0..size_x {
            let cx = x as f32 + 0.5;
            let cy = y as f32 + 0.5;
            let mut walkable = false;
            for d in 0..8 {
                let angle = d as f32 * core::f32::consts::FRAC_PI_4;
                let (hx, hy) = (libm::cosf(angle), libm::sinf(angle));
                let (lx, ly) = (-hy, hx);
                let front =
                    bilinear_height(heights, size_x, size_y, cx + hx * half_wb, cy + hy * half_wb);
                let rear =
                    bilinear_height(heights, size_x, size_y, cx - hx * half_wb, cy - hy * half_wb);
                let left =
                    bilinear_height(heights, size_x, size_y, cx + lx * half_tr, cy + ly * half_tr);
                let right =
                    bilinear_height(heights, size_x, size_y, cx - lx * half_tr, cy - ly * half_tr);
                let pitch = (front - rear).abs() * height_scale / wheelbase_px;
                let roll = (left - right).abs() * height_scale / track_px;
                if pitch <= max_pitch_tan && roll <= max_roll_tan {
                    walkable = true;
                    break;
                }
            }
            out[(y * size_x + x) as usize] = walkable as i32;
        }
    }
    out
}

/// Footprint-, slope- and jump-aware A* for a wheeled vehicle.
///
/// `walkable` is the combined grid (`structural` AND slope-feasible): `1` = a
/// normal walk.  `structural` is obstacle-free walkability, used only to check
/// that a jump's landing zone is not inside an obstacle.  `heights` is the
/// vertex grid `(size_x+1) × (size_y+1)`.  A *downward* step whose drop (in
/// pixels, `drop_units · height_scale`) is within `safe_fall_px` is allowed as
/// a jump even when `walkable` marks the target blocked (a small cliff the
/// suspension can absorb), provided the target is lower and obstacle-free; its
/// cost is scaled by `jump_cost` (≥ 1.0 discourages jumps).  `safe_fall_px ≤ 0`
/// disables jumps.
#[allow(clippy::too_many_arguments)]
pub fn find_path_for_footprint_with_jumps(
    walkable: &[i32],
    structural: &[i32],
    heights: &[f32],
    size_x: i32,
    size_y: i32,
    from: GridCell,
    to: GridCell,
    footprint: &[GridCell],
    height_scale: f32,
    safe_fall_px: f32,
    jump_cost: f32,
) -> Option<Vec<GridCell>> {
    let mut walk_eroded = erode_for_footprint(walkable, size_x, size_y, footprint);
    let struct_eroded = if safe_fall_px > 0.0 {
        erode_for_footprint(structural, size_x, size_y, footprint)
    } else {
        Vec::new()
    };
    // Relax the start: the agent is already there, so only neighbour cells are
    // held to the footprint rule during expansion.
    let from_idx = (from.0.clamp(0, size_x - 1) + from.1.clamp(0, size_y - 1) * size_x) as usize;
    if let Some(cell) = walk_eroded.get_mut(from_idx) {
        *cell = 1;
    }

    let vw = size_x as usize + 1;
    let cell_height = |x: i32, y: i32| -> f32 {
        let i = y as usize * vw + x as usize;
        (heights[i] + heights[i + 1] + heights[i + vw] + heights[i + vw + 1]) * 0.25
    };

    a_star(size_x, size_y, from, to, |current, neighbour| {
        let idx = (neighbour.0 + neighbour.1 * size_x) as usize;
        let diagonal = neighbour.0 != current.0 && neighbour.1 != current.1;
        let base = if diagonal { libm::sqrtf(2.0) } else { 1.0 };

        if walk_eroded[idx] == 1 {
            return Some(base);
        }
        if safe_fall_px > 0.0 && struct_eroded[idx] == 1 {
            let drop = (cell_height(current.0, current.1) - cell_height(neighbour.0, neighbour.1))
                * height_scale;
            if drop > 0.0 && drop <= safe_fall_px {
                return Some(base * jump_cost);
            }
        }
        None
    })
}

/// A snapshot of the terrain the vehicle pathfinder needs: the structural nav
/// grid (obstacle-free walkability) plus the vertex height grid it derives the
/// slope-feasibility grid from.
#[derive(Clone, Debug, PartialEq)]
pub struct VehicleNavSnapshot {
    pub size_x: i32,
    pub size_y: i32,
    /// Obstacle-free walkability (`1` = open, `0` = blocked), row-major.
    pub structural: Vec<i32>,
    /// Vertex height grid `(size_x + 1) × (size_y + 1)`, row-major.
    pub heights: Vec<f32>,
    /// Vertical world units per height unit (the tilemap `height_scale`).
    pub height_scale: f32,
    /// World size of a tile edge (the tilemap `scale.x`), used to convert the
    /// pixel wheelbase/track back into tile units for slope sampling.
    pub tile_scale: f32,
}

impl VehicleNavSnapshot {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        size_x: i32,
        size_y: i32,
        structural: Vec<i32>,
        heights: Vec<f32>,
        height_scale: f32,
        tile_scale: f32,
    ) -> Self {
        Self { size_x, size_y, structural, heights, height_scale, tile_scale }
    }
}

/// Footprint-, slope- and jump-aware vehicle A* over a [`VehicleNavSnapshot`].
///
/// Derives the slope-feasibility grid from `snapshot.heights`, combines it with
/// `snapshot.structural`, and runs [`find_path_for_footprint_with_jumps`].  The
/// worker-callable equivalent of `Engine::find_vehicle_path` (see
/// `classic-engine`), so native + web produce identical routes.
#[allow(clippy::too_many_arguments)]
pub fn find_vehicle_path_snapshot(
    snapshot: &VehicleNavSnapshot,
    from: GridCell,
    to: GridCell,
    footprint: &[GridCell],
    pitch_max: f32,
    roll_max: f32,
    wheelbase_px: f32,
    track_px: f32,
    safe_fall_px: f32,
    jump_cost: f32,
) -> Option<Vec<GridCell>> {
    let slope = derive_vehicle_slope_nav(
        &snapshot.heights,
        snapshot.size_x,
        snapshot.size_y,
        snapshot.height_scale,
        snapshot.tile_scale,
        wheelbase_px,
        track_px,
        pitch_max,
        roll_max,
    );
    find_vehicle_path_with_slope(snapshot, &slope, from, to, footprint, safe_fall_px, jump_cost)
}

/// Vehicle A* reusing a pre-derived slope-feasibility grid (`slope`, the output
/// of [`derive_vehicle_slope_nav`]).  The worker caches `slope` per
/// `(pitch, roll, wheelbase, track)` to avoid re-deriving it per request.
#[allow(clippy::too_many_arguments)]
pub fn find_vehicle_path_with_slope(
    snapshot: &VehicleNavSnapshot,
    slope: &[i32],
    from: GridCell,
    to: GridCell,
    footprint: &[GridCell],
    safe_fall_px: f32,
    jump_cost: f32,
) -> Option<Vec<GridCell>> {
    let walkable: Vec<i32> =
        snapshot.structural.iter().zip(slope.iter()).map(|(&s, &w)| s & w).collect();
    find_path_for_footprint_with_jumps(
        &walkable,
        &snapshot.structural,
        &snapshot.heights,
        snapshot.size_x,
        snapshot.size_y,
        from,
        to,
        footprint,
        snapshot.height_scale,
        safe_fall_px,
        jump_cost,
    )
}

/// Chebyshev-approximation heuristic (admissible for 8-directional grid).
/// Returns the estimate of cost from `a` to `b`.
fn heuristic(a: GridCell, b: GridCell) -> f32 {
    let dx = (a.0 - b.0).abs() as f32;
    let dy = (a.1 - b.1).abs() as f32;
    dx + dy + (libm::sqrtf(2.0) - 2.0) * dx.min(dy)
}

/// Walk backwards from `to` through `came_from` to rebuild the path.
fn reconstruct_path(
    came_from: &[Option<usize>],
    _g_cost: &[f32],
    _f_cost: &[f32],
    from: GridCell,
    to: GridCell,
    size_x: i32,
) -> Vec<GridCell> {
    let flatten = |x: i32, y: i32| -> usize { (x + y * size_x) as usize };
    let from_idx = flatten(from.0, from.1);
    let to_idx = flatten(to.0, to.1);

    let mut path = vec![to];
    let mut cur = to_idx;
    while cur != from_idx {
        match came_from[cur] {
            Some(prev) => {
                let px = (prev % size_x as usize) as i32;
                let py = (prev / size_x as usize) as i32;
                path.push((px, py));
                cur = prev;
            }
            None => break,
        }
    }
    path.reverse();
    path
}

/// The outcome of polling an asynchronous path request.
///
/// `Pending` means the search has not finished (or the id is unknown/consumed);
/// `Path` carries a found route (inclusive of both endpoints); `NoPath` means
/// the search finished but found no route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathPoll {
    Pending,
    Path(Vec<GridCell>),
    NoPath,
}

/// An immutable snapshot of the nav grid, safe to share across threads.
///
/// A `PathfinderWorker` (see `classic-worker`) owns an `Arc<NavSnapshot>` so a
/// background thread can run A* over a consistent copy of the walkability grid
/// without touching the engine.  Rebuilt (and re-shared) whenever the nav grid
/// changes (see `Engine::commit_terrain` / `set_nav_bulk` / `sync_nav_heights`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavSnapshot {
    pub size_x: i32,
    pub size_y: i32,
    /// Row-major walkability grid (`1` = walkable, `0` = blocked).
    pub data: Vec<i32>,
}

impl NavSnapshot {
    pub fn new(size_x: i32, size_y: i32, data: Vec<i32>) -> Self {
        Self { size_x, size_y, data }
    }

    /// Run A* over this snapshot (see [`find_path`]).
    pub fn find_path(&self, from: GridCell, to: GridCell) -> Option<Vec<GridCell>> {
        let cells = (self.size_x * self.size_y) as usize;
        if self.size_x <= 0 || self.size_y <= 0 || self.data.len() != cells {
            return None;
        }
        find_path(&self.data, self.size_x, self.size_y, from, to)
    }
}

/// A stateful pathfinder: the nav + vehicle snapshots plus the cached derived
/// slope grid.  Shared by the native worker thread and the wasm worker module so
/// both run identical searches with identical caching and invalidation.
pub struct PathfinderState {
    nav: NavSnapshot,
    vehicle: Option<VehicleNavSnapshot>,
    vehicle_slope_cache: Option<([u32; 4], Vec<i32>)>,
}

impl PathfinderState {
    pub fn new(nav: NavSnapshot) -> Self {
        Self { nav, vehicle: None, vehicle_slope_cache: None }
    }

    /// Replace the nav snapshot searched by [`Self::find`].
    pub fn set_nav(&mut self, nav: NavSnapshot) {
        self.nav = nav;
    }

    /// Replace the vehicle snapshot; clears the cached slope grid (the terrain
    /// changed).
    pub fn set_vehicle(&mut self, vehicle: VehicleNavSnapshot) {
        self.vehicle = Some(vehicle);
        self.vehicle_slope_cache = None;
    }

    /// Run plain humanoid A* over the current nav snapshot.
    pub fn find(&self, from: GridCell, to: GridCell) -> Option<Vec<GridCell>> {
        self.nav.find_path(from, to)
    }

    /// Run footprint-, slope- and jump-aware vehicle A* over the current vehicle
    /// snapshot, caching the derived slope grid per `(pitch, roll, wheelbase,
    /// track)`.
    #[allow(clippy::too_many_arguments)]
    pub fn find_vehicle(
        &mut self,
        from: GridCell,
        to: GridCell,
        footprint: &[GridCell],
        pitch_max: f32,
        roll_max: f32,
        wheelbase_px: f32,
        track_px: f32,
        safe_fall_px: f32,
        jump_cost: f32,
    ) -> Option<Vec<GridCell>> {
        let snap = self.vehicle.as_ref()?;
        let key =
            [pitch_max.to_bits(), roll_max.to_bits(), wheelbase_px.to_bits(), track_px.to_bits()];
        let slope: Vec<i32> = match self
            .vehicle_slope_cache
            .as_ref()
            .and_then(|(k, grid)| (*k == key).then(|| grid.clone()))
        {
            Some(grid) => grid,
            None => {
                let grid = derive_vehicle_slope_nav(
                    &snap.heights,
                    snap.size_x,
                    snap.size_y,
                    snap.height_scale,
                    snap.tile_scale,
                    wheelbase_px,
                    track_px,
                    pitch_max,
                    roll_max,
                );
                self.vehicle_slope_cache = Some((key, grid.clone()));
                grid
            }
        };
        find_vehicle_path_with_slope(snap, &slope, from, to, footprint, safe_fall_px, jump_cost)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn simple_grid() -> Vec<i32> {
        // 5x5 all walkable
        vec![1; 25]
    }

    #[test]
    fn straight_line_path() {
        let grid = simple_grid();
        let path = find_path(&grid, 5, 5, (0, 0), (4, 0)).expect("path");
        assert_eq!(path.len(), 5);
        assert_eq!(path[0], (0, 0));
        assert_eq!(path[path.len() - 1], (4, 0));
    }

    #[test]
    fn diagonal_path() {
        let grid = simple_grid();
        let path = find_path(&grid, 5, 5, (0, 0), (4, 4)).expect("path");
        assert_eq!(path[0], (0, 0));
        assert_eq!(path[path.len() - 1], (4, 4));
        // Should take diagonal (straight-line) route.
        for (i, &(x, y)) in path.iter().enumerate() {
            assert_eq!(x, y, "diagonal path step {i} should be on diagonal");
        }
    }

    #[test]
    fn blocked_no_path() {
        // 3x3 with a wall in the middle column
        let mut grid = vec![1_i32; 9];
        grid[1] = 0; // (1,0)
        grid[4] = 0; // (1,1)
        grid[7] = 0; // (1,2)
        let path = find_path(&grid, 3, 3, (0, 0), (2, 0));
        assert!(path.is_none(), "should be no path through wall");
    }

    #[test]
    fn navigate_around_wall() {
        // 3x3 with a single blocked cell at center
        let mut grid = vec![1_i32; 9];
        grid[4] = 0; // (1,1) blocked
        let path = find_path(&grid, 3, 3, (0, 0), (2, 2)).expect("path");
        assert_eq!(path[0], (0, 0));
        assert_eq!(path[path.len() - 1], (2, 2));
        // Path must go around center.
        for &(x, y) in &path {
            assert!(!(x == 1 && y == 1), "path must not pass through blocked cell");
        }
    }

    #[test]
    fn start_equals_goal() {
        let grid = simple_grid();
        let path = find_path(&grid, 5, 5, (2, 2), (2, 2)).expect("path");
        assert_eq!(path, vec![(2, 2), (2, 2)]);
    }

    fn footprint_3x3() -> Vec<GridCell> {
        let mut fp = Vec::new();
        for dy in -1..=1 {
            for dx in -1..=1 {
                fp.push((dx, dy));
            }
        }
        fp
    }

    #[test]
    fn footprint_point_equals_plain_a_star() {
        let mut grid = simple_grid();
        grid[6] = 0; // (1,1) blocked
        let plain = find_path(&grid, 5, 5, (0, 0), (4, 4)).unwrap();
        let foot = find_path_for_footprint(&grid, 5, 5, (0, 0), (4, 4), &[(0, 0)]).unwrap();
        assert_eq!(plain, foot);
    }

    #[test]
    fn three_wide_corridor_lets_3x3_through() {
        // 7x7 grid: block everything left of column 1 and right of column 3,
        // leaving an exactly-3-wide vertical corridor (columns 1..=3) that a
        // 3x3 vehicle fits through along x=2.
        let w = 7;
        let h = 7;
        let mut grid = vec![1_i32; (w * h) as usize];
        for y in 0..h {
            grid[(y * w) as usize] = 0;
            grid[(4 + y * w) as usize] = 0;
            grid[(5 + y * w) as usize] = 0;
            grid[(6 + y * w) as usize] = 0;
        }
        let fp = footprint_3x3();
        let path = find_path_for_footprint(&grid, w, h, (2, 1), (2, 5), &fp);
        assert!(path.is_some(), "3x3 should path a 3-wide corridor");
    }

    #[test]
    fn two_wide_corridor_blocks_3x3() {
        // Same wall layout but a 2-wide corridor (columns 2..=3) — a 3x3
        // vehicle cannot fit, so no path exists.
        let w = 7;
        let h = 7;
        let mut grid = vec![1_i32; (w * h) as usize];
        for y in 0..h {
            grid[(y * w) as usize] = 0;
            grid[(1 + y * w) as usize] = 0;
            grid[(4 + y * w) as usize] = 0;
            grid[(5 + y * w) as usize] = 0;
            grid[(6 + y * w) as usize] = 0;
        }
        let fp = footprint_3x3();
        let path = find_path_for_footprint(&grid, w, h, (2, 0), (3, 6), &fp);
        assert!(path.is_none(), "3x3 must not path a 2-wide corridor");
    }

    #[test]
    fn footprint_blocks_diagonal_corner_cut() {
        // A 1x1 agent can cut the diagonal gap between two blocked cells; a 3x3
        // footprint cannot (it would overlap a blocked cell).
        let w: i32 = 5;
        let h: i32 = 5;
        let mut grid = vec![1_i32; (w * h) as usize];
        grid[w as usize] = 0; // (0,1) blocked
        grid[1] = 0; // (1,0) blocked

        // 1x1 can squeeze diagonally from (0,0) to (2,2).
        assert!(find_path(&grid, w, h, (0, 0), (2, 2)).is_some());
        // 3x3 anchored at (0,0) overlaps blocked cells and cannot.
        let fp = footprint_3x3();
        assert!(find_path_for_footprint(&grid, w, h, (0, 0), (2, 2), &fp).is_none());
    }

    /// A vertex height grid of `(size+1)^2`, `h = slope * x`.
    fn ramp_heights(size: i32, slope: f32) -> Vec<f32> {
        let n = (size + 1) as usize;
        let mut h = vec![0.0f32; n * n];
        for y in 0..n {
            for x in 0..n {
                h[y * n + x] = slope * x as f32;
            }
        }
        h
    }

    /// Slope-nav params for a vehicle with ~2-tile wheelbase, 1-tile track,
    /// 20° pitch/roll limits (matching the LRV tuning).
    fn slope_params() -> (f32, f32, f32, f32) {
        let twenty_deg = 20.0 * core::f32::consts::PI / 180.0;
        (45.0 * 2.0, 45.0 * 1.0, twenty_deg, twenty_deg)
    }

    #[test]
    fn vehicle_slope_nav_flat_is_all_walkable() {
        let size = 4;
        let n = (size + 1) as usize;
        let heights = vec![1.0f32; n * n];
        let (wb, tr, pitch, roll) = slope_params();
        let nav = derive_vehicle_slope_nav(&heights, size, size, 32.0, 45.0, wb, tr, pitch, roll);
        assert!(nav.iter().all(|&v| v == 1));
    }

    #[test]
    fn vehicle_slope_nav_allows_gentle_and_blocks_steep() {
        let size = 16;
        // A 0.375 units/tile ramp is within 20° over a 2-tile wheelbase.
        let gentle = ramp_heights(size, 0.375);
        let (wb, tr, pitch, roll) = slope_params();
        let nav = derive_vehicle_slope_nav(&gentle, size, size, 32.0, 45.0, wb, tr, pitch, roll);
        assert!(nav.iter().all(|&v| v == 1), "0.375/tile should be drivable");

        // A 1.0 units/tile ramp exceeds 20° (pitch = atan(1·32/45) ≈ 35°).
        let steep = ramp_heights(size, 1.0);
        let nav = derive_vehicle_slope_nav(&steep, size, size, 32.0, 45.0, wb, tr, pitch, roll);
        assert!(nav.iter().all(|&v| v == 0), "1.0/tile should exceed the pitch limit");
    }

    #[test]
    fn vehicle_slope_nav_keeps_cliff_adjacent_flat_walkable() {
        // A 2-unit cliff at x=2: high plateau (x<2) drops to low (x>=2).  The
        // flat tile at x=2 (beside the cliff) must stay walkable — the vehicle
        // stands on flat ground even though one tile away is a cliff.
        let w = 8;
        let h = 8;
        let n = (w + 1) as usize;
        let mut heights = vec![0.0f32; n * n];
        for y in 0..n {
            for x in 0..n {
                heights[y * n + x] = if x < 2 { 2.0 } else { 0.0 };
            }
        }
        let (wb, tr, pitch, roll) = slope_params();
        let nav = derive_vehicle_slope_nav(&heights, w, h, 32.0, 45.0, wb, tr, pitch, roll);
        // The flat tile just past the cliff (x=2) is walkable.
        assert_eq!(nav[(2 + 4 * w) as usize], 1, "flat tile beside the cliff should be walkable");
        // The high plateau (x=0) is also flat and walkable.
        assert_eq!(nav[(4 * w) as usize], 1);
    }

    /// A 5x5 scene with a 2-unit cliff between a high plateau (x<2) and a low
    /// plateau (x>=2).  `walkable` marks the cliff-face column (x=1) blocked;
    /// `structural` is all-walkable.
    fn cliff_grids() -> (Vec<i32>, Vec<i32>, Vec<f32>) {
        let w = 5;
        let h = 5;
        let structural = vec![1_i32; (w * h) as usize];
        let mut walkable = structural.clone();
        for y in 0..h {
            walkable[(1 + y * w) as usize] = 0; // cliff face
        }
        let n = (w + 1) as usize;
        let mut heights = vec![0.0f32; n * n];
        for y in 0..n {
            for x in 0..n {
                heights[y * n + x] = if x < 2 { 2.0 } else { 0.0 };
            }
        }
        (walkable, structural, heights)
    }

    #[test]
    fn jump_crosses_a_small_cliff() {
        let (walkable, structural, heights) = cliff_grids();
        // 1-unit drop at height_scale 32 = 32 px; safe_fall 64 px allows it.
        let path = find_path_for_footprint_with_jumps(
            &walkable,
            &structural,
            &heights,
            5,
            5,
            (0, 2),
            (4, 2),
            &[(0, 0)],
            32.0,
            64.0,
            1.3,
        );
        assert!(path.is_some(), "should jump down the small cliff");
    }

    #[test]
    fn no_jump_when_safe_fall_disabled() {
        let (walkable, structural, heights) = cliff_grids();
        let path = find_path_for_footprint_with_jumps(
            &walkable,
            &structural,
            &heights,
            5,
            5,
            (0, 2),
            (4, 2),
            &[(0, 0)],
            32.0,
            0.0,
            1.3,
        );
        assert!(path.is_none(), "safe_fall 0 disables jumps");
    }

    #[test]
    fn no_jump_when_drop_too_large() {
        let (walkable, structural, heights) = cliff_grids();
        // 1-unit drop = 32 px exceeds a 16 px safe fall.
        let path = find_path_for_footprint_with_jumps(
            &walkable,
            &structural,
            &heights,
            5,
            5,
            (0, 2),
            (4, 2),
            &[(0, 0)],
            32.0,
            16.0,
            1.3,
        );
        assert!(path.is_none(), "drop larger than safe_fall must not jump");
    }

    #[test]
    fn no_jump_upward() {
        let (walkable, structural, heights) = cliff_grids();
        // Climbing the cliff is a negative drop — never a jump.
        let path = find_path_for_footprint_with_jumps(
            &walkable,
            &structural,
            &heights,
            5,
            5,
            (4, 2),
            (0, 2),
            &[(0, 0)],
            32.0,
            64.0,
            1.3,
        );
        assert!(path.is_none(), "cannot jump uphill");
    }

    #[test]
    fn vehicle_nav_snapshot_roundtrip() {
        let (w, h) = (4, 4);
        let structural = vec![1_i32; (w * h) as usize];
        let heights = vec![1.0f32; ((w + 1) * (h + 1)) as usize];
        let snap = VehicleNavSnapshot::new(w, h, structural.clone(), heights.clone(), 32.0, 45.0);
        assert_eq!(snap.size_x, w);
        assert_eq!(snap.size_y, h);
        assert_eq!(snap.structural, structural);
        assert_eq!(snap.heights, heights);
        assert_eq!(snap.height_scale, 32.0);
        assert_eq!(snap.tile_scale, 45.0);
    }

    #[test]
    fn vehicle_path_snapshot_matches_manual_pipeline() {
        let size = 16;
        let structural = vec![1_i32; (size * size) as usize];
        let heights = ramp_heights(size, 0.375);
        let (wb, tr, pitch, roll) = slope_params();
        let snap =
            VehicleNavSnapshot::new(size, size, structural.clone(), heights.clone(), 32.0, 45.0);

        let via_snapshot = find_vehicle_path_snapshot(
            &snap,
            (0, 0),
            (15, 15),
            &[(0, 0)],
            pitch,
            roll,
            wb,
            tr,
            64.0,
            1.3,
        );

        let slope = derive_vehicle_slope_nav(&heights, size, size, 32.0, 45.0, wb, tr, pitch, roll);
        let walkable: Vec<i32> =
            structural.iter().zip(slope.iter()).map(|(&s, &w)| s & w).collect();
        let manual = find_path_for_footprint_with_jumps(
            &walkable,
            &structural,
            &heights,
            size,
            size,
            (0, 0),
            (15, 15),
            &[(0, 0)],
            32.0,
            64.0,
            1.3,
        );
        assert_eq!(via_snapshot, manual);
    }

    #[test]
    fn vehicle_path_snapshot_flat_is_routable() {
        let size = 16;
        let structural = vec![1_i32; (size * size) as usize];
        let heights = vec![1.0f32; ((size + 1) * (size + 1)) as usize];
        let snap = VehicleNavSnapshot::new(size, size, structural, heights, 32.0, 45.0);
        let (wb, tr, pitch, roll) = slope_params();
        let path = find_vehicle_path_snapshot(
            &snap,
            (0, 0),
            (15, 15),
            &[(0, 0)],
            pitch,
            roll,
            wb,
            tr,
            0.0,
            1.3,
        );
        assert!(path.is_some(), "flat ground should route");
    }

    #[test]
    fn vehicle_path_snapshot_blocks_steep_ramp() {
        let size = 16;
        let structural = vec![1_i32; (size * size) as usize];
        let heights = ramp_heights(size, 1.0); // exceeds the 20° pitch limit
        let snap = VehicleNavSnapshot::new(size, size, structural, heights, 32.0, 45.0);
        let (wb, tr, pitch, roll) = slope_params();
        let path = find_vehicle_path_snapshot(
            &snap,
            (0, 8),
            (15, 8),
            &[(0, 0)],
            pitch,
            roll,
            wb,
            tr,
            0.0,
            1.3,
        );
        assert!(path.is_none(), "a 1.0/tile ramp exceeds the pitch limit");
    }

    #[test]
    fn pathfinder_state_caches_and_invalidates_slope() {
        let size = 8;
        let n = (size + 1) as usize;
        let mut state =
            PathfinderState::new(NavSnapshot::new(size, size, vec![1; (size * size) as usize]));
        let (wb, tr, pitch, roll) = slope_params();

        // Flat terrain: straight diagonal.
        state.set_vehicle(VehicleNavSnapshot::new(
            size,
            size,
            vec![1; (size * size) as usize],
            vec![1.0; n * n],
            32.0,
            45.0,
        ));
        assert!(state
            .find_vehicle((0, 0), (7, 7), &[(0, 0)], pitch, roll, wb, tr, 0.0, 1.3)
            .is_some());

        // Steep terrain (1.0/tile): no path — the slope grid was re-derived.
        let mut steep = vec![0.0f32; n * n];
        for y in 0..n {
            for x in 0..n {
                steep[y * n + x] = x as f32;
            }
        }
        state.set_vehicle(VehicleNavSnapshot::new(
            size,
            size,
            vec![1; (size * size) as usize],
            steep,
            32.0,
            45.0,
        ));
        assert!(state
            .find_vehicle((0, 0), (7, 7), &[(0, 0)], pitch, roll, wb, tr, 0.0, 1.3)
            .is_none());
    }
}
