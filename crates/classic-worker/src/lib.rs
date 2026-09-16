//! Background workers for heavy engine tasks (pathfinding, terrain kernels,
//! arbitrary guest work).
//!
//! Everything in this crate is `Send` and owns *copies* of read-only engine
//! state (e.g. an `Arc<NavSnapshot>`), never the engine itself.  This is what
//! lets the render thread offload `find_path` and, later, the generic grid
//! kernels and guest background tasks.
//!
//! # Architecture (AGENTS.md "Patterns" 2, 4, 5, 7)
//!
//! - **All background work starts here.**  [`JobQueue`] runs it — `threaded`
//!   (one named thread), `pooled` (the boot decode fan-out) or `synchronous`
//!   (inline) — and `spawn_thread` / `spawn_web_worker` are the only places
//!   a thread or a web `Worker` is created (`cargo xtask check-patterns`).
//! - **`JobQueue::synchronous` is the one determinism switch.**  The engine
//!   always submits; only how the queue was built decides whether the work runs
//!   inline, so the golden path never takes a separate code path.
//! - **Backends split at the crate boundary**, not inline: each worker is a
//!   `native.rs` / `web.rs` pair behind one API — natively a `JobQueue` on a
//!   thread, on web a dedicated `Worker` (or an inline queue when synchronous).
//! - **Web transport is zero-copy**: every buffer posted to or from a `Worker`
//!   goes through `post_transfer`, and `SharedArrayBuffer` use stays gated behind
//!   `sab_available`.

pub mod guest_worker;
pub mod jobs;
pub mod pathfinder_worker;
pub mod spawn;
#[cfg(target_arch = "wasm32")]
pub mod transcoder_worker;

pub use classic_core::pathfinder::PathPoll;
pub use guest_worker::{CompiledWorker, GuestWorker, TaskId, WorkerHost};
pub use jobs::{Job, JobId, JobQueue};
pub use pathfinder_worker::{PathfinderWorker, VehiclePathQuery, VehicleSnapshot};
#[cfg(not(target_arch = "wasm32"))]
pub use spawn::spawn_thread;
#[cfg(target_arch = "wasm32")]
pub use spawn::{post_transfer, sab_available, spawn_web_worker};
