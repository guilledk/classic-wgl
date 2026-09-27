//! Host-owned A* pathfinding worker: a platform-specific backend behind a
//! common `PathfinderWorker` API.
//!
//! Both backends can run synchronously on a [`JobQueue`](crate::JobQueue) (the
//! deterministic mode); in the background, native runs the queue on a
//! dedicated thread and web uses a dedicated `Worker` running the compiled
//! `pathfinder.wasm` (`worker.js`).

use std::sync::Arc;

use classic_core::pathfinder::{
    find_vehicle_path_snapshot, GridCell, PathfinderState, VehicleNavSnapshot,
};

use crate::jobs::Job;

/// Request/result correlation id (owned by the caller, e.g. the engine).
pub type PathId = u64;

/// A footprint-, slope- and jump-aware vehicle path query.
#[derive(Clone, Debug)]
pub struct VehiclePathQuery {
    pub from: GridCell,
    pub to: GridCell,
    pub footprint: Vec<GridCell>,
    pub pitch_max: f32,
    pub roll_max: f32,
    pub wheelbase_m: f32,
    pub track_m: f32,
    pub safe_fall_m: f32,
    pub jump_cost: f32,
    pub turn_cost: f32,
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
        match self {
            PathJob::Find { from, to } => state.find(from, to),
            PathJob::FindVehicle { query: q, snapshot: VehicleSnapshot::Current } => state
                .find_vehicle(
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
                ),
            PathJob::FindVehicle { query: q, snapshot: VehicleSnapshot::Fresh(Some(snapshot)) } => {
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
