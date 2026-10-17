//! Procedural electrical arcs and lightning bolts: seeded midpoint-displacement
//! polylines with branches, expanded on the CPU into camera-facing triangle
//! strips (core + glow halo in one quad, shaded by `ribbon.frag`).
//!
//! Geometry is regenerated a few dozen times a second (crawl/flicker) from a
//! [`SplitMix64`] keyed by the instance and the regeneration index, so a fixed
//! timestep reproduces the same bolts.

use classic_core::math::{iso_camera_matrix, DEPTH_FAR, DEPTH_NEAR};
use classic_core::tilemap::PPM_TARGET;
use glam::Vec3;

use super::spawn::SplitMix64;

/// Floats per ribbon vertex: screen `x, y` (pre-camera px), window depth,
/// across `v ∈ [-1, 1]`, core fraction (core width / glow width), intensity,
/// linear RGB.
pub use classic_gfx::RIBBON_VERTEX_FLOATS;

/// One polyline of a bolt (the trunk or a branch) with its relative width.
#[derive(Clone, Debug, PartialEq)]
pub struct Strand {
    pub points: Vec<Vec3>,
    /// Width multiplier (branches are thinner).
    pub width: f32,
}

/// Midpoint displacement between `a` and `b`: `2^depth` segments, each
/// midpoint pushed perpendicular to its segment by up to `jitter × length`.
pub fn displace(rng: &mut SplitMix64, a: Vec3, b: Vec3, depth: u32, jitter: f32) -> Vec<Vec3> {
    let mut pts = vec![a, b];
    for _ in 0..depth.min(8) {
        let mut next = Vec::with_capacity(pts.len() * 2 - 1);
        for w in pts.windows(2) {
            let (p, q) = (w[0], w[1]);
            let seg = q - p;
            let len = seg.length();
            let (t, s) = seg.try_normalize().unwrap_or(Vec3::X).any_orthonormal_pair();
            let ang = rng.f32() * std::f32::consts::TAU;
            let amt = (rng.f32() * 2.0 - 1.0) * jitter * len;
            next.push(p);
            next.push((p + q) * 0.5 + (t * ang.cos() + s * ang.sin()) * amt);
        }
        next.push(*pts.last().expect("non-empty"));
        pts = next;
    }
    pts
}

/// A bolt from `a` to `b`: the displaced trunk plus `branches` forks, each
/// leaving a random trunk point toward a point beyond it.
pub fn bolt(
    rng: &mut SplitMix64,
    a: Vec3,
    b: Vec3,
    depth: u32,
    jitter: f32,
    branches: u32,
) -> Vec<Strand> {
    let trunk = displace(rng, a, b, depth, jitter);
    let mut strands = Vec::with_capacity(1 + branches as usize);
    let span = (b - a).length();
    for _ in 0..branches {
        if trunk.len() < 4 {
            break;
        }
        let i = 1 + (rng.f32() * (trunk.len() - 3) as f32) as usize;
        let from = trunk[i];
        let along = (trunk[i + 1] - trunk[i - 1]).try_normalize().unwrap_or(Vec3::X);
        let (t, s) = along.any_orthonormal_pair();
        let ang = rng.f32() * std::f32::consts::TAU;
        let side = (t * ang.cos() + s * ang.sin()) * 0.8;
        let len = span * (0.15 + 0.25 * rng.f32());
        let to = from + (along + side).normalize() * len;
        strands.push(Strand {
            points: displace(rng, from, to, depth.saturating_sub(2), jitter),
            width: 0.55,
        });
    }
    strands.insert(0, Strand { points: trunk, width: 1.0 });
    strands
}

/// Project a world point to (pre-camera screen px, window depth), matching
/// `mesh.vert`/`particle.vert`.
fn project(p: Vec3) -> (f32, f32, f32) {
    let view = iso_camera_matrix() * p.extend(1.0);
    let depth = (DEPTH_NEAR - view.z) / (DEPTH_NEAR - DEPTH_FAR);
    (view.x * PPM_TARGET, -view.y * PPM_TARGET, depth)
}

/// Expand strands into a triangle list (`RIBBON_VERTEX_FLOATS` per vertex):
/// each strand becomes a joined strip of half-width `glow / 2` metres whose
/// core is `width / glow` of it.  `depth_bias` (metres) pulls the bolt toward
/// the camera so a ground strike is not swallowed by the terrain it hits.
pub fn build_vertices(
    strands: &[Strand],
    width: f32,
    glow: f32,
    intensity: f32,
    color: [f32; 3],
    depth_bias: f32,
    out: &mut Vec<f32>,
) {
    let glow = glow.max(width).max(1e-4);
    let core = (width / glow).clamp(0.02, 1.0);
    let bias = depth_bias / (DEPTH_NEAR - DEPTH_FAR);
    for strand in strands {
        let pts: Vec<(f32, f32, f32)> = strand.points.iter().map(|p| project(*p)).collect();
        if pts.len() < 2 {
            continue;
        }
        let half = 0.5 * glow * strand.width * PPM_TARGET;
        // Per-point perpendicular: the average of the adjacent segments'.
        let seg_perp = |i: usize| {
            let (x0, y0, _) = pts[i];
            let (x1, y1, _) = pts[i + 1];
            let (dx, dy) = (x1 - x0, y1 - y0);
            let l = (dx * dx + dy * dy).sqrt().max(1e-4);
            (-dy / l, dx / l)
        };
        let n = pts.len();
        let perps: Vec<(f32, f32)> = (0..n)
            .map(|i| {
                let a = if i > 0 { seg_perp(i - 1) } else { seg_perp(0) };
                let b = if i + 1 < n { seg_perp(i) } else { seg_perp(n - 2) };
                let (x, y) = (a.0 + b.0, a.1 + b.1);
                let l = (x * x + y * y).sqrt();
                if l < 1e-3 {
                    a
                } else {
                    (x / l, y / l)
                }
            })
            .collect();
        let vert = |out: &mut Vec<f32>, i: usize, side: f32| {
            let (x, y, d) = pts[i];
            let (px, py) = perps[i];
            // Taper the strand toward its far end.
            let taper = 1.0 - 0.5 * (i as f32 / (n - 1) as f32);
            out.extend_from_slice(&[
                x + px * half * side,
                y + py * half * side,
                d - bias,
                side,
                core,
                intensity * taper,
                color[0],
                color[1],
                color[2],
            ]);
        };
        for i in 0..n - 1 {
            vert(out, i, -1.0);
            vert(out, i, 1.0);
            vert(out, i + 1, -1.0);
            vert(out, i, 1.0);
            vert(out, i + 1, 1.0);
            vert(out, i + 1, -1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displacement_keeps_endpoints_and_doubles_segments() {
        let mut r = SplitMix64::new(5);
        let a = Vec3::new(0.0, 0.0, 0.0);
        let b = Vec3::new(4.0, 0.0, 2.0);
        let pts = displace(&mut r, a, b, 4, 0.3);
        assert_eq!(pts.len(), 17);
        assert_eq!(pts[0], a);
        assert_eq!(pts[16], b);
        // Bounded wander: no point strays further than the jitter allows.
        let span = (b - a).length();
        for p in &pts {
            let along = (*p - a).dot((b - a) / span);
            let off = ((*p - a) - (b - a) / span * along).length();
            assert!(off < span * 0.6, "point strayed {off}");
        }
    }

    #[test]
    fn bolts_are_reproducible_and_branch() {
        let a = Vec3::ZERO;
        let b = Vec3::new(0.0, 0.0, -10.0);
        let x = bolt(&mut SplitMix64::new(11), a, b, 5, 0.3, 3);
        let y = bolt(&mut SplitMix64::new(11), a, b, 5, 0.3, 3);
        assert_eq!(x, y);
        assert_eq!(x.len(), 4);
        assert!(x[1..].iter().all(|s| s.width < 1.0));
    }

    #[test]
    fn strips_emit_two_triangles_per_segment() {
        let strands = vec![Strand { points: vec![Vec3::ZERO, Vec3::X, Vec3::X * 2.0], width: 1.0 }];
        let mut out = Vec::new();
        build_vertices(&strands, 0.1, 0.5, 2.0, [1.0; 3], 0.0, &mut out);
        assert_eq!(out.len(), 2 * 6 * RIBBON_VERTEX_FLOATS);
        // Core fraction = width / glow; `v` spans both sides.
        assert!((out[4] - 0.2).abs() < 1e-6);
        assert_eq!(out[3], -1.0);
        assert_eq!(out[RIBBON_VERTEX_FLOATS + 3], 1.0);
    }
}
