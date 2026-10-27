//! Vehicle blocks: tile rects only vehicles may not drive over
//! (`set_vehicle_block`), a layer of its own beside the nav grid.  The nav
//! grid encodes human walkability (slope-limited), which the vehicle search
//! ignores (a vehicle derives its own slope limits), so a structure flush
//! with the ground (a crane's rails) has no way onto the vehicle search but
//! a `blocks_nav` collider.  The vehicle search's blocking grid is the
//! `blocks_nav` colliders' footprints and these rects; human and `IsoAgent`
//! pathing never see them.
//!
//! Rects come in named sets, one id per structure (a building's ~90 rects in
//! one call), so clearing one structure never reopens cells another still
//! covers.  A change re-shares the vehicle snapshot with the pathfinder
//! worker (native and web) and stops every vehicle whose remaining route it
//! cuts, re-planning that vehicle to the same goal.

use classic_core::components::IsoVehicle;

use super::kinematics::search_footprint;
use super::{VehicleGotoPoll, VehicleGotoSubmit};
use crate::Engine;

/// Zero every cell of `grid` (`size_x × size_y`) under the rects of
/// `blocks` (inclusive tile rects, clipped to the grid).
pub(crate) fn stamp_vehicle_blocks<'a>(
    grid: &mut [i32],
    size_x: i32,
    size_y: i32,
    blocks: impl Iterator<Item = &'a [i32; 4]>,
) {
    for &[x0, y0, x1, y1] in blocks {
        let (x0, x1) = (x0.max(0), x1.min(size_x - 1));
        let (y0, y1) = (y0.max(0), y1.min(size_y - 1));
        for y in y0..=y1 {
            for x in x0..=x1 {
                grid[(y * size_x + x) as usize] = 0;
            }
        }
    }
}

impl Engine {
    /// Set (or replace) vehicle block `id`: the inclusive tile rects
    /// `[x0, y0, x1, y1]` (either corner order) vehicles may not drive over.
    /// An empty list clears it.  Re-plans the vehicles whose routes it cuts.
    pub fn set_vehicle_block(&mut self, id: i32, rects: &[[i32; 4]]) {
        let rects: Vec<[i32; 4]> = rects
            .iter()
            .map(|&[x0, y0, x1, y1]| [x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)])
            .collect();
        if rects.is_empty() {
            self.clear_vehicle_block(id);
            return;
        }
        if self.vehicle_blocks.get(&id) == Some(&rects) {
            return;
        }
        self.vehicle_blocks.insert(id, rects);
        self.vehicle_blocks_changed();
    }

    /// Give vehicle block `id` an owner: vehicle `owner`'s own searches drive
    /// through it while every other vehicle's respect it (a lane reserved
    /// for the vehicle working in it); `None` makes it block everyone again.
    /// The block keeps its owner when its rects are replaced, and loses it
    /// when cleared.  Re-plans the routes the change cuts (the old owner's
    /// through it).  `false` when there is no block `id`.
    pub fn set_vehicle_block_owner(&mut self, id: i32, owner: Option<&str>) -> bool {
        if !self.vehicle_blocks.contains_key(&id) {
            return false;
        }
        let owner = owner.filter(|o| !o.is_empty()).map(str::to_string);
        if self.vehicle_block_owners.get(&id) == owner.as_ref() {
            return true;
        }
        match owner {
            Some(o) => self.vehicle_block_owners.insert(id, o),
            None => self.vehicle_block_owners.remove(&id),
        };
        self.replan_cut_vehicle_routes();
        true
    }

    /// Whether vehicle `name` owns any vehicle block.
    pub(crate) fn owns_vehicle_block(&self, name: &str) -> bool {
        self.vehicle_block_owners.values().any(|o| o == name)
    }

    /// Remove vehicle block `id`; `false` when there is none.
    pub fn clear_vehicle_block(&mut self, id: i32) -> bool {
        self.vehicle_block_owners.remove(&id);
        let had = self.vehicle_blocks.remove(&id).is_some();
        if had {
            self.vehicle_blocks_changed();
        }
        had
    }

    /// Re-share the vehicle snapshot and re-plan the routes the change cut.
    fn vehicle_blocks_changed(&mut self) {
        self.refresh_nav_snapshot();
        self.replan_cut_vehicle_routes();
    }

    /// Stop every vehicle whose remaining route (from the waypoint it left)
    /// is no longer clear on its vehicle blocking grid (the shared one, or an
    /// owner's with its own blocks open), and submit a fresh search to its
    /// goal, polled by [`Engine::poll_vehicle_replans`].  An airborne
    /// vehicle (which takes no goto) just stops.
    fn replan_cut_vehicle_routes(&mut self) {
        let snap = std::sync::Arc::clone(&self.vehicle_nav_snapshot);
        let (sx, sy) = (snap.size_x, snap.size_y);
        if snap.structural.len() != (sx * sy) as usize {
            return;
        }
        // an owner's view of the grid, built once per owner
        let obstacles = self.compute_nav_obstacles();
        let mut owner_grids: std::collections::HashMap<String, Vec<i32>> = Default::default();
        for owner in self.vehicle_block_owners.values() {
            if !owner_grids.contains_key(owner) {
                if let Some(s) = self.build_vehicle_nav_snapshot_for(&obstacles, Some(owner)) {
                    owner_grids.insert(owner.clone(), s.structural.clone());
                }
            }
        }
        let names: std::collections::HashMap<hecs::Entity, String> = self
            .world
            .query::<&IsoVehicle>()
            .iter()
            .map(|(e, _)| (e, self.debug_name(e)))
            .collect();
        let cut: Vec<(hecs::Entity, (i32, i32), Option<f32>)> = self
            .world
            .query::<&IsoVehicle>()
            .iter()
            .filter_map(|(e, v)| {
                let grid = names
                    .get(&e)
                    .and_then(|n| owner_grids.get(n))
                    .map_or(snap.structural.as_slice(), Vec::as_slice);
                if v.path_idx >= v.path.len() {
                    return None;
                }
                let from = v.path_idx.saturating_sub(1);
                let route: Vec<(i32, i32)> = v.path[from..].iter().map(|c| (c[0], c[1])).collect();
                // a pose route is checked along the chords it drives, in gear
                let pose = v.path_reverse.len() == v.path.len();
                let clear = if pose {
                    classic_core::pathfinder::route_clear_pose(
                        grid,
                        sx,
                        sy,
                        &route,
                        &v.path_reverse[from..],
                        &search_footprint(v),
                    )
                } else {
                    classic_core::pathfinder::route_clear(
                        grid,
                        sx,
                        sy,
                        &route,
                        &search_footprint(v),
                    )
                };
                let goal = *route.last()?;
                (!clear).then_some((e, goal, v.goal_heading.filter(|_| pose)))
            })
            .collect();
        for (ve, goal, heading) in cut {
            if let Ok(mut v) = self.world.get::<&mut IsoVehicle>(ve) {
                v.path.clear();
                v.path_idx = 0;
                v.path_reverse.clear();
                v.goal_heading = None;
            }
            let name = self.debug_name(ve);
            classic_core::cl_info!(
                classic_core::instrument::Chan::Path,
                "vehicle block cut {name}'s route: re-planning to {goal:?}"
            );
            let submit = match heading {
                Some(h) => self.vehicle_goto_pose(
                    &name,
                    goal.0 as f32 + 0.5,
                    goal.1 as f32 + 0.5,
                    h.to_degrees(),
                ),
                None => self.vehicle_goto(&name, goal.0, goal.1),
            };
            if let VehicleGotoSubmit::Submitted(id) = submit {
                self.vehicle_replans.push(id);
            }
        }
    }

    /// Have `update_vehicles` poll vehicle route request `id` and install its
    /// route when it resolves (as the vehicle-block re-plans are), for a host
    /// that submits a route without polling it itself (a test scenario).
    pub fn vehicle_goto_autopoll(&mut self, id: u64) {
        self.vehicle_replans.push(id);
    }

    /// Poll the re-plans [`Engine::set_vehicle_block`] submitted: an
    /// accepted route is installed (by `vehicle_goto_poll`), a refused one
    /// leaves the vehicle stopped.  Runs at the top of `update_vehicles`.
    pub(crate) fn poll_vehicle_replans(&mut self) {
        let ids = std::mem::take(&mut self.vehicle_replans);
        for id in ids {
            if matches!(self.vehicle_goto_poll(id), VehicleGotoPoll::Pending) {
                self.vehicle_replans.push(id);
            }
        }
    }
}
