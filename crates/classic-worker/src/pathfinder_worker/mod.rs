//! Host-owned A* pathfinding worker: a platform-specific backend behind a
//! common `PathfinderWorker` API.
//!
//! Native uses a dedicated `std::thread` + `mpsc`; web uses a dedicated
//! `Worker` running the A* algorithm in JavaScript (`worker.js`).

/// Request/result correlation id (owned by the caller, e.g. the engine).
pub type PathId = u64;

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(not(target_arch = "wasm32"))]
pub use native::PathfinderWorker;
#[cfg(target_arch = "wasm32")]
pub use web::PathfinderWorker;
