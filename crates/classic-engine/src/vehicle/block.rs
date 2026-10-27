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

    /// Remove vehicle block `id`; `false` when there is none.
    pub fn clear_vehicle_block(&mut self, id: i32) -> bool {
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
    /// is no longer clear on the vehicle blocking grid, and submit a fresh
    /// search to its goal, polled by [`Engine::poll_vehicle_replans`].  An
    /// airborne vehicle (which takes no goto) just stops.
    fn replan_cut_vehicle_routes(&mut self) {
        let snap = std::sync::Arc::clone(&self.vehicle_nav_snapshot);
        let (sx, sy) = (snap.size_x, snap.size_y);
        if snap.structural.len() != (sx * sy) as usize {
            return;
        }
        let cut: Vec<(hecs::Entity, (i32, i32))> = self
            .world
            .query::<&IsoVehicle>()
            .iter()
            .filter_map(|(e, v)| {
                if v.path_idx >= v.path.len() {
                    return None;
                }
                let route: Vec<(i32, i32)> =
                    v.path[v.path_idx.saturating_sub(1)..].iter().map(|c| (c[0], c[1])).collect();
                let clear = classic_core::pathfinder::route_clear(
                    &snap.structural,
                    sx,
                    sy,
                    &route,
                    &search_footprint(v),
                );
                let goal = *route.last()?;
                (!clear).then_some((e, goal))
            })
            .collect();
        for (ve, goal) in cut {
            if let Ok(mut v) = self.world.get::<&mut IsoVehicle>(ve) {
                v.path.clear();
                v.path_idx = 0;
            }
            let name = self.debug_name(ve);
            classic_core::cl_info!(
                classic_core::instrument::Chan::Path,
                "vehicle block cut {name}'s route: re-planning to {goal:?}"
            );
            if let VehicleGotoSubmit::Submitted(id) = self.vehicle_goto(&name, goal.0, goal.1) {
                self.vehicle_replans.push(id);
            }
        }
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
