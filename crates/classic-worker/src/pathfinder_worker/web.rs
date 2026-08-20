//! Host-owned A* pathfinding worker (web backend).
//!
//! Spawns a dedicated `Worker` running the A* algorithm in JavaScript (see
//! `worker.js`, a mirror of the retired TypeScript `pathfinder.ts`).  The
//! render thread posts a `snapshot` message when the nav grid changes and a
//! `find` message per request; results arrive via `onmessage` and are buffered
//! until [`PathfinderWorker::poll_path`] drains them.  A synchronous fallback
//! ([`PathfinderWorker::find_path_sync`]) runs the same search inline for the
//! deterministic test harness.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use classic_core::pathfinder::{GridCell, NavSnapshot, PathPoll};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use super::PathId;

const WORKER_SRC: &str = include_str!("worker.js");

/// Web pathfinding worker: a `Worker` running JS A* + a main-thread result map.
pub struct PathfinderWorker {
    worker: web_sys::Worker,
    results: Rc<RefCell<HashMap<PathId, PathPoll>>>,
    snapshot: Arc<NavSnapshot>,
}

impl PathfinderWorker {
    /// Spawn the web Worker over `snapshot`.  Panics if the browser Worker
    /// cannot be created (no offload is possible, so this is fatal for the
    /// async path — the engine falls back to the sync path under
    /// `synchronous_workers`).
    pub fn new(snapshot: Arc<NavSnapshot>) -> Self {
        let results: Rc<RefCell<HashMap<PathId, PathPoll>>> = Rc::new(RefCell::new(HashMap::new()));

        // Build the worker from an inline source Blob (mirrors the guest
        // worker runtime's approach).
        let blob_parts = js_sys::Array::of1(&JsValue::from_str(WORKER_SRC));
        let blob = web_sys::Blob::new_with_str_sequence(blob_parts.as_ref())
            .expect("failed to build pathfinder worker blob");
        let url = web_sys::Url::create_object_url_with_blob(&blob)
            .expect("failed to create pathfinder worker url");
        let worker = web_sys::Worker::new(&url).expect("failed to spawn pathfinder worker");

        // Install the result handler.  Uses `JsValue` for the event so no
        // `MessageEvent` web-sys feature is required.
        {
            let results = results.clone();
            let onmessage = Closure::wrap(Box::new(move |event: JsValue| {
                let data = js_sys::Reflect::get(&event, &JsValue::from_str("data"))
                    .unwrap_or(JsValue::NULL);
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
            }) as Box<dyn FnMut(JsValue)>);
            worker.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
            onmessage.forget();
        }

        let worker_handle = Self { worker, results, snapshot };
        worker_handle.push_snapshot(&worker_handle.snapshot);
        worker_handle
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
        let _ = self.worker.post_message(&msg);
    }

    /// Replace the nav snapshot both the worker and the sync fallback search
    /// against.  In-flight requests keep searching their original snapshot.
    pub fn set_snapshot(&mut self, snapshot: Arc<NavSnapshot>) {
        self.push_snapshot(&snapshot);
        self.snapshot = snapshot;
    }

    /// The latest snapshot shared with the worker.
    pub fn snapshot(&self) -> &Arc<NavSnapshot> {
        &self.snapshot
    }

    /// Submit a path request under a caller-chosen `id`.  Non-blocking.
    pub fn request_path(&mut self, id: PathId, from: GridCell, to: GridCell) {
        let from = js_sys::Array::of2(&JsValue::from(from.0), &JsValue::from(from.1));
        let to = js_sys::Array::of2(&JsValue::from(to.0), &JsValue::from(to.1));

        let msg = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&msg, &JsValue::from_str("type"), &JsValue::from_str("find"));
        let _ = js_sys::Reflect::set(&msg, &JsValue::from_str("id"), &JsValue::from_f64(id as f64));
        let _ = js_sys::Reflect::set(&msg, &JsValue::from_str("from"), &from);
        let _ = js_sys::Reflect::set(&msg, &JsValue::from_str("to"), &to);
        let _ = self.worker.post_message(&msg);
    }

    /// Poll a previously submitted request.  Non-blocking; returns
    /// [`PathPoll::Pending`] until the worker has delivered a result.
    pub fn poll_path(&mut self, id: PathId) -> PathPoll {
        self.results.borrow_mut().remove(&id).unwrap_or(PathPoll::Pending)
    }

    /// Synchronous fallback: run A* inline against the latest snapshot.
    pub fn find_path_sync(&self, from: GridCell, to: GridCell) -> Option<Vec<GridCell>> {
        self.snapshot.find_path(from, to)
    }

    /// Web has no blocking join; determinism is handled by the synchronous
    /// fallback (`synchronous_workers`), not this worker.
    pub fn join(&self) {}
}

impl Drop for PathfinderWorker {
    fn drop(&mut self) {
        self.worker.terminate();
    }
}
