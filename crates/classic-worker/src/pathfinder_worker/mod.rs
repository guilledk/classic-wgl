//! Host-owned A* pathfinding worker: a platform-specific backend behind a
//! common `PathfinderWorker` API.
//!
//! Both backends can run synchronously on a [`JobQueue`](crate::JobQueue) (the
//! deterministic mode); in the background, native runs the queue on a
//! dedicated thread and web uses a dedicated `Worker` running the compiled
//! `pathfinder.wasm` (`worker.js`).

use std::sync::Arc;

use classic_core::pathfinder::{
    find_vehicle_path_snapshot, find_vehicle_pose_snapshot, GridCell, PathfinderState, PoseGoal,
    VehicleFootprint, VehicleNavSnapshot,
};

use crate::jobs::Job;

/// Request/result correlation id (owned by the caller, e.g. the engine).
pub type PathId = u64;

/// A footprint-, slope- and jump-aware vehicle path query.
#[derive(Clone, Debug)]
pub struct VehiclePathQuery {
    pub from: GridCell,
    pub to: GridCell,
    pub footprint: VehicleFootprint,
    pub pitch_max: f32,
    pub roll_max: f32,
    pub wheelbase_m: f32,
    pub track_m: f32,
    pub safe_fall_m: f32,
    pub jump_cost: f32,
    pub turn_cost: f32,
    /// A pose goal: drive to a point facing a heading (`to` unused; the
    /// waypoints carry their gear, `pathfinder::unpack_waypoint`).
    pub pose: Option<PoseGoal>,
    /// Inclusive tile rects `[x0, y0, x1, y1]` closed on top of the current
    /// snapshot for this search (the vehicle blocks other vehicles own).  A
    /// `Fresh` snapshot is built with them already.
    pub closed: Vec<[i32; 4]>,
}

/// Which vehicle nav snapshot a vehicle query searches.
#[derive(Clone, Debug)]
pub enum VehicleSnapshot {
    /// The snapshot last pushed with `set_vehicle_snapshot` (the worker caches
    /// its derived slope grid).
    Current,
    /// A snapshot built for this query (`None` when there is no terrain to
    /// build one from, which finds no path).  The synchronous engine path uses
    /// this so it searches the live world, exactly like an inline search.
    /// The query's `closed` rects are not applied (the snapshot has them).
    Fresh(Option<Arc<VehicleNavSnapshot>>),
}

/// A pathfinding job run against a [`PathfinderState`].
pub(crate) enum PathJob {
    Find { from: GridCell, to: GridCell },
    FindVehicle { query: VehiclePathQuery, snapshot: VehicleSnapshot },
}

impl Job for PathJob {
    type State = PathfinderState;
    type Output = Option<Vec<GridCell>>;

    fn run(self, state: &mut PathfinderState) -> Self::Output {
        // the search's own time (native; the web Worker runs pathfinder.wasm)
        #[cfg(not(target_arch = "wasm32"))]
        let (t0, kind) = (
            std::time::Instant::now(),
            match &self {
                PathJob::FindVehicle { query: VehiclePathQuery { pose: Some(_), .. }, .. } => {
                    "vehicle pose"
                }
                PathJob::FindVehicle { .. } => "vehicle",
                PathJob::Find { .. } => "",
            },
        );
        let out = self.run_job(state);
        #[cfg(not(target_arch = "wasm32"))]
        if !kind.is_empty() {
            classic_core::cl_info!(
                classic_core::instrument::Chan::Path,
                "{kind} search: {:.1} ms, {}",
                t0.elapsed().as_secs_f64() * 1e3,
                if out.is_some() { "found" } else { "no path" }
            );
        }
        out
    }
}

impl PathJob {
    fn run_job(self, state: &mut PathfinderState) -> Option<Vec<GridCell>> {
        match self {
            PathJob::Find { from, to } => state.find(from, to),
            PathJob::FindVehicle { query: q, snapshot: VehicleSnapshot::Current } => {
                match &q.pose {
                    Some(goal) => state.find_vehicle_pose(
                        q.from,
                        goal,
                        &q.footprint,
                        q.pitch_max,
                        q.roll_max,
                        q.wheelbase_m,
                        q.track_m,
                        q.turn_cost,
                        &q.closed,
                    ),
                    None => state.find_vehicle(
                        q.from,
                        q.to,
                        &q.footprint,
                        q.pitch_max,
                        q.roll_max,
                        q.wheelbase_m,
                        q.track_m,
                        q.safe_fall_m,
                        q.jump_cost,
                        q.turn_cost,
                        &q.closed,
                    ),
                }
            }
            PathJob::FindVehicle { query: q, snapshot: VehicleSnapshot::Fresh(Some(snapshot)) } => {
                if let Some(goal) = &q.pose {
                    return find_vehicle_pose_snapshot(
                        &snapshot,
                        q.from,
                        goal,
                        &q.footprint,
                        q.pitch_max,
                        q.roll_max,
                        q.wheelbase_m,
                        q.track_m,
                        q.turn_cost,
                    );
                }
                find_vehicle_path_snapshot(
                    &snapshot,
                    q.from,
                    q.to,
                    &q.footprint,
                    q.pitch_max,
                    q.roll_max,
                    q.wheelbase_m,
                    q.track_m,
                    q.safe_fall_m,
                    q.jump_cost,
                    q.turn_cost,
                )
            }
            PathJob::FindVehicle { snapshot: VehicleSnapshot::Fresh(None), .. } => None,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(not(target_arch = "wasm32"))]
pub use native::PathfinderWorker;
#[cfg(target_arch = "wasm32")]
pub use web::PathfinderWorker;
