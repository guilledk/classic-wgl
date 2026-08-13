#![no_std]

//! ROM guest for the `demo` scene, compiled to `.wasm` and run by the host
//! against the `classic-guest` SDK.
//!
//! Drives the `navAgent` entity: at boot it computes an A* path to a patrol
//! target via `find_path`, then each frame steps the agent along that path
//! with `get_pos`/`set_pos` and idles at the end.

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}

mod host {
    #[link(wasm_import_module = "env")]
    extern "C" {
        pub fn get_pos(name_ptr: i32, name_len: i32, out_ptr: i32) -> i32;
        pub fn set_pos(name_ptr: i32, name_len: i32, x: f64, y: f64) -> i32;
        pub fn find_path(sx: i32, sy: i32, ex: i32, ey: i32, out_ptr: i32, out_cap: i32) -> i32;
    }
}

const MAX_WAYPOINTS: usize = 512;
const SPEED: f64 = 2.6;

static AGENT: &[u8] = b"navAgent";

static mut PATH: [[i32; 2]; MAX_WAYPOINTS] = [[0; 2]; MAX_WAYPOINTS];
static mut PATH_LEN: usize = 0;
static mut PATH_IDX: usize = 0;
static mut CUR_X: f64 = 0.0;
static mut CUR_Y: f64 = 0.0;
static mut BOOTED: bool = false;

#[inline]
fn agent_name() -> (i32, i32) {
    (AGENT.as_ptr() as i32, AGENT.len() as i32)
}

#[inline]
fn read_f64(buf: &[u8]) -> f64 {
    f64::from_le_bytes([buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7]])
}

/// Compute the patrol path once, from the agent's spawn position to a fixed
/// target a few tiles away.
fn boot() {
    // SAFETY: single-threaded guest; static state is only mutated from `update`.
    unsafe {
        if BOOTED {
            return;
        }
        BOOTED = true;

        let (np, nl) = agent_name();
        let mut pos = [0u8; 16];
        if host::get_pos(np, nl, pos.as_mut_ptr() as i32) != 1 {
            return;
        }
        let sx = read_f64(&pos[0..8]) as i32;
        let sy = read_f64(&pos[8..16]) as i32;
        CUR_X = sx as f64;
        CUR_Y = sy as f64;

        let (tx, ty) = (sx + 8, sy);
        let mut buf = [0i32; MAX_WAYPOINTS * 2];
        let n = host::find_path(sx, sy, tx, ty, buf.as_mut_ptr() as i32, (buf.len() * 4) as i32);
        if n <= 0 {
            return;
        }
        for i in 0..n as usize {
            PATH[i][0] = buf[i * 2];
            PATH[i][1] = buf[i * 2 + 1];
        }
        PATH_LEN = n as usize;
        PATH_IDX = 1;
    }
}

/// Move the agent one step along the path.
fn step(dt: f64) {
    // SAFETY: single-threaded guest.
    unsafe {
        if PATH_IDX >= PATH_LEN {
            return;
        }
        let tx = PATH[PATH_IDX][0] as f64 + 0.5;
        let ty = PATH[PATH_IDX][1] as f64 + 0.5;
        let dx = tx - CUR_X;
        let dy = ty - CUR_Y;
        // Manhattan magnitude avoids `sqrt` (unavailable in core).
        let mag = dx.abs() + dy.abs();
        let step = SPEED * dt;
        if mag <= step || mag == 0.0 {
            CUR_X = tx;
            CUR_Y = ty;
            PATH_IDX += 1;
        } else {
            let f = step / mag;
            CUR_X += dx * f;
            CUR_Y += dy * f;
        }
        let (np, nl) = agent_name();
        host::set_pos(np, nl, CUR_X, CUR_Y);
    }
}

/// Called once per frame with the frame delta in seconds.
#[no_mangle]
pub extern "C" fn update(dt: f64) {
    boot();
    step(dt);
}
