//! Host-owned A* pathfinding worker (native backend).
//!
//! Owns an `Arc<NavSnapshot>` and runs `classic_core::pathfinder` searches on a
//! [`JobQueue`]: on a dedicated `std::thread` (so the render thread can submit a
//! request and poll for the result without blocking mid-frame), or inline in
//! the deterministic synchronous mode.  A separate inline fallback
//! ([`PathfinderWorker::find_path_sync`]) runs a search against the latest
//! snapshot without going through the queue.

use std::sync::Arc;

use classic_core::pathfinder::{
    GridCell, NavSnapshot, PathPoll, PathfinderState, VehicleNavSnapshot,
};

use super::{PathId, PathJob, VehiclePathQuery, VehicleSnapshot};
use crate::jobs::JobQueue;

/// Native pathfinding worker: a [`JobQueue`] of path searches.
pub struct PathfinderWorker {
    queue: JobQueue<PathJob>,
    snapshot: Arc<NavSnapshot>,
}

impl PathfinderWorker {
    /// Spawn the worker thread over `snapshot`.
    pub fn new(snapshot: Arc<NavSnapshot>) -> Self {
        let state = PathfinderState::new((*snapshot).clone());
        let queue = JobQueue::threaded("classic-pathfinder", state)
            .expect("failed to spawn pathfinder thread");
        Self { queue, snapshot }
    }

    /// A worker that runs every search inline at submit time (the
    /// deterministic test/golden mode).
    pub fn new_synchronous(snapshot: Arc<NavSnapshot>) -> Self {
        let queue = JobQueue::synchronous(PathfinderState::new((*snapshot).clone()));
        Self { queue, snapshot }
    }

    /// Whether searches run inline.
    pub fn is_synchronous(&self) -> bool {
        self.queue.is_synchronous()
    }

    /// Replace the nav snapshot both workers and the sync fallback search
    /// against.  In-flight requests keep searching their original snapshot.
    pub fn set_snapshot(&mut self, snapshot: Arc<NavSnapshot>) {
        self.snapshot = Arc::clone(&snapshot);
        self.queue.update(move |state| state.set_nav((*snapshot).clone()));
    }

    /// The latest snapshot shared with the worker.
    pub fn snapshot(&self) -> &Arc<NavSnapshot> {
        &self.snapshot
    }

    /// Submit a path request under a caller-chosen `id`.  Non-blocking.
    pub fn request_path(&mut self, id: PathId, from: GridCell, to: GridCell) {
        self.queue.submit(id, PathJob::Find { from, to });
    }

    /// Poll a previously submitted request.  Non-blocking; returns
    /// [`PathPoll::Pending`] until the worker has delivered a result.
    pub fn poll_path(&mut self, id: PathId) -> PathPoll {
        match self.queue.poll(id) {
            None => PathPoll::Pending,
            Some(Some(path)) => PathPoll::Path(path),
            Some(None) => PathPoll::NoPath,
        }
    }

    /// Replace the vehicle nav snapshot the worker derives the slope grid from.
    /// Also clears the cached slope grid (the terrain changed).
    pub fn set_vehicle_snapshot(&mut self, snapshot: Arc<VehicleNavSnapshot>) {
        self.queue.update(move |state| state.set_vehicle((*snapshot).clone()));
    }

    /// Submit a vehicle path request under a caller-chosen `id`, searching the
    /// `snapshot` source.  Non-blocking.
    pub fn request_vehicle(
        &mut self,
        id: PathId,
        query: VehiclePathQuery,
        snapshot: VehicleSnapshot,
    ) {
        self.queue.submit(id, PathJob::FindVehicle { query, snapshot });
    }

    /// Submit a vehicle path request over the current vehicle snapshot under a
    /// caller-chosen `id`.  Non-blocking.
    #[allow(clippy::too_many_arguments)]
    pub fn request_vehicle_path(
        &mut self,
        id: PathId,
        from: GridCell,
        to: GridCell,
        footprint: Vec<GridCell>,
        pitch_max: f32,
        roll_max: f32,
        wheelbase_m: f32,
        track_m: f32,
        safe_fall_m: f32,
        jump_cost: f32,
        turn_cost: f32,
    ) {
        let query = VehiclePathQuery {
            from,
            to,
            footprint,
            pitch_max,
            roll_max,
            wheelbase_m,
            track_m,
            safe_fall_m,
            jump_cost,
            turn_cost,
        };
        self.request_vehicle(id, query, VehicleSnapshot::Current);
    }

    /// Poll a previously submitted vehicle request (non-blocking).  Shares the
    /// same result map as [`PathfinderWorker::poll_path`], so ids must be
    /// unique across both request kinds.
    pub fn poll_vehicle_path(&mut self, id: PathId) -> PathPoll {
        self.poll_path(id)
    }

    /// Synchronous fallback: run A* inline against the latest snapshot.
    pub fn find_path_sync(&self, from: GridCell, to: GridCell) -> Option<Vec<GridCell>> {
        self.snapshot.find_path(from, to)
    }

    /// Block until every previously submitted request has been processed (the
    /// determinism barrier at frame boundaries; a no-op when synchronous).
    pub fn join(&self) {
        self.queue.join();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    fn open_snapshot(w: i32, h: i32) -> Arc<NavSnapshot> {
        Arc::new(NavSnapshot::new(w, h, vec![1; (w * h) as usize]))
    }

    fn poll_until(worker: &mut PathfinderWorker, id: PathId) -> PathPoll {
        for _ in 0..1000 {
            match worker.poll_path(id) {
                PathPoll::Pending => thread::sleep(Duration::from_millis(1)),
                result => return result,
            }
        }
        PathPoll::Pending
    }

    #[test]
    fn finds_a_path_async() {
        let mut worker = PathfinderWorker::new(open_snapshot(5, 5));
        worker.request_path(0, (0, 0), (4, 4));
        match poll_until(&mut worker, 0) {
            PathPoll::Path(path) => {
                assert_eq!(path.first(), Some(&(0, 0)));
                assert_eq!(path.last(), Some(&(4, 4)));
            }
            other => panic!("expected a path, got {other:?}"),
        }
    }

    #[test]
    fn reports_no_path() {
        let mut grid = vec![1_i32; 9];
        grid[1] = 0;
        grid[4] = 0;
        grid[7] = 0;
        let mut worker = PathfinderWorker::new(Arc::new(NavSnapshot::new(3, 3, grid)));
        worker.request_path(7, (0, 0), (2, 0));
        assert_eq!(poll_until(&mut worker, 7), PathPoll::NoPath);
    }

    #[test]
    fn sync_fallback_matches_async() {
        let mut worker = PathfinderWorker::new(open_snapshot(8, 8));
        worker.request_path(3, (1, 1), (6, 5));
        let sync = worker.find_path_sync((1, 1), (6, 5)).unwrap();
        match poll_until(&mut worker, 3) {
            PathPoll::Path(path) => assert_eq!(path, sync),
            other => panic!("expected a path, got {other:?}"),
        }
    }

    #[test]
    fn snapshot_update_repins_search() {
        // A blocked centre cell forces the route around it.
        let mut grid = vec![1_i32; 9];
        grid[4] = 0;
        let mut worker = PathfinderWorker::new(Arc::new(NavSnapshot::new(3, 3, grid)));
        worker.request_path(1, (0, 0), (2, 2));
        match poll_until(&mut worker, 1) {
            PathPoll::Path(path) => {
                assert_eq!(path.first(), Some(&(0, 0)));
                assert_eq!(path.last(), Some(&(2, 2)));
                assert!(!path.contains(&(1, 1)), "must route around the blocked centre");
            }
            other => panic!("expected a path around the wall, got {other:?}"),
        }

        // Clear the wall; the same query now takes the straight diagonal.
        worker.set_snapshot(open_snapshot(3, 3));
        worker.request_path(2, (0, 0), (2, 2));
        assert_eq!(poll_until(&mut worker, 2), PathPoll::Path(vec![(0, 0), (1, 1), (2, 2)]));
    }

    #[test]
    fn join_barrier_waits_for_inflight() {
        let mut worker = PathfinderWorker::new(open_snapshot(64, 64));
        worker.request_path(0, (0, 0), (63, 63));
        worker.join();
        // After the barrier, the result must already be buffered (no Pending).
        assert_ne!(worker.poll_path(0), PathPoll::Pending);
    }

    fn tile_m() -> f32 {
        45.0 / 64.0
    }

    fn open_vehicle_snapshot(w: i32, h: i32) -> Arc<VehicleNavSnapshot> {
        Arc::new(VehicleNavSnapshot::new(
            w,
            h,
            vec![1; (w * h) as usize],
            vec![1.0; ((w + 1) * (h + 1)) as usize],
            tile_m(),
        ))
    }

    fn steep_vehicle_snapshot(w: i32, h: i32) -> Arc<VehicleNavSnapshot> {
        let n = (w + 1) as usize;
        let mut heights = vec![0.0f32; n * n];
        for y in 0..n {
            for x in 0..n {
                heights[y * n + x] = x as f32; // 1.0 m/tile ramp
            }
        }
        Arc::new(VehicleNavSnapshot::new(w, h, vec![1; (w * h) as usize], heights, tile_m()))
    }

    fn vehicle_params() -> (f32, f32, f32, f32) {
        let pitch = 20.0f32.to_radians();
        let roll = 20.0f32.to_radians();
        (pitch, roll, 2.0 * tile_m(), tile_m())
    }

    #[test]
    fn finds_a_vehicle_path_async() {
        let mut worker = PathfinderWorker::new(open_snapshot(8, 8));
        worker.set_vehicle_snapshot(open_vehicle_snapshot(8, 8));
        let (pitch, roll, wb, tr) = vehicle_params();
        worker.request_vehicle_path(
            0,
            (0, 0),
            (7, 7),
            vec![(0, 0)],
            pitch,
            roll,
            wb,
            tr,
            0.0,
            1.3,
            0.0,
        );
        match poll_until(&mut worker, 0) {
            PathPoll::Path(path) => {
                assert_eq!(path.first(), Some(&(0, 0)));
                assert_eq!(path.last(), Some(&(7, 7)));
            }
            other => panic!("expected a vehicle path, got {other:?}"),
        }
    }

    #[test]
    fn vehicle_snapshot_update_repins_search() {
        let mut worker = PathfinderWorker::new(open_snapshot(8, 8));
        let (pitch, roll, wb, tr) = vehicle_params();

        // Flat terrain: straight diagonal route.
        worker.set_vehicle_snapshot(open_vehicle_snapshot(8, 8));
        worker.request_vehicle_path(
            1,
            (0, 0),
            (7, 7),
            vec![(0, 0)],
            pitch,
            roll,
            wb,
            tr,
            0.0,
            1.3,
            0.0,
        );
        match poll_until(&mut worker, 1) {
            PathPoll::Path(path) => {
                assert_eq!(path.first(), Some(&(0, 0)));
                assert_eq!(path.last(), Some(&(7, 7)));
            }
            other => panic!("expected a path on flat terrain, got {other:?}"),
        }

        // Push a steep snapshot (1.0/tile exceeds the 20° pitch limit): the same
        // query must now find no path — the slope grid was re-derived.
        worker.set_vehicle_snapshot(steep_vehicle_snapshot(8, 8));
        worker.request_vehicle_path(
            2,
            (0, 0),
            (7, 7),
            vec![(0, 0)],
            pitch,
            roll,
            wb,
            tr,
            0.0,
            1.3,
            0.0,
        );
        assert_eq!(poll_until(&mut worker, 2), PathPoll::NoPath);
    }

    #[test]
    fn join_barrier_waits_for_vehicle_inflight() {
        let mut worker = PathfinderWorker::new(open_snapshot(64, 64));
        worker.set_vehicle_snapshot(open_vehicle_snapshot(64, 64));
        let (pitch, roll, wb, tr) = vehicle_params();
        worker.request_vehicle_path(
            0,
            (0, 0),
            (63, 63),
            vec![(0, 0)],
            pitch,
            roll,
            wb,
            tr,
            0.0,
            1.3,
            0.0,
        );
        worker.join();
        assert_ne!(worker.poll_vehicle_path(0), PathPoll::Pending);
    }

    #[test]
    fn synchronous_mode_resolves_at_submit() {
        let mut worker = PathfinderWorker::new_synchronous(open_snapshot(8, 8));
        assert!(worker.is_synchronous());
        worker.request_path(3, (1, 1), (6, 5));
        let inline = worker.find_path_sync((1, 1), (6, 5)).unwrap();
        assert_eq!(worker.poll_path(3), PathPoll::Path(inline));
    }

    #[test]
    fn fresh_vehicle_snapshot_overrides_the_current_one() {
        let mut worker = PathfinderWorker::new_synchronous(open_snapshot(8, 8));
        worker.set_vehicle_snapshot(steep_vehicle_snapshot(8, 8));
        let (pitch_max, roll_max, wheelbase_m, track_m) = vehicle_params();
        let query = VehiclePathQuery {
            from: (0, 0),
            to: (7, 7),
            footprint: vec![(0, 0)],
            pitch_max,
            roll_max,
            wheelbase_m,
            track_m,
            safe_fall_m: 0.0,
            jump_cost: 1.3,
            turn_cost: 0.0,
        };

        worker.request_vehicle(1, query.clone(), VehicleSnapshot::Current);
        assert_eq!(worker.poll_vehicle_path(1), PathPoll::NoPath, "current = steep");

        let flat = VehicleSnapshot::Fresh(Some(open_vehicle_snapshot(8, 8)));
        worker.request_vehicle(2, query.clone(), flat);
        assert!(matches!(worker.poll_vehicle_path(2), PathPoll::Path(_)), "fresh = flat");

        worker.request_vehicle(3, query, VehicleSnapshot::Fresh(None));
        assert_eq!(worker.poll_vehicle_path(3), PathPoll::NoPath, "no terrain, no path");
    }
}
