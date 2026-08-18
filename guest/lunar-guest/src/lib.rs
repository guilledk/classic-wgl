#![cfg_attr(target_arch = "wasm32", no_std)]
#![allow(static_mut_refs)]

//! ROM guest for the `lunar` scene: the procedural lunar map generator.
//!
//! Owns the *whole* map-generation behaviour.  `init` submits the initial
//! 400x400 surface generation to the background guest worker (Tier 3), and
//! `update` polls for it, bulk-uploads the grids + tileset, and commits the
//! terrain; `R` re-rolls it the same way.  The heavy, engine-free generation
//! runs off-thread in the `lunar-worker` module, so the render thread never
//! blocks on it.
//!
//! The host is a generic terrain engine (storage + rebuild + pathfinding); the
//! map algorithm lives in the ROM (in the shared generator sources, compiled
//! into both this guest and `lunar-worker`).
//!
//! The generator (`material`/`lunar`/`tileset`) is pure and builds on the open
//! `classic-terrain` noise primitives; the wasm entrypoint (`host` imports,
//! `init`/`update`) is `#[cfg(target_arch = "wasm32")]`-gated so the generator
//! can be unit-tested natively (see `tests/terrain_lunar.rs`).

extern crate alloc;

pub mod lunar;
pub mod material;
pub mod tileset;

pub use lunar::{generate_lunar, LandingZone, LunarParams, LunarStats, LunarTerrain};
pub use material::LunarMaterial;
pub use tileset::{build_default_lunar_tileset, build_lunar_tileset};

#[cfg(target_arch = "wasm32")]
use alloc::{format, string::String, vec};

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
        pub fn was_key_pressed(ptr: i32, len: i32) -> i32;
        pub fn set_tiles(ptr: i32, len: i32) -> i32;
        pub fn set_heights(ptr: i32, len: i32) -> i32;
        pub fn set_nav(ptr: i32, len: i32) -> i32;
        pub fn set_tileset(ptr: i32, len: i32, w: i32, h: i32) -> i32;
        pub fn commit_terrain(height_scale: f64) -> i32;
        pub fn iso_to_screen(x: f64, y: f64, out_ptr: i32) -> i32;
        pub fn set_camera(x: f64, y: f64, scale: f64) -> i32;
        pub fn set_light(
            a0: f64,
            a1: f64,
            a2: f64,
            d0: f64,
            d1: f64,
            d2: f64,
            c0: f64,
            c1: f64,
            c2: f64,
        ) -> i32;
        pub fn set_grid(show: i32) -> i32;
        pub fn set_pos(name_ptr: i32, name_len: i32, x: f64, y: f64, z: f64) -> i32;
        pub fn height_at(x: f64, y: f64) -> f64;
        pub fn start_anim(
            name_ptr: i32,
            name_len: i32,
            anim_ptr: i32,
            anim_len: i32,
            repeat: i32,
        ) -> i32;
        pub fn was_pressed(btn: i32) -> i32;
        pub fn mouse_iso(out_ptr: i32) -> i32;
        pub fn ui_consumed_click() -> i32;
        pub fn vehicle_teleport(name_ptr: i32, name_len: i32, x: f64, y: f64) -> i32;
        pub fn vehicle_goto(name_ptr: i32, name_len: i32, tx: i32, ty: i32) -> i32;
        pub fn vehicle_spawn(
            def_ptr: i32,
            def_len: i32,
            name_ptr: i32,
            name_len: i32,
            x: f64,
            y: f64,
        ) -> i32;
        pub fn spawn_task(entry_ptr: i32, entry_len: i32, arg_ptr: i32, arg_len: i32) -> i32;
        pub fn poll_task(id: i32, out_ptr: i32, out_cap: i32) -> i32;
    }
}

/// Vertical exaggeration for the lunar relief (matches `LUNAR_HEIGHT_SCALE`).
#[cfg(target_arch = "wasm32")]
const HEIGHT_SCALE: f64 = 14.0;

#[cfg(target_arch = "wasm32")]
static KEY_R: &[u8] = b"KeyR";

#[cfg(target_arch = "wasm32")]
static ROCKET: &[u8] = b"rocket";

/// One-shot animation used to (re)start the landing rocket.  `start_anim`
/// zeroes the transient `counter`/`frame`/`offset` fields and replays from
/// frame 0.
#[cfg(target_arch = "wasm32")]
static ROCKET_ANIM: &[u8] = b"rocketLanding";

#[cfg(target_arch = "wasm32")]
static LRV: &[u8] = b"lrv";

/// Vehicle-definition name to spawn (matches the ROM manifest `vehicles` entry).
#[cfg(target_arch = "wasm32")]
static LRV_DEF: &[u8] = b"lrv";

#[cfg(target_arch = "wasm32")]
static mut SEED_N: u32 = 0;

/// The `lunar-worker` export that runs `generate_lunar` off-thread.
#[cfg(target_arch = "wasm32")]
static GENERATE_WORKER: &[u8] = b"generate_worker";

/// Buffer cap for the worker result.  A 400x400 map serializes to ~2.1 MiB
/// (see `lunar-worker::serialize`); 4 MiB leaves ample headroom.
#[cfg(target_arch = "wasm32")]
const RESULT_CAP: usize = 4 * 1024 * 1024;

/// The in-flight generation task id, or `-1` when idle.
#[cfg(target_arch = "wasm32")]
static mut PENDING_TASK: i32 = -1;

/// Whether the pending generation is the *initial* one (runs `setup_view`).
#[cfg(target_arch = "wasm32")]
static mut PENDING_INITIAL: bool = false;

/// Build the seed string for generation `n` (`"0"`, `"1"`, ...).
#[cfg(target_arch = "wasm32")]
fn seed_for(n: u32) -> String {
    format!("{n}")
}

/// Submit a generation to the background worker, recording the task id.
#[cfg(target_arch = "wasm32")]
fn spawn_generation(seed: &str, initial: bool) {
    // SAFETY: single-threaded guest; the seed stays alive across the import
    // (the host copies the argument bytes synchronously).
    unsafe {
        let (sp, sl) = (seed.as_ptr() as i32, seed.len() as i32);
        let (wp, wl) = (GENERATE_WORKER.as_ptr() as i32, GENERATE_WORKER.len() as i32);
        PENDING_TASK = host::spawn_task(wp, wl, sp, sl);
        PENDING_INITIAL = initial;
    }
}

/// Read one little-endian `i32` from a byte buffer, advancing `off`.
#[cfg(target_arch = "wasm32")]
#[inline]
fn read_i32_at(buf: &[u8], off: &mut usize) -> i32 {
    let v = i32::from_le_bytes([buf[*off], buf[*off + 1], buf[*off + 2], buf[*off + 3]]);
    *off += 4;
    v
}

/// Decode a worker result and bulk-upload the grids + tileset, then commit the
/// terrain.  Returns the first landing-zone spawn point (tile coords).
#[cfg(target_arch = "wasm32")]
fn apply_result(buf: &[u8]) -> (i32, i32) {
    let mut off = 0usize;
    let sx = read_i32_at(buf, &mut off);
    let sy = read_i32_at(buf, &mut off);
    let heights_len = (sx + 1) * (sy + 1);
    let tiles_len = sx * sy;

    let base = buf.as_ptr() as i32;

    let heights_ptr = base + off as i32;
    off += heights_len as usize * 4;
    let tiles_ptr = base + off as i32;
    off += tiles_len as usize * 4;
    let nav_ptr = base + off as i32;
    off += tiles_len as usize * 4;

    let spawn_count = read_i32_at(buf, &mut off);
    let spawn = if spawn_count > 0 {
        let x = read_i32_at(buf, &mut off);
        let y = read_i32_at(buf, &mut off);
        (x, y)
    } else {
        (0, 0)
    };
    for _ in 1..spawn_count.max(0) {
        off += 8;
    }

    let tw = read_i32_at(buf, &mut off);
    let th = read_i32_at(buf, &mut off);
    let rgba_ptr = base + off as i32;
    let rgba_len = tw * th * 4;

    // SAFETY: single-threaded guest; `buf` (the worker result) stays alive
    // across these imports, and each pointer points into `buf`.
    unsafe {
        host::set_heights(heights_ptr, heights_len * 4);
        host::set_tiles(tiles_ptr, tiles_len * 4);
        host::set_nav(nav_ptr, tiles_len * 4);
        host::set_tileset(rgba_ptr, rgba_len, tw, th);
        host::commit_terrain(HEIGHT_SCALE);
    }

    spawn
}

/// Poll the pending generation; when it lands, upload the grids, commit the
/// terrain, and (re)place the rocket.  No-op when idle or still pending.
#[cfg(target_arch = "wasm32")]
fn poll_generation() {
    unsafe {
        if PENDING_TASK < 0 {
            return;
        }
        let mut buf = vec![0u8; RESULT_CAP];
        let n = host::poll_task(PENDING_TASK, buf.as_mut_ptr() as i32, buf.len() as i32);
        if n <= 0 {
            // 0 = still pending, -1 = error, -2 = buffer too small.
            return;
        }
        let spawn = apply_result(&buf[..n as usize]);
        if PENDING_INITIAL {
            setup_view(spawn);
        }
        reset_rocket(spawn);
        if PENDING_INITIAL {
            spawn_rover(spawn);
        } else {
            reset_rover(spawn);
        }
        PENDING_TASK = -1;
    }
}

/// Read one little-endian `f64` from a guest buffer.
#[cfg(target_arch = "wasm32")]
#[inline]
fn read_f64(buf: &[u8]) -> f64 {
    f64::from_le_bytes([buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7]])
}

/// The lunar light preset (airless: near-zero ambient, hard low sun).  Direction
/// is stored un-normalised and normalised here to match the host's former
/// `apply_light_preset("lunar")`.
#[cfg(target_arch = "wasm32")]
const LUNAR_AMBIENT: [f64; 3] = [0.20, 0.20, 0.25];
#[cfg(target_arch = "wasm32")]
const LUNAR_DIR: [f64; 3] = [0.42, 0.30, 0.60];
#[cfg(target_arch = "wasm32")]
const LUNAR_COLOR: [f64; 3] = [0.88, 0.86, 0.83];

/// The lunar scene owns its own look: zoom out to show the terrain, centre on
/// the first landing zone, apply the airless light preset, and hide the grid.
#[cfg(target_arch = "wasm32")]
fn setup_view(spawn: (i32, i32)) {
    // Normalise the light direction (f32, matching glam's `Vec3::normalize`).
    let dx = LUNAR_DIR[0] as f32;
    let dy = LUNAR_DIR[1] as f32;
    let dz = LUNAR_DIR[2] as f32;
    let len = libm::sqrtf(dx * dx + dy * dy + dz * dz);
    let recip = 1.0 / len;
    let ndx = dx * recip;
    let ndy = dy * recip;
    let ndz = dz * recip;

    // SAFETY: single-threaded guest.
    unsafe {
        let mut screen = [0u8; 16];
        if host::iso_to_screen(spawn.0 as f64, spawn.1 as f64, screen.as_mut_ptr() as i32) == 1 {
            let sx = read_f64(&screen[0..8]);
            let sy = read_f64(&screen[8..16]);
            host::set_camera(sx, sy, 0.32);
        }
        host::set_light(
            LUNAR_AMBIENT[0],
            LUNAR_AMBIENT[1],
            LUNAR_AMBIENT[2],
            ndx as f64,
            ndy as f64,
            ndz as f64,
            LUNAR_COLOR[0],
            LUNAR_COLOR[1],
            LUNAR_COLOR[2],
        );
        host::set_grid(0);
    }
}

/// Place the landing rocket at the first landing-zone spawn point and (re)start
/// its one-shot landing animation.  Called on the initial generation and again
/// whenever the map is re-rolled, so the rocket replays its landing from frame
/// zero on the fresh surface.
#[cfg(target_arch = "wasm32")]
fn reset_rocket(spawn: (i32, i32)) {
    let (sx, sy) = spawn;
    // SAFETY: single-threaded guest.
    let h = unsafe { host::height_at(sx as f64, sy as f64) };
    let (rp, rl) = (ROCKET.as_ptr() as i32, ROCKET.len() as i32);
    // SAFETY: single-threaded guest.
    unsafe {
        host::set_pos(rp, rl, sx as f64 + 0.5, sy as f64 + 0.5, h);
        let (ap, al) = (ROCKET_ANIM.as_ptr() as i32, ROCKET_ANIM.len() as i32);
        host::start_anim(rp, rl, ap, al, 0);
    }
}

/// Spawn the LRV rover (body + 4 wheels) beside the landing rocket on the same
/// flat pad, within the default camera view.  Called once on initial generation.
#[cfg(target_arch = "wasm32")]
fn spawn_rover(spawn: (i32, i32)) {
    let (sx, sy) = spawn;
    let (dp, dl) = (LRV_DEF.as_ptr() as i32, LRV_DEF.len() as i32);
    let (rp, rl) = (LRV.as_ptr() as i32, LRV.len() as i32);
    // SAFETY: single-threaded guest.
    unsafe {
        host::vehicle_spawn(dp, dl, rp, rl, sx as f64 + 4.0, sy as f64 + 3.0);
    }
}

/// Reposition the LRV rover (body + wheels) and reset its suspension physics.
/// Called on every map re-roll (the entity persists across terrain re-rolls).
#[cfg(target_arch = "wasm32")]
fn reset_rover(spawn: (i32, i32)) {
    let (sx, sy) = spawn;
    let (rp, rl) = (LRV.as_ptr() as i32, LRV.len() as i32);
    // SAFETY: single-threaded guest.
    unsafe {
        host::vehicle_teleport(rp, rl, sx as f64 + 4.0, sy as f64 + 3.0);
    }
}

/// Click-to-move: a left click (not consumed by UI) drives the LRV to the
/// tile under the cursor via the host's `vehicle_goto` (host-side A*).
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
        let (rp, rl) = (LRV.as_ptr() as i32, LRV.len() as i32);
        host::vehicle_goto(rp, rl, mx, my);
    }
}

/// Called once, before the first frame, to submit the initial map generation to
/// the background worker.  When the worker is synchronous (the deterministic
/// test/golden harness), the result is already buffered here, so `poll` applies
/// it immediately — terrain, view, rocket and LRV are all in place before the
/// first frame, exactly like the old inline generation.  Under the async
/// worker, `poll` is a no-op and everything lands a frame or two later, once
/// `update` polls it.
#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn init() {
    spawn_generation(&seed_for(0), true);
    poll_generation();
}

/// Called once per frame.  Polls the pending generation, then handles input:
/// a left click drives the LRV to the clicked tile, `R` re-rolls the terrain
/// with a fresh seed (when no generation is already in flight).
#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn update(_dt: f64) {
    handle_click();
    poll_generation();

    let (kp, kl) = (KEY_R.as_ptr() as i32, KEY_R.len() as i32);
    // SAFETY: single-threaded guest.
    let pressed = unsafe { host::was_key_pressed(kp, kl) };
    if pressed == 0 {
        return;
    }

    // Ignore a re-roll request while a generation is already in flight.
    if unsafe { PENDING_TASK >= 0 } {
        return;
    }

    let n = unsafe {
        SEED_N += 1;
        SEED_N
    };
    spawn_generation(&seed_for(n), false);
}
