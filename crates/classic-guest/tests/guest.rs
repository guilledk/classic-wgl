//! Integration tests for the WASM guest runtime, driving wasmi with small
//! hand-written WAT guest modules.

use classic_core::components::{NavMesh, Role};
use classic_core::RoleKind;
use classic_engine::Engine;
use classic_guest::{GuestError, GuestLimits, GuestRuntime, WasmiRuntime};
use glam::Vec3;

fn runtime_from_wat(wat: &str, limits: &GuestLimits) -> Result<WasmiRuntime, GuestError> {
    let wasm = wat::parse_str(wat).expect("valid WAT");
    WasmiRuntime::new(&wasm, limits)
}

#[test]
fn noop_guest_runs() {
    let mut engine = Engine::new_for_test();
    let mut rt = runtime_from_wat(
        r#"(module (func (export "update") (param f64)))"#,
        &GuestLimits::default(),
    )
    .unwrap();
    rt.update(&mut engine, 0.016).unwrap();
}

#[test]
fn guest_can_spawn_and_move_entities() {
    let mut engine = Engine::new_for_test();
    let mut rt = runtime_from_wat(
        r#"(module
            (import "env" "spawn" (func $spawn (param i32 i32) (result i32)))
            (import "env" "set_pos" (func $set_pos (param i32 i32 f64 f64) (result i32)))
            (import "env" "log" (func $log (param i32 i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "unit")
            (data (i32.const 16) "hello")
            (func (export "update") (param f64)
                (drop (call $spawn (i32.const 0) (i32.const 4)))
                (drop (call $set_pos (i32.const 0) (i32.const 4) (f64.const 10.0) (f64.const 20.0)))
                (call $log (i32.const 16) (i32.const 5))))"#,
        &GuestLimits::default(),
    )
    .unwrap();

    rt.update(&mut engine, 0.016).unwrap();

    assert!(engine.has_name("unit"));
    assert_eq!(engine.get_pos("unit"), Some((10.0, 20.0)));
}

#[test]
fn infinite_loop_halts_on_fuel_budget() {
    let mut engine = Engine::new_for_test();
    let mut rt = runtime_from_wat(
        r#"(module
            (func (export "update") (param f64)
                (loop $l
                    (i32.const 1)
                    (i32.const 2)
                    (i32.add)
                    (drop)
                    (br $l))))"#,
        &GuestLimits { fuel_per_frame: 10_000, ..GuestLimits::default() },
    )
    .unwrap();

    let err = rt.update(&mut engine, 0.016).unwrap_err();
    assert!(matches!(err, GuestError::FuelExhausted), "expected fuel exhaustion, got {err}");
}

#[test]
fn memory_growth_past_cap_traps() {
    let mut engine = Engine::new_for_test();
    let mut rt = runtime_from_wat(
        r#"(module
            (memory (export "memory") 1)
            (func (export "update") (param f64)
                (drop (memory.grow (i32.const 32)))))"#,
        &GuestLimits { max_memory_bytes: 1 << 20, ..GuestLimits::default() },
    )
    .unwrap();

    assert!(rt.update(&mut engine, 0.016).is_err());
}

/// Install a small (3x3) fully-walkable nav mesh on a role-tagged entity.
fn install_test_navmesh(engine: &mut Engine) {
    let nav = NavMesh {
        position: Vec3::ZERO,
        scale: Vec3::ONE,
        map_entity: "tilemap".into(),
        tile_set: "navTileset".into(),
        data: vec![1u32; 9],
        size_x: 3,
        size_y: 3,
    };
    let entity = engine.world.spawn((nav, Role::new(RoleKind::NavMesh)));
    engine.names.insert("navmesh".into(), entity);
}

#[test]
fn find_path_returns_waypoints() {
    let mut engine = Engine::new_for_test();
    install_test_navmesh(&mut engine);

    assert_eq!(engine.find_path((0, 0), (2, 0)), Some(vec![(0, 0), (1, 0), (2, 0)]));
    assert_eq!(engine.find_path((0, 0), (2, 2)), Some(vec![(0, 0), (1, 1), (2, 2)]));
}

#[test]
fn guest_find_path_import_is_wired() {
    let mut engine = Engine::new_for_test();
    install_test_navmesh(&mut engine);

    let mut rt = runtime_from_wat(
        r#"(module
            (import "env" "find_path" (func $find_path (param i32 i32 i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (func (export "update") (param f64)
                (drop (call $find_path (i32.const 0) (i32.const 0) (i32.const 2) (i32.const 0)
                    (i32.const 64) (i32.const 256)))))"#,
        &GuestLimits::default(),
    )
    .unwrap();

    rt.update(&mut engine, 0.016).unwrap();
}
