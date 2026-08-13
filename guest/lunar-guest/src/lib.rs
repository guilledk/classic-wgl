#![no_std]
#![allow(static_mut_refs)]

//! ROM guest for the `lunar` scene, compiled to `.wasm` and run by the host
//! against the `classic-guest` SDK.
//!
//! Owns the lunar-specific runtime behaviour: pressing `R` re-rolls the
//! procedural terrain via the generic `generate_terrain` host import (the
//! `lunar` generator lives in `classic-core`; this guest picks the kind, seed,
//! and trigger).

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}

mod host {
    #[link(wasm_import_module = "env")]
    extern "C" {
        pub fn was_key_pressed(ptr: i32, len: i32) -> i32;
        pub fn generate_terrain(
            kind_ptr: i32,
            kind_len: i32,
            seed_ptr: i32,
            seed_len: i32,
            height_scale: f64,
        ) -> i32;
    }
}

/// Vertical exaggeration for the lunar relief (matches `LUNAR_HEIGHT_SCALE`).
const HEIGHT_SCALE: f64 = 14.0;

static KIND: &[u8] = b"lunar";
static KEY_R: &[u8] = b"KeyR";

static mut SEED_N: u32 = 0;
static mut SEED: [u8; 16] = [0; 16];
static mut SEED_LEN: i32 = 0;

/// Write the next seed (`"1"`, `"2"`, ...) into `SEED`/`SEED_LEN`.
fn bump_seed() {
    // SAFETY: single-threaded guest.
    unsafe {
        SEED_N += 1;
        let mut v = SEED_N;
        let mut digits = [0u8; 16];
        let mut dlen = 0usize;
        loop {
            digits[dlen] = b'0' + (v % 10) as u8;
            dlen += 1;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        let mut i = 0usize;
        while dlen > 0 {
            dlen -= 1;
            SEED[i] = digits[dlen];
            i += 1;
        }
        SEED_LEN = i as i32;
    }
}

/// Called once per frame with the frame delta in seconds.
#[no_mangle]
pub extern "C" fn update(_dt: f64) {
    let (kp, kl) = (KEY_R.as_ptr() as i32, KEY_R.len() as i32);
    // SAFETY: single-threaded guest.
    let pressed = unsafe { host::was_key_pressed(kp, kl) };
    if pressed == 0 {
        return;
    }

    bump_seed();
    let (kind_p, kind_l) = (KIND.as_ptr() as i32, KIND.len() as i32);
    // SAFETY: single-threaded guest; SEED is a static buffer.
    unsafe {
        host::generate_terrain(kind_p, kind_l, SEED.as_ptr() as i32, SEED_LEN, HEIGHT_SCALE);
    }
}
