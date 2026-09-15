//! Background workers for heavy engine tasks (pathfinding, terrain kernels,
//! arbitrary guest work).
//!
//! Everything in this crate is `Send` and owns *copies* of read-only engine
//! state (e.g. an `Arc<NavSnapshot>`), never the engine itself.  This is what
//! lets the render thread offload `find_path` and, later, the generic grid
//! kernels and guest background tasks.
//!
//! Background work runs on a [`JobQueue`] (a dedicated thread natively, or
//! inline in the deterministic synchronous mode); on web, true background work
//! uses a dedicated `Worker`.

pub mod guest_worker;
pub mod jobs;
pub mod pathfinder_worker;
#[cfg(not(target_arch = "wasm32"))]
pub mod pool;
pub mod spawn;

pub use classic_core::pathfinder::PathPoll;
pub use guest_worker::{CompiledWorker, GuestWorker, TaskId, WorkerHost};
pub use jobs::{Job, JobId, JobQueue};
pub use pathfinder_worker::{PathfinderWorker, VehiclePathQuery, VehicleSnapshot};
#[cfg(not(target_arch = "wasm32"))]
pub use pool::ThreadPool;
#[cfg(not(target_arch = "wasm32"))]
pub use spawn::spawn_thread;
#[cfg(target_arch = "wasm32")]
pub use spawn::{post_transfer, sab_available, spawn_web_worker};
