#![no_std]
#![allow(static_mut_refs)]

//! ROM guest for the `lunar` scene: the procedural lunar map generator.
//!
//! Owns the *whole* map-generation behaviour.  `init` generates the 400x400
//! lunar surface (via the `classic-terrain` crate) and bulk-uploads the grids
//! + tileset + landing zones to the host; `update` re-rolls it when `R` is
//! pressed.  The host is a generic terrain engine (storage + rebuild +
//! pathfinding); the map algorithm lives here, in the ROM.

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use classic_terrain::lunar::{generate_lunar, LunarParams};
use classic_terrain::tileset::build_lunar_tileset;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}

#[global_allocator]
static ALLOC: dlmalloc::GlobalDlmalloc = dlmalloc::GlobalDlmalloc;

mod host {
    #[link(wasm_import_module = "env")]
    extern "C" {
        pub fn was_key_pressed(ptr: i32, len: i32) -> i32;
        pub fn set_tiles(ptr: i32, len: i32) -> i32;
        pub fn set_heights(ptr: i32, len: i32) -> i32;
        pub fn set_nav(ptr: i32, len: i32) -> i32;
        pub fn set_tileset(ptr: i32, len: i32, w: i32, h: i32) -> i32;
        pub fn set_spawn_points(ptr: i32, len: i32) -> i32;
        pub fn commit_terrain(height_scale: f64) -> i32;
    }
}

/// Vertical exaggeration for the lunar relief (matches `LUNAR_HEIGHT_SCALE`).
const HEIGHT_SCALE: f64 = 14.0;

static KEY_R: &[u8] = b"KeyR";

static mut SEED_N: u32 = 0;

/// Build the seed string for generation `n` (`"0"`, `"1"`, ...).
fn seed_for(n: u32) -> String {
    format!("{n}")
}

/// Generate the lunar map for a seed and bulk-upload every grid to the host,
/// then commit the terrain (install on first call, rebuild afterwards).
fn generate(seed: &str) {
    let params = LunarParams { seed: String::from(seed), ..LunarParams::default() };
    let terrain = generate_lunar(&params);
    let (rgba, tw, th) = build_lunar_tileset(&format!("{seed}:tileset"), 32, 8, 8);

    // SAFETY: single-threaded guest; the Vecs stay alive across the imports.
    unsafe {
        host::set_tiles(
            terrain.tiles.as_ptr() as i32,
            (terrain.tiles.len() * 4) as i32,
        );
        host::set_heights(
            terrain.heights.as_ptr() as i32,
            (terrain.heights.len() * 4) as i32,
        );
        host::set_nav(terrain.nav.as_ptr() as i32, (terrain.nav.len() * 4) as i32);
        host::set_tileset(rgba.as_ptr() as i32, rgba.len() as i32, tw as i32, th as i32);
    }

    let mut spawns: Vec<i32> = Vec::with_capacity(terrain.spawn_points.len() * 2);
    for (x, y) in &terrain.spawn_points {
        spawns.push(*x);
        spawns.push(*y);
    }
    // SAFETY: single-threaded guest.
    unsafe {
        host::set_spawn_points(spawns.as_ptr() as i32, (spawns.len() * 4) as i32);
        host::commit_terrain(HEIGHT_SCALE);
    }
}

/// Called once, before the first frame, to generate the initial map.
#[no_mangle]
pub extern "C" fn init() {
    generate(&seed_for(0));
}

/// Called once per frame.  `R` re-rolls the terrain with a fresh seed.
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
    generate(&seed_for(n));
}
