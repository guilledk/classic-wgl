#![cfg_attr(target_arch = "wasm32", no_std)]
#![allow(static_mut_refs)]

//! ROM guest for the `lrvtest` scene: a hand-authored ramp course for visually
//! tuning the wheeled-vehicle suspension/jump.
//!
//! The map (tiles/heights/nav grids) is declared inline in the scene's
//! `state.json` and hydrated by the host; this guest commits it, spawns the LRV
//! beside the course, and drives it with click-to-move.

#[cfg(target_arch = "wasm32")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}

#[cfg(target_arch = "wasm32")]
#[global_allocator]
static ALLOC: dlmalloc::GlobalDlmalloc = dlmalloc::GlobalDlmalloc;

#[cfg(target_arch = "wasm32")]
mod host {
    #[link(wasm_import_module = "env")]
    extern "C" {
        pub fn commit_terrain(height_scale: f64) -> i32;
        pub fn iso_to_screen(x: f64, y: f64, out_ptr: i32) -> i32;
        pub fn set_camera(x: f64, y: f64, scale: f64) -> i32;
        pub fn set_grid(show: i32) -> i32;
        pub fn was_pressed(btn: i32) -> i32;
        pub fn mouse_iso(out_ptr: i32) -> i32;
        pub fn ui_consumed_click() -> i32;
        pub fn vehicle_spawn(
            def_ptr: i32,
            def_len: i32,
            name_ptr: i32,
            name_len: i32,
            x: f64,
            y: f64,
        ) -> i32;
        pub fn vehicle_goto(name_ptr: i32, name_len: i32, tx: i32, ty: i32) -> i32;
    }
}

#[cfg(target_arch = "wasm32")]
static LRV: &[u8] = b"lrv";

#[cfg(target_arch = "wasm32")]
static LRV_DEF: &[u8] = b"lrv";

#[cfg(target_arch = "wasm32")]
#[inline]
fn read_f64(buf: &[u8]) -> f64 {
    f64::from_le_bytes([buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7]])
}

/// Called once, before the first frame: commit the hand-authored grids, spawn
/// the LRV at a flat corner, and frame the camera over the ramp course.
#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn init() {
    // SAFETY: single-threaded guest.
    unsafe {
        host::commit_terrain(32.0);
    }

    let (dp, dl) = (LRV_DEF.as_ptr() as i32, LRV_DEF.len() as i32);
    let (np, nl) = (LRV.as_ptr() as i32, LRV.len() as i32);
    // SAFETY: single-threaded guest.
    unsafe {
        host::vehicle_spawn(dp, dl, np, nl, 5.0, 5.0);
    }

    // SAFETY: single-threaded guest.
    unsafe {
        let mut screen = [0u8; 16];
        if host::iso_to_screen(24.0, 24.0, screen.as_mut_ptr() as i32) == 1 {
            let sx = read_f64(&screen[0..8]);
            let sy = read_f64(&screen[8..16]);
            host::set_camera(sx, sy, 0.8);
        }
        host::set_grid(0);
    }
}

/// Click-to-move: a left click (not consumed by UI) drives the LRV to the tile
/// under the cursor via the host's `vehicle_goto` (host-side A*).
#[cfg(target_arch = "wasm32")]
fn handle_click() {
    // SAFETY: single-threaded guest.
    unsafe {
        if host::was_pressed(0) == 0 || host::ui_consumed_click() != 0 {
            return;
        }
        let mut mouse = [0u8; 16];
        if host::mouse_iso(mouse.as_mut_ptr() as i32) != 1 {
            return;
        }
        let mx = read_f64(&mouse[0..8]) as i32;
        let my = read_f64(&mouse[8..16]) as i32;
        let (np, nl) = (LRV.as_ptr() as i32, LRV.len() as i32);
        host::vehicle_goto(np, nl, mx, my);
    }
}

/// Called once per frame.
#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn update(_dt: f64) {
    handle_click();
}
