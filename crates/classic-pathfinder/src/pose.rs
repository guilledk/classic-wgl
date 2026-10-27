//! Pose goals: drive to a tile point *facing* a heading, turning as the
//! vehicle can, and backing up where a forward arc has no room (a three-point
//! or K-turn).
//!
//! A small state lattice: a state is a cell, one of 16 headings (22.5°
//! apart) and the gear it arrived in.  From each heading a fixed set of
//! motion primitives leaves the cell centre: straight, or a 22.5° arc of the
//! vehicle's turning radius left or right (then a short straight), each
//! forward and, when reversing is allowed, backward; a `pivot` vehicle also
//! turns 22.5° in place.  A primitive's end is rounded to the nearest cell
//! centre (its length is picked from a range to keep that rounding small),
//! and its swept body (the body rectangle sampled every 0.1 tile along it,
//! turned to the heading there) is pre-rasterized into row spans and checked
//! like the oriented search's steps.  The start is relaxed as there: the
//! first primitive ignores the cells under the body where it stands.
//!
//! Costs are the primitive's length (reversing at `reverse_cost` times),
//! plus `turn_cost` per 45° of heading change and a fixed cost per gear
//! change, so the vehicle backs up only when a forward route is much longer
//! or absent.  The heuristic is the straight-line distance, which no
//! primitive undercuts (a primitive costs at least its displacement).
//!
//! The search runs in a window round the start and the goal (the cells a
//! manoeuvre needs), so its state count stays small on a big map.  The route
//! comes back as waypoints whose `y` carries [`REVERSE_FLAG`] on a segment
//! driven backward ([`pack_waypoint`] / [`unpack_waypoint`]).

use alloc::collections::{BTreeSet, BinaryHeap};
use alloc::vec;
use alloc::vec::Vec;

use crate::oriented::{rect_spans_at, Blocked, BodyRect, Spans};
use crate::GridCell;

/// Headings of the lattice (22.5° apart).
const HEADINGS: usize = 16;
const STEP: f32 = core::f32::consts::TAU / HEADINGS as f32;
/// Cost of a gear change (tiles of driving).
const SWITCH_COST: f32 = 3.0;
/// Sample spacing (tiles) along a primitive for its swept body.
const SAMPLE: f32 = 0.1;

/// A pose goal: tile point `(x, y)` facing tile heading `heading` (radians,
/// `atan2(dy, dx)`, `+y` South), from a vehicle facing `start_heading`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PoseGoal {
    pub x: f32,
    pub y: f32,
    pub heading: f32,
    pub start_heading: f32,
    /// The vehicle's turning radius (tiles).
    pub turn_radius: f32,
    /// Whether the route may back up.
    pub reverse: bool,
    /// Cost multiplier of a backward tile (≥ 1).
    pub reverse_cost: f32,
}

/// The bit a pose route sets in a waypoint's `y` when the segment into it is
/// driven backward.
pub const REVERSE_FLAG: i32 = 1 << 24;

/// A waypoint with its gear packed into `y` (see [`REVERSE_FLAG`]).
pub fn pack_waypoint(c: GridCell, reverse: bool) -> GridCell {
    (c.0, if reverse { c.1 | REVERSE_FLAG } else { c.1 })
}

/// The cell and gear of a packed waypoint.
pub fn unpack_waypoint(c: GridCell) -> (GridCell, bool) {
    ((c.0, c.1 & !REVERSE_FLAG), c.1 & REVERSE_FLAG != 0)
}

/// The lattice heading nearest tile heading `h` (radians).
pub fn heading_index(h: f32) -> usize {
    let k = libm::roundf(h / STEP) as i32;
    k.rem_euclid(HEADINGS as i32) as usize
}

fn unit(h: f32) -> (f32, f32) {
    (libm::cosf(h), libm::sinf(h))
}

/// One motion primitive from a heading.
struct Prim {
    /// Heading change in lattice steps (`-1`, `0`, `1`).
    dk: i32,
    reverse: bool,
    /// End cell offset.
    end: (i32, i32),
    /// The cell halfway along (an extra waypoint on a turn).
    mid: Option<(i32, i32)>,
    /// Swept body cells, relative to the start cell.
    cells: Spans,
    cost: f32,
}

/// Row runs of a cell set.
fn runs(cells: &BTreeSet<(i32, i32)>) -> Spans {
    let mut out: Spans = Vec::new();
    for &(dy, dx) in cells {
        match out.last_mut() {
            Some(last) if last.0 == dy && last.2 + 1 == dx => last.2 = dx,
            _ => out.push((dy, dx, dx)),
        }
    }
    out
}

/// Add the body's cells at pose `(x, y, h)` to `set` (as `(dy, dx)`).
fn stamp(set: &mut BTreeSet<(i32, i32)>, b: &BodyRect, x: f32, y: f32, h: f32) {
    for (dy, x0, x1) in rect_spans_at(b.rear, b.front, b.right, b.left, unit(h), (x, y)) {
        for dx in x0..=x1 {
            set.insert((dy, dx));
        }
    }
}

/// The poses along a motion: `arc` radians of heading change (signed) over
/// an arc of radius `r`, then `extra` tiles straight, backward when
/// `reverse`; sampled every [`SAMPLE`] tiles.
fn poses(h0: f32, arc: f32, r: f32, extra: f32, reverse: bool) -> Vec<(f32, f32, f32)> {
    let dir = if reverse { -1.0 } else { 1.0 };
    let arc_len = libm::fabsf(arc) * r;
    let total = arc_len + extra;
    let n = libm::ceilf(total / SAMPLE).max(1.0) as usize;
    let ds = total / n as f32;
    let (mut x, mut y, mut h) = (0.0f32, 0.0f32, h0);
    let mut out = vec![(x, y, h)];
    for i in 0..n {
        let s = i as f32 * ds;
        // midpoint integration of the heading over the step
        let dh = if s < arc_len { arc.signum() * ds / r.max(1e-3) } else { 0.0 };
        let hm = h + 0.5 * dh;
        x += dir * libm::cosf(hm) * ds;
        y += dir * libm::sinf(hm) * ds;
        h += dh;
        out.push((x, y, h));
    }
    out
}

/// Pick the straight extension (from `range`, step 0.1) whose end rounds to
/// a cell centre most closely.
fn best_extra(h0: f32, arc: f32, r: f32, reverse: bool, range: (f32, f32)) -> f32 {
    let mut best = (f32::MAX, range.0);
    let mut e = range.0;
    while e <= range.1 + 1e-4 {
        let p = *poses(h0, arc, r, e, reverse).last().unwrap();
        let err = libm::hypotf(p.0 - libm::roundf(p.0), p.1 - libm::roundf(p.1));
        if err < best.0 - 1e-4 {
            best = (err, e);
        }
        e += 0.1;
    }
    best.1
}

/// The primitives from each heading.
fn primitives(b: &BodyRect, r: f32, reverse: bool, pivot: bool, turn_step: f32) -> Vec<Vec<Prim>> {
    (0..HEADINGS)
        .map(|k| {
            let h0 = k as f32 * STEP;
            let mut prims = Vec::new();
            for back in [false, true] {
                if back && !reverse {
                    continue;
                }
                for dk in [-1i32, 0, 1] {
                    let arc = dk as f32 * STEP;
                    let range = if dk == 0 { (2.0, 3.0) } else { (0.3, 1.8) };
                    let extra = best_extra(h0, arc, r, back, range);
                    let ps = poses(h0, arc, r, extra, back);
                    let (ex, ey, eh) = *ps.last().unwrap();
                    let end = (libm::roundf(ex) as i32, libm::roundf(ey) as i32);
                    let mut set = BTreeSet::new();
                    for &(x, y, h) in &ps {
                        stamp(&mut set, b, x, y, h);
                    }
                    // the correction from the true end onto the cell centre
                    for i in 0..=4 {
                        let t = i as f32 / 4.0;
                        stamp(
                            &mut set,
                            b,
                            ex + (end.0 as f32 - ex) * t,
                            ey + (end.1 as f32 - ey) * t,
                            eh,
                        );
                    }
                    let (mx, my, _) = ps[ps.len() / 2];
                    let mid = (libm::roundf(mx) as i32, libm::roundf(my) as i32);
                    let mid = (dk != 0 && mid != (0, 0) && mid != end).then_some(mid);
                    // the follower drives the chords between the waypoints:
                    // sweep the body along them too, facing along each
                    let mut corners = vec![(0, 0)];
                    corners.extend(mid);
                    corners.push(end);
                    for w in corners.windows(2) {
                        let (a, c) = (w[0], w[1]);
                        let (dx, dy) = ((c.0 - a.0) as f32, (c.1 - a.1) as f32);
                        let len = libm::hypotf(dx, dy);
                        let hc =
                            libm::atan2f(dy, dx) + if back { core::f32::consts::PI } else { 0.0 };
                        let n = libm::ceilf(len / SAMPLE).max(1.0) as usize;
                        for i in 0..=n {
                            let t = i as f32 / n as f32;
                            stamp(&mut set, b, a.0 as f32 + dx * t, a.1 as f32 + dy * t, hc);
                        }
                    }
                    let len = libm::fabsf(arc) * r + extra;
                    let disp = libm::hypotf(end.0 as f32, end.1 as f32);
                    prims.push(Prim {
                        dk,
                        reverse: back,
                        end,
                        mid,
                        cells: runs(&set),
                        cost: len.max(disp) + dk.unsigned_abs() as f32 * turn_step,
                    });
                }
            }
            if pivot {
                for dk in [-1i32, 1] {
                    let mut set = BTreeSet::new();
                    for i in 0..=8 {
                        stamp(&mut set, b, 0.0, 0.0, h0 + dk as f32 * STEP * i as f32 / 8.0);
                    }
                    prims.push(Prim {
                        dk,
                        reverse: false,
                        end: (0, 0),
                        mid: None,
                        cells: runs(&set),
                        cost: 0.5 + turn_step,
                    });
                }
            }
            prims
        })
        .collect()
}

/// Whether a pose route stays clear on `grid`: the body swept along each
/// chord between waypoints (the line the follower drives), facing along it
/// (backward on a reverse segment), every [`SAMPLE`] tiles.  The first
/// segment checks only its end (the vehicle already stands at the start).
pub(crate) fn route_clear_pose(
    grid: &[i32],
    size_x: i32,
    size_y: i32,
    path: &[GridCell],
    reverse: &[bool],
    body: &BodyRect,
) -> bool {
    let blocked = Blocked::new(grid, size_x, size_y);
    path.windows(2).enumerate().all(|(i, w)| {
        let back = reverse.get(i + 1).copied().unwrap_or(false);
        let (a, b) = (w[0], w[1]);
        let (dx, dy) = ((b.0 - a.0) as f32, (b.1 - a.1) as f32);
        let len = libm::hypotf(dx, dy);
        if len < 1e-6 {
            return true;
        }
        let h = libm::atan2f(dy, dx) + if back { core::f32::consts::PI } else { 0.0 };
        let n = libm::ceilf(len / SAMPLE) as usize;
        let first = if i == 0 { n } else { 0 };
        (first..=n).all(|j| {
            let t = j as f32 / n as f32;
            let (px, py) = (a.0 as f32 + dx * t, a.1 as f32 + dy * t);
            let (cx, cy) = (libm::roundf(px) as i32, libm::roundf(py) as i32);
            let spans = rect_spans_at(
                body.rear,
                body.front,
                body.right,
                body.left,
                unit(h),
                (px - cx as f32, py - cy as f32),
            );
            blocked.clear(cx, cy, &spans)
        })
    })
}

#[derive(Copy, Clone, PartialEq)]
struct Key {
    cost: f32,
    state: u32,
}
impl Eq for Key {}
impl PartialOrd for Key {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Key {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        other.cost.total_cmp(&self.cost).then_with(|| self.state.cmp(&other.state))
    }
}

/// Search a pose route on `walkable` (`1` open) for a vehicle at cell `from`
/// (see the module docs).  `turn_cost` is per 45° of heading change.
#[allow(clippy::too_many_arguments)]
pub(crate) fn find_pose(
    walkable: &[i32],
    size_x: i32,
    size_y: i32,
    from: GridCell,
    goal: &PoseGoal,
    body: &BodyRect,
    pivot: bool,
    turn_cost: f32,
) -> Option<Vec<GridCell>> {
    if size_x <= 0 || size_y <= 0 || !goal.x.is_finite() || !goal.y.is_finite() {
        return None;
    }
    let from = (from.0.clamp(0, size_x - 1), from.1.clamp(0, size_y - 1));
    let to = (libm::floorf(goal.x) as i32, libm::floorf(goal.y) as i32);
    if to.0 < 0 || to.1 < 0 || to.0 >= size_x || to.1 >= size_y {
        return None;
    }
    let k0 = heading_index(goal.start_heading);
    let kg = heading_index(goal.heading);
    if from == to && k0 == kg {
        return Some(vec![from, from]);
    }
    let r = goal.turn_radius.max(0.5);
    let reverse_cost = goal.reverse_cost.max(1.0);
    let prims = primitives(body, r, goal.reverse, pivot, turn_cost.max(0.0) * 0.5);
    let blocked = Blocked::new(walkable, size_x, size_y);

    // the window: start and goal, with room to manoeuvre round them
    let reach = libm::fabsf(body.rear)
        .max(libm::fabsf(body.front))
        .max(libm::fabsf(body.left))
        .max(libm::fabsf(body.right));
    let margin = libm::ceilf(8.0 + 3.0 * r + reach) as i32;
    let wx0 = (from.0.min(to.0) - margin).max(0);
    let wy0 = (from.1.min(to.1) - margin).max(0);
    let wx1 = (from.0.max(to.0) + margin).min(size_x - 1);
    let wy1 = (from.1.max(to.1) + margin).min(size_y - 1);
    let (ww, wh) = ((wx1 - wx0 + 1) as usize, (wy1 - wy0 + 1) as usize);
    let total = ww * wh * HEADINGS * 2;
    let state = |c: GridCell, k: usize, back: bool| {
        ((((c.1 - wy0) as usize * ww + (c.0 - wx0) as usize) * HEADINGS + k) * 2 + back as usize)
            as u32
    };
    let decode = |s: u32| {
        let s = s as usize;
        let back = s % 2 == 1;
        let k = (s / 2) % HEADINGS;
        let c = s / 2 / HEADINGS;
        (((c % ww) as i32 + wx0, (c / ww) as i32 + wy0), k, back)
    };

    // the cells under the body where it stands (relaxed for the first move)
    let mut start_body = BTreeSet::new();
    stamp(&mut start_body, body, 0.0, 0.0, k0 as f32 * STEP);
    let clear_cell = |x: i32, y: i32| blocked.clear(x, y, &vec![(0, 0, 0)]);

    let h = |c: GridCell| libm::hypotf((c.0 - to.0) as f32, (c.1 - to.1) as f32);
    let mut g = vec![f32::MAX; total];
    let mut came: Vec<(u32, u16)> = vec![(u32::MAX, 0); total];
    let start = state(from, k0, false);
    g[start as usize] = 0.0;
    let mut open = BinaryHeap::new();
    open.push(Key { cost: h(from), state: start });
    let mut done = vec![false; total];

    while let Some(Key { state: s, .. }) = open.pop() {
        if done[s as usize] {
            continue;
        }
        done[s as usize] = true;
        let (c, k, back) = decode(s);
        if c == to && k == kg {
            // rebuild: the primitives taken, then their waypoints
            let mut chain = Vec::new();
            let mut cur = s;
            while came[cur as usize].0 != u32::MAX {
                let (prev, p) = came[cur as usize];
                chain.push((prev, p));
                cur = prev;
            }
            chain.reverse();
            let mut path = vec![from];
            for (prev, p) in chain {
                let (pc, pk, _) = decode(prev);
                let prim = &prims[pk][p as usize];
                let mut push = |cell: GridCell| {
                    let last = unpack_waypoint(*path.last().unwrap()).0;
                    if cell != last {
                        path.push(pack_waypoint(cell, prim.reverse));
                    }
                };
                if let Some((mx, my)) = prim.mid {
                    push((pc.0 + mx, pc.1 + my));
                }
                push((pc.0 + prim.end.0, pc.1 + prim.end.1));
            }
            if path.len() == 1 {
                path.push(from);
            }
            return Some(path);
        }
        let at_start = s == start;
        for (pi, prim) in prims[k].iter().enumerate() {
            let e = (c.0 + prim.end.0, c.1 + prim.end.1);
            if e.0 < wx0 || e.1 < wy0 || e.0 > wx1 || e.1 > wy1 {
                continue;
            }
            let fits = if at_start {
                prim.cells.iter().all(|&(dy, x0, x1)| {
                    (x0..=x1)
                        .all(|dx| start_body.contains(&(dy, dx)) || clear_cell(c.0 + dx, c.1 + dy))
                })
            } else {
                blocked.clear(c.0, c.1, &prim.cells)
            };
            if !fits {
                continue;
            }
            let mut cost = prim.cost * if prim.reverse { reverse_cost } else { 1.0 };
            if !at_start && prim.reverse != back && prim.end != (0, 0) {
                cost += SWITCH_COST;
            }
            let nk = (k as i32 + prim.dk).rem_euclid(HEADINGS as i32) as usize;
            // a pivot keeps the gear it stood in
            let nback = if prim.end == (0, 0) { back } else { prim.reverse };
            let n = state(e, nk, nback);
            let ng = g[s as usize] + cost;
            if ng < g[n as usize] {
                g[n as usize] = ng;
                came[n as usize] = (s, pi as u16);
                open.push(Key { cost: ng + h(e), state: n });
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;

    fn mule() -> BodyRect {
        let t = 0.703125;
        BodyRect { rear: -1.83 / t, front: 1.64 / t, right: -2.07 / t, left: 2.07 / t }
    }

    fn goal(x: f32, y: f32, deg: f32, start_deg: f32, reverse: bool) -> PoseGoal {
        PoseGoal {
            x,
            y,
            heading: deg.to_radians(),
            start_heading: start_deg.to_radians(),
            turn_radius: 2.6,
            reverse,
            reverse_cost: 2.0,
        }
    }

    /// The gear and heading (degrees) of each segment of a route.
    fn segments(path: &[GridCell]) -> Vec<(bool, f32)> {
        path.windows(2)
            .map(|w| {
                let (a, _) = unpack_waypoint(w[0]);
                let (b, back) = unpack_waypoint(w[1]);
                let d = libm::atan2f((b.1 - a.1) as f32, (b.0 - a.0) as f32).to_degrees();
                (back, if back { d + 180.0 } else { d })
            })
            .collect()
    }

    fn diff(a: f32, b: f32) -> f32 {
        ((a - b + 540.0) % 360.0 - 180.0).abs()
    }

    #[test]
    fn waypoints_pack_their_gear() {
        assert_eq!(unpack_waypoint(pack_waypoint((7, 9), true)), ((7, 9), true));
        assert_eq!(unpack_waypoint(pack_waypoint((7, 9), false)), ((7, 9), false));
        assert_eq!(heading_index(-0.1), 0);
        assert_eq!(heading_index(std::f32::consts::PI), 8);
    }

    #[test]
    fn open_ground_arrives_facing_the_goal_heading() {
        let n = 80;
        let g = vec![1; (n * n) as usize];
        for (deg, start_deg) in [(90.0, 0.0), (180.0, 0.0), (45.0, 270.0), (0.0, 180.0)] {
            let p = find_pose(
                &g,
                n,
                n,
                (30, 30),
                &goal(45.5, 38.5, deg, start_deg, true),
                &mule(),
                false,
                0.4,
            )
            .unwrap_or_else(|| panic!("{deg}"));
            let (last, _) = unpack_waypoint(*p.last().unwrap());
            assert_eq!(last, (45, 38));
            let segs = segments(&p);
            let (_, end) = *segs.last().unwrap();
            assert!(diff(end, deg) <= 30.0, "{deg}: arrives at {end} ({segs:?})");
        }
    }

    #[test]
    fn a_closed_lane_mouth_needs_a_k_turn_and_the_body_stays_clear() {
        // A 13-cell corridor (rows 17..=29) along x, walled north and
        // south: facing East near its west end, reach a point further east
        // facing West.  The mule (5.9 tiles across, 4.9 long, turning 2.6
        // tiles) cannot U-turn forward in it, so the route backs up.
        let n = 60;
        let mut g = vec![0; (n * n) as usize];
        for y in 17..=29 {
            for x in 3..57 {
                g[(y * n + x) as usize] = 1;
            }
        }
        let body = mule();
        let fwd =
            find_pose(&g, n, n, (12, 23), &goal(30.5, 23.5, 180.0, 0.0, false), &body, false, 0.4);
        assert!(fwd.is_none(), "no forward-only U-turn: {fwd:?}");
        let p =
            find_pose(&g, n, n, (12, 23), &goal(30.5, 23.5, 180.0, 0.0, true), &body, false, 0.4)
                .expect("a K-turn");
        let segs = segments(&p);
        assert!(segs.iter().any(|s| s.0), "backs up: {segs:?}");
        assert!(segs.iter().any(|s| !s.0), "and drives forward");
        // every waypoint's body, at its segment heading, is on open cells
        let blocked = Blocked::new(&g, n, n);
        for (w, (_, deg)) in p.windows(2).zip(&segs) {
            let (b, _) = unpack_waypoint(w[1]);
            let spans = crate::oriented::rect_spans(
                body.rear,
                body.front,
                body.right,
                body.left,
                unit(deg.to_radians()),
            );
            assert!(blocked.clear(b.0, b.1, &spans), "body at {b:?} facing {deg}");
        }
    }

    #[test]
    fn an_unreachable_pose_is_refused() {
        let n = 40;
        let mut g = vec![1; (n * n) as usize];
        for y in 0..n {
            g[(y * n + 20) as usize] = 0;
        }
        assert!(find_pose(
            &g,
            n,
            n,
            (8, 20),
            &goal(32.5, 20.5, 0.0, 0.0, true),
            &mule(),
            false,
            0.4
        )
        .is_none());
        // a goal off the map, or with no room for the body there
        assert!(find_pose(
            &g,
            n,
            n,
            (8, 20),
            &goal(-3.0, 4.0, 0.0, 0.0, true),
            &mule(),
            false,
            0.4
        )
        .is_none());
        assert!(find_pose(&g, n, n, (8, 20), &goal(1.5, 1.5, 0.0, 0.0, true), &mule(), false, 0.4)
            .is_none());
    }

    #[test]
    #[ignore = "perf check: prints timings"]
    fn perf_pose_search() {
        use std::time::Instant;
        let n = 400;
        let g = vec![1; (n * n) as usize];
        let t = Instant::now();
        let p = find_pose(
            &g,
            n,
            n,
            (100, 100),
            &goal(140.5, 125.5, 180.0, 0.0, true),
            &mule(),
            false,
            0.4,
        );
        std::println!(
            "400² pose search 40x25 tiles away: {:?} ({} waypoints)",
            t.elapsed(),
            p.map_or(0, |p| p.len())
        );
    }
}
