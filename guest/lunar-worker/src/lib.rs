#![cfg_attr(target_arch = "wasm32", no_std)]
#![allow(static_mut_refs)]
// The generator sources are shared verbatim with `lunar-guest`, so items the
// foreground uses (e.g. `LunarTerrain::landing_zones`, `height_at`) are unused
// in this worker-only compilation.
#![allow(dead_code)]

//! ROM worker guest for the `lunar` scene: the pure map-generation half of the
//! lunar algorithm, compiled as a **separate** `.wasm` module that runs in the
//! host's background guest worker (Tier 3).
//!
//! The foreground `lunar-guest` guest owns the engine-facing behaviour (view
//! setup, light, rocket placement, `commit_terrain`); this worker owns only the
//! heavy, engine-free computation — generating the height/tile/nav grids and
//! painting the tileset — and returns them as bytes.  Because it imports only
//! the reduced pure surface (`task_arg`/`task_return`), it can run off the
//! render thread without ever touching `&mut Engine`.
//!
//! The generator sources (`lunar.rs`, `material.rs`, `tileset.rs`) are shared
//! verbatim with `lunar-guest` via `#[path]` includes, so the two modules can
//! never drift apart.

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

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
        pub fn task_arg(out_ptr: i32, out_cap: i32) -> i32;
        pub fn task_return(ptr: i32, len: i32);
    }
}

#[path = "../../lunar-guest/src/material.rs"]
mod material;
#[path = "../../lunar-guest/src/tileset.rs"]
mod tileset;
#[path = "../../lunar-guest/src/lunar.rs"]
mod lunar;

use lunar::{generate_lunar, LunarParams, LunarTerrain};

/// Serialize a generated terrain plus its tileset into a single little-endian
/// byte buffer (the worker result, decoded by the foreground guest).
///
/// Layout: `size_x` (i32), `size_y` (i32), `heights` (f32), `tiles` (u32),
/// `nav` (u32), `spawn_point_count` (i32), `spawn_points` (i32 pairs), `tw`
/// (i32), `th` (i32), `tileset rgba` (u8).
fn serialize(terrain: &LunarTerrain, rgba: &[u8], tw: u32, th: u32) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&terrain.size_x.to_le_bytes());
    out.extend_from_slice(&terrain.size_y.to_le_bytes());
    for h in &terrain.heights {
        out.extend_from_slice(&h.to_le_bytes());
    }
    for t in &terrain.tiles {
        out.extend_from_slice(&t.to_le_bytes());
    }
    for n in &terrain.nav {
        out.extend_from_slice(&n.to_le_bytes());
    }
    out.extend_from_slice(&(terrain.spawn_points.len() as i32).to_le_bytes());
    for (x, y) in &terrain.spawn_points {
        out.extend_from_slice(&x.to_le_bytes());
        out.extend_from_slice(&y.to_le_bytes());
    }
    out.extend_from_slice(&(tw as i32).to_le_bytes());
    out.extend_from_slice(&(th as i32).to_le_bytes());
    out.extend_from_slice(rgba);
    out
}

/// Generate the lunar map for a seed and return the grids + tileset as bytes.
///
/// The seed is read from the task argument (`task_arg`); the result is written
/// with `task_return`.
#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn generate_worker() {
    let mut arg = vec![0u8; 256];
    // SAFETY: single-threaded guest; the buffer stays alive across the import.
    let n = unsafe { host::task_arg(arg.as_mut_ptr() as i32, arg.len() as i32) }.max(0) as usize;
    let seed = String::from(core::str::from_utf8(&arg[..n]).unwrap_or("apollo"));

    let params = LunarParams { seed: seed.clone(), ..LunarParams::default() };
    let terrain = generate_lunar(&params);
    let (rgba, tw, th) = tileset::build_lunar_tileset(&format!("{seed}:tileset"), 32, 8, 8);

    let out = serialize(&terrain, &rgba, tw, th);
    // SAFETY: single-threaded guest; `out` stays alive across the import.
    unsafe {
        host::task_return(out.as_ptr() as i32, out.len() as i32);
    }
}

// The pure generator is natively unit-testable when not compiling for wasm32.
#[cfg(not(target_arch = "wasm32"))]
#[allow(dead_code)]
fn _generate(seed: &str) -> (LunarTerrain, Vec<u8>, u32, u32) {
    let params = LunarParams { seed: String::from(seed), ..LunarParams::default() };
    let terrain = generate_lunar(&params);
    let (rgba, tw, th) = tileset::build_lunar_tileset(&format!("{seed}:tileset"), 32, 8, 8);
    (terrain, rgba, tw, th)
}

#[cfg(not(target_arch = "wasm32"))]
#[cfg(test)]
mod tests {
    use super::*;

    fn f32_le(v: f32) -> Vec<u8> {
        v.to_le_bytes().to_vec()
    }

    fn u32_le(v: u32) -> Vec<u8> {
        v.to_le_bytes().to_vec()
    }

    fn i32_le(v: i32) -> Vec<u8> {
        v.to_le_bytes().to_vec()
    }

    #[test]
    fn serialize_roundtrips_header_and_grids() {
        let terrain = LunarTerrain {
            size_x: 2,
            size_y: 2,
            heights: vec![0.5, 1.5, 2.5, 3.5, 4.5, 5.5, 6.5, 7.5, 8.5],
            tiles: vec![1, 2, 3, 4],
            nav: vec![1, 0, 1, 1],
            landing_zones: vec![],
            spawn_points: vec![(0, 0), (1, 1)],
            stats: lunar::LunarStats::default(),
        };
        let rgba = [10u8, 20, 30, 40];
        let out = serialize(&terrain, &rgba, 8, 8);

        let mut expected = Vec::new();
        expected.extend_from_slice(&i32_le(2));
        expected.extend_from_slice(&i32_le(2));
        for h in [0.5f32, 1.5, 2.5, 3.5, 4.5, 5.5, 6.5, 7.5, 8.5] {
            expected.extend_from_slice(&f32_le(h));
        }
        for t in [1u32, 2, 3, 4] {
            expected.extend_from_slice(&u32_le(t));
        }
        for n in [1u32, 0, 1, 1] {
            expected.extend_from_slice(&u32_le(n));
        }
        expected.extend_from_slice(&i32_le(2));
        for (x, y) in [(0i32, 0i32), (1, 1)] {
            expected.extend_from_slice(&i32_le(x));
            expected.extend_from_slice(&i32_le(y));
        }
        expected.extend_from_slice(&i32_le(8));
        expected.extend_from_slice(&i32_le(8));
        expected.extend_from_slice(&rgba);

        assert_eq!(out, expected);
    }
}
