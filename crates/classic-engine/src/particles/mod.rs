//! # Skill: `classic-gfx`
//!
//! **Read `.agents/skills/classic-gfx/SKILL.md` (§19 Particles) before working
//! on this module.**
//!
//! The particle / VFX system: effect instances driven by `Emitter` components
//! (or fired one-shot by [`Engine::fx_emit`] and sub-effects), spawning
//! immutable records into per-(instance, layer) GPU rings that `particle.vert`
//! evaluates analytically; procedural arc/lightning ribbons; and companion
//! dynamic lights.
//!
//! Frame flow: [`Engine::update_particles`] (after the update closures and the
//! vehicle sim, before the render list) advances the clock, resolves emitter
//! frames, spawns, schedules sub-effects and drives light layers;
//! [`ParticleSystem::draw`] runs as render phase 3c (after the model ghost,
//! before the UI).  A scene with no emitters never creates an instance, so
//! its render trace is unchanged.

mod ribbon;
mod spawn;

use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use classic_core::components::{Emitter, Light, LightKind, Model, Tilemap};
use classic_core::effects::{
    Blend, EffectDef, EffectLibrary, GroundMode, LayerDef, LightLayerDef, ParticleLayerDef,
    ParticleShape, RenderMode, RibbonLayerDef, RibbonMode, LUNAR_G,
};
use classic_core::math::{iso_camera_matrix, iso_view_depth};
use classic_core::tilemap::{sample_height_mesh, TILE_M};
use classic_core::{RoleKind, Transform};
use classic_gfx::{
    DynamicVb, Gfx, ParticleGpu, ParticleLayerUniforms, ParticleRing, RenderSettings,
};
use glam::{Mat3, Mat4, Vec3};

pub use ribbon::{bolt, build_vertices, displace, Strand, RIBBON_VERTEX_FLOATS};
pub use spawn::{cone_dir, hash_str, spawn_particle, Landing, SpawnFrame, SplitMix64};

use crate::golden::{TraceCollector, TraceItemParams};
use crate::light::LightHandle;
use crate::Engine;

/// Seconds the particle clock may run past its epoch before every live record
/// is rebased (keeps `u_time - t_birth` precise in `f32`).
const EPOCH_SPAN: f64 = 1024.0;

/// Hard cap on one instance's ring (per layer).
const MAX_RING: u32 = 65_536;

/// Hard cap on queued sub-effect firings (a runaway cascade guard).
const MAX_PENDING_SUBS: usize = 512;

/// Where an instance's frame comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    /// An `Emitter` component (follows it, driven by its channels).
    Emitter(hecs::Entity),
    /// A fire-and-forget effect at a fixed frame.
    OneShot,
}

struct ParticleRt {
    def_index: usize,
    ring: ParticleRing,
    gpu: Option<ParticleGpu>,
    /// Fractional spawn carry.
    acc: f32,
    /// Whether the one-shot start burst has fired.
    started: bool,
}

struct RibbonRt {
    def_index: usize,
    strands: Vec<Strand>,
    regen: i64,
    /// Lightning: clock time of the next strike / end of the current one.
    next_strike: f64,
    strike_end: f64,
    strike_target: Vec3,
    strike_count: u64,
    /// Strikes whose flash light has been fired.
    flashed: u64,
    flicker: f32,
}

struct LightRt {
    def_index: usize,
    handle: Option<LightHandle>,
}

enum LayerRt {
    Particles(ParticleRt),
    Ribbon(RibbonRt),
    Light(LightRt),
}

struct Instance {
    /// Determinism key (stable across runs: hashed from the emitter name or
    /// the one-shot sequence).
    id: u64,
    /// Trace / debug name.
    name: String,
    effect: Rc<EffectDef>,
    source: Source,
    /// Clock time the instance started.
    start: f64,
    frame: SpawnFrame,
    prev_frame: Option<SpawnFrame>,
    tint: [f32; 3],
    intensity: f32,
    rate: f32,
    last_burst: f32,
    /// Clock time of the latest burst (emitter-driven instances), the start
    /// of a duration-limited light layer.
    last_burst_at: Option<f64>,
    /// Whether the source is currently emitting.
    emitting: bool,
    /// Whether the source still exists (a despawned emitter's instance
    /// finishes its live particles and then dies).
    source_alive: bool,
    layers: Vec<LayerRt>,
}

/// A queued sub-effect firing.
struct PendingSub {
    time: f64,
    effect: String,
    pos: Vec3,
    scale: f32,
    tint: [f32; 3],
    seq: u64,
}

/// The engine-owned particle system (see the module docs).
#[derive(Default)]
pub struct ParticleSystem {
    /// Absolute particle clock (s) and the epoch records are relative to.
    clock: f64,
    epoch: f64,
    frame: u64,
    effects: BTreeMap<String, Rc<EffectDef>>,
    instances: Vec<Instance>,
    by_entity: HashMap<hecs::Entity, u64>,
    pending: Vec<PendingSub>,
    next_seq: u64,
    graveyard: Vec<ParticleGpu>,
    ribbon_vb: Option<DynamicVb>,
    ribbon_verts: Vec<f32>,
}

/// Per-frame stats (tests, trace, HUD).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ParticleStats {
    pub instances: usize,
    pub live: usize,
    pub ribbons: usize,
}

impl ParticleSystem {
    /// Register every effect of `lib` under `{ns}::{name}` (bare when `ns`
    /// is empty).  Later registrations replace earlier ones by name.
    pub fn register_library(&mut self, ns: &str, lib: EffectLibrary) {
        for fx in lib.effects {
            let key = if ns.is_empty() { fx.name.clone() } else { format!("{ns}::{}", fx.name) };
            self.effects.insert(key, Rc::new(fx));
        }
    }

    /// Resolve an effect name: exact, else the unique-or-first (sorted) key
    /// whose last `::` segment matches.
    pub fn resolve(&self, name: &str) -> Option<Rc<EffectDef>> {
        if let Some(fx) = self.effects.get(name) {
            return Some(fx.clone());
        }
        let bare = name.rsplit("::").next().unwrap_or(name);
        self.effects
            .iter()
            .find(|(k, _)| k.rsplit("::").next() == Some(bare))
            .map(|(_, v)| v.clone())
    }

    pub fn effect_names(&self) -> impl Iterator<Item = &str> {
        self.effects.keys().map(String::as_str)
    }

    /// Clock time on the records' epoch (what `u_time` is set to).
    fn local_time(&self) -> f32 {
        (self.clock - self.epoch) as f32
    }

    pub fn stats(&self) -> ParticleStats {
        let t = self.local_time();
        let mut s = ParticleStats { instances: self.instances.len(), ..Default::default() };
        for inst in &self.instances {
            for l in &inst.layers {
                match l {
                    LayerRt::Particles(p) => s.live += p.ring.live_count(t),
                    LayerRt::Ribbon(r) => s.ribbons += usize::from(!r.strands.is_empty()),
                    LayerRt::Light(_) => {}
                }
            }
        }
        s
    }

    /// `(name, origin, live particles)` per instance (debug / HUD).
    pub fn instance_summary(&self) -> Vec<(String, Vec3, usize)> {
        let t = self.local_time();
        self.instances
            .iter()
            .map(|i| {
                let live = i
                    .layers
                    .iter()
                    .map(|l| match l {
                        LayerRt::Particles(p) => p.ring.live_count(t),
                        _ => 0,
                    })
                    .sum();
                (i.name.clone(), i.frame.pos, live)
            })
            .collect()
    }

    fn new_instance(
        &mut self,
        id: u64,
        name: String,
        effect: Rc<EffectDef>,
        source: Source,
        frame: SpawnFrame,
    ) -> Instance {
        let layers = effect
            .layers
            .iter()
            .enumerate()
            .map(|(i, l)| match l {
                LayerDef::Particles(p) => LayerRt::Particles(ParticleRt {
                    def_index: i,
                    ring: ParticleRing::new(p.max.clamp(1, MAX_RING) as usize),
                    gpu: None,
                    acc: 0.0,
                    started: false,
                }),
                LayerDef::Ribbon(_) => LayerRt::Ribbon(RibbonRt {
                    def_index: i,
                    strands: Vec::new(),
                    regen: -1,
                    next_strike: self.clock,
                    strike_end: f64::NEG_INFINITY,
                    strike_target: Vec3::ZERO,
                    strike_count: 0,
                    flashed: 0,
                    flicker: 1.0,
                }),
                LayerDef::Light(_) => LayerRt::Light(LightRt { def_index: i, handle: None }),
            })
            .collect();
        Instance {
            id,
            name,
            effect,
            source,
            start: self.clock,
            frame,
            prev_frame: None,
            tint: [1.0; 3],
            intensity: 1.0,
            rate: 1.0,
            last_burst: 0.0,
            last_burst_at: None,
            emitting: true,
            source_alive: true,
            layers,
        }
    }

    /// Start a one-shot instance of `effect` at a world position.
    pub fn emit(&mut self, effect: &str, pos: Vec3, dir: Vec3, scale: f32, tint: [f32; 3]) -> bool {
        let Some(fx) = self.resolve(effect) else {
            classic_core::cl_warn!(
                classic_core::instrument::Chan::Render,
                "fx_emit: unknown effect {effect:?}"
            );
            return false;
        };
        let seq = self.next_seq;
        self.next_seq += 1;
        let frame = SpawnFrame {
            pos,
            basis: Mat3::IDENTITY,
            dir: dir.try_normalize().unwrap_or(Vec3::Z),
            vel: Vec3::ZERO,
            scale,
            speed: 1.0,
        };
        let name = format!("fx#{seq}:{}", fx.name);
        let id = hash_str(&name) ^ seq;
        let mut inst = self.new_instance(id, name, fx, Source::OneShot, frame);
        inst.tint = tint;
        self.instances.push(inst);
        true
    }

    /// Whether a finished instance can be dropped.
    fn instance_done(&self, inst: &Instance) -> bool {
        let t = self.local_time();
        let rings_dead = inst.layers.iter().all(|l| match l {
            LayerRt::Particles(p) => p.ring.last_death() < t,
            LayerRt::Ribbon(r) => r.strands.is_empty(),
            LayerRt::Light(l) => l.handle.is_none(),
        });
        match inst.source {
            Source::Emitter(_) => !inst.source_alive && rings_dead,
            Source::OneShot => {
                self.clock > inst.start + inst.effect.emit_span() as f64 + 1e-6 && rings_dead
            }
        }
    }

    /// Rebase the records onto a fresh epoch when the clock has run far past
    /// it (rewrites every ring once, a full upload).
    fn maybe_rebase(&mut self) {
        if self.clock - self.epoch < EPOCH_SPAN {
            return;
        }
        let shift = (self.clock - self.epoch - 64.0).floor();
        self.epoch += shift;
        let s = shift as f32;
        for inst in &mut self.instances {
            for l in &mut inst.layers {
                if let LayerRt::Particles(p) = l {
                    p.ring.rewrite(|r| r.t_birth -= s);
                }
            }
        }
    }
}

/// Whether a particle layer glows (additive or emissive) and so draws in the
/// second, over-everything phase.
fn is_glow(def: &ParticleLayerDef) -> bool {
    def.render.blend == Blend::Additive || def.render.emissive > 0.0
}

/// Layer → shader uniforms for an instance drawn at clock `time`.
pub fn layer_uniforms(
    def: &ParticleLayerDef,
    scale: f32,
    tint: [f32; 3],
    intensity: f32,
    time: f32,
) -> ParticleLayerUniforms {
    let m = &def.motion;
    let (size_t, size_v) = def.size_curve.gpu_keys();
    let (color_t, color_v) = def.color.gpu_keys();
    let (s0, s1, s2) = match &m.swirl {
        Some(s) => (
            [s.slots.max(1.0), s.kick_radius * scale, s.omega, s.decay],
            [s.lift * scale, s.lift_time, s.growth, s.drift * scale],
            [s.arms, s.core * scale, s.pinch_time, s.kick_jitter],
        ),
        None => ([0.0; 4], [0.0; 4], [0.0; 4]),
    };
    let r = &def.render;
    ParticleLayerUniforms {
        time,
        motion: [0.0, 0.0, -LUNAR_G * m.gravity * scale, m.drag],
        turbulence: [
            m.turbulence.amp * scale,
            m.turbulence.freq,
            m.turbulence.ramp,
            m.spin.max.abs().max(m.spin.min.abs()),
        ],
        swirl0: s0,
        swirl1: s1,
        swirl2: s2,
        ground: [
            match def.ground.mode {
                GroundMode::None => 0.0,
                GroundMode::Clamp => 1.0,
                GroundMode::Bounce => 2.0,
                GroundMode::Die => 3.0,
            },
            def.ground.restitution,
            def.ground.offset * scale,
            0.0,
        ],
        size_t,
        size_v,
        color_t,
        color_v,
        render: [
            match r.mode {
                RenderMode::Billboard => 0.0,
                RenderMode::Streak => 1.0,
            },
            r.streak,
            r.depth_bias,
            0.0,
        ],
        shape: [
            match r.shape {
                ParticleShape::Soft => 0.0,
                ParticleShape::Smoke => 1.0,
                ParticleShape::Glow => 2.0,
            },
            r.softness,
            match r.blend {
                Blend::Alpha => 0.0,
                Blend::Additive => 1.0,
            },
            1.0,
        ],
        shade: [if r.lit { 1.0 } else { 0.0 }, r.emissive * intensity, 0.0, 0.0],
        tint,
        flipbook: match &r.flipbook {
            Some(f) => [f.cols as f32, f.rows as f32, f.frames as f32, f.fps],
            None => [0.0; 4],
        },
        texture: r.texture.clone(),
    }
}

/// Terrain height sampler over the primary tilemap (world metres in, world z
/// out; `None` off the map).
struct Ground<'a> {
    tm: hecs::Ref<'a, Tilemap>,
    origin: Vec3,
}

impl Ground<'_> {
    fn at(&self, x: f32, y: f32) -> Option<f32> {
        let tx = (x - self.origin.x) / TILE_M;
        let ty = -(y - self.origin.y) / TILE_M;
        if tx < 0.0 || ty < 0.0 || tx > self.tm.size_x as f32 || ty > self.tm.size_y as f32 {
            return None;
        }
        let tm = &*self.tm;
        Some(sample_height_mesh(&tm.height_data, tm.size_x, tm.size_y, tx, ty) + self.origin.z)
    }
}

impl Engine {
    /// Register an effect library (normally from a ROM `data[]` artifact).
    pub fn register_effects(&mut self, ns: &str, lib: EffectLibrary) {
        self.particles.register_library(ns, lib);
    }

    /// Fire a one-shot effect at a world position (metres), emitting along
    /// `dir`.  Returns `false` when the effect is unknown.
    pub fn fx_emit(&mut self, effect: &str, pos: Vec3, dir: Vec3, scale: f32) -> bool {
        self.fx_emit_tinted(effect, pos, dir, scale, [1.0; 3])
    }

    fn fx_emit_tinted(
        &mut self,
        effect: &str,
        pos: Vec3,
        dir: Vec3,
        scale: f32,
        tint: [f32; 3],
    ) -> bool {
        self.particles.emit(effect, pos, dir, scale, tint)
    }

    /// Particle-system stats for this frame.
    pub fn particle_stats(&self) -> ParticleStats {
        self.particles.stats()
    }

    /// The world frame of an emitter-like attachment: absolute `position`, or
    /// an offset from the parent's ground point + animated frame offset, or —
    /// with `node` on a `Model` parent — `position`/`dir` in that node's frame.
    pub(crate) fn attachment_frame(
        &self,
        parent: Option<&str>,
        node: Option<&str>,
        position: Vec3,
        dir: Vec3,
    ) -> Option<(Vec3, Mat3, Vec3)> {
        let Some(parent) = parent else {
            return Some((position, Mat3::IDENTITY, dir));
        };
        let &pe = self.names.get(parent)?;
        if self.is_disabled(pe) {
            return None;
        }
        let tf = self.world.get::<&Transform>(pe).ok()?;
        if let Some(node) = node {
            if let Ok(m) = self.world.get::<&Model>(pe) {
                if let Some(world) = self.model_node_world(&tf, &m, node) {
                    let basis = Mat3::from_mat4(world);
                    let nb = Mat3::from_cols(
                        basis.x_axis.normalize_or_zero(),
                        basis.y_axis.normalize_or_zero(),
                        basis.z_axis.normalize_or_zero(),
                    );
                    return Some((
                        world.transform_point3(position),
                        nb,
                        (nb * dir).try_normalize().unwrap_or(Vec3::Z),
                    ));
                }
            }
        }
        let base = self.iso_to_world(tf.position.x, tf.position.y, 0.0)?;
        Some((base + self.parent_frame_offset(pe) + position, Mat3::IDENTITY, dir))
    }

    /// Advance the particle system by `dt` (see the module docs).
    pub fn update_particles(&mut self, dt: f32) {
        let mut ps = std::mem::take(&mut self.particles);
        ps.clock += dt.max(0.0) as f64;
        ps.frame += 1;
        ps.maybe_rebase();
        self.sync_emitter_instances(&mut ps, dt);
        self.fire_pending_subs(&mut ps);
        self.spawn_all(&mut ps, dt);
        self.drive_light_layers(&mut ps);
        // Retire finished instances (their GPU rings are freed at draw time).
        let mut i = 0;
        while i < ps.instances.len() {
            if ps.instance_done(&ps.instances[i]) {
                let inst = ps.instances.swap_remove(i);
                if let Source::Emitter(e) = inst.source {
                    ps.by_entity.remove(&e);
                }
                for l in inst.layers {
                    match l {
                        LayerRt::Particles(p) => ps.graveyard.extend(p.gpu),
                        LayerRt::Light(l) => {
                            if let Some(h) = l.handle {
                                self.light_handles.release(&mut self.world, h);
                            }
                        }
                        LayerRt::Ribbon(_) => {}
                    }
                }
            } else {
                i += 1;
            }
        }
        self.particles = ps;
    }

    /// Create / refresh one instance per `Emitter` entity and copy its drive
    /// channels.
    fn sync_emitter_instances(&self, ps: &mut ParticleSystem, dt: f32) {
        let mut emitters: Vec<(String, hecs::Entity, Emitter)> = self
            .world
            .query::<&Emitter>()
            .iter()
            .map(|(e, em)| (self.debug_name(e), e, em.clone()))
            .collect();
        // Deterministic order (hecs order follows spawn history).
        emitters.sort_by(|a, b| a.0.cmp(&b.0));
        for inst in &mut ps.instances {
            if let Source::Emitter(e) = inst.source {
                inst.source_alive = self.world.contains(e);
                if !inst.source_alive {
                    inst.emitting = false;
                }
            }
        }
        for (name, entity, em) in emitters {
            let resolved = self.attachment_frame(
                em.parent.as_deref(),
                em.node.as_deref(),
                em.position,
                em.dir,
            );
            let active = em.enabled && !self.is_disabled(entity) && resolved.is_some();
            let id = match ps.by_entity.get(&entity) {
                Some(&id) => id,
                None => {
                    let Some(fx) = ps.resolve(&em.effect) else {
                        continue;
                    };
                    let id = hash_str(&name);
                    let (pos, basis, dir) =
                        resolved.unwrap_or((em.position, Mat3::IDENTITY, em.dir));
                    let frame = SpawnFrame {
                        pos,
                        basis,
                        dir,
                        vel: Vec3::ZERO,
                        scale: em.scale,
                        speed: em.speed,
                    };
                    let mut inst =
                        ps.new_instance(id, name.clone(), fx, Source::Emitter(entity), frame);
                    inst.last_burst = em.burst;
                    ps.instances.push(inst);
                    ps.by_entity.insert(entity, id);
                    id
                }
            };
            let Some(inst) = ps.instances.iter_mut().find(|i| i.id == id) else { continue };
            inst.emitting = active;
            inst.tint = em.color;
            inst.intensity = em.intensity;
            inst.rate = em.rate;
            inst.prev_frame = Some(inst.frame);
            if let Some((pos, basis, dir)) = resolved {
                let vel = if dt > 0.0 { (pos - inst.frame.pos) / dt } else { Vec3::ZERO };
                inst.frame = SpawnFrame { pos, basis, dir, vel, scale: em.scale, speed: em.speed };
            }
            // Burst counter: every integer step fires one burst per layer.
            let fired = if em.burst < inst.last_burst {
                em.burst.floor().max(0.0)
            } else {
                em.burst.floor() - inst.last_burst.floor()
            };
            inst.last_burst = em.burst;
            if fired > 0.0 && active {
                self.burst_instance(ps, id, fired as u32);
            }
        }
    }

    /// Fire `n` bursts of every particle layer of instance `id` now.
    fn burst_instance(&self, ps: &mut ParticleSystem, id: u64, n: u32) {
        let t = ps.local_time();
        let frame_no = ps.frame;
        let Some(inst) = ps.instances.iter_mut().find(|i| i.id == id) else { return };
        inst.last_burst_at = Some(ps.clock);
        let ground = self.ground_sampler();
        let gfn = |x: f32, y: f32| ground.as_ref().and_then(|g| g.at(x, y));
        for (li, l) in inst.layers.iter_mut().enumerate() {
            let LayerRt::Particles(p) = l else { continue };
            let LayerDef::Particles(def) = &inst.effect.layers[p.def_index] else { continue };
            let count = def.spawn.burst * n;
            let mut rng = SplitMix64::keyed(inst.id, frame_no, 0x1000 + li as u64);
            for _ in 0..count {
                let (rec, _) = spawn_particle(&mut rng, def, &inst.frame, t, &gfn);
                p.ring.push(rec);
            }
        }
    }

    fn ground_sampler(&self) -> Option<Ground<'_>> {
        let e = self.entity_by_role(RoleKind::Tilemap)?;
        let tm = self.world.get::<&Tilemap>(e).ok()?;
        let origin = self.world.get::<&Transform>(e).ok()?.position;
        Some(Ground { tm, origin })
    }

    /// Fire due sub-effects.
    fn fire_pending_subs(&self, ps: &mut ParticleSystem) {
        if ps.pending.is_empty() {
            return;
        }
        let now = ps.clock;
        let mut due: Vec<PendingSub> = Vec::new();
        ps.pending.retain_mut(|s| {
            if s.time <= now {
                due.push(PendingSub {
                    time: s.time,
                    effect: std::mem::take(&mut s.effect),
                    pos: s.pos,
                    scale: s.scale,
                    tint: s.tint,
                    seq: s.seq,
                });
                false
            } else {
                true
            }
        });
        due.sort_by(|a, b| a.time.total_cmp(&b.time).then(a.seq.cmp(&b.seq)));
        for s in due {
            ps.emit(&s.effect, s.pos, Vec3::Z, s.scale, s.tint);
        }
    }

    /// Continuous-rate + one-shot-start spawning for every instance.
    fn spawn_all(&self, ps: &mut ParticleSystem, dt: f32) {
        let t_now = ps.local_time();
        let clock = ps.clock;
        let frame_no = ps.frame;
        let ground = self.ground_sampler();
        let gfn = |x: f32, y: f32| ground.as_ref().and_then(|g| g.at(x, y));
        let mut subs: Vec<PendingSub> = Vec::new();
        let epoch = ps.epoch;
        for inst in &mut ps.instances {
            let age = (clock - inst.start) as f32;
            let one_shot = inst.source == Source::OneShot;
            for (li, l) in inst.layers.iter_mut().enumerate() {
                let LayerRt::Particles(p) = l else { continue };
                let LayerDef::Particles(def) = &inst.effect.layers[p.def_index] else { continue };
                let mut rng = SplitMix64::keyed(inst.id, frame_no, li as u64);
                let mut births: Vec<(f32, f32)> = Vec::new(); // (birth time, frame lerp)
                if one_shot && !p.started && age >= def.delay {
                    p.started = true;
                    for _ in 0..def.spawn.burst {
                        births.push((t_now - (age - def.delay).min(dt), 1.0));
                    }
                }
                let emitting_now = if one_shot {
                    let d = def.duration.unwrap_or(0.0);
                    age >= def.delay && age - dt < def.delay + d
                } else {
                    inst.emitting && age >= def.delay
                };
                if emitting_now && def.spawn.rate > 0.0 && dt > 0.0 {
                    let mut rate = def.spawn.rate * inst.rate;
                    if let (true, Some([h0, h1])) = (def.spawn.on_ground, def.spawn.ground_falloff)
                    {
                        let h = gfn(inst.frame.pos.x, inst.frame.pos.y)
                            .map(|g| inst.frame.pos.z - g)
                            .unwrap_or(f32::MAX);
                        let f = if h1 > h0 { ((h1 - h) / (h1 - h0)).clamp(0.0, 1.0) } else { 1.0 };
                        rate *= f;
                    }
                    p.acc += rate * dt;
                    let n = p.acc.floor();
                    p.acc -= n;
                    let n = (n as u32).min(p.ring.capacity() as u32);
                    for k in 0..n {
                        // Spread births across the frame (oldest first) so a
                        // moving emitter leaves a continuous trail.
                        let u = (k as f32 + rng.f32()) / n.max(1) as f32;
                        births.push((t_now - dt * (1.0 - u), u));
                    }
                }
                for (tb, u) in births {
                    let frame = match inst.prev_frame {
                        Some(prev) if !one_shot => {
                            SpawnFrame { pos: prev.pos.lerp(inst.frame.pos, u), ..inst.frame }
                        }
                        _ => inst.frame,
                    };
                    let (rec, landing) = spawn_particle(&mut rng, def, &frame, tb, &gfn);
                    p.ring.push(rec);
                    if let (Some(land), Some(sub)) = (landing, def.ground.sub_effect.as_ref()) {
                        subs.push(PendingSub {
                            time: land.time as f64 + epoch,
                            effect: sub.clone(),
                            pos: land.pos,
                            scale: inst.frame.scale,
                            tint: inst.tint,
                            seq: 0,
                        });
                    }
                }
            }
            // Ribbons.
            for (li, l) in inst.layers.iter_mut().enumerate() {
                let LayerRt::Ribbon(r) = l else { continue };
                let LayerDef::Ribbon(def) = &inst.effect.layers[r.def_index] else { continue };
                update_ribbon(
                    r,
                    def,
                    inst.id,
                    li as u64,
                    &inst.frame,
                    inst.emitting,
                    age,
                    clock,
                    &gfn,
                );
            }
        }
        for mut s in subs {
            if ps.pending.len() >= MAX_PENDING_SUBS {
                break;
            }
            s.seq = ps.next_seq;
            ps.next_seq += 1;
            ps.pending.push(s);
        }
    }

    /// Spawn / update / release the companion lights.
    fn drive_light_layers(&mut self, ps: &mut ParticleSystem) {
        let clock = ps.clock;
        for inst in &mut ps.instances {
            let age = (clock - inst.start) as f32;
            // Lightning flashes: one transient light per new strike.
            for l in &mut inst.layers {
                let LayerRt::Ribbon(r) = l else { continue };
                let LayerDef::Ribbon(def) = &inst.effect.layers[r.def_index] else { continue };
                if r.strike_count > r.flashed {
                    r.flashed = r.strike_count;
                    if def.flash > 0.0 {
                        let light = Light {
                            kind: LightKind::Point,
                            position: r.strike_target + Vec3::Z * 1.5,
                            color: def.color,
                            intensity: def.flash * inst.intensity,
                            radius: def.flash_radius * inst.frame.scale,
                            ..Default::default()
                        };
                        let ttl = Some(def.strike_time.max(0.05));
                        self.light_handles.spawn(&mut self.world, light, ttl);
                    }
                }
            }
            for l in &mut inst.layers {
                let LayerRt::Light(lr) = l else { continue };
                let LayerDef::Light(def) = &inst.effect.layers[lr.def_index] else { continue };
                let since_burst = inst.last_burst_at.map(|b| (clock - b) as f32);
                let level = light_level(def, inst.source, inst.emitting, age, since_burst, inst.id);
                match (level, lr.handle) {
                    (Some(level), handle) => {
                        let light = Light {
                            kind: LightKind::Point,
                            position: inst.frame.pos
                                + inst.frame.basis
                                    * Vec3::from_array(def.offset)
                                    * inst.frame.scale,
                            color: [
                                def.color[0] * inst.tint[0],
                                def.color[1] * inst.tint[1],
                                def.color[2] * inst.tint[2],
                            ],
                            intensity: level * def.peak * inst.intensity,
                            radius: def.radius * inst.frame.scale,
                            ..Default::default()
                        };
                        match handle {
                            Some(h) => {
                                self.light_handles.set(&mut self.world, h, light);
                            }
                            None => {
                                lr.handle = self.light_handles.spawn(&mut self.world, light, None);
                            }
                        }
                    }
                    (None, Some(h)) => {
                        self.light_handles.release(&mut self.world, h);
                        lr.handle = None;
                    }
                    (None, None) => {}
                }
            }
        }
    }
}

/// The current light level (`0..`) of a light layer, `None` = off.
///
/// A duration-limited layer plays once from the instance start (one-shot) or
/// from the latest burst (`since_burst`, emitter-driven); an open-ended one
/// stays on while the emitter emits.
fn light_level(
    def: &LightLayerDef,
    source: Source,
    emitting: bool,
    age: f32,
    since_burst: Option<f32>,
    id: u64,
) -> Option<f32> {
    let base = match (source, def.duration) {
        (_, Some(d)) => {
            let t = match source {
                Source::OneShot => age,
                Source::Emitter(_) => since_burst?,
            } - def.delay;
            if t < 0.0 || t > d {
                return None;
            }
            def.intensity.sample(t / d.max(1e-3))
        }
        (Source::OneShot, None) if age >= def.delay => def.intensity.sample(0.0),
        (Source::Emitter(_), _) if emitting && age >= def.delay => def.intensity.sample(0.0),
        _ => return None,
    };
    let flick = if def.flicker > 0.0 {
        let step = (age * def.flicker_hz.max(1.0)) as u64;
        let mut r = SplitMix64::keyed(id, step, 0x11);
        1.0 - def.flicker * r.f32()
    } else {
        1.0
    };
    Some(base * flick)
}

#[allow(clippy::too_many_arguments)]
fn update_ribbon(
    r: &mut RibbonRt,
    def: &RibbonLayerDef,
    id: u64,
    layer: u64,
    frame: &SpawnFrame,
    emitting: bool,
    age: f32,
    clock: f64,
    ground: &dyn Fn(f32, f32) -> Option<f32>,
) {
    let active = emitting && age >= def.delay && def.duration.is_none_or(|d| age <= def.delay + d);
    match def.mode {
        RibbonMode::Arc => {
            if !active {
                r.strands.clear();
                return;
            }
            let regen = ((age - def.delay) * def.regen_hz.max(1.0)) as i64;
            if regen != r.regen {
                r.regen = regen;
                let mut rng = SplitMix64::keyed(id, regen as u64, 0x2000 + layer);
                let a = frame.pos;
                let b = a + frame.basis * Vec3::from_array(def.end) * frame.scale;
                r.strands = bolt(&mut rng, a, b, def.depth, def.jitter, def.branches);
                r.flicker = 0.65 + 0.35 * rng.f32();
            }
        }
        RibbonMode::Lightning => {
            if clock >= r.next_strike && active {
                let mut rng = SplitMix64::keyed(id, r.strike_count, 0x3000 + layer);
                r.strike_count += 1;
                let ang = rng.f32() * std::f32::consts::TAU;
                let rad = def.strike_radius * frame.scale * rng.f32().sqrt();
                let x = frame.pos.x + ang.cos() * rad;
                let y = frame.pos.y + ang.sin() * rad;
                let z = ground(x, y).unwrap_or(frame.pos.z - 10.0);
                r.strike_target = Vec3::new(x, y, z);
                r.strike_end = clock + def.strike_time as f64;
                r.next_strike = clock + rng.range(def.interval).max(0.05) as f64;
                r.regen = -1;
            }
            if clock > r.strike_end {
                r.strands.clear();
                return;
            }
            // Re-jitter twice per strike (the classic double flash).
            let phase = 1.0 - ((r.strike_end - clock) as f32 / def.strike_time.max(1e-3));
            let regen = (phase * 2.0) as i64;
            if regen != r.regen {
                r.regen = regen;
                let mut rng =
                    SplitMix64::keyed(id, r.strike_count * 4 + regen as u64, 0x4000 + layer);
                r.strands =
                    bolt(&mut rng, frame.pos, r.strike_target, def.depth, def.jitter, def.branches);
                r.flicker = if regen == 0 { 1.0 } else { 0.55 };
            }
        }
    }
}

impl ParticleSystem {
    /// Phase 3c: draw every live instance (back to front by origin depth),
    /// then every ribbon.  Pushes one `Particles` trace item per drawn layer
    /// and one `Ribbons` item when any ribbon is live.
    pub fn draw(
        &mut self,
        gfx: &mut Gfx,
        camera: &Mat4,
        settings: &RenderSettings,
        mut trace: Option<&mut TraceCollector>,
    ) {
        for g in self.graveyard.drain(..) {
            g.delete(&gfx.gl);
        }
        if self.instances.is_empty() {
            return;
        }
        let t = self.local_time();
        let world_matrix = iso_camera_matrix();
        let mut order: Vec<usize> = (0..self.instances.len()).collect();
        order.sort_by(|&a, &b| {
            let da = iso_view_depth(self.instances[a].frame.pos);
            let db = iso_view_depth(self.instances[b].frame.pos);
            da.total_cmp(&db).then(self.instances[a].id.cmp(&self.instances[b].id))
        });
        self.ribbon_verts.clear();
        // Two phases, each back to front by instance origin: every alpha
        // (absorbing) layer first, then every glowing layer (additive or
        // emissive).  Particles write no depth, so draw order decides who
        // covers whom; a flame must add its light over the smoke and dust
        // around it, never be composited under them (fire over smoke).
        for glow_phase in [false, true] {
            for &ii in &order {
                let inst = &mut self.instances[ii];
                for l in &mut inst.layers {
                    match l {
                        LayerRt::Particles(p) => {
                            let LayerDef::Particles(def) = &inst.effect.layers[p.def_index] else {
                                continue;
                            };
                            if is_glow(def) != glow_phase {
                                continue;
                            }
                            if p.ring.last_death() < t {
                                continue;
                            }
                            let gpu = p.gpu.get_or_insert_with(|| {
                                ParticleGpu::new(&gfx.gl, p.ring.capacity())
                            });
                            gpu.upload(&gfx.gl, &mut p.ring);
                            let u =
                                layer_uniforms(def, inst.frame.scale, inst.tint, inst.intensity, t);
                            if let Some(tr) = trace.as_mut() {
                                let name = format!("{}/{}", inst.name, def.name);
                                tr.push(TraceItemParams {
                                    order: iso_view_depth(inst.frame.pos),
                                    kind: "Particles",
                                    name: &name,
                                    model: &Mat4::from_translation(inst.frame.pos),
                                    camera_ignored: false,
                                    texture: def.render.texture.as_deref(),
                                    frame: Some(p.ring.live_count(t) as f32),
                                    color: None,
                                    depth: None,
                                    normal: None,
                                    screen: None,
                                });
                            }
                            gfx.draw_particles(gpu, camera, &world_matrix, settings, &u);
                        }
                        LayerRt::Ribbon(r) => {
                            let LayerDef::Ribbon(def) = &inst.effect.layers[r.def_index] else {
                                continue;
                            };
                            if !glow_phase || r.strands.is_empty() {
                                continue;
                            }
                            let tint = [
                                def.color[0] * inst.tint[0],
                                def.color[1] * inst.tint[1],
                                def.color[2] * inst.tint[2],
                            ];
                            build_vertices(
                                &r.strands,
                                def.width * inst.frame.scale,
                                def.glow * inst.frame.scale,
                                def.intensity * inst.intensity * r.flicker,
                                tint,
                                1.0,
                                &mut self.ribbon_verts,
                            );
                        }
                        LayerRt::Light(_) => {}
                    }
                }
            }
        }
        if !self.ribbon_verts.is_empty() {
            let count = (self.ribbon_verts.len() / RIBBON_VERTEX_FLOATS) as i32;
            let vb = self.ribbon_vb.get_or_insert_with(|| DynamicVb::new(&gfx.gl, 1 << 16));
            vb.upload(&gfx.gl, &self.ribbon_verts);
            if let Some(tr) = trace.as_mut() {
                tr.push(TraceItemParams {
                    order: 0.0,
                    kind: "Ribbons",
                    name: "ribbons",
                    model: &Mat4::IDENTITY,
                    camera_ignored: false,
                    texture: None,
                    frame: Some(count as f32),
                    color: None,
                    depth: None,
                    normal: None,
                    screen: None,
                });
            }
            gfx.draw_ribbons(vb, count, camera);
        }
    }
}

#[cfg(test)]
mod tests;
