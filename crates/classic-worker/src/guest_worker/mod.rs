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
/// argument/result buffer.  Engine-mutating imports are registered as trap
/// stubs (any call raises `$trap("name")`); the remaining SDK imports are
/// simply absent, so a worker guest that imports them fails to link.
///
/// `$err` is the backend's error type, `$trap` a `fn(&str) -> $err` that
/// produces a trap, and the remaining parameters are backend-local memory
/// marshalling helpers (`read_str`, `read_bytes`, `write_bytes`).
macro_rules! install_worker_imports {
    ($linker:ident, $host:ty, $err:ty, $trap:path, $read_str:path, $read_bytes:path, $write_bytes:path) => {{
        let m = "env";

        $linker.func_wrap(m, "log", |mut caller: Caller<'_, $host>, ptr: i32, len: i32| {
            let msg = $read_str(&mut caller, ptr, len);
            caller.data_mut().log(&msg);
        })?;

        // ---- Bulk noise fields (host generates → guest buffer) --------------

        $linker.func_wrap(
            m,
            "fbm_field",
            |mut caller: Caller<'_, $host>,
             w: i32,
             h: i32,
             seed_ptr: i32,
             seed_len: i32,
             octaves: i32,
             freq: f64,
             lacunarity: f64,
             gain: f64,
             out_ptr: i32,
             out_cap: i32|
             -> i32 {
                let seed = $read_str(&mut caller, seed_ptr, seed_len);
                let field = caller.data_mut().fbm_field(
                    w,
                    h,
                    &seed,
                    octaves.max(0) as u32,
                    freq,
                    lacunarity,
                    gain,
                );
                let bytes = classic_core::abi::f32_array_bytes(&field);
                if bytes.len() > out_cap.max(0) as usize {
                    return -1;
                }
                $write_bytes(&mut caller, out_ptr, &bytes);
                bytes.len() as i32
            },
        )?;

        $linker.func_wrap(
            m,
            "ridged_field",
            |mut caller: Caller<'_, $host>,
             w: i32,
             h: i32,
             seed_ptr: i32,
             seed_len: i32,
             octaves: i32,
             freq: f64,
             lacunarity: f64,
             gain: f64,
             warp_amp: f64,
             out_ptr: i32,
             out_cap: i32|
             -> i32 {
                let seed = $read_str(&mut caller, seed_ptr, seed_len);
                let field = caller.data_mut().ridged_field(
                    w,
                    h,
                    &seed,
                    octaves.max(0) as u32,
                    freq,
                    lacunarity,
                    gain,
                    warp_amp,
                );
                let bytes = classic_core::abi::f32_array_bytes(&field);
                if bytes.len() > out_cap.max(0) as usize {
                    return -1;
                }
                $write_bytes(&mut caller, out_ptr, &bytes);
                bytes.len() as i32
            },
        )?;

        $linker.func_wrap(
            m,
            "billow_field",
            |mut caller: Caller<'_, $host>,
             w: i32,
             h: i32,
             seed_ptr: i32,
             seed_len: i32,
             octaves: i32,
             freq: f64,
             lacunarity: f64,
             gain: f64,
             out_ptr: i32,
             out_cap: i32|
             -> i32 {
                let seed = $read_str(&mut caller, seed_ptr, seed_len);
                let field = caller.data_mut().billow_field(
                    w,
                    h,
                    &seed,
                    octaves.max(0) as u32,
                    freq,
                    lacunarity,
                    gain,
                );
                let bytes = classic_core::abi::f32_array_bytes(&field);
                if bytes.len() > out_cap.max(0) as usize {
                    return -1;
                }
                $write_bytes(&mut caller, out_ptr, &bytes);
                bytes.len() as i32
            },
        )?;

        $linker.func_wrap(
            m,
            "tiling_field",
            |mut caller: Caller<'_, $host>,
             w: i32,
             h: i32,
             seed_ptr: i32,
             seed_len: i32,
             period: f64,
             octaves: i32,
             radius: f64,
             out_ptr: i32,
             out_cap: i32|
             -> i32 {
                let seed = $read_str(&mut caller, seed_ptr, seed_len);
                let field = caller.data_mut().tiling_field(
                    w,
                    h,
                    &seed,
                    period,
                    octaves.max(0) as u32,
                    radius,
                );
                let bytes = classic_core::abi::f32_array_bytes(&field);
                if bytes.len() > out_cap.max(0) as usize {
                    return -1;
                }
                $write_bytes(&mut caller, out_ptr, &bytes);
                bytes.len() as i32
            },
        )?;

        $linker.func_wrap(
            m,
            "noise_field",
            |mut caller: Caller<'_, $host>,
             w: i32,
             h: i32,
             seed_ptr: i32,
             seed_len: i32,
             freq_x: f64,
             freq_y: f64,
             out_ptr: i32,
             out_cap: i32|
             -> i32 {
                let seed = $read_str(&mut caller, seed_ptr, seed_len);
                let field = caller.data_mut().noise_field(w, h, &seed, freq_x, freq_y);
                let bytes = classic_core::abi::f32_array_bytes(&field);
                if bytes.len() > out_cap.max(0) as usize {
                    return -1;
                }
                $write_bytes(&mut caller, out_ptr, &bytes);
                bytes.len() as i32
            },
        )?;

        $linker.func_wrap(
            m,
            "noise2d",
            |mut caller: Caller<'_, $host>, seed_ptr: i32, seed_len: i32, x: f64, y: f64| -> f64 {
                let seed = $read_str(&mut caller, seed_ptr, seed_len);
                caller.data_mut().noise2d(&seed, x, y)
            },
        )?;

        // ---- Field-buffer registry + grid kernels --------------------------

        $linker.func_wrap(
            m,
            "alloc_field",
            |mut caller: Caller<'_, $host>,
             ptr: i32,
             len: i32,
             w: i32,
             h: i32,
             dtype: i32|
             -> i32 {
                let name = $read_str(&mut caller, ptr, len);
                caller.data_mut().alloc_field(&name, w, h, dtype)
            },
        )?;

        $linker.func_wrap(
            m,
            "free_field",
            |mut caller: Caller<'_, $host>, ptr: i32, len: i32| -> i32 {
                let name = $read_str(&mut caller, ptr, len);
                caller.data_mut().free_field(&name)
            },
        )?;

        $linker.func_wrap(
            m,
            "write_field",
            |mut caller: Caller<'_, $host>,
             ptr: i32,
             len: i32,
             data_ptr: i32,
             data_len: i32|
             -> i32 {
                let name = $read_str(&mut caller, ptr, len);
                let data =
                    classic_core::abi::bytes_to_f32(&$read_bytes(&mut caller, data_ptr, data_len));
                caller.data_mut().write_field(&name, &data)
            },
        )?;

        $linker.func_wrap(
            m,
            "write_field_u32",
            |mut caller: Caller<'_, $host>,
             ptr: i32,
             len: i32,
             data_ptr: i32,
             data_len: i32|
             -> i32 {
                let name = $read_str(&mut caller, ptr, len);
                let data =
                    classic_core::abi::bytes_to_u32(&$read_bytes(&mut caller, data_ptr, data_len));
                caller.data_mut().write_field_u32(&name, &data)
            },
        )?;

        $linker.func_wrap(
            m,
            "read_field",
            |mut caller: Caller<'_, $host>,
             ptr: i32,
             len: i32,
             out_ptr: i32,
             out_cap: i32|
             -> i32 {
                let name = $read_str(&mut caller, ptr, len);
                let field = caller.data_mut().read_field(&name);
                let bytes = classic_core::abi::f32_array_bytes(&field);
                if bytes.len() > out_cap.max(0) as usize {
                    return -1;
                }
                $write_bytes(&mut caller, out_ptr, &bytes);
                bytes.len() as i32
            },
        )?;

        $linker.func_wrap(
            m,
            "map_field",
            |mut caller: Caller<'_, $host>,
             op: i32,
             dst_ptr: i32,
             dst_len: i32,
             src_ptr: i32,
             src_len: i32|
             -> i32 {
                let dst = $read_str(&mut caller, dst_ptr, dst_len);
                let src = $read_str(&mut caller, src_ptr, src_len);
                caller.data_mut().map_field(op, &dst, &src)
            },
        )?;

        $linker.func_wrap(
            m,
            "map_scalar",
            |mut caller: Caller<'_, $host>,
             op: i32,
             dst_ptr: i32,
             dst_len: i32,
             scalar: f64|
             -> i32 {
                let dst = $read_str(&mut caller, dst_ptr, dst_len);
                caller.data_mut().map_scalar(op, &dst, scalar)
            },
        )?;

        $linker.func_wrap(
            m,
            "blur_box_field",
            |mut caller: Caller<'_, $host>, ptr: i32, len: i32, radius: i32| -> i32 {
                let name = $read_str(&mut caller, ptr, len);
                caller.data_mut().blur_box_field(&name, radius)
            },
        )?;

        $linker.func_wrap(
            m,
            "relax_slopes_field",
            |mut caller: Caller<'_, $host>,
             ptr: i32,
             len: i32,
             max_slope: f64,
             iterations: i32,
             tolerance: f64,
             pinned_ptr: i32,
             pinned_len: i32|
             -> f64 {
                let name = $read_str(&mut caller, ptr, len);
                let pinned = $read_str(&mut caller, pinned_ptr, pinned_len);
                caller
                    .data_mut()
                    .relax_slopes_field(&name, max_slope, iterations, tolerance, &pinned)
            },
        )?;

        $linker.func_wrap(
            m,
            "gradient_magnitude_field",
            |mut caller: Caller<'_, $host>,
             heights_ptr: i32,
             heights_len: i32,
             dst_ptr: i32,
             dst_len: i32|
             -> i32 {
                let heights = $read_str(&mut caller, heights_ptr, heights_len);
                let dst = $read_str(&mut caller, dst_ptr, dst_len);
                caller.data_mut().gradient_magnitude_field(&heights, &dst)
            },
        )?;

        $linker.func_wrap(
            m,
            "threshold_le_field",
            |mut caller: Caller<'_, $host>,
             src_ptr: i32,
             src_len: i32,
             dst_ptr: i32,
             dst_len: i32,
             t: f64|
             -> i32 {
                let src = $read_str(&mut caller, src_ptr, src_len);
                let dst = $read_str(&mut caller, dst_ptr, dst_len);
                caller.data_mut().threshold_le_field(&src, &dst, t)
            },
        )?;

        $linker.func_wrap(
            m,
            "prune_components_field",
            |mut caller: Caller<'_, $host>, ptr: i32, len: i32| -> i32 {
                let name = $read_str(&mut caller, ptr, len);
                caller.data_mut().prune_components_field(&name)
            },
        )?;

        $linker.func_wrap(
            m,
            "reduce_field",
            |mut caller: Caller<'_, $host>, ptr: i32, len: i32, op: i32| -> f64 {
                let name = $read_str(&mut caller, ptr, len);
                caller.data_mut().reduce_field(&name, op)
            },
        )?;

        // ---- Pathfinding (synchronous, over the shared snapshot) -----------

        $linker.func_wrap(
            m,
            "find_path",
            |mut caller: Caller<'_, $host>,
             sx: i32,
             sy: i32,
             ex: i32,
             ey: i32,
             out_ptr: i32,
             out_cap: i32|
             -> i32 {
                let Some(cells) = caller.data_mut().find_path(sx, sy, ex, ey) else {
                    return -1;
                };
                let bytes = classic_core::abi::path_cells_bytes(&cells);
                if bytes.len() > out_cap.max(0) as usize {
                    return -2;
                }
                $write_bytes(&mut caller, out_ptr, &bytes);
                cells.len() as i32
            },
        )?;

        // ---- Task argument / result buffer ---------------------------------

        $linker.func_wrap(
            m,
            "task_arg",
            |mut caller: Caller<'_, $host>, out_ptr: i32, out_cap: i32| -> i32 {
                let arg = caller.data_mut().task_arg();
                if arg.len() > out_cap.max(0) as usize {
                    return -1;
                }
                $write_bytes(&mut caller, out_ptr, &arg)
            },
        )?;

        $linker.func_wrap(
            m,
            "task_return",
            |mut caller: Caller<'_, $host>, ptr: i32, len: i32| {
                let bytes = $read_bytes(&mut caller, ptr, len);
                caller.data_mut().task_return(bytes);
            },
        )?;

        // ---- Engine-mutating imports: trap ---------------------------------
        // Registered (with their real signatures) so a worker guest that reaches
        // for engine state traps with a clear message instead of failing to link
        // with an opaque "unknown import".  Remaining SDK imports are absent.

        $linker.func_wrap(
            m,
            "spawn",
            |_caller: Caller<'_, $host>, _ptr: i32, _len: i32| -> Result<i32, $err> {
                Err($trap("spawn"))
            },
        )?;

        $linker.func_wrap(
            m,
            "despawn",
            |_caller: Caller<'_, $host>, _ptr: i32, _len: i32| -> Result<i32, $err> {
                Err($trap("despawn"))
            },
        )?;

        $linker.func_wrap(
            m,
            "has",
            |_caller: Caller<'_, $host>, _ptr: i32, _len: i32| -> Result<i32, $err> {
                Err($trap("has"))
            },
        )?;

        $linker.func_wrap(
            m,
            "set_pos",
            |_caller: Caller<'_, $host>,
             _ptr: i32,
             _len: i32,
             _x: f64,
             _y: f64,
             _z: f64|
             -> Result<i32, $err> { Err($trap("set_pos")) },
        )?;

        $linker.func_wrap(
            m,
            "get_pos",
            |_caller: Caller<'_, $host>,
             _ptr: i32,
             _len: i32,
             _out_ptr: i32|
             -> Result<i32, $err> { Err($trap("get_pos")) },
        )?;

        $linker.func_wrap(
            m,
            "height_at",
            |_caller: Caller<'_, $host>, _x: f64, _y: f64| -> Result<f64, $err> {
                Err($trap("height_at"))
            },
        )?;

        $linker.func_wrap(
            m,
            "set_anim",
            |_caller: Caller<'_, $host>,
             _ptr: i32,
             _len: i32,
             _anim_ptr: i32,
             _anim_len: i32|
             -> Result<i32, $err> { Err($trap("set_anim")) },
        )?;

        $linker.func_wrap(
            m,
            "start_anim",
            |_caller: Caller<'_, $host>,
             _ptr: i32,
             _len: i32,
             _anim_ptr: i32,
             _anim_len: i32,
             _repeat: i32|
             -> Result<i32, $err> { Err($trap("start_anim")) },
        )?;

        $linker.func_wrap(
            m,
            "set_tile",
            |_caller: Caller<'_, $host>, _x: i32, _y: i32, _id: i32| -> Result<i32, $err> {
                Err($trap("set_tile"))
            },
        )?;

        $linker.func_wrap(
            m,
            "set_height",
            |_caller: Caller<'_, $host>, _x: i32, _y: i32, _h: f64| -> Result<i32, $err> {
                Err($trap("set_height"))
            },
        )?;

        $linker.func_wrap(
            m,
            "rebuild_terrain",
            |_caller: Caller<'_, $host>| -> Result<i32, $err> { Err($trap("rebuild_terrain")) },
        )?;

        $linker.func_wrap(
            m,
            "request_path",
            |_caller: Caller<'_, $host>,
             _sx: i32,
             _sy: i32,
             _ex: i32,
             _ey: i32|
             -> Result<i32, $err> { Err($trap("request_path")) },
        )?;

        $linker.func_wrap(
            m,
            "poll_path",
            |_caller: Caller<'_, $host>,
             _id: i32,
             _out_ptr: i32,
             _out_cap: i32|
             -> Result<i32, $err> { Err($trap("poll_path")) },
        )?;

        $linker.func_wrap(
            m,
            "get_camera",
            |_caller: Caller<'_, $host>, _out_ptr: i32| -> Result<i32, $err> {
                Err($trap("get_camera"))
            },
        )?;

        $linker.func_wrap(
            m,
            "set_camera",
            |_caller: Caller<'_, $host>, _x: f64, _y: f64, _scale: f64| -> Result<i32, $err> {
                Err($trap("set_camera"))
            },
        )?;

        $linker.func_wrap(
            m,
            "set_grid",
            |_caller: Caller<'_, $host>, _show: i32| -> Result<i32, $err> {
                Err($trap("set_grid"))
            },
        )?;

        $linker.func_wrap(
            m,
            "get_light",
            |_caller: Caller<'_, $host>, _out_ptr: i32| -> Result<i32, $err> {
                Err($trap("get_light"))
            },
        )?;

        $linker.func_wrap(
            m,
            "set_light",
            |_caller: Caller<'_, $host>,
             _a0: f64,
             _a1: f64,
             _a2: f64,
             _d0: f64,
             _d1: f64,
             _d2: f64,
             _c0: f64,
             _c1: f64,
             _c2: f64|
             -> Result<i32, $err> { Err($trap("set_light")) },
        )?;

        $linker.func_wrap(
            m,
            "set_tiles",
            |_caller: Caller<'_, $host>, _ptr: i32, _len: i32| -> Result<i32, $err> {
                Err($trap("set_tiles"))
            },
        )?;

        $linker.func_wrap(
            m,
            "set_heights",
            |_caller: Caller<'_, $host>, _ptr: i32, _len: i32| -> Result<i32, $err> {
                Err($trap("set_heights"))
            },
        )?;

        $linker.func_wrap(
            m,
            "set_nav",
            |_caller: Caller<'_, $host>, _ptr: i32, _len: i32| -> Result<i32, $err> {
                Err($trap("set_nav"))
            },
        )?;

        $linker.func_wrap(
            m,
            "set_tileset",
            |_caller: Caller<'_, $host>,
             _ptr: i32,
             _len: i32,
             _w: i32,
             _h: i32|
             -> Result<i32, $err> { Err($trap("set_tileset")) },
        )?;

        $linker.func_wrap(
            m,
            "commit_terrain",
            |_caller: Caller<'_, $host>, _height_scale: f64| -> Result<i32, $err> {
                Err($trap("commit_terrain"))
            },
        )?;

        Ok(())
    }};
}

pub(crate) use install_worker_imports;
