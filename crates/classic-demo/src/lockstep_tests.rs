//! A model clip and a guest timer advance in lockstep through the real frame
//! loop: the demo's model system (`prefabs::init_model_system`) and a guest
//! installed with [`crate::init_guest`], driven by `Engine::frame`.  Guest
//! choreography (the lunar crane unload) times itself with its own timer while
//! the engine plays the clip, so the two must agree on every frame, across a
//! clip swap.

use std::cell::RefCell;
use std::rc::Rc;

use classic_core::components::{Model, Transform};
use classic_core::model::{Interpolation, ModelAsset, ModelChannel, ModelClip, TrsProperty};
use classic_engine::Engine;
use classic_guest::GuestLimits;
use classic_rom::NullBootSink;
use glam::{Vec3, Vec4};

/// A one-node glb with one clip named `name` (the engine plays the clip named
/// after the model) lasting `duration` seconds.
fn clip_asset(name: &str, duration: f32) -> ModelAsset {
    ModelAsset {
        nodes: vec![classic_core::model::ModelNode {
            name: "rig".into(),
            parent: None,
            children: vec![],
            translation: Vec3::ZERO,
            rotation: glam::Quat::IDENTITY,
            scale: Vec3::ONE,
            mesh: None,
        }],
        meshes: vec![],
        clips: vec![ModelClip {
            name: name.into(),
            duration,
            channels: vec![ModelChannel {
                node: 0,
                property: TrsProperty::Translation,
                interpolation: Interpolation::Linear,
                times: vec![0.0, duration],
                values: vec![Vec4::ZERO, Vec4::new(0.0, 0.0, 10.0, 0.0)],
            }],
            node_channels: vec![],
        }],
        images: vec![],
        root: Some(0),
    }
}

/// The guest: on its first update starts clip `a` on `crane`; from then on
/// adds each `dt` to its timer, and once the timer passes [`SWAP_S`] starts
/// clip `b` and restarts the timer.  Every update it writes `(timer,
/// model_clip_time("crane"), phase)` to entity `t`.
const GUEST: &str = r#"(module
    (import "env" "start_model_clip" (func $start (param i32 i32 i32 i32 i32) (result i32)))
    (import "env" "model_clip_time" (func $ct (param i32 i32) (result f64)))
    (import "env" "set_pos" (func $set (param i32 i32 f64 f64 f64) (result i32)))
    (memory (export "memory") 1)
    (data (i32.const 0) "crane")
    (data (i32.const 8) "a")
    (data (i32.const 12) "b")
    (data (i32.const 16) "t")
    (global $phase (mut i32) (i32.const 0))
    (global $timer (mut f64) (f64.const 0))
    (func (export "update") (param $dt f64)
        (if (i32.eqz (global.get $phase))
            (then
                (drop (call $start (i32.const 0) (i32.const 5) (i32.const 8) (i32.const 1)
                    (i32.const 0)))
                (global.set $phase (i32.const 1)))
            (else
                (global.set $timer (f64.add (global.get $timer) (local.get $dt)))
                (if (i32.and (i32.eq (global.get $phase) (i32.const 1))
                             (f64.ge (global.get $timer) (f64.const 0.75)))
                    (then
                        (drop (call $start (i32.const 0) (i32.const 5) (i32.const 12)
                            (i32.const 1) (i32.const 0)))
                        (global.set $timer (f64.const 0))
                        (global.set $phase (i32.const 2))))))
        (drop (call $set (i32.const 16) (i32.const 1) (global.get $timer)
            (call $ct (i32.const 0) (i32.const 5))
            (f64.convert_i32_s (global.get $phase))))))"#;

/// The guest's clip-swap threshold (seconds into clip `a`).
const SWAP_S: f32 = 0.75;

#[test]
fn model_clip_and_guest_timer_agree_every_fixed_step_across_a_swap() {
    let mut e = Engine::new_for_test();
    e.models.insert("a".into(), clip_asset("a", 2.0));
    e.models.insert("b".into(), clip_asset("b", 3.0));
    e.spawn_named("crane");
    let crane = e.names["crane"];
    e.world.insert_one(crane, Transform::new(Vec3::ZERO, Vec3::ONE)).unwrap();
    e.world.insert_one(crane, Model::new("a", "tilemap")).unwrap();
    e.spawn_named("t");
    let t = e.names["t"];
    e.world.insert_one(t, Transform::new(Vec3::ZERO, Vec3::ONE)).unwrap();

    // The boot's registration order: model system, then the guest.
    crate::prefabs::init_model_system(&mut e);
    let state: crate::state::DemoStateRef = Rc::new(RefCell::new(Default::default()));
    let wasm = wat::parse_str(GUEST).expect("valid WAT");
    crate::init_guest(
        &mut e,
        &state,
        &wasm,
        &GuestLimits::default(),
        "",
        "lockstep",
        &NullBootSink,
    );

    // An awkward fixed step (not a power of two): 150 frames = 2.5 s, the
    // swap at 0.75 s into `a`, then 1.75 s into the 3 s `b`.
    let dt = 1.0 / 60.0;
    let mut platform = e.input.clone();
    let mut swapped_at = None;
    for frame in 0..150 {
        e.frame(&mut platform, 640.0, 360.0, dt);
        let (timer, clip_time, phase) = e.get_pos("t").expect("the guest wrote its clock");
        assert!(phase >= 1.0, "frame {frame}: the guest started clip a");
        assert!(
            (timer - clip_time).abs() < 1e-4,
            "frame {frame} (phase {phase}): guest timer {timer} s, model clip {clip_time} s"
        );
        if phase == 2.0 && swapped_at.is_none() {
            swapped_at = Some(frame);
            assert_eq!(clip_time, 0.0, "the swap frame restarts the clip");
            assert_eq!(e.world.get::<&Model>(crane).unwrap().model, "b");
        }
    }
    // Clip `a` ran to the first step at or past the threshold (frame 0 only
    // started it), then `b` ran the remaining frames.
    let swap = swapped_at.expect("the guest swapped clips");
    assert_eq!(swap, (SWAP_S / dt).ceil() as usize);
    let left = (149 - swap) as f32 * dt;
    assert!((e.model_clip_time("crane") - left).abs() < 1e-4, "{}", e.model_clip_time("crane"));
}
