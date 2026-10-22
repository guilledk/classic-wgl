//! Particle / VFX effect definitions: the JSON effect library a ROM ships via
//! its `data[]` resources (e.g. `lunar-common::effects`), referenced by name
//! from `Emitter` components, one-shot `fx_emit` calls and sub-effects.
//!
//! Pure serde types plus the small amount of maths shared by the CPU spawner
//! and the GPU shader contract (curves, ranges).  The simulation model is
//! *analytic*: a particle is an immutable spawn record and the vertex shader
//! evaluates its state in closed form, so everything here describes initial
//! conditions and closed-form motion parameters, never per-frame forces.
//!
//! ```json
//! { "effects": [ { "name": "dust_puff", "layers": [
//!     { "type": "particles", "spawn": { "burst": 40, "shape": "disc", "radius": 0.3 },
//!       "velocity": { "radial": [0.5, 2.0], "up": [0.5, 1.5] },
//!       "life": [1.0, 2.0], "size": [0.2, 0.4],
//!       "color": [[0, [0.6, 0.58, 0.55, 0.0]], [0.1, [0.6, 0.58, 0.55, 0.8]], [1, [0.6, 0.58, 0.55, 0.0]]],
//!       "render": { "lit": true } } ] } ] }
//! ```

use serde::{Deserialize, Serialize};

/// Lunar surface gravity (m/s²); a layer's `gravity` scales it.
pub const LUNAR_G: f32 = 1.62;

/// Maximum number of keys a curve uploads to the GPU (one `vec4` of times +
/// one of values).  Longer curves are resampled.
pub const CURVE_KEYS: usize = 4;

/// A scalar `[min, max]` range, also accepted as a single number.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(from = "RangeRepr", into = "RangeRepr")]
pub struct Range {
    pub min: f32,
    pub max: f32,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum RangeRepr {
    One(f32),
    Two([f32; 2]),
}

impl From<RangeRepr> for Range {
    fn from(r: RangeRepr) -> Self {
        match r {
            RangeRepr::One(v) => Range { min: v, max: v },
            RangeRepr::Two([a, b]) => Range { min: a, max: b },
        }
    }
}

impl From<Range> for RangeRepr {
    fn from(r: Range) -> Self {
        if r.min == r.max {
            RangeRepr::One(r.min)
        } else {
            RangeRepr::Two([r.min, r.max])
        }
    }
}

impl Range {
    pub const ZERO: Range = Range { min: 0.0, max: 0.0 };

    pub const fn new(min: f32, max: f32) -> Self {
        Self { min, max }
    }

    /// Linear interpolation by `u ∈ [0, 1]`.
    pub fn lerp(&self, u: f32) -> f32 {
        self.min + (self.max - self.min) * u
    }
}

impl Default for Range {
    fn default() -> Self {
        Range::ZERO
    }
}

/// A piecewise-linear scalar curve over normalised life `[0, 1]`:
/// `[[t, v], ...]`, sorted by `t`.  An empty curve is the constant `1`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Curve(pub Vec<(f32, f32)>);

impl Curve {
    pub fn constant(v: f32) -> Self {
        Curve(vec![(0.0, v)])
    }

    /// Sample at `t` (clamped at the ends).
    pub fn sample(&self, t: f32) -> f32 {
        sample_keys(&self.0, t, 1.0, |a, b, u| a + (b - a) * u)
    }

    /// The GPU form: exactly [`CURVE_KEYS`] `(times, values)`.  Short curves
    /// repeat their last key; long ones are resampled at even times.
    pub fn gpu_keys(&self) -> ([f32; 4], [f32; 4]) {
        gpu_keys(&self.0, 1.0, |c| self.sample(c))
    }
}

/// A piecewise-linear linear-RGBA curve over normalised life.  An empty curve
/// is opaque white.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ColorCurve(pub Vec<(f32, [f32; 4])>);

impl ColorCurve {
    pub fn sample(&self, t: f32) -> [f32; 4] {
        sample_keys(&self.0, t, [1.0; 4], |a, b, u| {
            [
                a[0] + (b[0] - a[0]) * u,
                a[1] + (b[1] - a[1]) * u,
                a[2] + (b[2] - a[2]) * u,
                a[3] + (b[3] - a[3]) * u,
            ]
        })
    }

    /// The GPU form: [`CURVE_KEYS`] key times + RGBA values.
    pub fn gpu_keys(&self) -> ([f32; 4], [[f32; 4]; 4]) {
        gpu_keys(&self.0, [1.0; 4], |c| self.sample(c))
    }
}

fn sample_keys<V: Copy>(keys: &[(f32, V)], t: f32, empty: V, lerp: impl Fn(V, V, f32) -> V) -> V {
    let Some(first) = keys.first() else { return empty };
    if t <= first.0 {
        return first.1;
    }
    for w in keys.windows(2) {
        let (t0, v0) = w[0];
        let (t1, v1) = w[1];
        if t <= t1 {
            let u = if t1 > t0 { (t - t0) / (t1 - t0) } else { 1.0 };
            return lerp(v0, v1, u);
        }
    }
    keys[keys.len() - 1].1
}

fn gpu_keys<V: Copy>(keys: &[(f32, V)], empty: V, sample: impl Fn(f32) -> V) -> ([f32; 4], [V; 4]) {
    if keys.is_empty() {
        return ([0.0, 1.0, 2.0, 3.0], [empty; 4]);
    }
    if keys.len() <= CURVE_KEYS {
        let mut ts = [0.0f32; 4];
        let mut vs = [keys[0].1; 4];
        for i in 0..CURVE_KEYS {
            let (t, v) = keys[i.min(keys.len() - 1)];
            // Pad by pushing repeated keys past 1 so the shader's search never
            // divides by a zero-length span.
            ts[i] = if i < keys.len() { t } else { 1.0 + i as f32 };
            vs[i] = v;
        }
        return (ts, vs);
    }
    let ts = [0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0];
    (ts, ts.map(sample))
}

/// The top-level effect library document (one `data[]` artifact).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EffectLibrary {
    pub effects: Vec<EffectDef>,
}

impl EffectLibrary {
    /// Parse a library from JSON bytes.
    pub fn from_json(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }

    /// Sniff whether a `data[]` artifact is an effect library (a JSON object
    /// with a top-level `effects` array) without a full parse.
    pub fn sniff(bytes: &[u8]) -> bool {
        serde_json::from_slice::<serde_json::Value>(bytes)
            .ok()
            .is_some_and(|v| v.get("effects").is_some_and(|e| e.is_array()))
    }
}

/// One named effect: an ordered stack of layers.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EffectDef {
    pub name: String,
    pub layers: Vec<LayerDef>,
}

impl EffectDef {
    /// Seconds after start at which a one-shot instance of this effect has
    /// nothing left to emit (excluding the particles' own lifetimes).
    pub fn emit_span(&self) -> f32 {
        self.layers
            .iter()
            .map(|l| match l {
                LayerDef::Particles(p) => p.delay + p.duration.unwrap_or(0.0),
                LayerDef::Ribbon(r) => r.delay + r.duration.unwrap_or(0.0),
                LayerDef::Light(l) => l.delay + l.duration.unwrap_or(0.0),
            })
            .fold(0.0, f32::max)
    }

    /// Whether any layer emits light into the bloom pass.
    pub fn is_emissive(&self) -> bool {
        self.layers.iter().any(|l| match l {
            LayerDef::Particles(p) => p.render.emissive > 0.0,
            LayerDef::Ribbon(_) => true,
            LayerDef::Light(_) => false,
        })
    }
}

/// A layer of an effect.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LayerDef {
    Particles(Box<ParticleLayerDef>),
    Ribbon(Box<RibbonLayerDef>),
    Light(Box<LightLayerDef>),
}

impl LayerDef {
    pub fn name(&self) -> &str {
        match self {
            LayerDef::Particles(p) => &p.name,
            LayerDef::Ribbon(r) => &r.name,
            LayerDef::Light(l) => &l.name,
        }
    }
}

/// Spawn volume shape, in the emitter frame (`+Z` = up for `disc`/`ring`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpawnShape {
    #[default]
    Point,
    Sphere,
    /// Horizontal disc (uniform area) of `radius`.
    Disc,
    /// Horizontal ring at `radius` with `thickness`.
    Ring,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SpawnDef {
    /// Particles per second while the layer is emitting (scaled by the
    /// emitter's `rate`).
    pub rate: f32,
    /// Particles fired at the layer start of a one-shot instance and on every
    /// `emitter.burst` step.
    pub burst: u32,
    pub shape: SpawnShape,
    /// Shape radius (metres, scaled by the emitter scale).
    pub radius: f32,
    /// Ring thickness (metres).
    pub thickness: f32,
    /// Spawn on the terrain below the emitter instead of at the emitter.
    pub on_ground: bool,
    /// `[h0, h1]`: with `on_ground`, the rate multiplier falls linearly from
    /// 1 at emitter height `h0` above the ground to 0 at `h1` (rocket ground
    /// blast fading with altitude).
    pub ground_falloff: Option<[f32; 2]>,
    /// Fraction of the emitter's own velocity inherited by new particles.
    pub inherit: f32,
}

impl Default for SpawnDef {
    fn default() -> Self {
        Self {
            rate: 0.0,
            burst: 0,
            shape: SpawnShape::Point,
            radius: 0.0,
            thickness: 0.0,
            on_ground: false,
            ground_falloff: None,
            inherit: 0.0,
        }
    }
}

/// Initial velocity, composed of independent terms (all m/s, scaled by the
/// emitter `speed`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct VelocityDef {
    /// Speed along the emitter direction, randomised inside `cone`.
    pub speed: Range,
    /// Cone half-angle (radians) around the emitter direction.
    pub cone: f32,
    /// Horizontal outward speed from the emitter axis (direction from the
    /// spawn centre, or a random azimuth at the centre).
    pub radial: Range,
    /// World-up speed.
    pub up: Range,
    /// Horizontal counter-clockwise speed around the emitter axis.
    pub tangential: Range,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TurbulenceDef {
    /// Displacement amplitude (metres).
    pub amp: f32,
    /// Wobble frequency (Hz-ish).
    pub freq: f32,
    /// Seconds over which the amplitude ramps in from zero.
    pub ramp: f32,
}

/// Closed-form ground vortex ("dust devil") motion: a particle travelling
/// outward along the ground is captured at radius `kick_radius` by one of
/// `slots` vortices spaced around the emitter axis, then spirals up it.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SwirlDef {
    /// Number of vortices around the ring.
    pub slots: f32,
    /// Ring radius at which particles are captured (metres).
    pub kick_radius: f32,
    /// Relative random spread of the capture radius per particle.
    pub kick_jitter: f32,
    /// Initial angular speed (rad/s); alternate slots counter-rotate.
    pub omega: f32,
    /// Angular-speed decay rate (1/s).
    pub decay: f32,
    /// Column height (metres) the particle rises to.
    pub lift: f32,
    /// Lift time constant (s).
    pub lift_time: f32,
    /// Funnel widening: extra radius per metre of height.
    pub growth: f32,
    /// Vortex core radius (metres) particles are pulled toward.
    pub core: f32,
    /// Seconds over which a captured particle is pulled to the core.
    pub pinch_time: f32,
    /// Outward drift of the vortex centre (metres) over its life.
    pub drift: f32,
    /// Spiral arms per vortex (`1` = a single helix).
    pub arms: f32,
}

impl Default for SwirlDef {
    fn default() -> Self {
        Self {
            slots: 6.0,
            kick_radius: 8.0,
            kick_jitter: 0.15,
            omega: 4.0,
            decay: 0.4,
            lift: 6.0,
            lift_time: 1.5,
            growth: 0.25,
            core: 0.6,
            pinch_time: 0.4,
            drift: 2.0,
            arms: 2.0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct MotionDef {
    /// Multiplier on [`LUNAR_G`] (pointing down).
    pub gravity: f32,
    /// Linear drag coefficient `k` (1/s), `dv/dt = g - k v`.
    pub drag: f32,
    pub turbulence: TurbulenceDef,
    pub swirl: Option<SwirlDef>,
    /// Rotation speed range (rad/s).
    pub spin: Range,
}

impl Default for MotionDef {
    fn default() -> Self {
        Self {
            gravity: 1.0,
            drag: 0.0,
            turbulence: TurbulenceDef::default(),
            swirl: None,
            spin: Range::ZERO,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroundMode {
    /// Ignore the ground.
    None,
    /// Clamp to the spawn-time ground height (slides along it).
    #[default]
    Clamp,
    /// One bounce with `restitution`, then clamp.
    Bounce,
    /// Die on landing (and fire `sub_effect` there).
    Die,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GroundDef {
    pub mode: GroundMode,
    pub restitution: f32,
    /// Height above the ground the particle centre rests at (metres).
    pub offset: f32,
    /// Effect fired where a particle lands (`bounce`/`die`).
    pub sub_effect: Option<String>,
    /// Fraction of landings that fire the sub-effect (`0..=1`).
    pub sub_chance: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderMode {
    /// Camera-facing quad.
    #[default]
    Billboard,
    /// Quad stretched along the screen-projected velocity.
    Streak,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParticleShape {
    /// Soft round disc.
    #[default]
    Soft,
    /// Noise-broken puff (procedural smoke/dust).
    Smoke,
    /// Hot core + falloff (sparks, flares).
    Glow,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Blend {
    /// Ordinary alpha blending.
    #[default]
    Alpha,
    /// Additive light.
    Additive,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct FlipbookDef {
    pub cols: u32,
    pub rows: u32,
    /// Frames used (`<= cols * rows`); `0` = all.
    pub frames: u32,
    /// Frames per second; `0` = play once over the particle's life.
    pub fps: f32,
}

impl Default for FlipbookDef {
    fn default() -> Self {
        Self { cols: 1, rows: 1, frames: 0, fps: 0.0 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct RenderDef {
    pub mode: RenderMode,
    pub shape: ParticleShape,
    pub blend: Blend,
    /// Edge softness exponent of the procedural shapes.
    pub softness: f32,
    /// Lit by the sun + dynamic lights (shared lighting block).
    pub lit: bool,
    /// Emissive intensity (`0` = not emissive); emissive layers skip lighting
    /// and feed the bloom pass.
    pub emissive: f32,
    /// Streak trail length in seconds of motion.
    pub streak: f32,
    /// Optional texture (a ROM `textures[]` name or a host-registered one).
    pub texture: Option<String>,
    pub flipbook: Option<FlipbookDef>,
    /// Depth bias toward the camera, as a fraction of the particle size, so
    /// ground-hugging quads are not half-buried in the terrain.
    pub depth_bias: f32,
}

impl Default for RenderDef {
    fn default() -> Self {
        Self {
            mode: RenderMode::Billboard,
            shape: ParticleShape::Soft,
            blend: Blend::Alpha,
            softness: 1.0,
            lit: false,
            emissive: 0.0,
            streak: 0.05,
            texture: None,
            flipbook: None,
            depth_bias: 0.5,
        }
    }
}

/// A particle layer.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ParticleLayerDef {
    pub name: String,
    /// Ring-buffer capacity per instance (live particle cap).
    pub max: u32,
    /// Seconds after the instance start before the layer begins.
    pub delay: f32,
    /// Seconds the layer emits at `rate` on a one-shot instance (`None` =
    /// bursts only).  Emitter-driven instances emit while enabled.
    pub duration: Option<f32>,
    pub spawn: SpawnDef,
    pub velocity: VelocityDef,
    pub motion: MotionDef,
    /// Lifetime range (s).
    pub life: Range,
    /// Base size range (metres, quad diameter).
    pub size: Range,
    /// Size multiplier over life.
    pub size_curve: Curve,
    /// RGBA over life (alpha fades in/out here).
    pub color: ColorCurve,
    pub render: RenderDef,
    pub ground: GroundDef,
}

impl Default for ParticleLayerDef {
    fn default() -> Self {
        Self {
            name: String::new(),
            max: 1024,
            delay: 0.0,
            duration: None,
            spawn: SpawnDef::default(),
            velocity: VelocityDef::default(),
            motion: MotionDef::default(),
            life: Range::new(1.0, 1.0),
            size: Range::new(0.2, 0.2),
            size_curve: Curve::default(),
            color: ColorCurve::default(),
            render: RenderDef::default(),
            ground: GroundDef::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RibbonMode {
    /// A continuously crawling arc between the emitter and `end`.
    #[default]
    Arc,
    /// Discrete strikes from the emitter down to a random ground point.
    Lightning,
}

/// A procedural electrical arc / lightning bolt.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct RibbonLayerDef {
    pub name: String,
    pub mode: RibbonMode,
    pub delay: f32,
    pub duration: Option<f32>,
    /// Arc end point relative to the emitter (emitter frame, metres).
    pub end: [f32; 3],
    /// Lightning: horizontal radius of the random ground strike point.
    pub strike_radius: f32,
    /// Midpoint-displacement depth (`2^depth` segments).
    pub depth: u32,
    /// Displacement as a fraction of the segment length.
    pub jitter: f32,
    /// Branches per bolt.
    pub branches: u32,
    /// Core width (metres).
    pub width: f32,
    /// Glow halo width (metres).
    pub glow: f32,
    pub color: [f32; 3],
    pub intensity: f32,
    /// Shape regenerations per second.
    pub regen_hz: f32,
    /// Lightning: seconds between strikes `[min, max]`.
    pub interval: Range,
    /// Lightning: strike visible duration (s).
    pub strike_time: f32,
    /// Lightning: peak intensity of the flash light at each strike (`0` = none).
    pub flash: f32,
    /// Lightning: flash light radius (metres).
    pub flash_radius: f32,
}

impl Default for RibbonLayerDef {
    fn default() -> Self {
        Self {
            name: String::new(),
            mode: RibbonMode::Arc,
            delay: 0.0,
            duration: None,
            end: [2.0, 0.0, 0.0],
            strike_radius: 3.0,
            depth: 5,
            jitter: 0.3,
            branches: 1,
            width: 0.06,
            glow: 0.5,
            color: [0.6, 0.8, 1.0],
            intensity: 2.0,
            regen_hz: 24.0,
            interval: Range::new(0.4, 1.5),
            strike_time: 0.18,
            flash: 0.0,
            flash_radius: 12.0,
        }
    }
}

/// A companion dynamic light (flash, flicker, burn glow).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct LightLayerDef {
    pub name: String,
    pub delay: f32,
    /// Seconds the light lives on a one-shot instance (`None` = while the
    /// emitter is alive).
    pub duration: Option<f32>,
    pub color: [f32; 3],
    /// Intensity over normalised duration (constant `1` when emitter-driven).
    pub intensity: Curve,
    /// Peak intensity multiplier.
    pub peak: f32,
    /// Light radius (metres), scaled by the instance scale unless
    /// `fixed_radius`.
    pub radius: f32,
    /// Keep `radius` in absolute metres whatever the instance scale (a
    /// building's flame scaled down to a small kit still lights its
    /// surroundings).  `offset` still scales.  Serde default `false`;
    /// omitted when `false`.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub fixed_radius: bool,
    /// Offset from the emitter (metres).
    pub offset: [f32; 3],
    /// Random flicker amount (`0..1`) and rate (Hz).
    pub flicker: f32,
    pub flicker_hz: f32,
}

impl LightLayerDef {
    /// The light radius (metres) of an instance at `scale`.
    pub fn radius_at(&self, scale: f32) -> f32 {
        if self.fixed_radius {
            self.radius
        } else {
            self.radius * scale
        }
    }
}

impl Default for LightLayerDef {
    fn default() -> Self {
        Self {
            name: String::new(),
            delay: 0.0,
            duration: Some(0.5),
            color: [1.0, 0.8, 0.5],
            intensity: Curve::default(),
            peak: 4.0,
            radius: 10.0,
            fixed_radius: false,
            offset: [0.0, 0.0, 0.5],
            flicker: 0.0,
            flicker_hz: 20.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Closed-form motion (CPU mirror of `particle.vert`)
// ---------------------------------------------------------------------------

/// Ballistic + linear-drag position at age `t`: `dv/dt = g - k v`.
pub fn ballistic(p0: [f32; 3], v0: [f32; 3], g: [f32; 3], k: f32, t: f32) -> [f32; 3] {
    let mut out = [0.0; 3];
    for i in 0..3 {
        out[i] = if k > 1e-3 {
            let e = -(-k * t).exp_m1() / k;
            p0[i] + (v0[i] - g[i] / k) * e + (g[i] / k) * t
        } else {
            p0[i] + v0[i] * t + 0.5 * g[i] * t * t
        };
    }
    out
}

/// Vertical velocity at age `t` under gravity `gz` and drag `k`.
pub fn ballistic_vz(vz: f32, gz: f32, k: f32, t: f32) -> f32 {
    if k > 1e-3 {
        gz / k + (vz - gz / k) * (-k * t).exp()
    } else {
        vz + gz * t
    }
}

/// The age at which a particle starting at height `z0` with vertical speed
/// `vz` comes down to `ground` (`None` if it never does within `max_t`).
/// Mirrors `landTime` in `particle.vert`: the drag-free root refined by Newton
/// steps on the drag curve.
pub fn land_time(z0: f32, vz: f32, gz: f32, k: f32, ground: f32, max_t: f32) -> Option<f32> {
    if gz >= 0.0 {
        return None;
    }
    let c = z0 - ground;
    let disc = vz * vz - 2.0 * gz * c;
    if disc < 0.0 {
        return None;
    }
    let mut t = (-vz - disc.sqrt()) / gz;
    if !(t.is_finite()) || t < 0.0 {
        return None;
    }
    if k > 1e-3 {
        for _ in 0..4 {
            let z = ballistic([0.0, 0.0, z0], [0.0, 0.0, vz], [0.0, 0.0, gz], k, t)[2] - ground;
            let v = ballistic_vz(vz, gz, k, t);
            if v.abs() < 1e-4 {
                break;
            }
            t -= z / v;
        }
        if !(t.is_finite()) || t < 0.0 {
            return None;
        }
    }
    (t <= max_t).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_fixed_radius_defaults_off_and_round_trips() {
        let def: LightLayerDef = serde_json::from_str(r#"{"radius": 6}"#).unwrap();
        assert!(!def.fixed_radius);
        let out = serde_json::to_value(&def).unwrap();
        assert!(out.get("fixed_radius").is_none(), "omitted when false: {out}");
        let def: LightLayerDef =
            serde_json::from_str(r#"{"radius": 6, "fixed_radius": true}"#).unwrap();
        assert!(def.fixed_radius);
        let back: LightLayerDef =
            serde_json::from_value(serde_json::to_value(&def).unwrap()).unwrap();
        assert!(back.fixed_radius);
        // Through the tagged layer enum too.
        let layer: LayerDef =
            serde_json::from_str(r#"{"type": "light", "fixed_radius": true}"#).unwrap();
        assert!(matches!(layer, LayerDef::Light(l) if l.fixed_radius));
    }

    #[test]
    fn light_radius_scales_unless_fixed() {
        let mut def = LightLayerDef { radius: 6.0, ..Default::default() };
        assert_eq!([0.25, 1.0, 3.0].map(|s| def.radius_at(s)), [1.5, 6.0, 18.0]);
        def.fixed_radius = true;
        assert_eq!([0.25, 1.0, 3.0].map(|s| def.radius_at(s)), [6.0; 3]);
    }

    #[test]
    fn range_accepts_number_or_pair() {
        let r: Range = serde_json::from_str("2.5").unwrap();
        assert_eq!(r, Range::new(2.5, 2.5));
        let r: Range = serde_json::from_str("[1, 3]").unwrap();
        assert_eq!(r, Range::new(1.0, 3.0));
        assert_eq!(r.lerp(0.5), 2.0);
    }

    #[test]
    fn curve_samples_and_clamps() {
        let c = Curve(vec![(0.0, 0.0), (0.5, 1.0), (1.0, 0.0)]);
        assert_eq!(c.sample(-1.0), 0.0);
        assert_eq!(c.sample(0.25), 0.5);
        assert_eq!(c.sample(0.75), 0.5);
        assert_eq!(c.sample(2.0), 0.0);
        assert_eq!(Curve::default().sample(0.3), 1.0);
    }

    #[test]
    fn gpu_keys_pad_short_and_resample_long_curves() {
        let (t, v) = Curve(vec![(0.0, 2.0), (1.0, 4.0)]).gpu_keys();
        assert_eq!(&t[..2], &[0.0, 1.0]);
        assert!(t[2] > 1.0 && t[3] > t[2], "padding keys sit past the end");
        assert_eq!(v, [2.0, 4.0, 4.0, 4.0]);
        let long = Curve(vec![(0.0, 0.0), (0.25, 1.0), (0.5, 2.0), (0.75, 3.0), (1.0, 4.0)]);
        let (t, v) = long.gpu_keys();
        assert_eq!(t, [0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0]);
        assert!((v[1] - 4.0 / 3.0).abs() < 1e-5);
    }

    #[test]
    fn library_parses_tagged_layers_with_defaults() {
        let json = br#"{"effects":[{"name":"puff","layers":[
            {"type":"particles","spawn":{"burst":10},"life":[1,2]},
            {"type":"light","peak":3},
            {"type":"ribbon","mode":"lightning"}]}]}"#;
        assert!(EffectLibrary::sniff(json));
        let lib = EffectLibrary::from_json(json).unwrap();
        let fx = &lib.effects[0];
        assert_eq!(fx.layers.len(), 3);
        match &fx.layers[0] {
            LayerDef::Particles(p) => {
                assert_eq!(p.spawn.burst, 10);
                assert_eq!(p.life, Range::new(1.0, 2.0));
                assert_eq!(p.motion.gravity, 1.0);
                assert_eq!(p.ground.mode, GroundMode::Clamp);
            }
            _ => panic!("particles first"),
        }
        assert!(fx.is_emissive(), "a ribbon layer is emissive");
        assert!(!EffectLibrary::sniff(br#"{"anchors":[]}"#));
    }

    #[test]
    fn drag_free_ballistic_matches_kinematics() {
        let p = ballistic([0.0, 0.0, 1.0], [2.0, 0.0, 3.0], [0.0, 0.0, -LUNAR_G], 0.0, 2.0);
        assert!((p[0] - 4.0).abs() < 1e-5);
        assert!((p[2] - (1.0 + 6.0 - 0.5 * LUNAR_G * 4.0)).abs() < 1e-5);
    }

    #[test]
    fn drag_approaches_drag_free_as_k_vanishes() {
        let a = ballistic([0.0; 3], [3.0, 1.0, 4.0], [0.0, 0.0, -LUNAR_G], 0.0, 1.5);
        let b = ballistic([0.0; 3], [3.0, 1.0, 4.0], [0.0, 0.0, -LUNAR_G], 2e-3, 1.5);
        for i in 0..3 {
            assert!((a[i] - b[i]).abs() < 1e-2, "{a:?} vs {b:?}");
        }
        // Drag bounds horizontal travel by v0/k.
        let far = ballistic([0.0; 3], [10.0, 0.0, 0.0], [0.0; 3], 2.0, 100.0);
        assert!((far[0] - 5.0).abs() < 1e-3);
    }

    #[test]
    fn land_time_lands_on_the_ground() {
        let g = -LUNAR_G;
        let t = land_time(2.0, 3.0, g, 0.0, 0.0, 100.0).unwrap();
        let z = ballistic([0.0, 0.0, 2.0], [0.0, 0.0, 3.0], [0.0, 0.0, g], 0.0, t)[2];
        assert!(z.abs() < 1e-3);
        let t = land_time(2.0, 3.0, g, 0.8, 0.0, 100.0).unwrap();
        let z = ballistic([0.0, 0.0, 2.0], [0.0, 0.0, 3.0], [0.0, 0.0, g], 0.8, t)[2];
        assert!(z.abs() < 1e-2, "drag landing z={z}");
        assert_eq!(land_time(2.0, 3.0, 0.0, 0.0, 0.0, 100.0), None);
        assert_eq!(land_time(2.0, 3.0, g, 0.0, 0.0, 0.5), None);
    }
}
