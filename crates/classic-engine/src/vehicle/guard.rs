//! The path follower's body guard.  The planner keeps the oriented body
//! clear of blocked cells along the route's lines, but the pure-pursuit
//! follower aims a look-ahead point down the route, so it cuts inside
//! corners, a few centimetres onto cells the route went round.  Two checks
//! keep it off them:
//!
//! - **Look-ahead** ([`GuardGrid::safe_lookahead`]): when a blocked cell
//!   lies within reach (a square scan), the follower plays its pursuit
//!   forward over the next stretch of the route (0.1 tile steps, its turn
//!   rate per tile) and takes the longest of its look-ahead, two thirds or
//!   two fifths of it (at least half a tile) whose played path keeps the
//!   body off blocked cells.  A shorter look-ahead hugs the route's lines,
//!   which the planner checked.
//! - **Step** ([`follow_guard`]): each step's pose may not put the body on
//!   a blocked cell it was not already on (a relaxed start keeps its own).
//!   A move that would is replaced by the first clear one of the step at
//!   its own heading, aimed at the route's next waypoint, or straight on,
//!   at the full step, then ½, ¼ and 1/10 of it (slower, the same turn
//!   rate turns tighter).
//!
//! None clear (a corner the plain planner turns tighter than the vehicle
//! can: the lattice turns its body at a cell, the vehicle turns while it
//! drives), it takes the move adding the fewest blocked cells and drives
//! on: holding still would never clear (the grid does not change under a
//! held vehicle, and a block change re-plans the routes it cuts anyway),
//! so the guard never stops a vehicle.  Each check rasterizes the body
//! (`pathfinder::body_blocked_cells`, ~80 cells for the mule) against the
//! shared vehicle grid plus the blocks other vehicles own.

use classic_core::components::IsoVehicle;
use classic_core::pathfinder::{body_blocked_cells, GridCell, VehicleFootprint};

use super::kinematics::{lookahead, search_footprint, wrap_pi};
use crate::Engine;

/// The blocking grid a vehicle's follower is checked against.
pub(super) struct GuardGrid {
    snapshot: std::sync::Arc<classic_core::pathfinder::VehicleNavSnapshot>,
    closed: Vec<[i32; 4]>,
    footprint: VehicleFootprint,
}

impl GuardGrid {
    fn blocked(&self, at: (f32, f32), heading: f32, out: &mut Vec<GridCell>) {
        let s = &self.snapshot;
        body_blocked_cells(
            &s.structural,
            s.size_x,
            s.size_y,
            &self.closed,
            &self.footprint,
            at,
            heading,
            out,
        );
    }
}

/// Look-ahead fractions tried, longest first, and the shortest (tiles).
const LOOKAHEAD_FRACTIONS: [f32; 3] = [1.0, 2.0 / 3.0, 0.4];
const LOOKAHEAD_MIN: f32 = 0.5;
/// The step (tiles) the look-ahead check plays the pursuit forward in.
const PLAY_STEP: f32 = 0.1;

impl GuardGrid {
    /// The longest look-ahead (from `base`, see the module docs) whose
    /// pursuit, played from `pose` along `path` (from waypoint `idx`;
    /// `back`: reversing), keeps the body off blocked cells it is not on
    /// now.  `turn_per_tile`: the heading change (radians) a tile of
    /// driving allows.
    pub(super) fn safe_lookahead(
        &self,
        path: &[[i32; 2]],
        idx: usize,
        pose: (f32, f32, f32),
        back: bool,
        turn_per_tile: f32,
        base: f32,
    ) -> f32 {
        // nothing blocked within reach of the played stretch: as it is
        if !self.blocked_near((pose.0, pose.1), base + 1.0) {
            return base;
        }
        let mut here = Vec::new();
        self.blocked((pose.0, pose.1), pose.2, &mut here);
        let mut cells = Vec::new();
        let mut tried = f32::MAX;
        for f in LOOKAHEAD_FRACTIONS {
            let l = (base * f).max(LOOKAHEAD_MIN);
            if l < tried {
                tried = l;
                if self.pursuit_clear(path, idx, pose, back, turn_per_tile, l, &here, &mut cells) {
                    return l;
                }
            }
        }
        LOOKAHEAD_MIN
    }

    /// Whether any blocked cell (or the map edge) lies within `reach`
    /// tiles of the body's footprint at `at` (a square scan).
    fn blocked_near(&self, at: (f32, f32), reach: f32) -> bool {
        let s = &self.snapshot;
        let b = classic_core::pathfinder::footprint_reach(&self.footprint);
        let r = (b + reach).ceil() as i32;
        let (cx, cy) = (at.0.floor() as i32, at.1.floor() as i32);
        let near = |x: i32, y: i32| {
            x < 0
                || y < 0
                || x >= s.size_x
                || y >= s.size_y
                || s.structural[(y * s.size_x + x) as usize] == 0
        };
        (cy - r..=cy + r).any(|y| (cx - r..=cx + r).any(|x| near(x, y)))
            || self.closed.iter().any(|&[x0, y0, x1, y1]| {
                x1 >= cx - r && x0 <= cx + r && y1 >= cy - r && y0 <= cy + r
            })
    }

    /// Whether pure pursuit with look-ahead `l` stays clear (see
    /// [`Self::safe_lookahead`]) over the next `l + 1` tiles of driving.
    #[allow(clippy::too_many_arguments)]
    fn pursuit_clear(
        &self,
        path: &[[i32; 2]],
        mut idx: usize,
        (mut x, mut y, mut h): (f32, f32, f32),
        back: bool,
        turn_per_tile: f32,
        l: f32,
        here: &[GridCell],
        cells: &mut Vec<GridCell>,
    ) -> bool {
        let Some(end) = path.last() else { return true };
        let end = (end[0] as f32 + 0.5, end[1] as f32 + 0.5);
        let dir = if back { -1.0 } else { 1.0 };
        let steps = ((l + 1.0) / PLAY_STEP).ceil() as usize;
        for _ in 0..steps {
            if (end.0 - x).hypot(end.1 - y) < PLAY_STEP {
                break;
            }
            let (gx, gy, i) = lookahead(path, idx, x, y, l);
            idx = i;
            let travel = if back { h + std::f32::consts::PI } else { h };
            let err = wrap_pi((gy - y).atan2(gx - x) - travel);
            let max_turn = turn_per_tile * PLAY_STEP;
            h = wrap_pi(h + err.clamp(-max_turn, max_turn));
            x += dir * h.cos() * PLAY_STEP;
            y += dir * h.sin() * PLAY_STEP;
            self.blocked((x, y), h, cells);
            if cells.iter().any(|c| !here.contains(c)) {
                return false;
            }
        }
        true
    }
}

impl Engine {
    /// The grid `ve`'s follower is guarded on, or `None` without one (no
    /// terrain) or off a route.
    pub(super) fn follow_guard_grid(&self, ve: hecs::Entity) -> Option<GuardGrid> {
        let v = self.world.get::<&IsoVehicle>(ve).ok()?;
        if !self.vehicle_follow_guard || v.path.is_empty() {
            return None;
        }
        let s = &self.vehicle_nav_snapshot;
        if s.size_x <= 0 || s.structural.len() != (s.size_x * s.size_y) as usize {
            return None;
        }
        Some(GuardGrid {
            snapshot: std::sync::Arc::clone(s),
            closed: self.closed_vehicle_blocks(&self.debug_name(ve)),
            footprint: search_footprint(&v),
        })
    }
}

/// Check this step's move (`to`, with `v.heading` and `drive` tiles/s, from
/// `from`) against `grid`; return the move to make (see the module docs).
pub(super) fn follow_guard(
    grid: &GuardGrid,
    v: &mut IsoVehicle,
    from: (f32, f32, f32),
    to: (f32, f32),
    drive: f32,
    delta: f32,
) -> (f32, f32, f32) {
    let (x0, y0, h0) = from;
    let mut here = Vec::new();
    grid.blocked((x0, y0), h0, &mut here);
    let mut there = Vec::new();
    // the blocked cells a pose adds to those the body is on now
    let mut entered = |at: (f32, f32), h: f32| {
        grid.blocked(at, h, &mut there);
        there.iter().filter(|c| !here.contains(c)).count()
    };
    let first = entered(to, v.heading);
    if first == 0 {
        return (to.0, to.1, drive);
    }
    let step = drive.abs() * delta;
    let dir = drive.signum();
    // toward the route's next waypoint, the turn rate-limited
    let toward = {
        let w = v.path[v.path_idx.min(v.path.len() - 1)];
        let (wx, wy) = (w[0] as f32 + 0.5, w[1] as f32 + 0.5);
        let travel = if dir < 0.0 { h0 + std::f32::consts::PI } else { h0 };
        let err = wrap_pi((wy - y0).atan2(wx - x0) - travel);
        let max_turn = v.turn_rate * delta;
        wrap_pi(h0 + err.clamp(-max_turn, max_turn))
    };
    let mut best = (first, to.0, to.1, v.heading, drive);
    // slower, the same turn is tighter (the turn rate is per second)
    for frac in [1.0, 0.5, 0.25, 0.1] {
        for h in [v.heading, toward, h0] {
            let s = step * frac;
            let at = (x0 + dir * h.cos() * s, y0 + dir * h.sin() * s);
            let n = entered(at, h);
            if n < best.0 {
                best = (n, at.0, at.1, h, drive * frac);
            }
            if n == 0 {
                break;
            }
        }
        if best.0 == 0 {
            break;
        }
    }
    let (_, x, y, h, d) = best;
    v.turned += wrap_pi(h - v.heading);
    v.heading = h;
    (x, y, d)
}
