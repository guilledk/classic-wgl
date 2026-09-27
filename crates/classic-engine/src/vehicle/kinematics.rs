//! Pure vehicle kinematics: direction/heading maths, springs, steering
//! quantization, pure-pursuit lookahead, and wheel-offset derivation.

use classic_core::math::iso_camera_px_inverse;
use classic_core::tilemap::TILE_M;
use glam::Vec2;

use super::{
    REVERSE_ENTER, REVERSE_EXIT, SPRING_DAMPING, SPRING_STIFFNESS, SUSP_DAMPING, SUSP_STIFFNESS,
};

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

/// The heading angle (tile-space radians, `atan2(dy, dx)`) for a direction
/// frame.  `0` = East (frame 0), `FRAC_PI_2` = South (frame 2), etc.
pub(super) fn dir_to_heading(dir: u32) -> f32 {
    (dir % 8) as f32 * std::f32::consts::FRAC_PI_4
}

/// Quantize a continuous heading angle to the nearest of the 8 direction
/// frames (0..7), wrapping at `2π`.
pub(super) fn heading_to_dir(heading: f32) -> u32 {
    let theta = heading.rem_euclid(std::f32::consts::TAU);
    (theta / std::f32::consts::FRAC_PI_4).round() as u32 % 8
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

/// Lerp a per-direction `[f32; 2]` value (wheel offset or anchor) between the
/// two direction frames straddling a continuous heading.
pub(super) fn lerp_dir2(vals: &[[f32; 2]; 8], heading: f32) -> [f32; 2] {
    let theta = heading.rem_euclid(std::f32::consts::TAU);
    let dir_f = theta / std::f32::consts::FRAC_PI_4;
    let a = dir_f.floor() as usize % 8;
    let b = (a + 1) % 8;
    let t = dir_f - dir_f.floor();
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

/// Advance a body pitch/roll spring one step (semi-implicit Euler), returning
/// the new `(angle, vel)`.  Underdamped, so the body bobs with momentum.
pub(super) fn step_spring(angle: f32, vel: f32, target: f32, dt: f32) -> (f32, f32) {
    let accel = SPRING_STIFFNESS * (target - angle) - SPRING_DAMPING * vel;
    let new_vel = vel + accel * dt;
    (angle + new_vel * dt, new_vel)
}

/// Advance a per-wheel suspension spring one step (semi-implicit Euler),
/// returning the new `(height, vel)`.  Kept behind this seam so the spring can
/// be swapped for a cheaper critically-damped/exponential smoothing (a single
/// multiply with no velocity state) without touching the chassis-plane logic —
/// the scale knob for many-vehicle scenes.
pub(super) fn advance_suspension(height: f32, vel: f32, target: f32, dt: f32) -> (f32, f32) {
    let accel = SUSP_STIFFNESS * (target - height) - SUSP_DAMPING * vel;
    let new_vel = vel + accel * dt;
    (height + new_vel * dt, new_vel)
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

/// A part's anchors as a fixed 8-slot array (padded/truncated to 8 directions).
pub(super) fn anchors8(anchors: &[[f32; 2]]) -> [[f32; 2]; 8] {
    let mut out = [[0.5f32, 0.5f32]; 8];
    for (i, a) in anchors.iter().take(8).enumerate() {
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
    body_anchors: &[[f32; 2]; 8],
    wheel_anchors: &[[[f32; 2]; 8]; 4],
    cell: [f32; 2],
    _tile_scale: f32,
) -> [[[f32; 2]; 8]; 4] {
    let mut offsets = [[[0.0f32; 2]; 8]; 4];
    for (i, anchors) in wheel_anchors.iter().enumerate() {
        for d in 0..8 {
            let dx = (anchors[d][0] - body_anchors[d][0]) * cell[0];
            let dy = (anchors[d][1] - body_anchors[d][1]) * cell[1];
            let world = iso_camera_px_inverse(Vec2::new(dx, dy));
            offsets[i][d] = [world.x / TILE_M, -world.y / TILE_M];
        }
    }
    offsets
}
