//! Background guest execution (Tier 3): run a ROM guest's *pure* entry points
//! off the render thread against a reduced, engine-free import surface.
//!
//! The foreground guest runtime (`classic-guest`) runs a guest's `update(dt)`
//! against the full SDK, which can mutate the engine.  A guest that also wants
//! to do heavy *computation* (procedural generation, pathfinding composition,
//! noise synthesis) can ship a second entry point that runs here, in a
//! background `.wasm` instance whose host surface is deliberately narrowed to
//! the pure building blocks — noise fields, grid kernels over a host-owned
//! [`FieldRegistry`], A* over a shared [`NavSnapshot`], and a result buffer.
//!
//! Anything that would touch `&mut Engine` (spawn, `set_*`, `commit_terrain`,
//! camera/light, input, UI) is a **trap** in this surface: the import is
//! registered but raising a trap if called, so a worker that reaches for
//! engine state fails loudly instead of silently corrupting nothing.

use std::sync::Arc;

use classic_core::fields::FieldRegistry;
use classic_core::pathfinder::{GridCell, NavSnapshot};
use classic_core::terrain::kernels::{FieldOp, Reduce};

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(not(target_arch = "wasm32"))]
pub use native::{CompiledWorker, GuestWorker};
#[cfg(target_arch = "wasm32")]
pub use web::{CompiledWorker, GuestWorker};

/// Request/result correlation id (owned by the caller, e.g. the engine).
pub type TaskId = u64;

/// Map an integer to a [`FieldOp`] (0 add, 1 sub, 2 mul, 3 min, 4 max).
fn field_op(i: i32) -> FieldOp {
    match i {
        1 => FieldOp::Sub,
        2 => FieldOp::Mul,
        3 => FieldOp::Min,
        4 => FieldOp::Max,
        _ => FieldOp::Add,
    }
}

/// Map an integer to a [`Reduce`] (0 min, 1 max, 2 mean, 3 variance).
fn reduce_op(i: i32) -> Reduce {
    match i {
        1 => Reduce::Max,
        2 => Reduce::Mean,
        3 => Reduce::Variance,
        _ => Reduce::Min,
    }
}

/// The store data for a worker guest instance: owned, engine-free, `Send`.
///
/// This is the crux of Tier 3 — unlike [`classic_guest::sdk::GuestHost`], which
/// holds a `*mut Engine`, a worker owns only read-only/copyable state.  The
/// worker cannot reach the engine at all, so "mutating imports trap" is a
/// clarity guarantee rather than the only line of defence.
pub struct WorkerHost {
    /// Shared, immutable nav grid for A* (re-shared by the engine on rebuild).
    nav: Arc<NavSnapshot>,
    /// Host-owned field-buffer scratch for grid kernels.
    fields: FieldRegistry,
    /// The current task's input argument (bytes copied in by the worker).
    arg: Vec<u8>,
    /// The current task's result (bytes written by the guest via `task_return`).
    result: Vec<u8>,
}

impl WorkerHost {
    pub fn new(nav: Arc<NavSnapshot>) -> Self {
        Self { nav, fields: FieldRegistry::default(), arg: Vec::new(), result: Vec::new() }
    }

    /// Replace the shared nav snapshot (called when the engine rebuilds it).
    pub fn set_nav(&mut self, nav: Arc<NavSnapshot>) {
        self.nav = nav;
    }

    /// Set the input argument for the task about to run.
    pub fn set_arg(&mut self, arg: Vec<u8>) {
        self.arg = arg;
    }

    /// Read the current task's input argument (owned copy for the ABI).
    pub fn task_arg(&mut self) -> Vec<u8> {
        self.arg.clone()
    }

    /// Record the guest's result bytes.
    pub fn task_return(&mut self, bytes: Vec<u8>) {
        self.result = bytes;
    }

    /// Take the accumulated result bytes (clearing the buffer).
    pub fn take_result(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.result)
    }

    pub fn log(&mut self, msg: &str) {
        classic_core::cl_info!(classic_core::instrument::Chan::Guest, "{}", msg);
    }

    // ---- noise fields (pure host generation) --------------------------------

    #[allow(clippy::too_many_arguments)]
    pub fn fbm_field(
        &mut self,
        w: i32,
        h: i32,
        seed: &str,
        octaves: u32,
        freq: f64,
        lacunarity: f64,
        gain: f64,
    ) -> Vec<f32> {
        classic_core::terrain::noise_fields::fbm_field(w, h, seed, octaves, freq, lacunarity, gain)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn ridged_field(
        &mut self,
        w: i32,
        h: i32,
        seed: &str,
        octaves: u32,
        freq: f64,
        lacunarity: f64,
        gain: f64,
        warp_amp: f64,
    ) -> Vec<f32> {
        classic_core::terrain::noise_fields::ridged_field(
            w, h, seed, octaves, freq, lacunarity, gain, warp_amp,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn billow_field(
        &mut self,
        w: i32,
        h: i32,
        seed: &str,
        octaves: u32,
        freq: f64,
        lacunarity: f64,
        gain: f64,
    ) -> Vec<f32> {
        classic_core::terrain::noise_fields::billow_field(
            w, h, seed, octaves, freq, lacunarity, gain,
        )
    }

    pub fn tiling_field(
        &mut self,
        w: i32,
        h: i32,
        seed: &str,
        period: f64,
        octaves: u32,
        radius: f64,
    ) -> Vec<f32> {
        classic_core::terrain::noise_fields::tiling_field(w, h, seed, period, octaves, radius)
    }

    pub fn noise_field(
        &mut self,
        w: i32,
        h: i32,
        seed: &str,
        freq_x: f64,
        freq_y: f64,
    ) -> Vec<f32> {
        classic_core::terrain::noise_fields::noise_field(w, h, seed, freq_x, freq_y)
    }

    pub fn noise2d(&mut self, seed: &str, x: f64, y: f64) -> f64 {
        classic_core::terrain::noise_fields::noise2d(seed, x, y)
    }

    // ---- field-buffer registry + grid kernels ------------------------------

    pub fn alloc_field(&mut self, name: &str, w: i32, h: i32, dtype: i32) -> i32 {
        self.fields.alloc(name, w, h, classic_core::fields::FieldDtype::from_i32(dtype)) as i32
    }

    pub fn free_field(&mut self, name: &str) -> i32 {
        self.fields.free(name) as i32
    }

    pub fn write_field(&mut self, name: &str, data: &[f32]) -> i32 {
        self.fields.write(name, data) as i32
    }

    pub fn write_field_u32(&mut self, name: &str, data: &[u32]) -> i32 {
        self.fields.write_u32(name, data) as i32
    }

    pub fn read_field(&mut self, name: &str) -> Vec<f32> {
        self.fields.f32(name).map(|(d, _, _)| d.to_vec()).unwrap_or_default()
    }

    pub fn map_field(&mut self, op: i32, dst: &str, src: &str) -> i32 {
        self.fields.map_field(field_op(op), dst, src) as i32
    }

    pub fn map_scalar(&mut self, op: i32, dst: &str, scalar: f64) -> i32 {
        self.fields.map_scalar(field_op(op), dst, scalar as f32) as i32
    }

    pub fn blur_box_field(&mut self, name: &str, radius: i32) -> i32 {
        self.fields.blur_box(name, radius) as i32
    }

    pub fn relax_slopes_field(
        &mut self,
        name: &str,
        max_slope: f64,
        iterations: i32,
        tolerance: f64,
        pinned: &str,
    ) -> f64 {
        let pinned = if pinned.is_empty() { None } else { Some(pinned) };
        self.fields
            .relax_slopes(
                name,
                max_slope as f32,
                iterations.max(0) as u32,
                tolerance as f32,
                pinned,
            )
            .map(|(_, worst)| worst as f64)
            .unwrap_or(-1.0)
    }

    pub fn gradient_magnitude_field(&mut self, heights: &str, dst: &str) -> i32 {
        self.fields.gradient_magnitude(heights, dst) as i32
    }

    pub fn threshold_le_field(&mut self, src: &str, dst: &str, t: f64) -> i32 {
        self.fields.threshold_le(src, dst, t as f32) as i32
    }

    pub fn prune_components_field(&mut self, name: &str) -> i32 {
        self.fields.prune_components(name) as i32
    }

    pub fn reduce_field(&mut self, name: &str, op: i32) -> f64 {
        self.fields.reduce(name, reduce_op(op)).unwrap_or(f32::NAN) as f64
    }

    // ---- pathfinding over the shared snapshot ------------------------------

    /// Synchronous A* over the worker's shared nav snapshot.
    pub fn find_path(&mut self, sx: i32, sy: i32, ex: i32, ey: i32) -> Option<Vec<GridCell>> {
        self.nav.find_path((sx, sy), (ex, ey))
    }
}

/// Generate the `install_imports` body for the worker guest runtime backends.
///
/// The worker surface is the *pure* subset of the console SDK — `log`, noise
/// fields, the field/kernel registry, synchronous pathfinding, and the task
/// argument/result buffer — i.e. the ABI table's `tier3` entries
/// (`classic_core::abi_manifest`).  Engine-mutating imports (`tier3_trap`) are
/// registered as trap stubs with their real signatures (any call raises
/// `$trap("name")`); the remaining SDK imports are simply absent, so a worker
/// guest that imports them fails to link.
///
/// `$err` is the backend's error type, `$trap` a `fn(&str) -> $err` that
/// produces a trap, and the remaining parameters are backend-local memory
/// marshalling helpers (`read_str`, `read_bytes`, `write_bytes`).
macro_rules! install_worker_imports {
    ($linker:ident, $host:ty, $err:ty, $trap:path, $read_str:path, $read_bytes:path, $write_bytes:path) => {{
        classic_core::link_host_imports!(tier3 {
            linker: $linker,
            host: $host,
            module: "env",
            access: [],
            read_str: $read_str,
            read_bytes: $read_bytes,
            write_bytes: $write_bytes,
            err: $err,
            trap: $trap,
        });
        Ok(())
    }};
}

pub(crate) use install_worker_imports;
