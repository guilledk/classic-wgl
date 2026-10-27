//! Pure vehicle kinematics: direction/heading maths, springs, steering
//! quantization, pure-pursuit lookahead, and wheel-offset derivation.

use classic_core::components::IsoVehicle;
use classic_core::math::iso_camera_px_inverse;
use classic_core::pathfinder::{BodyRect, VehicleFootprint};
use classic_core::tilemap::TILE_M;
use classic_core::types::VehicleDef;
use glam::Vec2;

use super::{
    REVERSE_ENTER, REVERSE_EXIT, SPRING_DAMPING, SPRING_STIFFNESS, SUSP_DAMPING, SUSP_STIFFNESS,
};

/// The body rectangle (tiles) the vehicle search turns to each move's
/// heading: the def's `path_body_m`, else `contacts_m` (the wheel contacts'
/// `[rear, front, right, left]`, metres).  `None` under an explicit
/// `path_footprint`, which the search applies as authored.
pub(super) fn path_body(def: &VehicleDef, contacts_m: [f32; 4]) -> Option<BodyRect> {
    if def.path_footprint.is_some() {
        return None;
    }
    let [rear, front, right, left] = def.path_body_m.unwrap_or(contacts_m);
    Some(BodyRect {
        rear: rear.min(front) / TILE_M,
        front: rear.max(front) / TILE_M,
        right: right.min(left) / TILE_M,
        left: right.max(left) / TILE_M,
    })
}

/// The half extents (tiles, rounded out) of a vehicle's axis box, its
/// `path_footprint` when the def authors none: its `path_body_m` facing
/// East, else half its wheelbase along `x` and half its track along `y`.
pub(super) fn axis_box_half(def: &VehicleDef, wheelbase_m: f32, track_m: f32) -> (i32, i32) {
    let half = |m: f32| (m / TILE_M).ceil() as i32;
    match def.path_body_m {
        Some([rear, front, right, left]) => {
            (half(rear.abs().max(front.abs())), half(right.abs().max(left.abs())))
        }
        None => (half(wheelbase_m * 0.5), half(track_m * 0.5)),
    }
}

/// The footprint a vehicle searches with: its turned body, else its cells.
pub(super) fn search_footprint(v: &IsoVehicle) -> VehicleFootprint {
    match v.path_body {
        Some(body) => VehicleFootprint::Oriented { body, pivot: v.pivot_turn },
        None => VehicleFootprint::Cells(v.path_footprint.clone()),
    }
}

/// Map a tile-step delta to a sprite-sheet direction frame (0..7), matching the
/// 8-direction layout used by the assets pipeline (`0=East, 1=SouthEast, …`).
pub fn dir_index(dx: i32, dy: i32) -> usize {
    match (dx.signum(), dy.signum()) {
        (1, 0) => 0,
        (1, 1) => 1,
        (0, 1) => 2,
        (-1, 1) => 3,
        (-1, 0) => 4,
        (-1, -1) => 5,
        (0, -1) => 6,
        (1, -1) => 7,
        _ => 0,
    }
}

/// The heading angle (tile-space radians, `atan2(dy, dx)`) for direction
/// frame `dir` of `n`.  `0` = East (frame 0), then clockwise on screen (for
/// 8 directions `FRAC_PI_2` = South = frame 2).
pub(super) fn dir_to_heading(dir: u32, n: u32) -> f32 {
    let n = n.max(1);
    (dir % n) as f32 * std::f32::consts::TAU / n as f32
}

/// Quantize a continuous heading angle to the nearest of `n` direction
/// frames, wrapping at `2π`.
pub(super) fn heading_to_dir(heading: f32, n: u32) -> u32 {
    classic_core::vehicle_def::heading_to_dir_n(heading, n)
}

/// Wrap an angle to `[-π, π]`.
pub(super) fn wrap_pi(a: f32) -> f32 {
    let mut a = a;
    while a > std::f32::consts::PI {
        a -= std::f32::consts::TAU;
    }
    while a < -std::f32::consts::PI {
        a += std::f32::consts::TAU;
    }
    a
}

/// A per-direction table's entry for direction `dir`, wrapping on the
/// table's own length (a table never has to match the quantized count),
/// `[0, 0]` for an empty one.
pub(super) fn dir_entry(vals: &[[f32; 2]], dir: u32) -> [f32; 2] {
    if vals.is_empty() {
        return [0.0, 0.0];
    }
    vals[dir as usize % vals.len()]
}

/// Lerp a per-direction `[f32; 2]` value (wheel offset or anchor) between the
/// two direction frames straddling a continuous heading (`vals.len()`
/// directions).
pub(super) fn lerp_dir2(vals: &[[f32; 2]], heading: f32) -> [f32; 2] {
    if vals.is_empty() {
        return [0.0, 0.0];
    }
    let (a, b, t) = classic_core::vehicle_def::heading_neighbours(heading, vals.len() as u32);
    let (a, b) = (a as usize, b as usize);
    [vals[a][0] * (1.0 - t) + vals[b][0] * t, vals[a][1] * (1.0 - t) + vals[b][1] * t]
}

/// Pure-pursuit look-ahead over a waypoint path.
///
/// Returns `(target_x, target_y, next_idx)` where the target is the point
/// `lookahead` tiles along the path polyline from `(x, y)` (interpolated
/// between waypoint centers) and `next_idx` is the index of the waypoint whose
/// segment contains the target.  The caller advances `path_idx` to `next_idx`
/// each frame, which is always monotonic — a forward-only vehicle never
/// backtracks to a corner it already rounded.  Waypoints the vehicle has passed
/// are skipped up front, so a target that lies "behind" the vehicle (e.g. after
/// overshooting a corner) can't make it spin back around.  When the path is
/// exhausted before `lookahead`, the target is the final waypoint center and
/// `next_idx == path.len()`.
pub(super) fn lookahead(
    path: &[[i32; 2]],
    path_idx: usize,
    x: f32,
    y: f32,
    lookahead: f32,
) -> (f32, f32, usize) {
    let mut idx = path_idx;

    // Skip waypoints whose segment the vehicle has already traversed (its
    // perpendicular projection onto `path[idx] -> path[idx+1]` is past the end).
    while idx + 1 < path.len() {
        let (ax, ay) = (path[idx][0] as f32 + 0.5, path[idx][1] as f32 + 0.5);
        let (bx, by) = (path[idx + 1][0] as f32 + 0.5, path[idx + 1][1] as f32 + 0.5);
        let segx = bx - ax;
        let segy = by - ay;
        let seglen2 = segx * segx + segy * segy;
        let proj = ((x - ax) * segx + (y - ay) * segy) / seglen2.max(1e-6);
        if proj >= 1.0 {
            idx += 1;
        } else {
            break;
        }
    }

    // Walk forward from the vehicle accumulating `lookahead` tiles of distance.
    let mut remaining = lookahead;
    let (mut px, mut py) = (x, y);
    while idx < path.len() {
        let (wx, wy) = (path[idx][0] as f32 + 0.5, path[idx][1] as f32 + 0.5);
        let dx = wx - px;
        let dy = wy - py;
        let seg = (dx * dx + dy * dy).sqrt();
        if remaining <= seg {
            let t = if seg > 1e-6 { remaining / seg } else { 0.0 };
            return (px + dx * t, py + dy * t, idx);
        }
        remaining -= seg;
        px = wx;
        py = wy;
        idx += 1;
    }

    // Path exhausted: target the final waypoint center.
    let (lx, ly) = (path[path.len() - 1][0] as f32 + 0.5, path[path.len() - 1][1] as f32 + 0.5);
    (lx, ly, path.len())
}

/// Quantize a signed pitch angle (radians) to a pitch frame index in
/// `0..pitch_levels`, mapping `[-pitch_max, +pitch_max]` onto the frame range:
/// `0` = nose-down, `(pitch_levels - 1) / 2` = level (odd counts),
/// `pitch_levels - 1` = nose-up.
pub(super) fn pitch_index(angle: f32, pitch_max: f32, pitch_levels: u32) -> u32 {
    let levels = pitch_levels.max(1);
    if levels <= 1 {
        return 0;
    }
    let max = pitch_max.abs().max(1e-4);
    let t = (angle / max).clamp(-1.0, 1.0);
    let idx = ((t + 1.0) * 0.5 * (levels - 1) as f32).round() as u32;
    idx.min(levels - 1)
}

/// Longest single semi-implicit Euler step (seconds) the vehicle springs take.
/// The scheme is only stable below `2 / damping` (0.133 s for the
/// suspension); past it every step amplifies the error, so a run of long
/// frames grew the state to `inf` and then `NaN`.  A longer `dt` is split
/// into equal sub-steps; at or below this (every fixed-dt golden: 1/60, 0.05)
/// it is one step, bit-identical to before.
pub(super) const MAX_SPRING_STEP: f32 = 0.05;

/// Cap on the sub-steps one call takes (bounds the cost of an absurd `dt`):
/// past `MAX_SPRING_SUBSTEPS × MAX_SPRING_STEP` (3.2 s, long after either
/// spring settles) the rest of `dt` is dropped rather than taken unstably.
const MAX_SPRING_SUBSTEPS: u32 = 64;

/// Integrate a damped spring `x'' = k·(target − x) − c·x'` over `dt`, in
/// sub-steps no longer than [`MAX_SPRING_STEP`].
fn integrate_spring(x: f32, vel: f32, target: f32, dt: f32, k: f32, c: f32) -> (f32, f32) {
    let (n, h) = if dt > MAX_SPRING_STEP {
        let n = (dt / MAX_SPRING_STEP).ceil() as u32;
        if n > MAX_SPRING_SUBSTEPS {
            (MAX_SPRING_SUBSTEPS, MAX_SPRING_STEP)
        } else {
            (n, dt / n as f32)
        }
    } else {
        (1, dt)
    };
    let (mut x, mut vel) = (x, vel);
    for _ in 0..n {
        let accel = k * (target - x) - c * vel;
        vel += accel * h;
        x += vel * h;
    }
    (x, vel)
}

/// Advance a body pitch/roll spring by `dt` (semi-implicit Euler, sub-stepped
/// above [`MAX_SPRING_STEP`]), returning the new `(angle, vel)`.
/// Underdamped, so the body bobs with momentum.
pub(super) fn step_spring(angle: f32, vel: f32, target: f32, dt: f32) -> (f32, f32) {
    integrate_spring(angle, vel, target, dt, SPRING_STIFFNESS, SPRING_DAMPING)
}

/// Advance a per-wheel suspension spring by `dt` (semi-implicit Euler,
/// sub-stepped like [`step_spring`]),
/// returning the new `(height, vel)`.  Kept behind this seam so the spring can
/// be swapped for a cheaper critically-damped/exponential smoothing (a single
/// multiply with no velocity state) without touching the chassis-plane logic —
/// the scale knob for many-vehicle scenes.
pub(super) fn advance_suspension(height: f32, vel: f32, target: f32, dt: f32) -> (f32, f32) {
    integrate_spring(height, vel, target, dt, SUSP_STIFFNESS, SUSP_DAMPING)
}

/// Soft dead-zone on a signed slope angle (radians).  Slopes below `dz` are
/// zeroed so the body ignores sub-frame terrain noise (an OpenRA-style
/// terrain-orientation margin); the excess beyond `dz` passes through
/// unchanged.
pub(super) fn soft_deadzone(slope: f32, dz: f32) -> f32 {
    if dz <= 0.0 {
        return slope;
    }
    (slope.abs() - dz).max(0.0) * slope.signum()
}

/// Quantize a signed steering demand (radians, positive = turn left) to a
/// steering frame index in `0..steer_levels`.  `0` = full-right, the centre
/// (`(steer_levels - 1) / 2`) = straight, `steer_levels - 1` = full-left,
/// matching the exporter's steer-major frame order (frame `0` is the +max
/// Z-yaw render, which projects as the vehicle's right).
pub(super) fn steer_index(demand: f32, steer_max: f32, steer_levels: u32) -> u32 {
    if steer_levels <= 1 {
        return 0;
    }
    let max = steer_max.abs().max(1e-4);
    // demand: turn-left (+demand) maps up to the top frame (full-left).
    let t = (demand / max).clamp(-1.0, 1.0);
    let idx = ((t + 1.0) * 0.5 * (steer_levels - 1) as f32).round() as u32;
    idx.min(steer_levels - 1)
}

/// The "straight" steering frame index for a given level count (centre of an
/// odd range, 0 for a single level).
pub(super) fn straight_steer(steer_levels: u32) -> u32 {
    steer_levels.saturating_sub(1) / 2
}

/// Advance the steering angle (radians, positive = turn left) toward `demand`
/// at a max rate of `steer_rate` (rad/s), clamped to `[-steer_max, steer_max]`.
/// Rate-limiting the steering *state* (rather than quantizing the raw heading
/// error) is what makes the tires sweep smoothly through their steer frames.
pub(super) fn step_steer(steer: f32, demand: f32, steer_max: f32, steer_rate: f32, dt: f32) -> f32 {
    let demand = demand.clamp(-steer_max, steer_max);
    let max_delta = steer_rate * dt;
    (steer + (demand - steer).clamp(-max_delta, max_delta)).clamp(-steer_max, steer_max)
}

/// Decide whether the vehicle should reverse to reach a target whose relative
/// heading error is `err` (radians, wrapped).  Hysteresis: enter reverse past
/// `REVERSE_ENTER`, exit only below `REVERSE_EXIT`, so the gear doesn't flap.
pub(super) fn should_reverse(err: f32, reversing: bool) -> bool {
    let enter = REVERSE_ENTER;
    let exit = REVERSE_EXIT;
    let abs = err.abs();
    if reversing {
        abs > exit
    } else {
        abs > enter
    }
}

/// A part's anchors for `n` directions (padded with the neutral `0.5, 0.5`,
/// or truncated).
pub(super) fn anchors_n(anchors: &[[f32; 2]], n: u32) -> Vec<[f32; 2]> {
    let n = n.max(1) as usize;
    let mut out = vec![[0.5f32, 0.5f32]; n];
    for (i, a) in anchors.iter().take(n).enumerate() {
        out[i] = *a;
    }
    out
}

/// Derive per-wheel, per-direction tile-space offsets from the part anchors.
///
/// `delta_px = (wheel_anchor - body_anchor) * cell` is the wheel's screen
/// displacement from the body origin in the sprite's own frame; converting it
/// through the orthographic camera (the anchors are baked by the exporter's
/// 30° `iso_basis` camera at `PPM_TARGET` px/m) yields the tile offset that
/// reproduces that displacement exactly.
pub(super) fn derive_wheel_offsets(
    body_anchors: &[[f32; 2]],
    wheel_anchors: &[Vec<[f32; 2]>; 4],
    cell: [f32; 2],
    _tile_scale: f32,
) -> [Vec<[f32; 2]>; 4] {
    let n = body_anchors.len();
    let mut offsets: [Vec<[f32; 2]>; 4] = std::array::from_fn(|_| vec![[0.0f32; 2]; n]);
    for (i, anchors) in wheel_anchors.iter().enumerate() {
        for d in 0..n.min(anchors.len()) {
            let dx = (anchors[d][0] - body_anchors[d][0]) * cell[0];
            let dy = (anchors[d][1] - body_anchors[d][1]) * cell[1];
            let world = iso_camera_px_inverse(Vec2::new(dx, dy));
            offsets[i][d] = [world.x / TILE_M, -world.y / TILE_M];
        }
    }
    offsets
}
