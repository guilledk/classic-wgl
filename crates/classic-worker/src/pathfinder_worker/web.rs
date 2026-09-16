//! Host-owned A* pathfinding worker (web backend).
//!
//! In the background mode it spawns a dedicated `Worker` running the compiled
//! `pathfinder.wasm` module (the same Rust pathfinder the native worker thread
//! runs — see `worker.js`, which only instantiates the wasm and forwards
//! messages).  The render thread posts a `snapshot` message when the nav grid
//! changes and a `find` message per request; results arrive via `onmessage` and
//! are buffered until [`PathfinderWorker::poll_path`] drains them.  The
//! deterministic synchronous mode runs searches inline on a synchronous
//! [`JobQueue`] instead.  [`PathfinderWorker::find_path_sync`] runs a search
//! against the latest snapshot without going through either.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use classic_core::pathfinder::{
    GridCell, NavSnapshot, PathPoll, PathfinderState, VehicleNavSnapshot,
};
use wasm_bindgen::prelude::*;

use super::{PathId, PathJob, VehiclePathQuery, VehicleSnapshot};
use crate::jobs::JobQueue;

const WORKER_SRC: &str = include_str!("worker.js");
const PATHFINDER_WASM: &[u8] = include_bytes!("pathfinder.wasm");

enum Backend {
    /// A `Worker` running `pathfinder.wasm` + a main-thread result map.
    Worker { worker: web_sys::Worker, results: Rc<RefCell<HashMap<PathId, PathPoll>>> },
    /// Inline searches (the deterministic mode).
    Inline(JobQueue<PathJob>),
}

/// Web pathfinding worker.
pub struct PathfinderWorker {
    backend: Backend,
    snapshot: Arc<NavSnapshot>,
}

impl PathfinderWorker {
    /// Spawn the web Worker over `snapshot`.  Panics if the browser Worker
    /// cannot be created (no offload is possible, so this is fatal for the
    /// background path — the deterministic harness uses
    /// [`PathfinderWorker::new_synchronous`]).
    pub fn new(snapshot: Arc<NavSnapshot>) -> Self {
        let results: Rc<RefCell<HashMap<PathId, PathPoll>>> = Rc::new(RefCell::new(HashMap::new()));

        // Install the result handler.
        let on_message = {
            let results = results.clone();
            Box::new(move |data: JsValue| {
                let id = js_sys::Reflect::get(&data, &JsValue::from_str("id"))
                    .ok()
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0) as PathId;
                let path = js_sys::Reflect::get(&data, &JsValue::from_str("path")).ok();
                let poll = match path {
                    Some(p) if !p.is_null() && !p.is_undefined() => {
                        let flat = js_sys::Int32Array::new(&p).to_vec();
                        let cells = flat.as_chunks::<2>().0.iter().map(|c| (c[0], c[1])).collect();
                        PathPoll::Path(cells)
                    }
                    _ => PathPoll::NoPath,
                };
                results.borrow_mut().insert(id, poll);
            })
        };
        let worker = crate::spawn_web_worker(WORKER_SRC, Some(on_message))
            .expect("failed to spawn pathfinder worker");

        // Hand the compiled pathfinder.wasm bytes to the worker (it instantiates
        // them and queues any snapshot/find messages until ready).
        {
            let wasm = js_sys::Uint8Array::from(PATHFINDER_WASM);
            let init = js_sys::Object::new();
            let _ =
                js_sys::Reflect::set(&init, &JsValue::from_str("type"), &JsValue::from_str("init"));
            let _ = js_sys::Reflect::set(&init, &JsValue::from_str("wasm"), &wasm);
            let _ = crate::post_transfer(&worker, &init, &[&wasm.buffer()]);
        }

        let worker_handle = Self { backend: Backend::Worker { worker, results }, snapshot };
        worker_handle.push_snapshot(&worker_handle.snapshot);
        worker_handle
    }

    /// A worker that runs every search inline at submit time (the
    /// deterministic test/golden mode).
    pub fn new_synchronous(snapshot: Arc<NavSnapshot>) -> Self {
        let queue = JobQueue::synchronous(PathfinderState::new((*snapshot).clone()));
        Self { backend: Backend::Inline(queue), snapshot }
    }

    /// Whether searches run inline.
    pub fn is_synchronous(&self) -> bool {
        matches!(self.backend, Backend::Inline(_))
    }

    /// Post to the Worker (a no-op for the inline backend), transferring `transfer`.
    fn post(&self, msg: &js_sys::Object, transfer: &[&JsValue]) {
        if let Backend::Worker { worker, .. } = &self.backend {
            let _ = crate::post_transfer(worker, msg, transfer);
        }
    }

    /// Post the current snapshot to the worker (message ordering guarantees
    /// the worker holds it before any subsequent `find`).
    fn push_snapshot(&self, snapshot: &Arc<NavSnapshot>) {
        let data = js_sys::Int32Array::new_with_length(snapshot.data.len() as u32);
        data.copy_from(&snapshot.data);

        let msg = js_sys::Object::new();
        let _ =
            js_sys::Reflect::set(&msg, &JsValue::from_str("type"), &JsValue::from_str("snapshot"));
        let _ = js_sys::Reflect::set(
            &msg,
            &JsValue::from_str("sizeX"),
            &JsValue::from_f64(snapshot.size_x as f64),
        );
        let _ = js_sys::Reflect::set(
            &msg,
            &JsValue::from_str("sizeY"),
            &JsValue::from_f64(snapshot.size_y as f64),
        );
        let _ = js_sys::Reflect::set(&msg, &JsValue::from_str("data"), &data);
        self.post(&msg, &[&data.buffer()]);
    }

    /// Replace the nav snapshot both the worker and the sync fallback search
    /// against.  In-flight requests keep searching their original snapshot.
    pub fn set_snapshot(&mut self, snapshot: Arc<NavSnapshot>) {
        match &mut self.backend {
            Backend::Worker { .. } => self.push_snapshot(&snapshot),
            Backend::Inline(queue) => {
                let next = Arc::clone(&snapshot);
                queue.update(move |state| state.set_nav((*next).clone()));
            }
        }
        self.snapshot = snapshot;
    }

    /// The latest snapshot shared with the worker.
    pub fn snapshot(&self) -> &Arc<NavSnapshot> {
        &self.snapshot
    }

    /// Submit a path request under a caller-chosen `id`.  Non-blocking.
    pub fn request_path(&mut self, id: PathId, from: GridCell, to: GridCell) {
        if let Backend::Inline(queue) = &mut self.backend {
            queue.submit(id, PathJob::Find { from, to });
            return;
        }
        let from = js_sys::Array::of2(&JsValue::from(from.0), &JsValue::from(from.1));
        let to = js_sys::Array::of2(&JsValue::from(to.0), &JsValue::from(to.1));

        let msg = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&msg, &JsValue::from_str("type"), &JsValue::from_str("find"));
        let _ = js_sys::Reflect::set(&msg, &JsValue::from_str("id"), &JsValue::from_f64(id as f64));
        let _ = js_sys::Reflect::set(&msg, &JsValue::from_str("from"), &from);
        let _ = js_sys::Reflect::set(&msg, &JsValue::from_str("to"), &to);
        self.post(&msg, &[]);
    }

    /// Poll a previously submitted request.  Non-blocking; returns
    /// [`PathPoll::Pending`] until the worker has delivered a result.
    pub fn poll_path(&mut self, id: PathId) -> PathPoll {
        match &mut self.backend {
            Backend::Worker { results, .. } => {
                results.borrow_mut().remove(&id).unwrap_or(PathPoll::Pending)
            }
            Backend::Inline(queue) => match queue.poll(id) {
                None => PathPoll::Pending,
                Some(Some(path)) => PathPoll::Path(path),
                Some(None) => PathPoll::NoPath,
            },
        }
    }

    /// Post the current vehicle snapshot to the worker (message ordering
    /// guarantees the worker holds it before any subsequent `findVehicle`).
    fn push_vehicle_snapshot(&self, snapshot: &Arc<VehicleNavSnapshot>) {
        let structural = js_sys::Int32Array::new_with_length(snapshot.structural.len() as u32);
        structural.copy_from(&snapshot.structural);
        let heights = js_sys::Float32Array::new_with_length(snapshot.heights.len() as u32);
        heights.copy_from(&snapshot.heights);

        let msg = js_sys::Object::new();
        let _ = js_sys::Reflect::set(
            &msg,
            &JsValue::from_str("type"),
            &JsValue::from_str("vehicleSnapshot"),
        );
        let _ = js_sys::Reflect::set(
            &msg,
            &JsValue::from_str("sizeX"),
            &JsValue::from_f64(snapshot.size_x as f64),
        );
        let _ = js_sys::Reflect::set(
            &msg,
            &JsValue::from_str("sizeY"),
            &JsValue::from_f64(snapshot.size_y as f64),
        );
        let _ = js_sys::Reflect::set(&msg, &JsValue::from_str("structural"), &structural);
        let _ = js_sys::Reflect::set(&msg, &JsValue::from_str("heights"), &heights);
        let _ = js_sys::Reflect::set(
            &msg,
            &JsValue::from_str("tileM"),
            &JsValue::from_f64(snapshot.tile_m as f64),
        );
        self.post(&msg, &[&structural.buffer(), &heights.buffer()]);
    }

    /// Replace the vehicle nav snapshot the worker derives the slope grid from.
    pub fn set_vehicle_snapshot(&mut self, snapshot: Arc<VehicleNavSnapshot>) {
        match &mut self.backend {
            Backend::Worker { .. } => self.push_vehicle_snapshot(&snapshot),
            Backend::Inline(queue) => {
                queue.update(move |state| state.set_vehicle((*snapshot).clone()));
            }
        }
    }

    /// Submit a vehicle path request under a caller-chosen `id`, searching the
    /// `snapshot` source.  Non-blocking.
    pub fn request_vehicle(
        &mut self,
        id: PathId,
        query: VehiclePathQuery,
        snapshot: VehicleSnapshot,
    ) {
        if let Backend::Inline(queue) = &mut self.backend {
            queue.submit(id, PathJob::FindVehicle { query, snapshot });
            return;
        }
        match snapshot {
            VehicleSnapshot::Current => {}
            // The Worker keeps one current snapshot: install the fresh one first.
            VehicleSnapshot::Fresh(Some(fresh)) => self.set_vehicle_snapshot(fresh),
            VehicleSnapshot::Fresh(None) => {
                if let Backend::Worker { results, .. } = &self.backend {
                    results.borrow_mut().insert(id, PathPoll::NoPath);
                }
                return;
            }
        }

        let q = query;
        let from = js_sys::Array::of2(&JsValue::from(q.from.0), &JsValue::from(q.from.1));
        let to = js_sys::Array::of2(&JsValue::from(q.to.0), &JsValue::from(q.to.1));
        let fp = js_sys::Array::new();
        for (dx, dy) in q.footprint {
            fp.push(&js_sys::Array::of2(&JsValue::from(dx), &JsValue::from(dy)));
        }

        let msg = js_sys::Object::new();
        let _ = js_sys::Reflect::set(
            &msg,
            &JsValue::from_str("type"),
            &JsValue::from_str("findVehicle"),
        );
        let _ = js_sys::Reflect::set(&msg, &JsValue::from_str("id"), &JsValue::from_f64(id as f64));
        let _ = js_sys::Reflect::set(&msg, &JsValue::from_str("from"), &from);
        let _ = js_sys::Reflect::set(&msg, &JsValue::from_str("to"), &to);
        let _ = js_sys::Reflect::set(&msg, &JsValue::from_str("footprint"), &fp);
        for (key, value) in [
            ("pitchMax", q.pitch_max),
            ("rollMax", q.roll_max),
            ("wheelbaseM", q.wheelbase_m),
            ("trackM", q.track_m),
            ("safeFallM", q.safe_fall_m),
            ("jumpCost", q.jump_cost),
            ("turnCost", q.turn_cost),
        ] {
            let _ = js_sys::Reflect::set(
                &msg,
                &JsValue::from_str(key),
                &JsValue::from_f64(value as f64),
            );
        }
        self.post(&msg, &[]);
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

    /// Web has no blocking join: the Worker backend is not deterministic, and
    /// the synchronous backend has nothing in flight.
    pub fn join(&self) {}
}

impl Drop for PathfinderWorker {
    fn drop(&mut self) {
        if let Backend::Worker { worker, .. } = &self.backend {
            worker.terminate();
        }
    }
}
