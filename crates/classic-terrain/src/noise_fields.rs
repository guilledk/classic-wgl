//! Bulk noise field generators for the host guest-SDK.
//!
//! These fill `Vec<f32>` buffers with deterministic noise, so ROM guests can
//! compose terrain without shipping their own noise implementation.  Each
//! function is a pure function of `(seed, dims, params)` — no system clock —
//! so output is reproducible across targets and stable for golden traces.
//!
//! The guest-side SDK exposes these as host imports (`fbm_field`, `ridged_field`,
//! `billow_field`, `tiling_field`, `noise_field`, `noise2d`).

use crate::fractal::{domain_warp, tiling_fbm_2d, Fbm};
use crate::simplex_noise::SimplexNoise;
use alloc::vec;
use alloc::vec::Vec;

/// Fill a `w`×`h` grid with summed-octave fBm (normalised to `[-1, 1]`).
pub fn fbm_field(
    w: i32,
    h: i32,
    seed: &str,
    octaves: u32,
    freq: f64,
    lacunarity: f64,
    gain: f64,
) -> Vec<f32> {
    let noise = SimplexNoise::new(seed);
    let fbm = Fbm::new(octaves, freq, lacunarity, gain);
    fill(w, h, |x, y| fbm.sample(&noise, x, y) as f32)
}

/// Fill a `w`×`h` grid with ridged multifractal noise (`[0, 1]`), optionally
/// domain-warped (`warp_amp > 0` warps the sample point before sampling).
#[allow(clippy::too_many_arguments)]
pub fn ridged_field(
    w: i32,
    h: i32,
    seed: &str,
    octaves: u32,
    freq: f64,
    lacunarity: f64,
    gain: f64,
    warp_amp: f64,
) -> Vec<f32> {
    let noise = SimplexNoise::new(seed);
    let fbm = Fbm::new(octaves, freq, lacunarity, gain);
    fill(w, h, |x, y| {
        let (wx, wy) =
            if warp_amp > 0.0 { domain_warp(&noise, x, y, freq * 2.0, warp_amp) } else { (x, y) };
        fbm.sample_ridged(&noise, wx, wy) as f32
    })
}

/// Fill a `w`×`h` grid with billow (absolute-value) fBm (`[0, 1]`).
pub fn billow_field(
    w: i32,
    h: i32,
    seed: &str,
    octaves: u32,
    freq: f64,
    lacunarity: f64,
    gain: f64,
) -> Vec<f32> {
    let noise = SimplexNoise::new(seed);
    let fbm = Fbm::new(octaves, freq, lacunarity, gain);
    fill(w, h, |x, y| fbm.sample_billow(&noise, x, y) as f32)
}

/// Fill a `w`×`h` grid with seamlessly-tiling fBm of the given `period`.
pub fn tiling_field(
    w: i32,
    h: i32,
    seed: &str,
    period: f64,
    octaves: u32,
    radius: f64,
) -> Vec<f32> {
    let noise = SimplexNoise::new(seed);
    fill(w, h, |x, y| tiling_fbm_2d(&noise, x, y, period, octaves, radius) as f32)
}

/// Fill a `w`×`h` grid with raw (single-octave) 2D simplex, scaled per-axis.
pub fn noise_field(w: i32, h: i32, seed: &str, freq_x: f64, freq_y: f64) -> Vec<f32> {
    let noise = SimplexNoise::new(seed);
    fill(w, h, |x, y| noise.noise_2d(x * freq_x, y * freq_y) as f32)
}

/// Sample raw 2D simplex at one point.  For non-uniform sampling (e.g. crater
/// rays) where a whole field is overkill; the guest calls this per point.
pub fn noise2d(seed: &str, x: f64, y: f64) -> f64 {
    SimplexNoise::new(seed).noise_2d(x, y)
}

/// Row-major field fill over `(x, y)`, `x` varying fastest.
fn fill(w: i32, h: i32, mut f: impl FnMut(f64, f64) -> f32) -> Vec<f32> {
    let (w, h) = (w.max(0) as usize, h.max(0) as usize);
    let mut out = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            out[y * w + x] = f(x as f64, y as f64);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fbm_field_is_deterministic_and_sized() {
        let a = fbm_field(16, 16, "apollo", 4, 1.0 / 32.0, 2.0, 0.5);
        let b = fbm_field(16, 16, "apollo", 4, 1.0 / 32.0, 2.0, 0.5);
        assert_eq!(a.len(), 256);
        assert_eq!(a, b);
        assert!(a.iter().all(|v| *v >= -1.0 && *v <= 1.0));
    }

    #[test]
    fn ridged_and_billow_are_bounded() {
        for field in [
            ridged_field(16, 16, "apollo", 3, 0.03, 2.0, 0.5, 14.0),
            billow_field(16, 16, "apollo", 3, 0.03, 2.0, 0.5),
        ] {
            assert_eq!(field.len(), 256);
            assert!(field.iter().all(|v| (0.0..=1.0).contains(v)));
        }
    }

    #[test]
    fn tiling_field_is_periodic() {
        let period = 8.0;
        let (w, h) = (16, 16);
        let a = tiling_field(w, h, "x", period, 2, 0.3);
        for y in 0..h {
            let left = a[(y * w) as usize];
            let right = a[(y * w + (w - period as i32)) as usize];
            assert!((left - right).abs() < 1e-5, "seam mismatch at row {y}");
        }
    }
}
