//! GL-free particle-system tests over `Engine::new_for_test()` (the GPU side —
//! ring upload ranges — is covered in `classic-gfx`).

use classic_core::components::{DebugName, Emitter, Role, Tilemap};
use classic_core::effects::EffectLibrary;
use classic_core::{RoleKind, Transform};
use glam::Vec3;

use super::*;

const LIB: &str = r#"{"effects":[
  {"name":"stream","layers":[{"type":"particles","name":"p","max":4096,
     "spawn":{"rate":100},"velocity":{"up":[1,2]},"life":[1,1]}]},
  {"name":"pop","layers":[
     {"type":"particles","name":"p","max":256,"spawn":{"burst":12},"life":[0.5,0.5]},
     {"type":"light","name":"flash","duration":0.25,"peak":2}]},
  {"name":"rain","layers":[{"type":"particles","name":"drop","max":64,
     "spawn":{"burst":8},"velocity":{"up":[1,1]},"life":[10,10],
     "ground":{"mode":"die","sub_effect":"pop","sub_chance":1}}]},
  {"name":"zap","layers":[{"type":"ribbon","name":"a","mode":"arc","end":[2,0,0]}]},
  {"name":"torch","layers":[
     {"type":"particles","name":"p","max":4096,"spawn":{"rate":100},"life":[1,1]},
     {"type":"light","name":"glow","peak":2,"duration":null},
     {"type":"ribbon","name":"a","mode":"arc","end":[2,0,0]}]}
]}"#;

/// A test engine with a flat 32×32 tilemap at height 1 and the test library.
fn engine() -> Engine {
    let mut e = Engine::new_for_test();
    let n = 32;
    let tm = Tilemap {
        position: Vec3::ZERO,
        scale: Vec3::new(45.0, 45.0, 1.0),
        size_x: n,
        size_y: n,
        tile_set: "tileset".into(),
        tile_pixel_size: [32, 32],
        max_tile: 16,
        tiles_grid: None,
        heights_grid: None,
        data: vec![0; (n * n) as usize],
        height_data: vec![1.0; ((n + 1) * (n + 1)) as usize],
        height_scale: 14.0,
        tile_set_pixel_size: [0, 0],
        tiles_per_row: 0,
        mouse_iso_pos: Vec3::ZERO,
        selection_iso_begin: Vec3::splat(-1.0),
        selection_iso_end: Vec3::splat(-1.0),
        blend: None,
    };
    let ent = e.world.spawn((
        Transform::new(Vec3::ZERO, Vec3::ONE),
        tm,
        Role { value: RoleKind::Tilemap },
        DebugName("tilemap".into()),
    ));
    e.names.insert("tilemap".into(), ent);
    e.register_effects("t", EffectLibrary::from_json(LIB.as_bytes()).unwrap());
    e
}

fn spawn_emitter(e: &mut Engine, name: &str, em: Emitter) -> hecs::Entity {
    let ent = e.world.spawn((em, DebugName(name.into())));
    e.names.insert(name.into(), ent);
    ent
}

fn run(e: &mut Engine, frames: usize, dt: f32) {
    for _ in 0..frames {
        e.update_particles(dt);
    }
}

/// Every live record across all instances, in a stable order.
fn records(e: &Engine) -> Vec<classic_gfx::ParticleRecord> {
    let mut out = Vec::new();
    let mut insts: Vec<&Instance> = e.particles.instances.iter().collect();
    insts.sort_by_key(|i| i.id);
    for inst in insts {
        for l in &inst.layers {
            if let LayerRt::Particles(p) = l {
                out.extend(p.ring.records().iter().copied().filter(|r| r.life > 0.0));
            }
        }
    }
    out
}

#[test]
fn no_emitters_no_instances() {
    let mut e = engine();
    run(&mut e, 30, 1.0 / 60.0);
    assert_eq!(e.particle_stats(), ParticleStats::default());
}

#[test]
fn rate_spawning_matches_rate_and_lifetime() {
    let mut e = engine();
    spawn_emitter(&mut e, "s", Emitter::new("t::stream", Vec3::new(5.0, -5.0, 1.0)));
    run(&mut e, 120, 1.0 / 60.0); // 2 s at 100/s, life 1 s
    let s = e.particle_stats();
    assert_eq!(s.instances, 1);
    assert!((95..=105).contains(&s.live), "live {}", s.live);
    // Births are spread across each frame (no clumping on frame boundaries).
    let t = e.particles.local_time();
    let mut births: Vec<f32> =
        records(&e).iter().map(|r| r.t_birth).filter(|b| *b > t - 0.5).collect();
    births.sort_by(f32::total_cmp);
    births.dedup();
    assert!(births.len() > 45, "births collapse onto frame times: {}", births.len());
}

#[test]
fn spawn_stream_is_deterministic() {
    let mk = || {
        let mut e = engine();
        spawn_emitter(&mut e, "s", Emitter::new("t::stream", Vec3::new(5.0, -5.0, 1.0)));
        run(&mut e, 45, 1.0 / 60.0);
        records(&e)
    };
    let a = mk();
    assert!(!a.is_empty());
    assert_eq!(a, mk());
}

#[test]
fn rate_multiplier_and_disable_gate_spawning() {
    let mut e = engine();
    let mut em = Emitter::new("t::stream", Vec3::new(5.0, -5.0, 1.0));
    em.rate = 0.0;
    let ent = spawn_emitter(&mut e, "s", em);
    run(&mut e, 30, 1.0 / 60.0);
    assert_eq!(e.particle_stats().live, 0);
    e.world.get::<&mut Emitter>(ent).unwrap().rate = 2.0;
    run(&mut e, 30, 1.0 / 60.0);
    let live = e.particle_stats().live;
    assert!((95..=105).contains(&live), "2x rate over 0.5 s: {live}");
    e.world.get::<&mut Emitter>(ent).unwrap().enabled = false;
    run(&mut e, 90, 1.0 / 60.0);
    assert_eq!(e.particle_stats().live, 0, "disabled emitters stop spawning");
    assert_eq!(e.particle_stats().instances, 1, "the instance outlives a pause");
}

#[test]
fn burst_counter_fires_per_integer_step_and_on_wrap() {
    let mut e = engine();
    let ent = spawn_emitter(&mut e, "p", Emitter::new("t::pop", Vec3::new(5.0, -5.0, 1.0)));
    run(&mut e, 1, 1.0 / 60.0);
    assert_eq!(e.particle_stats().live, 0, "the initial counter value fires nothing");
    e.world.get::<&mut Emitter>(ent).unwrap().burst = 2.4;
    run(&mut e, 1, 1.0 / 60.0);
    assert_eq!(e.particle_stats().live, 24, "two steps crossed");
    e.world.get::<&mut Emitter>(ent).unwrap().burst = 2.9;
    run(&mut e, 1, 1.0 / 60.0);
    assert_eq!(e.particle_stats().live, 24, "no integer crossed");
    // Loop wrap: the counter restarts from 0 and climbs to 1.2.
    e.world.get::<&mut Emitter>(ent).unwrap().burst = 1.2;
    run(&mut e, 1, 1.0 / 60.0);
    assert_eq!(e.particle_stats().live, 36);
}

#[test]
fn one_shots_fire_light_layers_and_retire() {
    let mut e = engine();
    assert!(e.fx_emit("pop", Vec3::new(5.0, -5.0, 1.0), Vec3::Z, 1.0), "bare names resolve");
    assert!(!e.fx_emit("nope", Vec3::ZERO, Vec3::Z, 1.0));
    run(&mut e, 1, 1.0 / 60.0);
    assert_eq!(e.particle_stats().live, 12);
    assert_eq!(e.world.query::<&Light>().iter().count(), 1, "flash light spawned");
    run(&mut e, 20, 1.0 / 60.0); // 0.35 s: light over, particles alive
    assert_eq!(e.world.query::<&Light>().iter().count(), 0, "flash light released");
    run(&mut e, 30, 1.0 / 60.0);
    assert_eq!(e.particle_stats().instances, 0, "finished one-shots retire");
}

#[test]
fn dying_particles_fire_sub_effects_where_they_land() {
    let mut e = engine();
    e.fx_emit("t::rain", Vec3::new(5.0, -5.0, 1.0), Vec3::Z, 1.0);
    // Up at 1 m/s from ground height 1: lands after 2/1.62 ≈ 1.23 s.
    run(&mut e, 60, 1.0 / 60.0);
    assert_eq!(e.particle_stats().instances, 1);
    run(&mut e, 20, 1.0 / 60.0);
    // 8 landings → 8 `pop` instances (their lights included).
    assert_eq!(e.particle_stats().instances, 9);
    let pops: Vec<Vec3> = e
        .particles
        .instances
        .iter()
        .filter(|i| i.effect.name == "pop")
        .map(|i| i.frame.pos)
        .collect();
    assert!(pops.iter().all(|p| (p.z - 1.0).abs() < 1e-4), "pops on the ground: {pops:?}");
}

#[test]
fn parented_emitters_follow_their_parent() {
    let mut e = engine();
    let parent = e.world.spawn((Transform::new(Vec3::new(4.0, 4.0, 0.0), Vec3::ONE),));
    e.names.insert("carrier".into(), parent);
    let mut em = Emitter::new("t::stream", Vec3::new(0.0, 0.0, 2.0));
    em.parent = Some("carrier".into());
    spawn_emitter(&mut e, "s", em);
    run(&mut e, 2, 1.0 / 60.0);
    let base = e.iso_to_world(4.0, 4.0, 0.0).unwrap();
    let pos = e.particles.instances[0].frame.pos;
    assert!((pos - (base + Vec3::new(0.0, 0.0, 2.0))).length() < 1e-4, "{pos} vs {base}");
    // A dangling parent stops emission instead of teleporting the emitter.
    let mut em = Emitter::new("t::stream", Vec3::ZERO);
    em.parent = Some("ghost".into());
    spawn_emitter(&mut e, "s2", em);
    run(&mut e, 10, 1.0 / 60.0);
    let ghost = e.particles.instances.iter().find(|i| i.name == "s2").unwrap();
    assert!(!ghost.emitting);
}

#[test]
fn despawned_emitters_finish_and_retire() {
    let mut e = engine();
    let ent = spawn_emitter(&mut e, "s", Emitter::new("t::stream", Vec3::new(5.0, -5.0, 1.0)));
    run(&mut e, 30, 1.0 / 60.0);
    e.world.despawn(ent).unwrap();
    run(&mut e, 10, 1.0 / 60.0);
    assert_eq!(e.particle_stats().instances, 1, "live particles finish");
    run(&mut e, 60, 1.0 / 60.0);
    assert_eq!(e.particle_stats().instances, 0);
}

#[test]
fn arcs_regenerate_deterministically() {
    let mut e = engine();
    spawn_emitter(&mut e, "z", Emitter::new("t::zap", Vec3::new(5.0, -5.0, 2.0)));
    run(&mut e, 3, 1.0 / 60.0);
    assert_eq!(e.particle_stats().ribbons, 1);
    let shape = |e: &Engine| match &e.particles.instances[0].layers[0] {
        LayerRt::Ribbon(r) => r.strands.clone(),
        _ => unreachable!(),
    };
    let a = shape(&e);
    assert_eq!(a[0].points[0], Vec3::new(5.0, -5.0, 2.0));
    assert_eq!(*a[0].points.last().unwrap(), Vec3::new(7.0, -5.0, 2.0));
    let mut e2 = engine();
    spawn_emitter(&mut e2, "z", Emitter::new("t::zap", Vec3::new(5.0, -5.0, 2.0)));
    run(&mut e2, 3, 1.0 / 60.0);
    assert_eq!(a, shape(&e2));
}

#[test]
fn clock_rebase_keeps_particles_alive() {
    let mut e = engine();
    spawn_emitter(&mut e, "s", Emitter::new("t::stream", Vec3::new(5.0, -5.0, 1.0)));
    // Jump the clock close to the epoch span, then cross it.
    e.particles.clock = EPOCH_SPAN - 0.5;
    run(&mut e, 60, 1.0 / 60.0);
    let before = e.particle_stats().live;
    assert!(e.particles.epoch > 0.0, "rebased");
    assert!(e.particles.local_time() < 100.0);
    run(&mut e, 1, 1.0 / 60.0);
    let after = e.particle_stats().live;
    assert!(before > 50 && (after as i64 - before as i64).abs() < 5, "{before} → {after}");
}

#[test]
fn layer_uniforms_scale_motion_and_tint() {
    let lib = EffectLibrary::from_json(
        br#"{"effects":[{"name":"x","layers":[{"type":"particles",
            "motion":{"gravity":2,"drag":0.5,"swirl":{"kick_radius":4,"lift":3}},
            "render":{"mode":"streak","blend":"additive","emissive":2}}]}]}"#,
    )
    .unwrap();
    let LayerDef::Particles(def) = &lib.effects[0].layers[0] else { panic!() };
    let u = layer_uniforms(def, 2.0, [0.5, 1.0, 1.0], 1.5, 3.0);
    assert!((u.motion[2] + LUNAR_G * 2.0 * 2.0).abs() < 1e-5);
    assert_eq!(u.motion[3], 0.5);
    assert_eq!(u.swirl0[1], 8.0);
    assert_eq!(u.swirl1[0], 6.0);
    assert_eq!(u.render[0], 1.0);
    assert_eq!(u.shape[2], 1.0);
    assert_eq!(u.shade[1], 3.0);
    assert_eq!(u.tint, [0.5, 1.0, 1.0]);
    assert_eq!(u.time, 3.0);
}

#[test]
fn burst_driven_flash_lights_fire_per_burst_not_forever() {
    let mut e = engine();
    let ent = spawn_emitter(&mut e, "p", Emitter::new("t::pop", Vec3::new(5.0, -5.0, 1.0)));
    run(&mut e, 10, 1.0 / 60.0);
    assert_eq!(e.world.query::<&Light>().iter().count(), 0, "no flash before a burst");
    e.world.get::<&mut Emitter>(ent).unwrap().burst = 1.0;
    run(&mut e, 1, 1.0 / 60.0);
    assert_eq!(e.world.query::<&Light>().iter().count(), 1, "the burst fires the flash");
    run(&mut e, 30, 1.0 / 60.0);
    assert_eq!(e.world.query::<&Light>().iter().count(), 0, "the 0.25 s flash ends");
}

#[test]
fn guest_emitter_handles_spawn_set_and_release() {
    let mut e = engine();
    assert_eq!(e.spawn_emitter(Emitter::new("nope", Vec3::ZERO)), None, "unknown effect");
    let h = e.spawn_emitter(Emitter::new("t::stream", Vec3::new(5.0, -5.0, 1.0))).unwrap();
    run(&mut e, 30, 1.0 / 60.0);
    assert!(e.particle_stats().live > 0);

    let mut em = e.emitter_by_handle(h).unwrap();
    em.position = Vec3::new(8.0, -8.0, 1.0);
    em.rate = 0.0;
    assert!(e.update_emitter(h, em));
    assert_eq!(e.emitter_by_handle(h).unwrap().position.x, 8.0);
    run(&mut e, 70, 1.0 / 60.0);
    assert_eq!(e.particle_stats().live, 0, "rate 0 pauses spawning");
    assert_eq!(e.particle_stats().instances, 1);

    assert!(e.release_emitter(h));
    assert!(!e.release_emitter(h), "double release");
    assert!(!e.update_emitter(h, Emitter::new("t::stream", Vec3::ZERO)));
    assert!(e.emitter_by_handle(h).is_none());
}

#[test]
fn recycled_emitter_handles_get_fresh_instances() {
    // A released emitter keeps its instance until its particles die; the
    // next spawn recycles the handle (and the entity index) but must drive a
    // fresh instance, not the dying one.
    let mut e = engine();
    let a = e.spawn_emitter(Emitter::new("t::stream", Vec3::new(5.0, -5.0, 1.0))).unwrap();
    run(&mut e, 30, 1.0 / 60.0);
    assert!(e.release_emitter(a));
    let b = e.spawn_emitter(Emitter::new("t::stream", Vec3::new(9.0, -9.0, 1.0))).unwrap();
    assert_ne!(a, b, "the slot is recycled under a new generation");
    run(&mut e, 1, 1.0 / 60.0);
    assert_eq!(e.particle_stats().instances, 2, "the dying instance and the new one");
    run(&mut e, 120, 1.0 / 60.0);
    let s = e.particle_stats();
    assert_eq!(s.instances, 1, "only the dead emitter's instance retires");
    assert!(s.live > 0, "the new emitter keeps spawning");
}

#[test]
fn stale_emitter_handle_after_release_is_rejected() {
    let mut e = engine();
    let stale = e.spawn_emitter(Emitter::new("t::stream", Vec3::new(5.0, -5.0, 1.0))).unwrap();
    assert!(e.release_emitter(stale));
    let fresh = e.spawn_emitter(Emitter::new("t::stream", Vec3::new(9.0, -9.0, 1.0))).unwrap();

    assert!(e.emitter_by_handle(stale).is_none());
    assert!(!e.update_emitter(stale, Emitter::new("t::stream", Vec3::ZERO)), "stale set");
    assert!(!e.release_emitter(stale), "stale release");
    assert_eq!(e.emitter_by_handle(fresh).unwrap().position, Vec3::new(9.0, -9.0, 1.0));
}

#[test]
fn guest_emitter_pool_is_bounded() {
    let mut e = engine();
    for _ in 0..MAX_EMITTER_HANDLES {
        assert!(e.spawn_emitter(Emitter::new("t::stream", Vec3::ZERO)).is_some());
    }
    assert_eq!(e.spawn_emitter(Emitter::new("t::stream", Vec3::ZERO)), None);
}

/// A flat (mesh-less) glb asset: a `rig` root plus `nodes` (name, glTF-space
/// translation) as its children, registered as `name`.
fn insert_model(e: &mut Engine, name: &str, nodes: &[(&str, Vec3)]) {
    use classic_core::model::{ModelAsset, ModelNode};
    let node = |name: &str, parent: Option<usize>, t: Vec3| ModelNode {
        name: name.into(),
        parent,
        children: Vec::new(),
        translation: t,
        rotation: glam::Quat::IDENTITY,
        scale: Vec3::ONE,
        mesh: None,
    };
    let mut all = vec![node("rig", None, Vec3::ZERO)];
    for (n, t) in nodes {
        all.push(node(n, Some(0), *t));
    }
    all[0].children = (1..all.len()).collect();
    e.models.insert(name.into(), ModelAsset { nodes: all, root: Some(0), ..Default::default() });
}

/// A `rocket` Model entity on `landing` (three `fx_nozzle_*` nodes 3 m
/// apart, an `fx_pad` and an unrelated `fin`) plus a `launch` asset with
/// two nozzles, posed for frame 0.
fn rocket(e: &mut Engine) -> hecs::Entity {
    insert_model(
        e,
        "landing",
        &[
            ("fx_nozzle_0", Vec3::new(0.0, 0.0, 0.0)),
            ("fin", Vec3::new(0.0, 4.0, 0.0)),
            ("fx_nozzle_1", Vec3::new(3.0, 0.0, 0.0)),
            ("fx_nozzle_2", Vec3::new(-3.0, 0.0, 0.0)),
            ("fx_pad", Vec3::ZERO),
        ],
    );
    insert_model(
        e,
        "launch",
        &[("fx_nozzle_0", Vec3::new(0.0, 0.0, 3.0)), ("fx_nozzle_1", Vec3::new(0.0, 0.0, -3.0))],
    );
    let r = e.world.spawn((
        Transform::new(Vec3::new(8.0, 8.0, 0.0), Vec3::ONE),
        classic_core::components::Model::new("landing", "tilemap"),
        DebugName("rocket".into()),
    ));
    e.names.insert("rocket".into(), r);
    e.update_models(0.0);
    r
}

fn plume(node: &str) -> Emitter {
    let mut em = Emitter::new("t::stream", Vec3::ZERO);
    em.parent = Some("rocket".into());
    em.node = Some(node.into());
    em
}

/// World position of a rocket node (the engine's own exact-name resolve).
fn node_pos(e: &Engine, node: &str) -> Vec3 {
    e.attachment_frame(Some("rocket"), Some(node), Vec3::ZERO, Vec3::Z).unwrap().0
}

/// Live records whose spawn point is within 1 cm of `p` (horizontally: a
/// `stream` particle spawns on its site with an upward velocity).
fn born_at(recs: &[classic_gfx::ParticleRecord], p: Vec3) -> usize {
    recs.iter().filter(|r| Vec3::from_array(r.p0).truncate().distance(p.truncate()) < 0.01).count()
}

#[test]
fn node_pattern_spawns_from_every_matching_node() {
    let mut e = engine();
    rocket(&mut e);
    spawn_emitter(&mut e, "plume", plume("fx_nozzle_*"));
    run(&mut e, 60, 1.0 / 60.0);
    let inst = &e.particles.instances[0];
    let want: Vec<Vec3> =
        ["fx_nozzle_0", "fx_nozzle_1", "fx_nozzle_2"].iter().map(|n| node_pos(&e, n)).collect();
    let got: Vec<Vec3> = inst.sites.iter().map(|s| s.pos).collect();
    assert_eq!(got.len(), 3, "matched {got:?}");
    for (g, w) in got.iter().zip(&want) {
        assert!(g.distance(*w) < 1e-4, "{g} vs {w}");
    }
    // The instance frame is their centroid (what lights / review framing use).
    let c = want.iter().sum::<Vec3>() / 3.0;
    assert!(inst.frame.pos.distance(c) < 1e-4);
    assert!(e.named_world_point("plume").unwrap().distance(c) < 1e-4);
    // Every particle is born on one of the nozzles, split evenly, and each
    // nozzle runs the layer's full rate (100/s, life 1 s: ~100 live each).
    let recs = records(&e);
    assert!((285..=315).contains(&recs.len()), "live {}", recs.len());
    let counts: Vec<usize> = want.iter().map(|w| born_at(&recs, *w)).collect();
    assert_eq!(counts.iter().sum::<usize>(), recs.len(), "{counts:?} of {}", recs.len());
    let (lo, hi) = (counts.iter().min().unwrap(), counts.iter().max().unwrap());
    assert!(hi - lo <= 1 && *lo > 90, "uneven split {counts:?}");
}

#[test]
fn node_pattern_bursts_fire_on_every_node() {
    let mut e = engine();
    rocket(&mut e);
    let mut em = plume("fx_nozzle_*");
    em.effect = "t::pop".into();
    let pe = spawn_emitter(&mut e, "pops", em);
    run(&mut e, 2, 1.0 / 60.0);
    e.world.get::<&mut Emitter>(pe).unwrap().burst += 1.0;
    run(&mut e, 1, 1.0 / 60.0);
    // `pop` bursts 12: one burst per node.
    let recs = records(&e);
    let want = ["fx_nozzle_0", "fx_nozzle_1", "fx_nozzle_2"].map(|n| node_pos(&e, n));
    assert_eq!(want.map(|w| born_at(&recs, w)), [12, 12, 12]);
}

#[test]
fn exact_node_names_still_attach_to_one_node() {
    let mut e = engine();
    rocket(&mut e);
    spawn_emitter(&mut e, "plume", plume("fx_nozzle_1"));
    run(&mut e, 30, 1.0 / 60.0);
    let inst = &e.particles.instances[0];
    assert!(inst.sites.is_empty());
    let p = node_pos(&e, "fx_nozzle_1");
    assert!(inst.frame.pos.distance(p) < 1e-4);
    let recs = records(&e);
    assert_eq!(born_at(&recs, p), recs.len());
    // A one-node pattern is the same single frame.
    let mut e2 = engine();
    rocket(&mut e2);
    spawn_emitter(&mut e2, "plume", plume("fx_pad*"));
    run(&mut e2, 30, 1.0 / 60.0);
    assert!(e2.particles.instances[0].sites.is_empty());
    assert!(e2.particles.instances[0].frame.pos.distance(node_pos(&e2, "fx_pad")) < 1e-4);
}

#[test]
fn node_pattern_spawning_is_deterministic() {
    let mk = || {
        let mut e = engine();
        rocket(&mut e);
        spawn_emitter(&mut e, "plume", plume("fx_nozzle_*"));
        let mut b = plume("fx_nozzle_*");
        b.effect = "t::pop".into();
        spawn_emitter(&mut e, "pops", b);
        for i in 0..45 {
            if i % 10 == 0 {
                let pe = e.names["pops"];
                e.world.get::<&mut Emitter>(pe).unwrap().burst += 1.0;
            }
            e.update_particles(1.0 / 60.0);
        }
        records(&e)
    };
    let a = mk();
    assert!(!a.is_empty());
    assert_eq!(a, mk());
}

#[test]
fn node_pattern_re_resolves_when_the_glb_changes() {
    let mut e = engine();
    let r = rocket(&mut e);
    spawn_emitter(&mut e, "plume", plume("fx_nozzle_*"));
    run(&mut e, 30, 1.0 / 60.0);
    assert_eq!(e.particles.instances[0].sites.len(), 3);
    // Swap the model to the two-nozzle launch glb (what `start_model_clip`
    // does), pose it, and keep emitting.
    e.world.get::<&mut classic_core::components::Model>(r).unwrap().model = "launch".into();
    e.update_models(0.0);
    let t_swap = e.particles.local_time();
    run(&mut e, 30, 1.0 / 60.0);
    let inst = &e.particles.instances[0];
    assert_eq!(inst.sites.len(), 2);
    let want = [node_pos(&e, "fx_nozzle_0"), node_pos(&e, "fx_nozzle_1")];
    for (s, w) in inst.sites.iter().zip(&want) {
        assert!(s.pos.distance(*w) < 1e-4, "{} vs {w}", s.pos);
    }
    // Births after the swap come from the new nodes only (no lerp from the
    // old set's positions).
    let recs: Vec<_> = records(&e).into_iter().filter(|r| r.t_birth > t_swap).collect();
    assert!(!recs.is_empty());
    assert_eq!(born_at(&recs, want[0]) + born_at(&recs, want[1]), recs.len());
}

#[test]
fn unmatched_node_pattern_falls_back_to_the_parent_ground_point() {
    let mut e = engine();
    rocket(&mut e);
    spawn_emitter(&mut e, "plume", plume("no_such_*"));
    run(&mut e, 10, 1.0 / 60.0);
    let inst = &e.particles.instances[0];
    assert!(inst.sites.is_empty() && inst.emitting);
    let base = e.attachment_frame(Some("rocket"), None, Vec3::ZERO, Vec3::Z).unwrap().0;
    assert!(inst.frame.pos.distance(base) < 1e-4);
}

fn lights(e: &Engine) -> usize {
    e.world.query::<&Light>().iter().count()
}

fn torch_inst(e: &Engine) -> &Instance {
    e.particles.instances.iter().find(|i| i.effect.name == "torch").unwrap()
}

fn ribbons_live(e: &Engine) -> bool {
    torch_inst(e).layers.iter().any(|l| matches!(l, LayerRt::Ribbon(r) if !r.strands.is_empty()))
}

#[test]
fn hidden_fade_ramps_linearly_to_zero() {
    assert_eq!(hidden_fade(None, 10.0), 1.0);
    assert_eq!(hidden_fade(Some(10.0), 10.0), 1.0);
    assert!((hidden_fade(Some(10.0), 10.0 + HIDDEN_FADE_S as f64 * 0.5) - 0.5).abs() < 1e-5);
    assert_eq!(hidden_fade(Some(10.0), 10.0 + HIDDEN_FADE_S as f64), 0.0);
    assert_eq!(hidden_fade(Some(10.0), 99.0), 0.0);
}

/// Hide the emitter (`hide_parent`: its parent instead) after 1 s of a
/// steady torch and check the fade, the ring clear, the ribbon stop, the
/// light release and the reset on un-hide.
fn hidden_torch_fades_out(hide_parent: bool) {
    let mut e = engine();
    let parent = e.world.spawn((
        Transform::new(Vec3::new(4.0, 4.0, 0.0), Vec3::ONE),
        classic_core::components::Model::new("none", "tilemap"),
    ));
    e.names.insert("building".into(), parent);
    let mut em = Emitter::new("t::torch", Vec3::new(0.0, 0.0, 2.0));
    em.parent = Some("building".into());
    let ent = spawn_emitter(&mut e, "torch", em);
    let dt = 1.0 / 60.0;
    run(&mut e, 60, dt);
    let s = (e.particle_stats().live, lights(&e), ribbons_live(&e));
    assert!(s.0 > 90 && s.1 == 1 && s.2, "steady torch: {s:?}");

    let hidden = if hide_parent { parent } else { ent };
    e.world.insert_one(hidden, classic_core::components::Disabled).unwrap();
    run(&mut e, 1, dt);
    let at = torch_inst(&e).hidden_at.expect("hidden");
    assert_eq!(lights(&e), 0, "lights go out at once");
    assert!(!ribbons_live(&e), "ribbons stop at once");
    let live = e.particle_stats().live;
    assert!(live > 80, "particles stay for the fade: {live}");
    run(&mut e, 6, dt); // 0.1 s in
    let f = hidden_fade(Some(at), e.particles.clock);
    assert!((f - (1.0 - 6.0 * dt / HIDDEN_FADE_S)).abs() < 1e-3, "fading: {f}");
    run(&mut e, 10, dt); // past 0.25 s, well inside the 1 s particle life
    assert_eq!(e.particle_stats().live, 0, "rings cleared after the fade");
    assert_eq!(e.particle_stats().instances, 1, "the instance outlives the hide");

    e.world.remove_one::<classic_core::components::Disabled>(hidden).unwrap();
    run(&mut e, 30, dt);
    let inst = torch_inst(&e);
    assert!(inst.hidden_at.is_none(), "un-hide resets the fade");
    assert_eq!(hidden_fade(inst.hidden_at, e.particles.clock), 1.0);
    assert!(e.particle_stats().live > 40 && lights(&e) == 1 && ribbons_live(&e));
}

#[test]
fn hidden_emitter_fades_out_and_releases_its_lights() {
    hidden_torch_fades_out(false);
}

#[test]
fn hidden_parent_model_fades_its_emitters_out() {
    hidden_torch_fades_out(true);
}

#[test]
fn disabled_emitter_lets_particles_die_naturally() {
    let mut e = engine();
    let ent = spawn_emitter(&mut e, "torch", Emitter::new("t::torch", Vec3::new(5.0, -5.0, 1.0)));
    let dt = 1.0 / 60.0;
    run(&mut e, 60, dt);
    e.world.get::<&mut Emitter>(ent).unwrap().enabled = false;
    run(&mut e, 30, dt); // 0.5 s: past a fade, inside the 1 s life
    assert!(torch_inst(&e).hidden_at.is_none(), "`enabled = false` is not hiding");
    let live = e.particle_stats().live;
    assert!((40..=60).contains(&live), "the last 0.5 s of births live on: {live}");
    run(&mut e, 32, dt);
    assert_eq!(e.particle_stats().live, 0, "then die of old age");
}

#[test]
fn fixed_radius_light_layers_ignore_the_instance_scale() {
    let mut e = engine();
    let lib = r#"{"effects":[
      {"name":"lamp","layers":[{"type":"light","name":"l","duration":null,"radius":6,
         "offset":[0,0,1],"fixed_radius":true}]},
      {"name":"bulb","layers":[{"type":"light","name":"l","duration":null,"radius":6,
         "offset":[0,0,1]}]}]}"#;
    e.register_effects("k", EffectLibrary::from_json(lib.as_bytes()).unwrap());
    let origin = Vec3::new(5.0, -5.0, 1.0);
    for (fx, scale) in [("lamp", 0.5), ("lamp", 2.0), ("bulb", 0.5), ("bulb", 2.0)] {
        let mut em = Emitter::new(format!("k::{fx}"), origin);
        em.scale = scale;
        spawn_emitter(&mut e, &format!("{fx}{scale}"), em);
    }
    run(&mut e, 2, 1.0 / 60.0);
    let mut got: Vec<(f32, f32)> = e
        .world
        .query::<&Light>()
        .iter()
        .map(|(_, l)| (l.position.z - origin.z, l.radius))
        .collect();
    got.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    // Offsets scale for both; only the unfixed radius scales.
    assert_eq!(got, [(0.5, 3.0), (0.5, 6.0), (2.0, 6.0), (2.0, 12.0)]);
}

/// `index` points every id at its (first) instance.
fn assert_indexed(e: &Engine) {
    let ps = &e.particles;
    assert_eq!(ps.index.len(), ps.instances.len(), "one entry per instance");
    for (i, inst) in ps.instances.iter().enumerate() {
        assert_eq!(ps.index.get(&inst.id), Some(&i), "{}", inst.name);
    }
}

#[test]
fn instance_index_survives_retirement() {
    let mut e = engine();
    let ent = spawn_emitter(&mut e, "p", Emitter::new("t::pop", Vec3::new(5.0, -5.0, 1.0)));
    for k in 0..6 {
        e.fx_emit("pop", Vec3::new(4.0 + k as f32, -5.0, 1.0), Vec3::Z, 1.0);
        run(&mut e, 7, 1.0 / 60.0);
        assert_indexed(&e);
    }
    run(&mut e, 60, 1.0 / 60.0);
    assert_eq!(e.particle_stats().instances, 1, "the one-shots retired");
    assert_indexed(&e);
    // The surviving emitter instance is still found by id (burst lands).
    e.world.get::<&mut Emitter>(ent).unwrap().burst = 1.0;
    run(&mut e, 1, 1.0 / 60.0);
    assert_eq!(e.particle_stats().live, 12);
}

#[test]
fn budget_scale_is_one_until_the_budget_then_inverse() {
    assert_eq!(budget_scale(0), 1.0);
    assert_eq!(budget_scale(PARTICLE_BUDGET), 1.0);
    assert_eq!(budget_scale(PARTICLE_BUDGET * 2), 0.5);
    assert_eq!(budget_scale(PARTICLE_BUDGET * 4), 0.25);
}

/// Three 40k/s, 1 s-life streams: 120k resident unbudgeted.  The soft
/// budget settles where `R = 120k * B / R` (~88.7k); bursts are untouched.
#[test]
fn over_budget_scales_continuous_spawning_deterministically() {
    let lib = r#"{"effects":[
      {"name":"flood","layers":[{"type":"particles","name":"p","max":65536,
         "spawn":{"rate":40000},"life":[1,1]}]},
      {"name":"pop","layers":[{"type":"particles","name":"p","max":4096,
         "spawn":{"burst":3000},"life":[1,1]}]}]}"#;
    let mk = || {
        let mut e = engine();
        e.register_effects("b", EffectLibrary::from_json(lib.as_bytes()).unwrap());
        for k in 0..3 {
            let em = Emitter::new("b::flood", Vec3::new(4.0 + k as f32, -5.0, 1.0));
            spawn_emitter(&mut e, &format!("flood{k}"), em);
        }
        let pop = spawn_emitter(&mut e, "pop", Emitter::new("b::pop", Vec3::new(8.0, -5.0, 1.0)));
        run(&mut e, 150, 1.0 / 60.0);
        (e, pop)
    };
    let (mut e, pop) = mk();
    let live = e.particle_stats().live;
    assert!((80_000..100_000).contains(&live), "settles over the soft budget: {live}");
    // A burst while over budget is never throttled.
    e.world.get::<&mut Emitter>(pop).unwrap().burst = 1.0;
    run(&mut e, 1, 1.0 / 60.0);
    let pop_live = e
        .particles
        .instances
        .iter()
        .find(|i| i.effect.name == "pop")
        .map(|i| match &i.layers[0] {
            LayerRt::Particles(p) => p.ring.live_count(e.particles.local_time()),
            _ => 0,
        })
        .unwrap();
    assert_eq!(pop_live, 3000);
    // Same inputs, same records.
    let (a, b) = (mk().0, mk().0);
    assert!(records(&a) == records(&b), "budgeted spawning is deterministic");
}

#[test]
fn map_bounds_match_the_ground_sampler() {
    let mut e = engine();
    run(&mut e, 1, 1.0 / 60.0);
    let b = e.particles.map.expect("a tilemap gives bounds");
    // 32 tiles of TILE_M from the origin, tile y running along world -y.
    let span = 32.0 * TILE_M;
    assert_eq!(b, [0.0, -span, span, 0.0]);
    let g = e.ground_sampler().unwrap();
    let eps = 1e-3;
    for (x, y) in [(b[0], b[1]), (b[2], b[3]), (b[0], b[3]), (b[2], b[1])] {
        assert!(g.at(x, y).is_some(), "corner ({x}, {y}) is on the map");
    }
    assert!(g.at(b[0] - eps, -1.0).is_none() && g.at(b[2] + eps, -1.0).is_none());
    assert!(g.at(1.0, b[1] - eps).is_none() && g.at(1.0, b[3] + eps).is_none());
    // An offset tilemap shifts the bounds with it.
    assert_eq!(
        map_bounds(Vec3::new(3.0, 4.0, 9.0), 2, 1),
        [3.0, 4.0 - TILE_M, 3.0 + 2.0 * TILE_M, 4.0]
    );
}

#[test]
fn no_tilemap_no_map_fade() {
    let mut e = Engine::new_for_test();
    e.update_particles(1.0 / 60.0);
    assert!(e.particles.map.is_none());
}

/// A vehicle socket one-shot (`vehicle_socket_fx`) is oriented by the full
/// socket frame: the ribbon tracer, the particle velocities and the light
/// offset all follow the turret yaw and gun pitch (they used to keep the
/// world axes: the EMRLV rail always fired East).
#[test]
fn socket_fx_follows_turret_yaw_and_gun_pitch() {
    let mut e = engine();
    e.register_effects(
        "fx",
        EffectLibrary::from_json(
            br#"{"effects":[{"name":"rail","layers":[
              {"type":"ribbon","name":"tracer","mode":"arc","duration":1.0,"end":[9,0,0],
               "depth":4,"jitter":0.06,"branches":0},
              {"type":"particles","name":"sparks","max":64,"spawn":{"burst":8},
               "velocity":{"speed":[5,5],"cone":0.0},"motion":{"gravity":0.0},"life":[1,1]},
              {"type":"light","name":"flash","duration":1.0,"peak":2,"offset":[0.3,0,0]}]}]}"#,
        )
        .unwrap(),
    );
    let def: classic_core::types::VehicleDef = serde_json::from_value(serde_json::json!({
        "version": 2, "name": "tank", "directions": 16,
        "layers": [
            { "name": "hull", "kind": "hull", "texture": "tankHull" },
            { "name": "turret", "kind": "turret", "texture": "tankTurret",
              "offset": [0.0, 0.0, 1.2], "pitch_pivot": [0.5, 0.0, 0.6],
              "pitch": { "levels": 4, "min_deg": -5.0, "max_deg": 25.0 } }
        ],
        "sockets": { "fx_muzzle_0": { "layer": "turret", "offset": [2.5, 0.0, 0.6] } }
    }))
    .unwrap();
    e.vehicles.insert("tank".into(), def);
    assert!(e.spawn_vehicle("tank", "tank", 16.0, 16.0));

    for yaw in [0.0f32, 90.0, 180.0, 270.0] {
        for pitch in [0.0f32, 15.0] {
            assert!(e.vehicle_aim_angles("tank", yaw, pitch));
            e.time.delta = 1.0 / 60.0;
            e.update_vehicles();
            // Heading 0: a tile-space (clockwise-on-screen) turret yaw is a
            // negative world turn about +Z; the gun pitch lifts it.
            let (y, p) = (-yaw.to_radians(), pitch.to_radians());
            let fwd = Vec3::new(y.cos() * p.cos(), y.sin() * p.cos(), p.sin());
            let (origin, basis) = e.vehicle_socket_frame("tank", "fx_muzzle_0").unwrap();
            assert!((basis.x_axis - fwd).length() < 1e-4, "yaw {yaw} pitch {pitch}");

            e.particles.instances.clear();
            assert!(e.vehicle_socket_fx("tank", "fx_muzzle_0", "fx::rail"));
            e.update_particles(1.0 / 60.0);
            let inst = e.particles.instances.last().expect("the one-shot instance");
            assert!((inst.frame.basis.x_axis - fwd).length() < 1e-4);
            let mut checked = 0;
            for l in &inst.layers {
                match l {
                    LayerRt::Ribbon(r) => {
                        let pts = &r.strands[0].points;
                        let span = *pts.last().unwrap() - pts[0];
                        assert!((pts[0] - origin).length() < 1e-4);
                        assert!((span.normalize() - fwd).length() < 1e-3, "tracer {span:?}");
                        assert!((span.length() - 9.0).abs() < 1e-3);
                        checked += 1;
                    }
                    LayerRt::Particles(pr) => {
                        for r in pr.ring.records().iter().filter(|r| r.life > 0.0) {
                            let v = Vec3::from_array(r.v0);
                            assert!((v.normalize() - fwd).length() < 1e-3, "spark {v:?}");
                            checked += 1;
                        }
                    }
                    LayerRt::Light(_) => {}
                }
            }
            assert!(checked >= 2, "tracer and sparks checked");
            // The light layer's offset turns with the frame.
            let flash = e
                .gather_lights()
                .into_iter()
                .min_by(|a, b| {
                    let da = (a.position - origin).length();
                    let db = (b.position - origin).length();
                    da.total_cmp(&db)
                })
                .expect("the flash light");
            assert!((flash.position - (origin + fwd * 0.3)).length() < 1e-3, "flash");
        }
    }
}

const RAIL_FX: &[u8] = br#"{"effects":[{"name":"rail","layers":[
  {"type":"ribbon","name":"tracer","mode":"arc","duration":1.0,"end":[9,0,0],
   "depth":4,"jitter":0.06,"branches":0},
  {"type":"particles","name":"sparks","max":64,"spawn":{"burst":8},
   "velocity":{"speed":[5,5],"cone":0.0},"motion":{"gravity":0.0},"life":[1,1]},
  {"type":"light","name":"flash","duration":1.0,"peak":2,"radius":3,"offset":[0.3,0,0]}]}]}"#;

/// The last one-shot's tracer span (start, end), its live sparks' velocities
/// and the light nearest `near` (position, radius; earlier shots' lights
/// linger, cleared instances do not release them).
fn rail_shot(e: &mut Engine, near: Vec3) -> ((Vec3, Vec3), Vec<Vec3>, (Vec3, f32)) {
    e.update_particles(1.0 / 60.0);
    let inst = e.particles.instances.last().expect("the one-shot instance");
    let (mut tracer, mut sparks) = (None, Vec::new());
    for l in &inst.layers {
        match l {
            LayerRt::Ribbon(r) => {
                let pts = &r.strands[0].points;
                tracer = Some((pts[0], *pts.last().unwrap()));
            }
            LayerRt::Particles(pr) => {
                sparks.extend(
                    pr.ring
                        .records()
                        .iter()
                        .filter(|r| r.life > 0.0)
                        .map(|r| Vec3::from_array(r.v0)),
                );
            }
            LayerRt::Light(_) => {}
        }
    }
    let flash = e
        .gather_lights()
        .into_iter()
        .min_by(|a, b| (a.position - near).length().total_cmp(&(b.position - near).length()))
        .expect("the flash light");
    (tracer.expect("a tracer"), sparks, (flash.position, flash.radius))
}

#[test]
fn fx_to_a_point_stretches_only_the_ribbon_to_reach_it() {
    let mut e = engine();
    e.register_effects("fx", EffectLibrary::from_json(RAIL_FX).unwrap());
    let def: classic_core::types::VehicleDef = serde_json::from_value(serde_json::json!({
        "version": 2, "name": "tank", "directions": 16,
        "layers": [
            { "name": "hull", "kind": "hull", "texture": "tankHull" },
            { "name": "turret", "kind": "turret", "texture": "tankTurret",
              "offset": [0.0, 0.0, 1.2] }
        ],
        "sockets": { "muzzle": { "layer": "turret", "offset": [2.5, 0.0, 0.6] } }
    }))
    .unwrap();
    e.vehicles.insert("tank".into(), def);
    assert!(e.spawn_vehicle("tank", "tank", 16.0, 16.0));
    e.time.delta = 1.0 / 60.0;
    e.update_vehicles();
    let (origin, _) = e.vehicle_socket_frame("tank", "muzzle").unwrap();
    // the authored shot: a 9 m tracer, 8 sparks at 5 m/s, a 3 m flash 0.3 m out
    e.particles.instances.clear();
    assert!(e.vehicle_socket_fx("tank", "muzzle", "fx::rail"));
    let (_, sparks0, (_, radius0)) = rail_shot(&mut e, origin + Vec3::X * 0.3);
    assert_eq!(sparks0.len(), 8);
    for (d, dir) in [
        (3.0f32, Vec3::X),
        (9.0, Vec3::new(0.0, -1.0, 0.0)),
        (25.0, Vec3::new(-0.6, 0.6, -0.3)),
        (60.0, Vec3::new(0.2, 0.9, 0.4)),
        (14.0, Vec3::new(0.05, 0.0, -1.0)),
    ] {
        let fwd = dir.normalize();
        let target = origin + fwd * d;
        e.particles.instances.clear();
        assert!(e.vehicle_socket_fx_to("tank", "muzzle", "fx::rail", target));
        let ((a, b), sparks, (flash, radius)) = rail_shot(&mut e, origin + fwd * 0.3);
        assert!((a - origin).length() < 1e-4, "{d} m: from the socket");
        assert!((b - target).length() < 1e-3, "{d} m {dir}: the tracer ends at the target {b}");
        // the rest as authored: count, speed, along the shot; the flash's
        // size and offset
        assert_eq!(sparks.len(), sparks0.len());
        for v in sparks {
            assert!((v.length() - 5.0).abs() < 1e-3 && (v.normalize() - fwd).length() < 1e-3);
        }
        assert!((flash - (origin + fwd * 0.3)).length() < 1e-3, "{d} m: the flash 0.3 m out");
        assert_eq!(radius, radius0);
    }
    // the guest's frame-free form, world point to world point
    let (from, to) = (Vec3::new(3.0, -4.0, 1.0), Vec3::new(-8.0, 6.0, 2.5));
    e.particles.instances.clear();
    assert!(e.fx_emit_to("fx::rail", from, to));
    let ((a, b), sparks, _) = rail_shot(&mut e, from);
    assert!((a - from).length() < 1e-4 && (b - to).length() < 1e-3);
    assert!(sparks.iter().all(|v| (v.normalize() - (to - from).normalize()).length() < 1e-3));
    // straight up, and the refusals
    assert!(e.fx_emit_to("fx::rail", from, from + Vec3::Z * 5.0));
    assert!(!e.fx_emit_to("fx::rail", from, from), "no direction");
    assert!(!e.fx_emit_to("fx::nope", from, to));
    assert!(!e.vehicle_socket_fx_to("tank", "nope", "fx::rail", to));
}
