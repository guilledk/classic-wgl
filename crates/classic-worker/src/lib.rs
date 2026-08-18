//! Background workers for heavy engine tasks (pathfinding, terrain kernels,
//! arbitrary guest work).
//!
//! Everything in this crate is `Send` and owns *copies* of read-only engine
//! state (e.g. an `Arc<NavSnapshot>`), never the engine itself.  This is what
//! lets the render thread offload `find_path` and, later, the generic grid
//! kernels and guest background tasks.
//!
//! Native backends use `std::thread` + `std::sync::mpsc`; the web backend runs
//! A* in a dedicated `Worker`.

pub mod guest_worker;
pub mod pathfinder_worker;
#[cfg(not(target_arch = "wasm32"))]
pub mod pool;

pub use classic_core::pathfinder::PathPoll;
pub use guest_worker::{GuestWorker, TaskId, WorkerHost};
pub use pathfinder_worker::PathfinderWorker;
#[cfg(not(target_arch = "wasm32"))]
pub use pool::ThreadPool;
