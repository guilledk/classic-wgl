#![no_std]

//! ROM guest for the `demo` scene, compiled to `.wasm` and run by the host
//! against the `classic-guest` SDK.
//!
//! Drives the `navAgent` entity: click-to-move (A* path via `find_path` to the
//! iso tile under the cursor when the editor agent tool is active), direction-
//! aware idle/walk animation via `set_anim`, and terrain-height following via
//! `height_at`.

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}

mod host {
    #[link(wasm_import_module = "env")]
    extern "C" {
        pub fn get_pos(name_ptr: i32, name_len: i32, out_ptr: i32) -> i32;
        pub fn set_pos(name_ptr: i32, name_len: i32, x: f64, y: f64, z: f64) -> i32;
        pub fn find_path(sx: i32, sy: i32, ex: i32, ey: i32, out_ptr: i32, out_cap: i32) -> i32;
        pub fn mouse_iso(out_ptr: i32) -> i32;
        pub fn height_at(x: f64, y: f64) -> f64;
        pub fn set_anim(name_ptr: i32, name_len: i32, anim_ptr: i32, anim_len: i32) -> i32;
        pub fn agent_selected() -> i32;
        pub fn ui_consumed_click() -> i32;
        pub fn was_pressed(btn: i32) -> i32;
    }
}

const MAX_WAYPOINTS: usize = 512;
const SPEED: f64 = 2.6;

static AGENT: &[u8] = b"navAgent";

static IDLE_ANIMS: [&[u8]; 8] = [
    b"idleEast",
    b"idleSouthEast",
    b"idleSouth",
    b"idleSouthWest",
    b"idleWest",
    b"idleNorthWest",
    b"idleNorth",
    b"idleNorthEast",
];
static WALK_ANIMS: [&[u8]; 8] = [
    b"walkEast",
    b"walkSouthEast",
    b"walkSouth",
    b"walkSouthWest",
    b"walkWest",
    b"walkNorthWest",
    b"walkNorth",
    b"walkNorthEast",
];

static mut PATH: [[i32; 2]; MAX_WAYPOINTS] = [[0; 2]; MAX_WAYPOINTS];
static mut PATH_LEN: usize = 0;
static mut PATH_IDX: usize = 0;
static mut CUR_X: f64 = 0.0;
static mut CUR_Y: f64 = 0.0;
static mut CUR_Z: f64 = 0.0;
static mut FACING: usize = 0;
static mut WALKING: bool = false;
static mut BOOTED: bool = false;

#[inline]
fn agent_name() -> (i32, i32) {
    (AGENT.as_ptr() as i32, AGENT.len() as i32)
}

#[inline]
fn read_f64(buf: &[u8]) -> f64 {
    f64::from_le_bytes([buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7]])
}

/// Map an 8-way step delta (each component -1, 0, or 1) to a direction index.
fn dir_index(dx: i32, dy: i32) -> usize {
    match (dx, dy) {
        (1, 0) => 0,
        (1, 1) => 1,
        (0, 1) => 2,
        (-1, 1) => 3,
        (-1, 0) => 4,
        (-1, -1) => 5,
        (0, -1) => 6,
        _ => 7,
    }
}

#[inline]
fn play_anim(anim: &[u8]) {
    let (np, nl) = agent_name();
    // SAFETY: host import call, single-threaded guest.
    unsafe {
        host::set_anim(np, nl, anim.as_ptr() as i32, anim.len() as i32);
    }
}

fn boot() {
    // SAFETY: single-threaded guest; static state is only mutated from `update`.
    unsafe {
        if BOOTED {
            return;
        }
        BOOTED = true;

        let (np, nl) = agent_name();
        let mut pos = [0u8; 24];
        if host::get_pos(np, nl, pos.as_mut_ptr() as i32) != 1 {
            return;
        }
        CUR_X = read_f64(&pos[0..8]);
        CUR_Y = read_f64(&pos[8..16]);
        CUR_Z = read_f64(&pos[16..24]);
        FACING = 0;
    }
    play_anim(IDLE_ANIMS[0]);
}

fn handle_click() {
    // SAFETY: single-threaded guest.
    let (mx, my) = unsafe {
        if host::was_pressed(0) == 0
            || host::agent_selected() == 0
            || host::ui_consumed_click() != 0
        {
            return;
        }
        let mut mouse = [0u8; 16];
        if host::mouse_iso(mouse.as_mut_ptr() as i32) != 1 {
            return;
        }
        (read_f64(&mouse[0..8]) as i32, read_f64(&mouse[8..16]) as i32)
    };

    let (sx, sy) = unsafe { (CUR_X as i32, CUR_Y as i32) };
    if mx == sx && my == sy {
        return;
    }

    // SAFETY: single-threaded guest.
    let facing = unsafe {
        let mut buf = [0i32; MAX_WAYPOINTS * 2];
        let n = host::find_path(sx, sy, mx, my, buf.as_mut_ptr() as i32, (buf.len() * 4) as i32);
        if n <= 0 {
            return;
        }
        for i in 0..n as usize {
            PATH[i][0] = buf[i * 2];
            PATH[i][1] = buf[i * 2 + 1];
        }
        PATH_LEN = n as usize;
        PATH_IDX = 1;
        WALKING = true;
        let d = dir_index(PATH[1][0] - PATH[0][0], PATH[1][1] - PATH[0][1]);
        FACING = d;
        d
    };
    play_anim(WALK_ANIMS[facing]);
}

fn step(dt: f64) {
    // SAFETY: single-threaded guest.
    unsafe {
        if !WALKING || PATH_IDX >= PATH_LEN {
            return;
        }
        let tx = PATH[PATH_IDX][0] as f64 + 0.5;
        let ty = PATH[PATH_IDX][1] as f64 + 0.5;

        let d = dir_index(
            PATH[PATH_IDX][0] - PATH[PATH_IDX - 1][0],
            PATH[PATH_IDX][1] - PATH[PATH_IDX - 1][1],
        );
        let anim_changed = d != FACING;
        FACING = d;

        let dx = tx - CUR_X;
        let dy = ty - CUR_Y;
        let mag = dx.abs() + dy.abs();
        let step = SPEED * dt;
        let mut arrived = false;
        if mag <= step || mag == 0.0 {
            CUR_X = tx;
            CUR_Y = ty;
            PATH_IDX += 1;
            if PATH_IDX >= PATH_LEN {
                WALKING = false;
                arrived = true;
            }
        } else {
            let f = step / mag;
            CUR_X += dx * f;
            CUR_Y += dy * f;
        }

        let target_z = host::height_at(CUR_X, CUR_Y);
        let z_speed = if dt * 4.0 < 1.0 { dt * 4.0 } else { 1.0 };
        CUR_Z += (target_z - CUR_Z) * z_speed;

        let (np, nl) = agent_name();
        host::set_pos(np, nl, CUR_X, CUR_Y, CUR_Z);

        if arrived {
            play_anim(IDLE_ANIMS[FACING]);
        } else if anim_changed {
            play_anim(WALK_ANIMS[d]);
        }
    }
}

/// Called once per frame with the frame delta in seconds.
#[no_mangle]
pub extern "C" fn update(dt: f64) {
    boot();
    handle_click();
    step(dt);
}
