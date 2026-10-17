//! Deterministic particle spawning: the SplitMix64 stream and the mapping from
//! an effect layer's initial-condition description to one immutable
//! [`ParticleRecord`].  Pure (no GL, no ECS) so it is unit tested directly.

use classic_core::effects::{land_time, GroundMode, ParticleLayerDef, Range, SpawnShape, LUNAR_G};
use classic_gfx::ParticleRecord;
use glam::{Mat3, Vec3};

/// SplitMix64 — tiny, fast, statistically solid, and trivially reproducible:
/// the spawn stream of an emitter is keyed by `(instance id, frame, layer)`,
/// so under a fixed timestep every run spawns bit-identical particles.
#[derive(Clone, Debug)]
pub struct SplitMix64(u64);

impl SplitMix64 {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// A stream keyed by several values.
    pub fn keyed(a: u64, b: u64, c: u64) -> Self {
        let mut s = Self(a ^ 0xA076_1D64_78BD_642F);
        let x = s.next_u64() ^ b.wrapping_mul(0xE703_7ED1_A0B4_28DB);
        let mut s = Self(x);
        Self(s.next_u64() ^ c.wrapping_mul(0x8EBC_6AF0_9C88_C6E3))
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)` with 24 bits of precision.
    pub fn f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    pub fn range(&mut self, r: Range) -> f32 {
        r.lerp(self.f32())
    }
}

/// Stable 64-bit FNV-1a of a string (instance ids from emitter names).
pub fn hash_str(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// The emitter state a particle is spawned from.
#[derive(Clone, Copy, Debug)]
pub struct SpawnFrame {
    /// Emitter origin (world metres).
    pub pos: Vec3,
    /// Emitter orientation (columns = local axes in world space).
    pub basis: Mat3,
    /// Emission direction (world, unit).
    pub dir: Vec3,
    /// Emitter velocity (m/s, for `inherit`).
    pub vel: Vec3,
    /// Spatial scale.
    pub scale: f32,
    /// Speed multiplier.
    pub speed: f32,
}

/// A landing that should fire a sub-effect.
#[derive(Clone, Debug, PartialEq)]
pub struct Landing {
    /// Clock time (epoch-relative) of the landing.
    pub time: f32,
    pub pos: Vec3,
}

/// A unit vector inside the cone of half-angle `cone` around `axis`.
pub fn cone_dir(rng: &mut SplitMix64, axis: Vec3, cone: f32) -> Vec3 {
    let axis = axis.try_normalize().unwrap_or(Vec3::Z);
    if cone <= 1e-5 {
        return axis;
    }
    let cos_max = cone.min(std::f32::consts::PI).cos();
    let z = 1.0 - rng.f32() * (1.0 - cos_max);
    let r = (1.0 - z * z).max(0.0).sqrt();
    let phi = rng.f32() * std::f32::consts::TAU;
    let local = Vec3::new(r * phi.cos(), r * phi.sin(), z);
    // Rotate +Z onto `axis`.
    let (t, b) = axis.any_orthonormal_pair();
    t * local.x + b * local.y + axis * local.z
}

/// Spawn one particle of `layer` from `frame` at birth time `t_birth`
/// (epoch-relative clock seconds).  `ground` samples the terrain height under a
/// world `(x, y)`.  Returns the record plus an optional sub-effect landing.
pub fn spawn_particle(
    rng: &mut SplitMix64,
    layer: &ParticleLayerDef,
    frame: &SpawnFrame,
    t_birth: f32,
    ground: &dyn Fn(f32, f32) -> Option<f32>,
) -> (ParticleRecord, Option<Landing>) {
    let sp = &layer.spawn;
    let scale = frame.scale;
    let mut origin = frame.pos;
    if sp.on_ground {
        if let Some(z) = ground(origin.x, origin.y) {
            origin.z = z;
        }
    }
    // Shape offset: horizontal shapes live in the world XY plane (dust rings
    // stay level when the emitter leans); spheres in 3D.
    let offset = match sp.shape {
        SpawnShape::Point => Vec3::ZERO,
        SpawnShape::Sphere => {
            let d = cone_dir(rng, Vec3::Z, std::f32::consts::PI);
            d * sp.radius * rng.f32().cbrt()
        }
        SpawnShape::Disc => {
            let a = rng.f32() * std::f32::consts::TAU;
            let r = sp.radius * rng.f32().sqrt();
            Vec3::new(a.cos() * r, a.sin() * r, 0.0)
        }
        SpawnShape::Ring => {
            let a = rng.f32() * std::f32::consts::TAU;
            let r = sp.radius + (rng.f32() - 0.5) * sp.thickness;
            Vec3::new(a.cos() * r, a.sin() * r, 0.0)
        }
    } * scale;
    let p0 = origin + offset;

    let v = &layer.velocity;
    let k = frame.speed * scale;
    let mut vel = Vec3::ZERO;
    if v.speed.max > 0.0 || v.speed.min > 0.0 {
        vel += cone_dir(rng, frame.dir, v.cone) * rng.range(v.speed) * k;
    }
    let horiz = Vec3::new(offset.x, offset.y, 0.0);
    let radial_dir = horiz.try_normalize().unwrap_or_else(|| {
        let a = rng.f32() * std::f32::consts::TAU;
        Vec3::new(a.cos(), a.sin(), 0.0)
    });
    if v.radial.max != 0.0 || v.radial.min != 0.0 {
        vel += radial_dir * rng.range(v.radial) * k;
    }
    if v.tangential.max != 0.0 || v.tangential.min != 0.0 {
        vel += Vec3::new(-radial_dir.y, radial_dir.x, 0.0) * rng.range(v.tangential) * k;
    }
    if v.up.max != 0.0 || v.up.min != 0.0 {
        vel += Vec3::Z * rng.range(v.up) * k;
    }
    vel += frame.vel * sp.inherit;

    let life = rng.range(layer.life).max(1e-3);
    let size = rng.range(layer.size) * scale;
    let rot = rng.f32() * std::f32::consts::TAU;
    let seed = rng.f32();
    let ground_z = ground(p0.x, p0.y).unwrap_or(-1.0e4);
    let rec = ParticleRecord {
        p0: p0.to_array(),
        t_birth,
        v0: vel.to_array(),
        life,
        seed,
        size,
        rot,
        ground_z,
    };

    let g = &layer.ground;
    let landing = match (&g.sub_effect, g.mode) {
        (Some(_), GroundMode::Die | GroundMode::Bounce) if layer.motion.swirl.is_none() => {
            let gz = -LUNAR_G * layer.motion.gravity;
            let fire = rng.f32() < g.sub_chance.clamp(0.0, 1.0);
            land_time(p0.z, vel.z, gz, layer.motion.drag, ground_z + g.offset, life)
                .filter(|_| fire)
                .map(|tl| {
                    let p = classic_core::effects::ballistic(
                        p0.to_array(),
                        vel.to_array(),
                        [0.0, 0.0, gz],
                        layer.motion.drag,
                        tl,
                    );
                    Landing { time: t_birth + tl, pos: Vec3::new(p[0], p[1], ground_z) }
                })
        }
        _ => None,
    };
    (rec, landing)
}

#[cfg(test)]
mod tests {
    use super::*;
    use classic_core::effects::{GroundDef, VelocityDef};

    fn frame() -> SpawnFrame {
        SpawnFrame {
            pos: Vec3::new(10.0, -5.0, 2.0),
            basis: Mat3::IDENTITY,
            dir: Vec3::Z,
            vel: Vec3::ZERO,
            scale: 1.0,
            speed: 1.0,
        }
    }

    #[test]
    fn splitmix_is_reproducible_and_uniformish() {
        let a: Vec<u64> =
            (0..4).map(|_| 0).scan(SplitMix64::new(7), |s, _| Some(s.next_u64())).collect();
        let b: Vec<u64> =
            (0..4).map(|_| 0).scan(SplitMix64::new(7), |s, _| Some(s.next_u64())).collect();
        assert_eq!(a, b);
        assert_ne!(SplitMix64::keyed(1, 2, 3).next_u64(), SplitMix64::keyed(1, 2, 4).next_u64());
        let mut r = SplitMix64::new(42);
        let mean: f32 = (0..10_000).map(|_| r.f32()).sum::<f32>() / 10_000.0;
        assert!((mean - 0.5).abs() < 0.02, "mean {mean}");
    }

    #[test]
    fn cone_dir_stays_inside_the_cone() {
        let mut r = SplitMix64::new(1);
        let axis = Vec3::new(1.0, 1.0, 0.0).normalize();
        for _ in 0..500 {
            let d = cone_dir(&mut r, axis, 0.3);
            assert!((d.length() - 1.0).abs() < 1e-4);
            assert!(d.dot(axis) >= 0.3f32.cos() - 1e-4);
        }
    }

    #[test]
    fn spawn_is_deterministic_per_key() {
        let layer = ParticleLayerDef {
            spawn: classic_core::effects::SpawnDef {
                shape: SpawnShape::Disc,
                radius: 2.0,
                ..Default::default()
            },
            velocity: VelocityDef { radial: Range::new(1.0, 3.0), ..Default::default() },
            ..Default::default()
        };
        let ground = |_x: f32, _y: f32| Some(0.0);
        let a = spawn_particle(&mut SplitMix64::keyed(9, 3, 0), &layer, &frame(), 1.0, &ground);
        let b = spawn_particle(&mut SplitMix64::keyed(9, 3, 0), &layer, &frame(), 1.0, &ground);
        assert_eq!(a, b);
        let c = spawn_particle(&mut SplitMix64::keyed(9, 4, 0), &layer, &frame(), 1.0, &ground);
        assert_ne!(a.0, c.0);
        // Disc offset stays within the radius; radial velocity points outward.
        let p = Vec3::from_array(a.0.p0);
        let off = p - frame().pos;
        assert!(off.truncate().length() <= 2.0 + 1e-4 && off.z == 0.0);
        let v = Vec3::from_array(a.0.v0);
        assert!(v.truncate().dot(off.truncate()) > 0.0);
        assert_eq!(a.0.ground_z, 0.0);
    }

    #[test]
    fn on_ground_spawns_at_terrain_height() {
        let layer = ParticleLayerDef {
            spawn: classic_core::effects::SpawnDef { on_ground: true, ..Default::default() },
            ..Default::default()
        };
        let ground = |_x: f32, _y: f32| Some(-3.5);
        let (r, _) = spawn_particle(&mut SplitMix64::new(1), &layer, &frame(), 0.0, &ground);
        assert_eq!(r.p0[2], -3.5);
    }

    #[test]
    fn dying_particles_report_their_landing() {
        let layer = ParticleLayerDef {
            velocity: VelocityDef { up: Range::new(2.0, 2.0), ..Default::default() },
            life: Range::new(10.0, 10.0),
            ground: GroundDef {
                mode: GroundMode::Die,
                sub_effect: Some("puff".into()),
                sub_chance: 1.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let ground = |_x: f32, _y: f32| Some(0.0);
        let (r, landing) = spawn_particle(&mut SplitMix64::new(3), &layer, &frame(), 5.0, &ground);
        let l = landing.expect("lands within its life");
        // z0 = 2, vz = 2, g = 1.62: t = (2 + sqrt(4 + 6.48)) / 1.62.
        let expect = (2.0 + (4.0f32 + 2.0 * 1.62 * 2.0).sqrt()) / 1.62;
        assert!((l.time - (5.0 + expect)).abs() < 1e-3, "{} vs {}", l.time, 5.0 + expect);
        assert_eq!(l.pos.z, 0.0);
        assert_eq!(r.t_birth, 5.0);
    }
}
