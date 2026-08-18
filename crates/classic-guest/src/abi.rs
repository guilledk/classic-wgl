//! The guest ABI: the stable contract between a ROM guest module and the host.
//!
//! Guests import host functions from the module named [`HOST_MODULE`] and
//! export `fn update(dt: f64) -> ()`, plus the optional one-shot lifecycle
//! hooks `fn init()` (early, before the first frame) and `fn start()` (once,
//! after the first `update`).  Strings cross the boundary as `(ptr, len)`
//! pairs into guest linear memory; functions that return a byte slice write
//! into a caller-provided output buffer and return the number of bytes written
//! (`-1` if the buffer was too small).
//!
//! The backend-agnostic marshalling helpers live in [`classic_core::abi`] and
//! are re-exported here so the guest runtimes keep a single import path.

pub use classic_core::abi::{
    bytes_to_f32, bytes_to_u32, f32_array_bytes, f64_pair_bytes, f64_triple_bytes,
    path_cells_bytes, read_bytes_from, read_str_from, write_bytes_to, write_f64_pair_to,
    write_f64_triple_to, write_str_to,
};

/// The WASM module name under which host imports are defined.
pub const HOST_MODULE: &str = "env";

/// The guest export invoked once per frame.
pub const UPDATE_EXPORT: &str = "update";

/// The optional guest export invoked once, before the first frame.
pub const INIT_EXPORT: &str = "init";

/// The optional guest export invoked once, after the first `update`.
pub const START_EXPORT: &str = "start";

/// The name of the guest's linear memory export.
pub const MEMORY_EXPORT: &str = "memory";
