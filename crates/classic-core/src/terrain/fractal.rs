//! Fractal (multi-octave) combinators over [`SimplexNoise`].
//!
//! The base [`SimplexNoise`] generator produces a single octave in roughly
//! `[-1, 1]`.  Natural terrain needs several octaves summed at increasing
//! frequency and decreasing amplitude.  This module provides the standard
//! three combinators plus domain warping and a periodic (seamlessly tiling)
//! sampler.
//!
//! Nothing here allocates or uses the system clock — every function is a pure
//! function of `(noise, coords)`, which keeps generation deterministic for
//! golden traces and usable on `wasm32`.

use crate::simplex_noise::SimplexNoise;

/// Fractional Brownian motion parameters.
///
/// `octaves` samples are summed; each successive octave multiplies the
/// frequency by `lacunarity` and the amplitude by `gain`.  The result is
/// divided by the total amplitude so the output range matches a single
/// octave regardless of octave count.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fbm {
    pub octaves: u32,
    /// Base frequency in cycles per input unit (for a tile grid: 1/wavelength
    /// in tiles).
    pub frequency: f64,
    /// Frequency multiplier per octave (2.0 = one octave up).
    pub lacunarity: f64,
    /// Amplitude multiplier per octave (0.5 = pink-ish falloff).
    pub gain: f64,
}

impl Default for Fbm {
    fn default() -> Self {
        Self { octaves: 4, frequency: 1.0 / 32.0, lacunarity: 2.0, gain: 0.5 }
    }
}

impl Fbm {
    pub fn new(octaves: u32, frequency: f64, lacunarity: f64, gain: f64) -> Self {
        Self { octaves, frequency, lacunarity, gain }
    }

    /// Convenience constructor using the conventional `lacunarity = 2.0`,
    /// `gain = 0.5`.
    pub fn standard(octaves: u32, frequency: f64) -> Self {
        Self { octaves, frequency, lacunarity: 2.0, gain: 0.5 }
    }

    /// Classic summed-octave fBm.  Output is normalised to approximately
    /// `[-1, 1]` (same range as a single octave).
    pub fn sample(&self, n: &SimplexNoise, x: f64, y: f64) -> f64 {
        let mut freq = self.frequency;
        let mut amp = 1.0;
        let mut total = 0.0;
        let mut norm = 0.0;
        for _ in 0..self.octaves.max(1) {
            total += n.noise_2d(x * freq, y * freq) * amp;
            norm += amp;
            freq *= self.lacunarity;
            amp *= self.gain;
        }
        if norm > 0.0 {
            total / norm
        } else {
            0.0
        }
    }

    /// Ridged multifractal: `1 - |noise|` per octave, squared to sharpen the
    /// creases.  Output is approximately `[0, 1]` with ridges at 1.
    ///
    /// Used for wrinkle ridges and rilles.
    pub fn sample_ridged(&self, n: &SimplexNoise, x: f64, y: f64) -> f64 {
        let mut freq = self.frequency;
        let mut amp = 1.0;
        let mut total = 0.0;
        let mut norm = 0.0;
        for _ in 0..self.octaves.max(1) {
            let v = 1.0 - n.noise_2d(x * freq, y * freq).abs();
            total += v * v * amp;
            norm += amp;
            freq *= self.lacunarity;
            amp *= self.gain;
        }
        if norm > 0.0 {
            (total / norm).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// Billow (absolute-value) fBm — puffy, cratered-looking lumps.
    /// Output is approximately `[0, 1]`.
    pub fn sample_billow(&self, n: &SimplexNoise, x: f64, y: f64) -> f64 {
        let mut freq = self.frequency;
        let mut amp = 1.0;
        let mut total = 0.0;
        let mut norm = 0.0;
        for _ in 0..self.octaves.max(1) {
            total += n.noise_2d(x * freq, y * freq).abs() * amp;
            norm += amp;
            freq *= self.lacunarity;
            amp *= self.gain;
        }
        if norm > 0.0 {
            (total / norm).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

/// Offset the sample point by a noise field before sampling.  Breaks up the
/// visible axis alignment of raw simplex and makes ridges meander.
///
/// Returns the warped coordinates; feed them back into any sampler.
pub fn domain_warp(n: &SimplexNoise, x: f64, y: f64, freq: f64, amp: f64) -> (f64, f64) {
    // Two decorrelated lookups: offsetting the second by a large constant is
    // cheaper than building a second permutation table and is decorrelated
    // enough at the frequencies we use.
    let dx = n.noise_2d(x * freq, y * freq);
    let dy = n.noise_2d(x * freq + 137.31, y * freq - 91.77);
    (x + dx * amp, y + dy * amp)
}

/// Smooth Hermite interpolation between `edge0` and `edge1`, matching GLSL
/// `smoothstep`.  Returns 0 below `edge0`, 1 above `edge1`.
pub fn smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    if (edge1 - edge0).abs() < f64::EPSILON {
        return if x < edge0 { 0.0 } else { 1.0 };
    }
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Periodic 2D noise: seamless in both axes with period `period`.
///
/// Maps `(u, v)` onto the surface of a torus embedded in 4D and samples
/// [`SimplexNoise::noise_4d`] there.  Because the mapping is continuous and
/// periodic, `f(0, v) == f(period, v)` exactly.
///
/// This is what makes each cell of the generated tileset tile without seams —
/// the tilemap fragment shader samples cells with `fract()`, so a non-periodic
/// texture would show a hard edge wherever two same-id tiles meet.
///
/// `radius` controls how much of the 4D noise field the torus sweeps through;
/// larger values give more variation per period.
pub fn tiling_noise_2d(n: &SimplexNoise, u: f64, v: f64, period: f64, radius: f64) -> f64 {
    let tau = std::f64::consts::TAU;
    let p = if period.abs() < f64::EPSILON { 1.0 } else { period };
    // Wrapping the input first makes the period *bit-exact* at the seam.
    // Without it, `u = 0` and `u = period` differ by an ULP in the angle,
    // which the noise gradient amplifies into a faint but real seam.
    let au = u.rem_euclid(p) / p * tau;
    let av = v.rem_euclid(p) / p * tau;
    n.noise_4d(radius * au.cos(), radius * au.sin(), radius * av.cos(), radius * av.sin())
}

/// Multi-octave variant of [`tiling_noise_2d`].  Each octave doubles the
/// number of cycles inside the same `period`, so the sum stays periodic.
pub fn tiling_fbm_2d(
    n: &SimplexNoise,
    u: f64,
    v: f64,
    period: f64,
    octaves: u32,
    radius: f64,
) -> f64 {
    let mut amp = 1.0;
    let mut total = 0.0;
    let mut norm = 0.0;
    let mut r = radius;
    for _ in 0..octaves.max(1) {
        total += tiling_noise_2d(n, u, v, period, r) * amp;
        norm += amp;
        // Doubling the torus radius doubles the arc length traversed per
        // period, i.e. one octave up, while preserving periodicity.
        r *= 2.0;
        amp *= 0.5;
    }
    if norm > 0.0 {
        total / norm
    } else {
        0.0
    }
}
