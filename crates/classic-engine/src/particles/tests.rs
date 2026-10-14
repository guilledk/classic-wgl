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
  {"name":"zap","layers":[{"type":"ribbon","name":"a","mode":"arc","end":[2,0,0]}]}
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
    assert_eq!(a, b, "the handle is recycled");
    run(&mut e, 1, 1.0 / 60.0);
    assert_eq!(e.particle_stats().instances, 2, "the dying instance and the new one");
    run(&mut e, 120, 1.0 / 60.0);
    let s = e.particle_stats();
    assert_eq!(s.instances, 1, "only the dead emitter's instance retires");
    assert!(s.live > 0, "the new emitter keeps spawning");
}

#[test]
fn guest_emitter_pool_is_bounded() {
    let mut e = engine();
    for _ in 0..MAX_EMITTER_HANDLES {
        assert!(e.spawn_emitter(Emitter::new("t::stream", Vec3::ZERO)).is_some());
    }
    assert_eq!(e.spawn_emitter(Emitter::new("t::stream", Vec3::ZERO)), None);
}
