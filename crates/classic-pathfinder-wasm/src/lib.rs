//! Thin `#[no_mangle]` wasm ABI over `classic_pathfinder::PathfinderState`.
//!
//! Built to `pathfinder.wasm` (`cargo xtask build-pathfinder`) and instantiated
//! by the web `Worker` shim in `classic-worker`.  It is the *same* Rust
//! pathfinder the native worker thread runs, so native and web routes are
//! identical by construction.
//!
//! The module is stateful and single-threaded (a `Worker` is single-threaded):
//! `set_snapshot` / `set_vehicle_snapshot` upload a grid copy, `find` /
//! `find_vehicle` run the search and leave the result in an internal buffer,
//! and `result_ptr` exposes that buffer to the shim.  `alloc` returns a
//! caller-writable scratch buffer for uploading input grids.

// The raw pointers come from the wasm linear memory across the JS/WASM ABI
// boundary (`#[no_mangle] extern "C"` symbols instantiated by the Worker), so
// `not_unsafe_ptr_arg_deref`'s "mark it `unsafe fn`" does not apply here.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::sync::{Mutex, MutexGuard};

use classic_pathfinder::{GridCell, NavSnapshot, PathfinderState, VehicleNavSnapshot};

struct WState {
    pathfinder: PathfinderState,
    result: Vec<i32>,
}

static STATE: Mutex<Option<WState>> = Mutex::new(None);

fn state() -> MutexGuard<'static, Option<WState>> {
    STATE.lock().unwrap_or_else(|p| p.into_inner())
}

/// Allocate a caller-writable scratch buffer of `len` bytes.  Leaked (inputs are
/// uploaded rarely — once per terrain commit, or a tiny footprint per request).
#[no_mangle]
pub extern "C" fn alloc(len: i32) -> *mut u8 {
    let len = len.max(0) as usize;
    let mut v = Vec::<u8>::with_capacity(len);
    let p = v.as_mut_ptr();
    core::mem::forget(v);
    p
}

#[no_mangle]
pub extern "C" fn set_snapshot(size_x: i32, size_y: i32, ptr: *const i32, len: i32) {
    let data = unsafe { core::slice::from_raw_parts(ptr, len.max(0) as usize) }.to_vec();
    let nav = NavSnapshot::new(size_x, size_y, data);
    let mut st = state();
    match st.as_mut() {
        Some(st) => st.pathfinder.set_nav(nav),
        None => *st = Some(WState { pathfinder: PathfinderState::new(nav), result: Vec::new() }),
    }
}

#[no_mangle]
pub extern "C" fn set_vehicle_snapshot(
    size_x: i32,
    size_y: i32,
    structural: *const i32,
    structural_len: i32,
    heights: *const f32,
    heights_len: i32,
    tile_m: f32,
) {
    let structural =
        unsafe { core::slice::from_raw_parts(structural, structural_len.max(0) as usize) }.to_vec();
    let heights =
        unsafe { core::slice::from_raw_parts(heights, heights_len.max(0) as usize) }.to_vec();
    let snap = VehicleNavSnapshot::new(size_x, size_y, structural, heights, tile_m);
    let mut st = state();
    match st.as_mut() {
        Some(st) => st.pathfinder.set_vehicle(snap),
        None => {
            let mut pf = PathfinderState::new(NavSnapshot::new(0, 0, Vec::new()));
            pf.set_vehicle(snap);
            *st = Some(WState { pathfinder: pf, result: Vec::new() });
        }
    }
}

#[no_mangle]
pub extern "C" fn find(from_x: i32, from_y: i32, to_x: i32, to_y: i32) -> i32 {
    let mut st = state();
    let Some(st) = st.as_mut() else { return -1 };
    match st.pathfinder.find((from_x, from_y), (to_x, to_y)) {
        Some(path) => write_result(&mut st.result, path),
        None => -1,
    }
}

#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn find_vehicle(
    from_x: i32,
    from_y: i32,
    to_x: i32,
    to_y: i32,
    footprint: *const i32,
    footprint_len: i32,
    pitch_max: f32,
    roll_max: f32,
    wheelbase_m: f32,
    track_m: f32,
    safe_fall_m: f32,
    jump_cost: f32,
    turn_cost: f32,
) -> i32 {
    let fp_len = footprint_len.max(0) as usize;
    let footprint: Vec<GridCell> = unsafe { core::slice::from_raw_parts(footprint, fp_len * 2) }
        .chunks_exact(2)
        .map(|c| (c[0], c[1]))
        .collect();
    let mut st = state();
    let Some(st) = st.as_mut() else { return -1 };
    match st.pathfinder.find_vehicle(
        (from_x, from_y),
        (to_x, to_y),
        &footprint,
        pitch_max,
        roll_max,
        wheelbase_m,
        track_m,
        safe_fall_m,
        jump_cost,
        turn_cost,
    ) {
        Some(path) => write_result(&mut st.result, path),
        None => -1,
    }
}

/// Pointer to the internal result buffer (flat `[x0, y0, x1, y1, …]`), valid
/// until the next `find` / `find_vehicle`.
#[no_mangle]
pub extern "C" fn result_ptr() -> *const i32 {
    match state().as_ref() {
        Some(st) if !st.result.is_empty() => st.result.as_ptr(),
        _ => core::ptr::null(),
    }
}

fn write_result(result: &mut Vec<i32>, path: Vec<GridCell>) -> i32 {
    result.clear();
    for (x, y) in path {
        result.push(x);
        result.push(y);
    }
    result.len() as i32 / 2
}
