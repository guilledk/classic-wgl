//! Simplex noise (2D, 3D, 4D) — seedable.
//!
//! Port of `src/lib/simplex-noise.ts` (Jonas Wagner / Stefan Gustavson algorithm).

/// Fast PRNG with seed support. Simple xoshiro128**-like generator.
#[derive(Clone, Debug)]
pub struct Random(u32);

impl Random {
    pub fn from_seed(seed: u32) -> Self {
        let mut s = Self(seed.wrapping_mul(0x9e3779b9));
        s.next_f64();
        s
    }

    pub fn from_entropy() -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos();
        Self::from_seed(nanos)
    }

    /// Return a `f64` in `[0, 1)`.
    pub fn next_f64(&mut self) -> f64 {
        self.0 = self.0.wrapping_mul(1103515245).wrapping_add(12345);
        (self.0 >> 16) as f64 / 32768.0
    }
}

const F2: f64 = 0.5 * (1.7320508075688772 - 1.0); // 0.5 * (sqrt(3.0) - 1.0)
const G2: f64 = (3.0 - 1.7320508075688772) / 6.0; // (3.0 - sqrt(3.0)) / 6.0
const F3: f64 = 1.0 / 3.0;
const G3: f64 = 1.0 / 6.0;
const F4: f64 = (2.23606797749979 - 1.0) / 4.0; // (sqrt(5.0) - 1.0) / 4.0
const G4: f64 = (5.0 - 2.23606797749979) / 20.0; // (5.0 - sqrt(5.0)) / 20.0

const GRAD3: [[f64; 3]; 12] = [
    [1.0, 1.0, 0.0],
    [-1.0, 1.0, 0.0],
    [1.0, -1.0, 0.0],
    [-1.0, -1.0, 0.0],
    [1.0, 0.0, 1.0],
    [-1.0, 0.0, 1.0],
    [1.0, 0.0, -1.0],
    [-1.0, 0.0, -1.0],
    [0.0, 1.0, 1.0],
    [0.0, -1.0, 1.0],
    [0.0, 1.0, -1.0],
    [0.0, -1.0, -1.0],
];

const GRAD4: [[f64; 4]; 32] = [
    [0.0, 1.0, 1.0, 1.0],
    [0.0, 1.0, 1.0, -1.0],
    [0.0, 1.0, -1.0, 1.0],
    [0.0, 1.0, -1.0, -1.0],
    [0.0, -1.0, 1.0, 1.0],
    [0.0, -1.0, 1.0, -1.0],
    [0.0, -1.0, -1.0, 1.0],
    [0.0, -1.0, -1.0, -1.0],
    [1.0, 0.0, 1.0, 1.0],
    [1.0, 0.0, 1.0, -1.0],
    [1.0, 0.0, -1.0, 1.0],
    [1.0, 0.0, -1.0, -1.0],
    [-1.0, 0.0, 1.0, 1.0],
    [-1.0, 0.0, 1.0, -1.0],
    [-1.0, 0.0, -1.0, 1.0],
    [-1.0, 0.0, -1.0, -1.0],
    [1.0, 1.0, 0.0, 1.0],
    [1.0, 1.0, 0.0, -1.0],
    [1.0, -1.0, 0.0, 1.0],
    [1.0, -1.0, 0.0, -1.0],
    [-1.0, 1.0, 0.0, 1.0],
    [-1.0, 1.0, 0.0, -1.0],
    [-1.0, -1.0, 0.0, 1.0],
    [-1.0, -1.0, 0.0, -1.0],
    [1.0, 1.0, 1.0, 0.0],
    [1.0, 1.0, -1.0, 0.0],
    [1.0, -1.0, 1.0, 0.0],
    [1.0, -1.0, -1.0, 0.0],
    [-1.0, 1.0, 1.0, 0.0],
    [-1.0, 1.0, -1.0, 0.0],
    [-1.0, -1.0, 1.0, 0.0],
    [-1.0, -1.0, -1.0, 0.0],
];

fn build_perm(random: &mut Random) -> [u8; 512] {
    let mut p = [0u8; 256];
    for (i, v) in p.iter_mut().enumerate() {
        *v = i as u8;
    }
    for i in 0..255 {
        let r = (i + (random.next_f64() * (256 - i) as f64) as usize).min(255);
        p.swap(i, r);
    }
    let mut perm = [0u8; 512];
    for i in 0..512 {
        perm[i] = p[i & 255];
    }
    perm
}

/// Seedable simplex noise generator.
///
/// Matches the TS `SimplexNoise` class output for the same seed.
#[derive(Clone, Debug)]
pub struct SimplexNoise {
    perm: [u8; 512],
    perm_mod12: [u8; 512],
}

impl SimplexNoise {
    /// Create a new generator with the given seed string.
    pub fn new(seed: &str) -> Self {
        let mut r = {
            let h = hash_string(seed);
            Random::from_seed(h)
        };
        Self::from_random(&mut r)
    }

    /// Create a new generator from an existing `Random` source.
    pub fn from_random(random: &mut Random) -> Self {
        let perm = build_perm(random);
        let mut perm_mod12 = [0u8; 512];
        for i in 0..512 {
            perm_mod12[i] = perm[i] % 12;
        }
        Self { perm, perm_mod12 }
    }

    /// Create with a random seed (matches `new SimplexNoise()` in TS).
    pub fn unseeded() -> Self {
        let mut r = Random::from_entropy();
        Self::from_random(&mut r)
    }

    fn dot2(g: [f64; 3], x: f64, y: f64) -> f64 {
        g[0] * x + g[1] * y
    }

    fn dot3(g: [f64; 3], x: f64, y: f64, z: f64) -> f64 {
        g[0] * x + g[1] * y + g[2] * z
    }

    fn dot4(g: [f64; 4], x: f64, y: f64, z: f64, w: f64) -> f64 {
        g[0] * x + g[1] * y + g[2] * z + g[3] * w
    }

    pub fn noise_2d(&self, x: f64, y: f64) -> f64 {
        let s = (x + y) * F2;
        let i = (x + s).floor() as isize;
        let j = (y + s).floor() as isize;
        let t = (i + j) as f64 * G2;
        let x0 = x - (i as f64 - t);
        let y0 = y - (j as f64 - t);

        let (i1, j1) = if x0 > y0 { (1, 0) } else { (0, 1) };

        let x1 = x0 - i1 as f64 + G2;
        let y1 = y0 - j1 as f64 + G2;
        let x2 = x0 - 1.0 + 2.0 * G2;
        let y2 = y0 - 1.0 + 2.0 * G2;

        let ii = i & 255;
        let jj = j & 255;

        let gi0 = self.perm_mod12[(ii + self.perm[(jj) as usize] as isize) as usize] as usize;
        let gi1 =
            self.perm_mod12[(ii + i1 + self.perm[(jj + j1) as usize] as isize) as usize] as usize;
        let gi2 =
            self.perm_mod12[(ii + 1 + self.perm[(jj + 1) as usize] as isize) as usize] as usize;

        let mut n = 0.0;

        let mut t0 = 0.5 - x0 * x0 - y0 * y0;
        if t0 > 0.0 {
            t0 *= t0;
            n += t0 * t0 * Self::dot2(GRAD3[gi0], x0, y0);
        }

        let mut t1 = 0.5 - x1 * x1 - y1 * y1;
        if t1 > 0.0 {
            t1 *= t1;
            n += t1 * t1 * Self::dot2(GRAD3[gi1], x1, y1);
        }

        let mut t2 = 0.5 - x2 * x2 - y2 * y2;
        if t2 > 0.0 {
            t2 *= t2;
            n += t2 * t2 * Self::dot2(GRAD3[gi2], x2, y2);
        }

        70.0 * n // scale to roughly [-1, 1]
    }

    pub fn noise_3d(&self, x: f64, y: f64, z: f64) -> f64 {
        let s = (x + y + z) * F3;
        let i = (x + s).floor() as isize;
        let j = (y + s).floor() as isize;
        let k = (z + s).floor() as isize;
        let t = (i + j + k) as f64 * G3;
        let x0 = x - (i as f64 - t);
        let y0 = y - (j as f64 - t);
        let z0 = z - (k as f64 - t);

        let (i1, j1, k1, i2, j2, k2) = if x0 >= y0 {
            if y0 >= z0 {
                (1, 0, 0, 1, 1, 0)
            } else if x0 >= z0 {
                (1, 0, 0, 1, 0, 1)
            } else {
                (0, 0, 1, 1, 0, 1)
            }
        } else if y0 < z0 {
            (0, 0, 1, 0, 1, 1)
        } else if x0 < z0 {
            (0, 1, 0, 0, 1, 1)
        } else {
            (0, 1, 0, 1, 1, 0)
        };

        let x1 = x0 - i1 as f64 + G3;
        let y1 = y0 - j1 as f64 + G3;
        let z1 = z0 - k1 as f64 + G3;
        let x2 = x0 - i2 as f64 + 2.0 * G3;
        let y2 = y0 - j2 as f64 + 2.0 * G3;
        let z2 = z0 - k2 as f64 + 2.0 * G3;
        let x3 = x0 - 1.0 + 3.0 * G3;
        let y3 = y0 - 1.0 + 3.0 * G3;
        let z3 = z0 - 1.0 + 3.0 * G3;

        let ii = i & 255;
        let jj = j & 255;
        let kk = k & 255;

        let gi0 = self.perm_mod12
            [(ii + self.perm[(jj + self.perm[(kk) as usize] as isize) as usize] as isize) as usize]
            as usize;
        let gi1 = self.perm_mod12[(ii
            + i1
            + self.perm[(jj + j1 + self.perm[(kk + k1) as usize] as isize) as usize] as isize)
            as usize] as usize;
        let gi2 = self.perm_mod12[(ii
            + i2
            + self.perm[(jj + j2 + self.perm[(kk + k2) as usize] as isize) as usize] as isize)
            as usize] as usize;
        let gi3 = self.perm_mod12[(ii
            + 1
            + self.perm[(jj + 1 + self.perm[(kk + 1) as usize] as isize) as usize] as isize)
            as usize] as usize;

        let mut n = 0.0;

        let mut t0 = 0.6 - x0 * x0 - y0 * y0 - z0 * z0;
        if t0 > 0.0 {
            t0 *= t0;
            n += t0 * t0 * Self::dot3(GRAD3[gi0], x0, y0, z0);
        }

        let mut t1 = 0.6 - x1 * x1 - y1 * y1 - z1 * z1;
        if t1 > 0.0 {
            t1 *= t1;
            n += t1 * t1 * Self::dot3(GRAD3[gi1], x1, y1, z1);
        }

        let mut t2 = 0.6 - x2 * x2 - y2 * y2 - z2 * z2;
        if t2 > 0.0 {
            t2 *= t2;
            n += t2 * t2 * Self::dot3(GRAD3[gi2], x2, y2, z2);
        }

        let mut t3 = 0.6 - x3 * x3 - y3 * y3 - z3 * z3;
        if t3 > 0.0 {
            t3 *= t3;
            n += t3 * t3 * Self::dot3(GRAD3[gi3], x3, y3, z3);
        }

        32.0 * n
    }

    pub fn noise_4d(&self, x: f64, y: f64, z: f64, w: f64) -> f64 {
        let s = (x + y + z + w) * F4;
        let i = (x + s).floor() as isize;
        let j = (y + s).floor() as isize;
        let k = (z + s).floor() as isize;
        let l = (w + s).floor() as isize;
        let t = (i + j + k + l) as f64 * G4;
        let x0 = x - (i as f64 - t);
        let y0 = y - (j as f64 - t);
        let z0 = z - (k as f64 - t);
        let w0 = w - (l as f64 - t);

        let mut rankx = 0isize;
        let mut ranky = 0isize;
        let mut rankz = 0isize;
        let mut rankw = 0isize;
        if x0 > y0 {
            rankx += 1;
        } else {
            ranky += 1;
        }
        if x0 > z0 {
            rankx += 1;
        } else {
            rankz += 1;
        }
        if x0 > w0 {
            rankx += 1;
        } else {
            rankw += 1;
        }
        if y0 > z0 {
            ranky += 1;
        } else {
            rankz += 1;
        }
        if y0 > w0 {
            ranky += 1;
        } else {
            rankw += 1;
        }
        if z0 > w0 {
            rankz += 1;
        } else {
            rankw += 1;
        }

        let i1 = if rankx >= 3 { 1 } else { 0 };
        let j1 = if ranky >= 3 { 1 } else { 0 };
        let k1 = if rankz >= 3 { 1 } else { 0 };
        let l1 = if rankw >= 3 { 1 } else { 0 };
        let i2 = if rankx >= 2 { 1 } else { 0 };
        let j2 = if ranky >= 2 { 1 } else { 0 };
        let k2 = if rankz >= 2 { 1 } else { 0 };
        let l2 = if rankw >= 2 { 1 } else { 0 };
        let i3 = if rankx >= 1 { 1 } else { 0 };
        let j3 = if ranky >= 1 { 1 } else { 0 };
        let k3 = if rankz >= 1 { 1 } else { 0 };
        let l3 = if rankw >= 1 { 1 } else { 0 };

        let x1 = x0 - i1 as f64 + G4;
        let y1 = y0 - j1 as f64 + G4;
        let z1 = z0 - k1 as f64 + G4;
        let w1 = w0 - l1 as f64 + G4;
        let x2 = x0 - i2 as f64 + 2.0 * G4;
        let y2 = y0 - j2 as f64 + 2.0 * G4;
        let z2 = z0 - k2 as f64 + 2.0 * G4;
        let w2 = w0 - l2 as f64 + 2.0 * G4;
        let x3 = x0 - i3 as f64 + 3.0 * G4;
        let y3 = y0 - j3 as f64 + 3.0 * G4;
        let z3 = z0 - k3 as f64 + 3.0 * G4;
        let w3 = w0 - l3 as f64 + 3.0 * G4;
        let x4 = x0 - 1.0 + 4.0 * G4;
        let y4 = y0 - 1.0 + 4.0 * G4;
        let z4 = z0 - 1.0 + 4.0 * G4;
        let w4 = w0 - 1.0 + 4.0 * G4;

        let ii = i & 255;
        let jj = j & 255;
        let kk = k & 255;
        let ll = l & 255;

        let g0 = |dx: f64, dy: f64, dz: f64, dw: f64, idx: usize| -> f64 {
            let mut t = 0.6 - dx * dx - dy * dy - dz * dz - dw * dw;
            if t < 0.0 {
                return 0.0;
            }
            t *= t;
            t * t * Self::dot4(GRAD4[idx], dx, dy, dz, dw)
        };

        let idx = |a: isize, b: isize, c: isize, d: isize| -> usize {
            let p0 = self.perm[(ll + d) as usize] as isize;
            let p1 = self.perm[(kk + c + p0) as usize] as isize;
            let p2 = self.perm[(jj + b + p1) as usize] as isize;
            let p3 = self.perm[(ii + a + p2) as usize] as usize;
            p3 % 32
        };

        let n0 = g0(x0, y0, z0, w0, idx(0, 0, 0, 0));
        let n1 = g0(x1, y1, z1, w1, idx(i1, j1, k1, l1));
        let n2 = g0(x2, y2, z2, w2, idx(i2, j2, k2, l2));
        let n3 = g0(x3, y3, z3, w3, idx(i3, j3, k3, l3));
        let n4 = g0(x4, y4, z4, w4, idx(1, 1, 1, 1));

        27.0 * (n0 + n1 + n2 + n3 + n4)
    }
}

/// Simple string hash with good avalanche properties (FNV-1a style).
fn hash_string(s: &str) -> u32 {
    let mut h: u32 = 0x811c9dc5;
    for b in s.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x01000193);
    }
    // Finalize: additional mixing for better distribution
    h ^= h >> 16;
    h = h.wrapping_mul(0x85ebca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2ae35);
    h ^= h >> 16;
    h
}

/// Get a noise value mapped to a specific range.
/// Port of `getNoiseRange` from `utils.ts:626-628`.
pub fn noise_range(noise: &SimplexNoise, x: f32, y: f32, from: f32, to: f32) -> f32 {
    let n = noise.noise_2d(x as f64 / 50.0, y as f64 / 50.0);
    let t = ((n + 1.0) / 2.0) as f32; // normalize from [-1,1] to [0,1]
    t * (to - from) + from
}
