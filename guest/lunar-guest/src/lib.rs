#![cfg_attr(target_arch = "wasm32", no_std)]
#![allow(static_mut_refs)]

//! ROM guest for the `lunar` scene: the procedural lunar map generator.
//!
//! Owns the *whole* map-generation behaviour.  `init` generates the 400x400
//! lunar surface and bulk-uploads the grids + tileset + landing zones to the
//! host; `update` re-rolls it when `R` is pressed.  The host is a generic
//! terrain engine (storage + rebuild + pathfinding); the map algorithm lives
//! here, in the ROM.
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
use alloc::{format, string::String};

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
static mut SEED_N: u32 = 0;

/// Build the seed string for generation `n` (`"0"`, `"1"`, ...).
#[cfg(target_arch = "wasm32")]
fn seed_for(n: u32) -> String {
    format!("{n}")
}

/// Generate the lunar map for a seed and bulk-upload every grid to the host,
/// then commit the terrain (install on first call, rebuild afterwards).
/// Returns the first landing-zone spawn point (tile coords), for camera framing.
#[cfg(target_arch = "wasm32")]
fn generate(seed: &str) -> (i32, i32) {
    let params = LunarParams { seed: String::from(seed), ..LunarParams::default() };
    let terrain = generate_lunar(&params);
    let (rgba, tw, th) = build_lunar_tileset(&format!("{seed}:tileset"), 32, 8, 8);

    // SAFETY: single-threaded guest; the Vecs stay alive across the imports.
    unsafe {
        host::set_tiles(terrain.tiles.as_ptr() as i32, (terrain.tiles.len() * 4) as i32);
        host::set_heights(terrain.heights.as_ptr() as i32, (terrain.heights.len() * 4) as i32);
        host::set_nav(terrain.nav.as_ptr() as i32, (terrain.nav.len() * 4) as i32);
        host::set_tileset(rgba.as_ptr() as i32, rgba.len() as i32, tw as i32, th as i32);
    }

    // SAFETY: single-threaded guest.
    unsafe {
        host::commit_terrain(HEIGHT_SCALE);
    }

    terrain.spawn_points.first().copied().unwrap_or((0, 0))
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

/// Called once, before the first frame, to generate the initial map.
#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn init() {
    let spawn = generate(&seed_for(0));
    setup_view(spawn);
    reset_rocket(spawn);
}

/// Called once per frame.  `R` re-rolls the terrain with a fresh seed.
#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn update(_dt: f64) {
    let (kp, kl) = (KEY_R.as_ptr() as i32, KEY_R.len() as i32);
    // SAFETY: single-threaded guest.
    let pressed = unsafe { host::was_key_pressed(kp, kl) };
    if pressed == 0 {
        return;
    }

    let n = unsafe {
        SEED_N += 1;
        SEED_N
    };
    let spawn = generate(&seed_for(n));
    reset_rocket(spawn);
}
