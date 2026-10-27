//! The oriented-footprint vehicle search: the vehicle's body rectangle turned
//! to each move's heading, rather than one axis box applied in every heading.
//!
//! A search state is a cell plus the heading the vehicle entered it with (one
//! of the 8 move directions, or none at the start), so a step's legality and
//! its turn penalty both see the real incoming heading.  A step `a -> b` along
//! direction `d` is legal when every cell under the body *swept* from `a` to
//! `b` at heading `d` (the body rectangle with its front pushed out by the
//! step length) is open: that covers the body at `a` already turned to `d`
//! (so a turn is only planned where the body fits at both headings), the body
//! at `b`, and the cells it crosses in between.  A `pivot` (turn-in-place)
//! vehicle also needs the body clear at every 22.5° heading it turns through
//! at `a`.  The start cell is relaxed the way the axis-box search relaxes it:
//! the first step only checks the body at `b`, so a vehicle already grazing an
//! obstacle can drive out.
//!
//! Each footprint is pre-rasterized once per heading into row spans, and the
//! blocked cells into per-row prefix counts, so checking a footprint costs
//! one lookup per row, and each `(cell, heading)` answer is memoized.  Only
//! the cells the A* reaches are checked, so a short route on a big map costs
//! what its search touches, not a full erosion of the grid per heading.

use alloc::collections::BinaryHeap;
use alloc::vec;
use alloc::vec::Vec;

use crate::{heuristic, GridCell};

/// A vehicle's body on the ground, in tiles, in its chassis frame: `+x`
/// forward along the heading, `+y` to the left, the anchor cell's centre at
/// the origin.  `rear < front` and `right < left`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyRect {
    pub rear: f32,
    pub front: f32,
    pub right: f32,
    pub left: f32,
}

impl BodyRect {
    /// A rectangle centred on the anchor: `±length/2` along the heading and
    /// `±width/2` across it.
    pub fn centred(length: f32, width: f32) -> Self {
        Self { rear: -length * 0.5, front: length * 0.5, right: -width * 0.5, left: width * 0.5 }
    }
}

/// A vehicle's collision footprint for the vehicle search.
#[derive(Clone, Debug, PartialEq)]
pub enum VehicleFootprint {
    /// Fixed tile offsets from the anchor cell, the same in every heading (an
    /// authored `path_footprint`): the nav grid is eroded by them once and
    /// the 1×1 search runs on the result.
    Cells(Vec<GridCell>),
    /// The body rectangle turned to each move's heading (see the module
    /// docs).  `pivot` = the vehicle turns in place, so the body must clear
    /// every heading it sweeps through at a corner.
    Oriented { body: BodyRect, pivot: bool },
}

/// One rasterized footprint: `(dy, dx0, dx1)` rows of cell offsets, `dx0..=dx1`.
pub(crate) type Spans = Vec<(i32, i32, i32)>;

/// The 8 move directions, index `d` at `d · 45°` from East (tile space,
/// `+y` South): the neighbour offset.
pub(crate) const DIRS: [(i32, i32); 8] =
    [(1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1)];

/// Snap to a 1e-4 tile grid so a rotation's float noise (`cos 90°` is not
/// quite 0) never tips an exact edge into the next cell.
fn snap(v: f32) -> f32 {
    libm::roundf(v * 1e4) / 1e4
}

/// The unit heading at `k · 22.5°`.
fn heading16(k: usize) -> (f32, f32) {
    let a = k as f32 * core::f32::consts::FRAC_PI_8;
    (libm::cosf(a), libm::sinf(a))
}

/// The cells a body rectangle `[x0, x1] × [y0, y1]` (chassis frame) covers
/// at heading `(hx, hy)`, anchored on a cell centre: every cell its interior
/// overlaps (a cell it only touches is left out).
pub(crate) fn rect_spans(x0: f32, x1: f32, y0: f32, y1: f32, (hx, hy): (f32, f32)) -> Spans {
    // chassis (a, b) -> tile offset a·h + b·left, left = (hy, -hx)
    let at = |a: f32, b: f32| (snap(a * hx + b * hy), snap(a * hy - b * hx));
    let poly = [at(x0, y0), at(x1, y0), at(x1, y1), at(x0, y1)];
    let ymin = poly.iter().map(|p| p.1).fold(f32::MAX, f32::min);
    let ymax = poly.iter().map(|p| p.1).fold(f32::MIN, f32::max);
    // rows whose strip [dy - 0.5, dy + 0.5] the body's interior overlaps
    let dy0 = libm::floorf(ymin - 0.5) as i32 + 1;
    let dy1 = libm::ceilf(ymax + 0.5) as i32 - 1;
    let mut spans = Vec::new();
    for dy in dy0..=dy1 {
        let (lo, hi) = (dy as f32 - 0.5, dy as f32 + 0.5);
        // x-extent of the convex polygon clipped to the strip: its vertices
        // inside the strip plus its edges' crossings of the strip lines
        let (mut xmin, mut xmax) = (f32::MAX, f32::MIN);
        for i in 0..4 {
            let (p, q) = (poly[i], poly[(i + 1) % 4]);
            if p.1 >= lo && p.1 <= hi {
                xmin = xmin.min(p.0);
                xmax = xmax.max(p.0);
            }
            for line in [lo, hi] {
                if (p.1 - line) * (q.1 - line) < 0.0 {
                    let x = p.0 + (q.0 - p.0) * (line - p.1) / (q.1 - p.1);
                    xmin = xmin.min(x);
                    xmax = xmax.max(x);
                }
            }
        }
        if xmin > xmax {
            continue;
        }
        let dx0 = libm::floorf(snap(xmin) - 0.5) as i32 + 1;
        let dx1 = libm::ceilf(snap(xmax) + 0.5) as i32 - 1;
        if dx0 <= dx1 {
            spans.push((dy, dx0, dx1));
        }
    }
    spans
}

/// The body's footprints: at each of the 16 headings (`k · 22.5°`), and
/// swept along each of the 8 move directions over one step.
pub(crate) struct Masks {
    pub(crate) body16: Vec<Spans>,
    pub(crate) swept8: Vec<Spans>,
}

impl Masks {
    pub(crate) fn new(b: &BodyRect) -> Self {
        let body16 = (0..16).map(|k| rect_spans(b.rear, b.front, b.right, b.left, heading16(k)));
        let swept8 = (0..8).map(|d| {
            let len = if d % 2 == 1 { core::f32::consts::SQRT_2 } else { 1.0 };
            rect_spans(b.rear, b.front + len, b.right, b.left, heading16(2 * d))
        });
        Self { body16: body16.collect(), swept8: swept8.collect() }
    }
}

/// Per-row prefix counts of a grid's blocked cells (`0`): any footprint span
/// is clear in one subtraction.
pub(crate) struct Blocked {
    sx: i32,
    sy: i32,
    pre: Vec<u32>,
}

impl Blocked {
    pub(crate) fn new(grid: &[i32], sx: i32, sy: i32) -> Self {
        let w = sx as usize + 1;
        let mut pre = vec![0u32; w * sy as usize];
        for y in 0..sy as usize {
            for x in 0..sx as usize {
                pre[y * w + x + 1] = pre[y * w + x] + (grid[y * sx as usize + x] == 0) as u32;
            }
        }
        Self { sx, sy, pre }
    }

    /// Whether every cell of `spans` anchored at `(x, y)` is open (and on the
    /// grid).
    pub(crate) fn clear(&self, x: i32, y: i32, spans: &Spans) -> bool {
        spans.iter().all(|&(dy, dx0, dx1)| {
            let (yy, x0, x1) = (y + dy, x + dx0, x + dx1);
            if yy < 0 || yy >= self.sy || x0 < 0 || x1 >= self.sx {
                return false;
            }
            let r = yy as usize * (self.sx as usize + 1);
            self.pre[r + x1 as usize + 1] == self.pre[r + x0 as usize]
        })
    }
}

/// Memoized "is the swept footprint for direction `d` at this cell clear":
/// per cell, bit `d` = known, bit `8 + d` = clear.
struct Memo {
    bits: Vec<u16>,
}

impl Memo {
    fn new(total: usize) -> Self {
        Self { bits: vec![0; total] }
    }

    fn get(&mut self, idx: usize, d: usize, compute: impl FnOnce() -> bool) -> bool {
        let b = self.bits[idx];
        if b & (1 << d) != 0 {
            return b & (1 << (8 + d)) != 0;
        }
        let ok = compute();
        self.bits[idx] = b | (1 << d) | ((ok as u16) << (8 + d));
        ok
    }
}

/// Ordered key for the state priority queue (lower cost first).
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

/// The 45° steps between two move directions (0..=4).
fn octants(a: usize, b: usize) -> usize {
    let d = a.abs_diff(b);
    d.min(8 - d)
}

/// Whether a pivot from direction `din` to `dout` at `(x, y)` keeps the body
/// clear at every 22.5° heading strictly between them, turning the short way
/// (either way for a reversal).
fn pivot_clear(blocked: &Blocked, masks: &Masks, x: i32, y: i32, din: usize, dout: usize) -> bool {
    let (a, b) = (2 * din, 2 * dout);
    let sweep = |step: usize| {
        let mut k = (a + step) % 16;
        while k != b {
            if !blocked.clear(x, y, &masks.body16[k]) {
                return false;
            }
            k = (k + step) % 16;
        }
        true
    };
    let ccw = (b + 16 - a) % 16; // steps going +22.5°
    match ccw.cmp(&8) {
        core::cmp::Ordering::Less => sweep(1),
        core::cmp::Ordering::Greater => sweep(15),
        core::cmp::Ordering::Equal => sweep(1) || sweep(15),
    }
}

/// Oriented-footprint vehicle A* (see the module docs).  `walkable` is the
/// combined grid; with `safe_fall_m > 0` a step whose swept body is blocked
/// on `walkable` but open on `structural` is still a jump when it drops
/// (`cell_height`) by at most `safe_fall_m`, at `jump_cost` times the base
/// cost.  `turn_cost` is charged per 45° between the incoming and outgoing
/// direction.
#[allow(clippy::too_many_arguments)]
pub(crate) fn find_oriented(
    walkable: &[i32],
    structural: &[i32],
    size_x: i32,
    size_y: i32,
    from: GridCell,
    to: GridCell,
    body: &BodyRect,
    pivot: bool,
    safe_fall_m: f32,
    jump_cost: f32,
    turn_cost: f32,
    cell_height: impl Fn(i32, i32) -> f32,
) -> Option<Vec<GridCell>> {
    if size_x <= 0 || size_y <= 0 {
        return None;
    }
    let from = (from.0.clamp(0, size_x - 1), from.1.clamp(0, size_y - 1));
    let to = (to.0.clamp(0, size_x - 1), to.1.clamp(0, size_y - 1));
    if from == to {
        return Some(vec![from, to]);
    }

    let masks = Masks::new(body);
    let walk = Blocked::new(walkable, size_x, size_y);
    let jumps = safe_fall_m > 0.0;
    let structure = jumps.then(|| Blocked::new(structural, size_x, size_y));
    let cells = (size_x * size_y) as usize;
    let mut walk_memo = Memo::new(cells);
    let mut struct_memo = Memo::new(if jumps { cells } else { 0 });

    // Without a turn penalty or a pivot sweep the incoming heading changes
    // nothing a step sees, so one state per cell finds the same route.
    let layers = if turn_cost <= 0.0 && !pivot { 1 } else { 9 };
    const START: usize = 8;
    let state = |cell: usize, d: usize| (cell * layers + if layers == 1 { 0 } else { d }) as u32;
    let flatten = |x: i32, y: i32| (x + y * size_x) as usize;

    let total = cells * layers;
    let mut g_cost = vec![f32::MAX; total];
    let mut came_from = vec![u32::MAX; total];
    let mut in_open = vec![false; total];

    let start = state(flatten(from.0, from.1), START);
    g_cost[start as usize] = 0.0;
    let mut open = BinaryHeap::new();
    open.push(Key { cost: heuristic(from, to), state: start });

    while let Some(Key { state: s, .. }) = open.pop() {
        let su = s as usize;
        let cell = su / layers;
        let current = ((cell % size_x as usize) as i32, (cell / size_x as usize) as i32);
        if current == to {
            let mut path = vec![current];
            let mut cur = s;
            while came_from[cur as usize] != u32::MAX {
                cur = came_from[cur as usize];
                let c = cur as usize / layers;
                path.push(((c % size_x as usize) as i32, (c / size_x as usize) as i32));
            }
            path.reverse();
            return Some(path);
        }
        in_open[su] = false;
        // collapsed, the incoming heading is never looked at
        let din = if layers == 1 { START } else { su % layers };
        let at_start = s == start;

        for (d, &(dx, dy)) in DIRS.iter().enumerate() {
            let (nx, ny) = (current.0 + dx, current.1 + dy);
            if nx < 0 || nx >= size_x || ny < 0 || ny >= size_y {
                continue;
            }
            if pivot
                && din != START
                && din != d
                && !pivot_clear(&walk, &masks, current.0, current.1, din, d)
            {
                continue;
            }
            let base = if d % 2 == 1 { core::f32::consts::SQRT_2 } else { 1.0 };
            // the start is relaxed: only the body at the first step's end
            let fits = |grid: &Blocked| {
                if at_start {
                    grid.clear(nx, ny, &masks.body16[2 * d])
                } else {
                    grid.clear(current.0, current.1, &masks.swept8[d])
                }
            };
            let walk_ok =
                if at_start { fits(&walk) } else { walk_memo.get(cell, d, || fits(&walk)) };
            let step = if walk_ok {
                base
            } else if let Some(structure) = &structure {
                let struct_ok = if at_start {
                    fits(structure)
                } else {
                    struct_memo.get(cell, d, || fits(structure))
                };
                let drop = cell_height(current.0, current.1) - cell_height(nx, ny);
                if struct_ok && drop > 0.0 && drop <= safe_fall_m {
                    base * jump_cost
                } else {
                    continue;
                }
            } else {
                continue;
            };
            let turn = if din == START || turn_cost <= 0.0 {
                0.0
            } else {
                turn_cost * octants(din, d) as f32
            };
            let n = state(flatten(nx, ny), d) as usize;
            let g = g_cost[su] + step + turn;
            if g < g_cost[n] {
                g_cost[n] = g;
                came_from[n] = s;
                if !in_open[n] {
                    in_open[n] = true;
                    open.push(Key { cost: g + heuristic((nx, ny), to), state: n as u32 });
                }
            }
        }
    }
    None
}

/// Whether an oriented route stays clear on `grid`: the same per-step rule
/// as [`find_oriented`] (the first step only the body at its end, every
/// later step its swept body), pivots aside.  A step that is not one cell in
/// one of the 8 directions is refused.
pub(crate) fn route_clear_oriented(
    grid: &[i32],
    size_x: i32,
    size_y: i32,
    path: &[GridCell],
    body: &BodyRect,
) -> bool {
    let masks = Masks::new(body);
    let blocked = Blocked::new(grid, size_x, size_y);
    path.windows(2).enumerate().all(|(i, w)| {
        let (a, b) = (w[0], w[1]);
        let Some(d) = DIRS.iter().position(|&o| o == (b.0 - a.0, b.1 - a.1)) else {
            return false;
        };
        if i == 0 {
            blocked.clear(b.0, b.1, &masks.body16[2 * d])
        } else {
            blocked.clear(a.0, a.1, &masks.swept8[d])
        }
    })
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use crate::{erode_for_footprint, find_path, find_path_for_footprint_with_jumps};

    /// A long, narrow body: 6 tiles nose to tail, 2 across.
    fn long_body() -> BodyRect {
        BodyRect::centred(5.8, 1.8)
    }

    /// The mule (classic-assets `vehicles/mule`) out to its tyres' edges, in
    /// 0.703125 m tiles: wider than it is long, and off-centre front to back.
    fn mule_body() -> BodyRect {
        let t = 0.703125;
        BodyRect { rear: -1.83 / t, front: 1.64 / t, right: -2.07 / t, left: 2.07 / t }
    }

    fn search(
        grid: &[i32],
        n: i32,
        from: GridCell,
        to: GridCell,
        body: &BodyRect,
        turn: f32,
    ) -> Option<Vec<GridCell>> {
        let heights = vec![0.0; ((n + 1) * (n + 1)) as usize];
        let fp = VehicleFootprint::Oriented { body: *body, pivot: false };
        find_path_for_footprint_with_jumps(
            grid, grid, &heights, n, n, from, to, &fp, 0.0, 1.0, turn,
        )
    }

    /// Every cell a body point lands in at `p` (a cell centre) facing `(hx,
    /// hy)`, sampled every 0.05 tiles a hair inside the body: an
    /// independent check of [`rect_spans`].
    fn body_cells(b: &BodyRect, p: GridCell, (hx, hy): (f32, f32), mut f: impl FnMut(i32, i32)) {
        let e = 1e-3;
        let (nx, ny) = (((b.front - b.rear) / 0.05) as i32, ((b.left - b.right) / 0.05) as i32);
        for i in 0..=nx {
            let a = b.rear + e + (b.front - b.rear - 2.0 * e) * i as f32 / nx as f32;
            for j in 0..=ny {
                let c = b.right + e + (b.left - b.right - 2.0 * e) * j as f32 / ny as f32;
                let x = p.0 as f32 + 0.5 + a * hx + c * hy;
                let y = p.1 as f32 + 0.5 + a * hy - c * hx;
                f(libm::floorf(x) as i32, libm::floorf(y) as i32);
            }
        }
    }

    fn cells_of(spans: &Spans) -> Vec<GridCell> {
        let mut v: Vec<GridCell> =
            spans.iter().flat_map(|&(dy, x0, x1)| (x0..=x1).map(move |dx| (dx, dy))).collect();
        v.sort_unstable();
        v
    }

    #[test]
    fn a_body_turns_with_its_heading() {
        let b = BodyRect::centred(2.8, 4.8); // 3 cells long, 5 across
        let east = cells_of(&rect_spans(b.rear, b.front, b.right, b.left, heading16(0)));
        let south = cells_of(&rect_spans(b.rear, b.front, b.right, b.left, heading16(4)));
        let box_ = |hx: i32, hy: i32| {
            let mut v: Vec<GridCell> =
                (-hy..=hy).flat_map(|dy| (-hx..=hx).map(move |dx| (dx, dy))).collect();
            v.sort_unstable();
            v
        };
        assert_eq!(east, box_(1, 2));
        assert_eq!(south, box_(2, 1));
        // a cell the body only touches is left out: a unit body is one cell
        let unit = BodyRect::centred(1.0, 1.0);
        for k in [0, 4, 8, 12] {
            let c =
                cells_of(&rect_spans(unit.rear, unit.front, unit.right, unit.left, heading16(k)));
            assert_eq!(c, vec![(0, 0)], "heading {k}");
        }
    }

    #[test]
    fn rasterized_bodies_cover_every_body_point() {
        for b in [mule_body(), long_body(), BodyRect::centred(1.0, 1.0)] {
            for k in 0..16 {
                let cells = cells_of(&rect_spans(b.rear, b.front, b.right, b.left, heading16(k)));
                body_cells(&b, (0, 0), heading16(k), |x, y| {
                    assert!(cells.binary_search(&(x, y)).is_ok(), "{b:?} heading {k}: ({x}, {y})");
                });
            }
        }
    }

    /// An east-west lane 3 cells tall (`y` 9..=11) across a 30² grid, and a
    /// 3-wide branch running south from its middle.
    fn lane_and_branch() -> (Vec<i32>, i32) {
        let n = 30;
        let mut g = vec![0; (n * n) as usize];
        for y in 0..n {
            for x in 0..n {
                let lane = (9..=11).contains(&y);
                let branch = (14..=16).contains(&x) && y >= 9;
                g[(y * n + x) as usize] = (lane || branch) as i32;
            }
        }
        (g, n)
    }

    #[test]
    fn a_lane_that_fits_lengthwise_only_is_driven_along_and_refused_across() {
        let (g, n) = lane_and_branch();
        let body = long_body();
        // along the lane: straight east, no turn
        let path = search(&g, n, (3, 10), (26, 10), &body, 0.0).expect("along the lane");
        assert!(path.iter().all(|c| c.1 == 10), "{path:?}");
        // across it, down the branch: the 6-cell body cannot turn south in a
        // 3-cell lane, nor drive the branch from it
        assert_eq!(search(&g, n, (3, 10), (15, 25), &body, 0.0), None);
        assert_eq!(search(&g, n, (3, 10), (15, 25), &body, 0.5), None);
    }

    #[test]
    fn a_wide_body_is_refused_a_lane_only_as_long_as_it() {
        // 3 cells nose to tail, 5 across (the mule's proportions); a 3-wide
        // north-south lane and a 5-tall east-west one, crossing
        let n = 30;
        let mut g = vec![0; (n * n) as usize];
        for y in 0..n {
            for x in 0..n {
                g[(y * n + x) as usize] = ((14..=16).contains(&x) || (8..=12).contains(&y)) as i32;
            }
        }
        let body = BodyRect::centred(2.8, 4.8);
        assert!(search(&g, n, (2, 10), (27, 10), &body, 0.0).is_some(), "along the 5-tall lane");
        assert_eq!(search(&g, n, (15, 10), (15, 27), &body, 0.0), None, "down the 3-wide one");
        // the axis box (the body facing East in every heading) drives it
        let axis: Vec<GridCell> =
            (-2..=2).flat_map(|dy| (-1..=1).map(move |dx| (dx, dy))).collect();
        let heights = vec![0.0; ((n + 1) * (n + 1)) as usize];
        let boxed = find_path_for_footprint_with_jumps(
            &g,
            &g,
            &heights,
            n,
            n,
            (15, 10),
            (15, 27),
            &VehicleFootprint::Cells(axis),
            0.0,
            1.0,
            0.0,
        );
        assert!(boxed.is_some(), "the axis box takes the 3-wide lane");
    }

    /// A deterministic scatter of blocked cells (about one in `1/p`) over an
    /// `n²` grid, with a clear border-free interior.
    fn scatter(n: i32, seed: u32, p: f32) -> Vec<i32> {
        let mut s = seed;
        (0..n * n)
            .map(|_| {
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((s >> 8) as f32 / (1u32 << 24) as f32 >= p) as i32
            })
            .collect()
    }

    #[test]
    fn no_path_step_puts_the_turned_body_on_a_blocked_cell() {
        let n = 48;
        let mut checked = 0;
        for seed in 0..6 {
            // rubble to thread round: sparse blocks, each grown to 2x2
            let dots = scatter(n, seed, 0.012);
            let mut g = vec![1; (n * n) as usize];
            for y in 0..n {
                for x in 0..n {
                    if dots[(y * n + x) as usize] == 0 {
                        for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                            if x + dx < n && y + dy < n {
                                g[((y + dy) * n + x + dx) as usize] = 0;
                            }
                        }
                    }
                }
            }
            let free =
                |x: i32, y: i32| x >= 0 && y >= 0 && x < n && y < n && g[(y * n + x) as usize] == 1;
            for body in [mule_body(), long_body()] {
                for (from, to) in [
                    ((5, 5), (42, 42)),
                    ((42, 5), (5, 42)),
                    ((5, 24), (42, 24)),
                    ((24, 42), (24, 5)),
                ] {
                    let Some(path) = search(&g, n, from, to, &body, 0.3) else { continue };
                    checked += 1;
                    for (i, w) in path.windows(2).enumerate() {
                        let (a, b) = (w[0], w[1]);
                        let d = DIRS.iter().position(|&o| o == (b.0 - a.0, b.1 - a.1)).unwrap();
                        let h = heading16(2 * d);
                        // the start is relaxed (the vehicle is already there)
                        if i > 0 {
                            body_cells(&body, a, h, |x, y| {
                                assert!(free(x, y), "{a:?}->{b:?} at {a:?}")
                            });
                        }
                        body_cells(&body, b, h, |x, y| {
                            assert!(free(x, y), "{a:?}->{b:?} at {b:?}")
                        });
                    }
                }
            }
        }
        assert!(checked >= 20, "only {checked} routes found");
    }

    #[test]
    fn a_pivot_clears_every_heading_it_turns_through() {
        // the long body turning East -> South at (10, 10) sweeps the SE
        // diagonal; a block at (12, 12) is clear of both end headings
        let n = 21;
        let mut g = vec![1; (n * n) as usize];
        g[(12 * n + 12) as usize] = 0;
        let blocked = Blocked::new(&g, n, n);
        let masks = Masks::new(&long_body());
        assert!(blocked.clear(10, 10, &masks.body16[0]));
        assert!(blocked.clear(10, 10, &masks.body16[4]));
        assert!(!pivot_clear(&blocked, &masks, 10, 10, 0, 2), "E -> S sweeps the block");
        assert!(pivot_clear(&blocked, &masks, 10, 10, 0, 6), "E -> N turns away from it");
        // a reversal sweeps a half turn either way, so the tail crosses the
        // SE diagonal whichever way the nose goes
        assert!(!pivot_clear(&blocked, &masks, 10, 10, 0, 4));
        g[(12 * n + 12) as usize] = 1;
        let blocked = Blocked::new(&g, n, n);
        assert!(pivot_clear(&blocked, &masks, 10, 10, 0, 4));
    }

    #[test]
    fn a_route_check_follows_the_search_rule() {
        let n = 20;
        let mut g = vec![1; (n * n) as usize];
        let body = long_body();
        let path = search(&g, n, (4, 10), (15, 10), &body, 0.0).expect("open");
        assert!(route_clear_oriented(&g, n, n, &path, &body));
        // a block beside the route's far end, inside the turned body
        g[(11 * n + 15) as usize] = 0;
        assert!(!route_clear_oriented(&g, n, n, &path, &body));
        // a block behind the start (the vehicle is already there) is fine
        g[(11 * n + 15) as usize] = 1;
        g[(10 * n + 1) as usize] = 0;
        assert!(route_clear_oriented(&g, n, n, &path, &body));
        // a jump of two cells is no route
        assert!(!route_clear_oriented(&g, n, n, &[(4, 10), (6, 10)], &body));
    }

    #[test]
    fn the_start_cell_is_relaxed() {
        // the long body starts grazing a block behind it, and drives away
        let n = 20;
        let mut g = vec![1; (n * n) as usize];
        g[(10 * n + 6) as usize] = 0;
        let path = search(&g, n, (9, 10), (15, 10), &long_body(), 0.0).expect("drives out");
        assert_eq!(path[0], (9, 10));
    }

    #[test]
    fn an_unreachable_goal_finds_nothing() {
        let n = 24;
        let mut g = vec![1; (n * n) as usize];
        for i in 0..n {
            g[(i * n + 12) as usize] = 0; // a wall down x = 12
        }
        assert_eq!(search(&g, n, (4, 12), (20, 12), &mule_body(), 0.4), None);
        // and a cell-footprint search agrees on the open half
        let e = erode_for_footprint(&g, n, n, &[(0, 0)]);
        assert!(find_path(&e, n, n, (4, 12), (20, 12)).is_none());
    }

    /// Timings for the mule on lunar-sized grids (run with `--release
    /// --ignored --nocapture`), next to the axis-box search it replaces: a
    /// corner-to-corner route over scattered rubble, and a walled-off goal
    /// that exhausts the search.  Jumps on (the mule's 0.6 m safe fall), so
    /// a blocked step also checks the structural grid.
    #[test]
    #[ignore = "perf check: prints timings"]
    fn perf_on_lunar_sized_grids() {
        use std::time::{Duration, Instant};
        let axis: Vec<GridCell> =
            (-3..=3).flat_map(|dy| (-2..=2).map(move |dx| (dx, dy))).collect();
        let oriented = VehicleFootprint::Oriented { body: mule_body(), pivot: false };
        let boxed = VehicleFootprint::Cells(axis);
        for n in [256, 512] {
            let heights = vec![0.0; ((n + 1) * (n + 1)) as usize];
            let g = scatter(n, 7, 0.004);
            let mut walled = g.clone();
            for i in 0..n {
                walled[(i * n + n / 2) as usize] = 0;
            }
            let run = |grid: &[i32], fp: &VehicleFootprint, turn: f32| {
                let t = Instant::now();
                let p = find_path_for_footprint_with_jumps(
                    grid,
                    grid,
                    &heights,
                    n,
                    n,
                    (8, 8),
                    (n - 9, n - 9),
                    fp,
                    0.6,
                    1.0,
                    turn,
                );
                (p.is_some(), t.elapsed())
            };
            let ms = |d: Duration| d.as_secs_f64() * 1e3;
            for turn in [0.0, 0.4] {
                let (found, route) = run(&g, &oriented, turn);
                let (none, exhaust) = run(&walled, &oriented, turn);
                let (_, box_route) = run(&g, &boxed, turn);
                let (_, box_exhaust) = run(&walled, &boxed, turn);
                assert!(found && !none);
                std::println!(
                    "{n}² turn_cost {turn}: oriented route {:.1} ms, unreachable {:.1} ms \
                     | axis box route {:.1} ms, unreachable {:.1} ms",
                    ms(route),
                    ms(exhaust),
                    ms(box_route),
                    ms(box_exhaust),
                );
            }
        }
    }
}
