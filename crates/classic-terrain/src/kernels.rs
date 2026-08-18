//! Domain-neutral grid kernels for procedural map generation.
//!
//! These are the reusable "heavy loop" primitives a ROM guest composes into a
//! map algorithm, host-side at native speed.  Every kernel is a pure function
//! of its input grids — `#![no_std]`, GL-free, deterministic — so output is
//! reproducible across targets.  They carry **no** geological or game-specific
//! vocabulary: a crater is just a `stamp_radial` with guest-supplied profile
//! parameters, a mare mask is `smoothstep` over an `fbm_field`.
//!
//! Grids are row-major (`x` varying fastest), `f32` for continuous fields and
//! `u32` for categorical/boolean grids.

use alloc::vec;
use alloc::vec::Vec;

/// Elementwise combine op for [`map_field`], [`map_scalar`] and
/// [`stamp_radial`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldOp {
    Add,
    Sub,
    Mul,
    Min,
    Max,
}

/// A reduction statistic for [`reduce_field`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reduce {
    Min,
    Max,
    Mean,
    Variance,
}

// ---------------------------------------------------------------------------
// elementwise ops
// ---------------------------------------------------------------------------

/// In-place `dst = dst op src`, elementwise over two equal-length fields.
///
/// Panics if `dst.len() != src.len()`.
pub fn map_field(op: FieldOp, dst: &mut [f32], src: &[f32]) {
    assert_eq!(dst.len(), src.len(), "map_field: field length mismatch");
    for (d, &s) in dst.iter_mut().zip(src) {
        *d = combine(op, *d, s);
    }
}

/// In-place `dst = dst op scalar`, elementwise.
pub fn map_scalar(op: FieldOp, dst: &mut [f32], scalar: f32) {
    for d in dst.iter_mut() {
        *d = combine(op, *d, scalar);
    }
}

/// In-place clamp to `[lo, hi]`.
pub fn clamp(dst: &mut [f32], lo: f32, hi: f32) {
    for d in dst.iter_mut() {
        *d = d.clamp(lo, hi);
    }
}

/// In-place smoothstep between `edge0` and `edge1` (ramp `0 → 1`).
pub fn smoothstep(dst: &mut [f32], edge0: f32, edge1: f32) {
    for d in dst.iter_mut() {
        *d = smoothstep_f32(edge0, edge1, *d);
    }
}

#[inline]
fn combine(op: FieldOp, a: f32, b: f32) -> f32 {
    match op {
        FieldOp::Add => a + b,
        FieldOp::Sub => a - b,
        FieldOp::Mul => a * b,
        FieldOp::Min => a.min(b),
        FieldOp::Max => a.max(b),
    }
}

#[inline]
fn smoothstep_f32(edge0: f32, edge1: f32, x: f32) -> f32 {
    if (edge1 - edge0).abs() < f32::EPSILON {
        return if x < edge0 { 0.0 } else { 1.0 };
    }
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// ---------------------------------------------------------------------------
// blur / gradient
// ---------------------------------------------------------------------------

/// N×N box blur over a `w`×`h` field, edge-clamped.  Returns a new field.
pub fn blur_box(src: &[f32], w: i32, h: i32, radius: i32) -> Vec<f32> {
    let (w, h) = (w.max(0) as usize, h.max(0) as usize);
    let radius = radius.max(0);
    let mut out = vec![0f32; src.len()];
    for ty in 0..h as i32 {
        for tx in 0..w as i32 {
            let mut sum = 0f32;
            let mut n = 0f32;
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    let nx = (tx + dx).clamp(0, w as i32 - 1);
                    let ny = (ty + dy).clamp(0, h as i32 - 1);
                    sum += src[ny as usize * w + nx as usize];
                    n += 1.0;
                }
            }
            out[ty as usize * w + tx as usize] = sum / n;
        }
    }
    out
}

/// Per-tile gradient magnitude from a vertex height grid of `(w+1)×(h+1)`.
/// Returns a `w`×`h` tile grid of per-tile slope (height units per tile).
pub fn gradient_magnitude(heights: &[f32], w: i32, h: i32) -> Vec<f32> {
    let (w, h) = (w.max(0), h.max(0));
    let vw = w as usize + 1;
    let mut out = vec![0f32; (w * h) as usize];
    for ty in 0..h {
        for tx in 0..w {
            let nw = heights[ty as usize * vw + tx as usize];
            let ne = heights[ty as usize * vw + tx as usize + 1];
            let sw = heights[(ty + 1) as usize * vw + tx as usize];
            let se = heights[(ty + 1) as usize * vw + tx as usize + 1];
            let dzdx = ((ne + se) - (nw + sw)) * 0.5;
            let dzdy = ((sw + se) - (nw + ne)) * 0.5;
            out[ty as usize * w as usize + tx as usize] = libm::sqrtf(dzdx * dzdx + dzdy * dzdy);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// relaxation / slope measurement
// ---------------------------------------------------------------------------

/// Worst adjacent-vertex height difference over a `w`×`h` vertex grid
/// (right and down neighbours only).
pub fn max_adjacent_slope(heights: &[f32], w: usize, h: usize) -> f32 {
    let mut worst = 0f32;
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if x + 1 < w {
                let d = (heights[i] - heights[i + 1]).abs();
                if d > worst {
                    worst = d;
                }
            }
            if y + 1 < h {
                let d = (heights[i] - heights[i + w]).abs();
                if d > worst {
                    worst = d;
                }
            }
        }
    }
    worst
}

/// Jacobi thermal erosion / talus relaxation: repeatedly move material from any
/// vertex that overhangs a 4-neighbour by more than `max_slope`.  Vertices in
/// `pinned` are held fixed.  Returns `(iterations_used, worst_remaining)`.
pub fn relax_slopes(
    heights: &mut [f32],
    w: usize,
    h: usize,
    max_slope: f32,
    max_iterations: u32,
    tolerance: f32,
    pinned: Option<&[bool]>,
) -> (u32, f32) {
    if max_slope <= 0.0 || max_iterations == 0 {
        return (0, max_adjacent_slope(heights, w, h));
    }
    const RATE: f32 = 0.18;
    let mut scratch = heights.to_vec();
    let mut used = 0;

    for _ in 0..max_iterations {
        scratch.copy_from_slice(heights);
        let mut worst = 0f32;
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                if pinned.is_some_and(|m| m[i]) {
                    continue;
                }
                let hi = scratch[i];
                let mut delta = 0f32;
                let mut neigh = [usize::MAX; 4];
                if x > 0 {
                    neigh[0] = i - 1;
                }
                if x + 1 < w {
                    neigh[1] = i + 1;
                }
                if y > 0 {
                    neigh[2] = i - w;
                }
                if y + 1 < h {
                    neigh[3] = i + w;
                }
                for j in neigh {
                    if j == usize::MAX {
                        continue;
                    }
                    let d = hi - scratch[j];
                    if d > max_slope {
                        let excess = d - max_slope;
                        if excess > worst {
                            worst = excess;
                        }
                        delta -= RATE * excess;
                    } else if d < -max_slope {
                        let excess = -d - max_slope;
                        if excess > worst {
                            worst = excess;
                        }
                        delta += RATE * excess;
                    }
                }
                heights[i] = hi + delta;
            }
        }
        used += 1;
        if worst <= tolerance {
            break;
        }
    }

    (used, max_adjacent_slope(heights, w, h))
}

// ---------------------------------------------------------------------------
// classification / connectivity
// ---------------------------------------------------------------------------

/// Threshold a field into a `u32` grid: `1` where `field[i] <= threshold`,
/// else `0`.  Returns a new grid.
pub fn threshold_le(field: &[f32], threshold: f32) -> Vec<u32> {
    field.iter().map(|&v| if v <= threshold { 1 } else { 0 }).collect()
}

/// Classify a field into band ids by inclusive scalar ranges.  Cell `i` gets
/// `band_index + 1` for the first band `(lo, hi)` containing `field[i]`, else
/// `0`.  Returns a new grid.
pub fn classify_bands(field: &[f32], bands: &[(f32, f32)]) -> Vec<u32> {
    field
        .iter()
        .map(|&v| {
            bands
                .iter()
                .position(|&(lo, hi)| v >= lo && v <= hi)
                .map(|i| (i + 1) as u32)
                .unwrap_or(0)
        })
        .collect()
}

/// Label 8-connected walkable components of a `w`×`h` `u32` grid (`1` =
/// walkable).  Returns `(labels, largest_label, largest_size)`; blocked cells
/// are labelled `0`.
pub fn connected_components(nav: &[u32], w: i32, h: i32) -> (Vec<u32>, u32, usize) {
    let (tw, th) = (w.max(0) as usize, h.max(0) as usize);
    let mut labels = vec![0u32; tw * th];
    let mut next = 0u32;
    let mut best = 0u32;
    let mut best_size = 0usize;
    let mut stack: Vec<usize> = Vec::new();

    for start in 0..tw * th {
        if nav[start] != 1 || labels[start] != 0 {
            continue;
        }
        next += 1;
        let mut size = 0usize;
        stack.push(start);
        labels[start] = next;
        while let Some(i) = stack.pop() {
            size += 1;
            let x = (i % tw) as i32;
            let y = (i / tw) as i32;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    let nx = x + dx;
                    let ny = y + dy;
                    if nx < 0 || ny < 0 || nx >= w || ny >= h {
                        continue;
                    }
                    let j = ny as usize * tw + nx as usize;
                    if nav[j] == 1 && labels[j] == 0 {
                        labels[j] = next;
                        stack.push(j);
                    }
                }
            }
        }
        if size > best_size {
            best_size = size;
            best = next;
        }
    }

    (labels, best, best_size)
}

/// Blank every walkable cell not in the largest connected component.
pub fn prune_to_main_component(nav: &mut [u32], w: i32, h: i32) {
    let (labels, main, _) = connected_components(nav, w, h);
    if main == 0 {
        return;
    }
    for (i, v) in nav.iter_mut().enumerate() {
        if *v == 1 && labels[i] != main {
            *v = 0;
        }
    }
}

/// Flatten one corridor cell (and its 8-neighbourhood) in a height field until
/// the per-tile slope drops below `max_slope * 0.7`, then mark it walkable.
/// Used to breach isolated terrain into the main component.
#[allow(clippy::too_many_arguments)]
pub fn carve_corridor_cell(
    nav: &mut [u32],
    heights: &mut [f32],
    w: i32,
    h: i32,
    vw: usize,
    cx: i32,
    cy: i32,
    max_slope: f32,
) {
    let tw = w as usize;
    for dy in -1..=1 {
        for dx in -1..=1 {
            let tx = cx + dx;
            let ty = cy + dy;
            if tx < 0 || ty < 0 || tx >= w || ty >= h {
                continue;
            }
            for _ in 0..6 {
                let i00 = ty as usize * vw + tx as usize;
                let i10 = i00 + 1;
                let i01 = i00 + vw;
                let i11 = i01 + 1;
                let mean = (heights[i00] + heights[i10] + heights[i01] + heights[i11]) * 0.25;
                let dzdx = ((heights[i10] + heights[i11]) - (heights[i00] + heights[i01])) * 0.5;
                let dzdy = ((heights[i01] + heights[i11]) - (heights[i00] + heights[i10])) * 0.5;
                if libm::sqrtf(dzdx * dzdx + dzdy * dzdy) <= max_slope * 0.7 {
                    break;
                }
                for j in [i00, i10, i01, i11] {
                    heights[j] += (mean - heights[j]) * 0.5;
                }
            }
            nav[ty as usize * tw + tx as usize] = 1;
        }
    }
}

// ---------------------------------------------------------------------------
// reduction / stamps
// ---------------------------------------------------------------------------

/// Reduce a field to a single statistic.
pub fn reduce_field(field: &[f32], op: Reduce) -> f32 {
    match op {
        Reduce::Min => field.iter().copied().fold(f32::INFINITY, f32::min),
        Reduce::Max => field.iter().copied().fold(f32::NEG_INFINITY, f32::max),
        Reduce::Mean => {
            if field.is_empty() {
                return 0.0;
            }
            field.iter().sum::<f32>() / field.len() as f32
        }
        Reduce::Variance => {
            if field.is_empty() {
                return 0.0;
            }
            let mean = field.iter().sum::<f32>() / field.len() as f32;
            field.iter().map(|&v| (v - mean) * (v - mean)).sum::<f32>() / field.len() as f32
        }
    }
}

/// Stamp a smooth radial profile into a `w`×`h` field, combined with `op`.
///
/// The profile is a parabolic falloff `amplitude * (1 - d²)` for `d = dist /
/// radius < 1`, zero beyond the radius.  This is the domain-neutral building
/// block a guest maps its craters/domes onto; the lunar guest's crater pass
/// composes several of these (bowl + rim + ejecta) with lunar-specific nesting
/// and deposit semantics it owns.
#[allow(clippy::too_many_arguments)]
pub fn stamp_radial(
    field: &mut [f32],
    w: i32,
    h: i32,
    cx: f32,
    cy: f32,
    radius: f32,
    amplitude: f32,
    op: FieldOp,
) {
    let (w, h) = (w.max(0), h.max(0));
    if radius <= 0.0 {
        return;
    }
    let x0 = (libm::floorf(cx - radius) as i32).max(0);
    let x1 = (libm::ceilf(cx + radius) as i32).min(w - 1);
    let y0 = (libm::floorf(cy - radius) as i32).max(0);
    let y1 = (libm::ceilf(cy + radius) as i32).min(h - 1);
    for ty in y0..=y1 {
        for tx in x0..=x1 {
            let dx = tx as f32 + 0.5 - cx;
            let dy = ty as f32 + 0.5 - cy;
            let d = libm::sqrtf(dx * dx + dy * dy) / radius;
            if d >= 1.0 {
                continue;
            }
            let profile = amplitude * (1.0 - d * d);
            let i = ty as usize * w as usize + tx as usize;
            field[i] = combine(op, field[i], profile);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_field_and_scalar_apply_elementwise() {
        let mut a = vec![1.0, 2.0, 3.0];
        map_field(FieldOp::Add, &mut a, &[10.0, 20.0, 30.0]);
        assert_eq!(a, vec![11.0, 22.0, 33.0]);
        map_scalar(FieldOp::Mul, &mut a, 2.0);
        assert_eq!(a, vec![22.0, 44.0, 66.0]);
        map_scalar(FieldOp::Min, &mut a, 50.0);
        assert_eq!(a, vec![22.0, 44.0, 50.0]);
    }

    #[test]
    fn clamp_and_smoothstep() {
        let mut a = vec![-1.0, 0.5, 3.0];
        clamp(&mut a, 0.0, 1.0);
        assert_eq!(a, vec![0.0, 0.5, 1.0]);
        let mut b = vec![0.0, 0.25, 0.5, 1.0];
        smoothstep(&mut b, 0.0, 1.0);
        assert!((b[1] - 0.15625).abs() < 1e-6, "smoothstep(0.25) = {}", b[1]);
        assert_eq!(b[3], 1.0);
    }

    #[test]
    fn blur_box_averages() {
        let field = vec![1.0f32; 9];
        let blurred = blur_box(&field, 3, 3, 1);
        assert_eq!(blurred, vec![1.0; 9]);
        let field = vec![0.0, 0.0, 0.0, 0.0, 9.0, 0.0, 0.0, 0.0, 0.0];
        let blurred = blur_box(&field, 3, 3, 1);
        assert_eq!(blurred[4], 1.0, "centre of a 3x3 box blur of a spike = spike/9");
    }

    #[test]
    fn gradient_magnitude_of_flat_field_is_zero() {
        let heights = vec![1.0f32; (4 + 1) * (4 + 1)];
        let grad = gradient_magnitude(&heights, 4, 4);
        assert_eq!(grad.len(), 16);
        assert!(grad.iter().all(|&g| g < 1e-6));
    }

    #[test]
    fn gradient_magnitude_detects_a_ramp() {
        let (w, h) = (4usize, 4usize);
        let vw = w + 1;
        let mut heights = vec![0.0f32; vw * vw];
        for y in 0..vw {
            for x in 0..vw {
                heights[y * vw + x] = x as f32;
            }
        }
        let grad = gradient_magnitude(&heights, w as i32, h as i32);
        assert!(grad.iter().all(|&g| (g - 1.0).abs() < 1e-6), "ramp slope is 1.0/tile");
    }

    #[test]
    fn relax_slopes_bounds_a_spike() {
        let mut heights = vec![0.0f32; 5 * 5];
        heights[2 * 5 + 2] = 10.0;
        let (used, worst) = relax_slopes(&mut heights, 5, 5, 1.0, 100, 0.01, None);
        assert!(used > 0);
        assert!(worst < 1.05, "worst slope after relaxation: {worst}");
    }

    #[test]
    fn threshold_and_bands_classify() {
        let field = vec![0.0, 0.5, 1.0, 2.0];
        assert_eq!(threshold_le(&field, 0.6), vec![1, 1, 0, 0]);
        let bands = [(0.0, 0.4), (0.4, 1.0)];
        assert_eq!(classify_bands(&field, &bands), vec![1, 2, 2, 0]);
    }

    #[test]
    fn connected_components_find_the_main_blob() {
        // 3x3 with a wall down the middle column, but a bridge at (1,1).
        let nav = vec![1, 0, 1, 1, 1, 1, 1, 0, 1];
        let (labels, main, size) = connected_components(&nav, 3, 3);
        assert_eq!(main, 1);
        assert_eq!(size, 7, "all walkable cells belong to one component");
        assert_eq!(labels[1], 0);
        assert_eq!(labels[7], 0);
    }

    #[test]
    fn prune_removes_small_components() {
        // Two singleton components on a 1-row grid, separated by a blocked cell.
        let mut nav = vec![1, 0, 1];
        prune_to_main_component(&mut nav, 3, 1);
        let walkable: Vec<u32> = nav.iter().copied().filter(|&v| v == 1).collect();
        assert_eq!(walkable.len(), 1);
    }

    #[test]
    fn reduce_field_stats() {
        let field = vec![1.0, 2.0, 3.0, 4.0];
        assert_eq!(reduce_field(&field, Reduce::Min), 1.0);
        assert_eq!(reduce_field(&field, Reduce::Max), 4.0);
        assert_eq!(reduce_field(&field, Reduce::Mean), 2.5);
        assert!((reduce_field(&field, Reduce::Variance) - 1.25).abs() < 1e-6);
    }

    #[test]
    fn stamp_radial_depresses_a_disc() {
        let mut field = vec![0.0f32; 5 * 5];
        stamp_radial(&mut field, 5, 5, 2.0, 2.0, 2.0, 4.0, FieldOp::Sub);
        assert!(field[2 * 5 + 2] < 0.0, "centre is depressed");
        assert_eq!(field[0], 0.0, "far corner untouched");
    }
}
