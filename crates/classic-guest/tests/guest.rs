//! Integration tests for the WASM guest runtime, driving wasmi with small
//! hand-written WAT guest modules.

use classic_core::components::{ColliderData, NavMesh, Role, Shape};
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
            (import "env" "set_pos" (func $set_pos (param i32 i32 f64 f64 f64) (result i32)))
            (import "env" "log" (func $log (param i32 i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "unit")
            (data (i32.const 16) "hello")
            (func (export "update") (param f64)
                (drop (call $spawn (i32.const 0) (i32.const 4)))
                (drop (call $set_pos (i32.const 0) (i32.const 4) (f64.const 10.0) (f64.const 20.0) (f64.const 3.0)))
                (call $log (i32.const 16) (i32.const 5))))"#,
        &GuestLimits::default(),
    )
    .unwrap();

    rt.update(&mut engine, 0.016).unwrap();

    assert!(engine.has_name("unit"));
    assert_eq!(engine.get_pos("unit"), Some((10.0, 20.0, 3.0)));
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

#[test]
fn guest_was_key_pressed_triggers_action() {
    let mut engine = Engine::new_for_test();
    engine.input.keys_pressed.insert("KeyR".to_string(), true);

    let mut rt = runtime_from_wat(
        r#"(module
            (import "env" "was_key_pressed" (func $wp (param i32 i32) (result i32)))
            (import "env" "spawn" (func $spawn (param i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "KeyR")
            (data (i32.const 16) "marker")
            (func (export "update") (param f64)
                (if (call $wp (i32.const 0) (i32.const 4))
                    (then (drop (call $spawn (i32.const 16) (i32.const 6)))))))"#,
        &GuestLimits::default(),
    )
    .unwrap();

    rt.update(&mut engine, 0.016).unwrap();
    assert!(engine.has_name("marker"));
}

#[test]
fn generate_terrain_unknown_kind_returns_false() {
    let mut engine = Engine::new_for_test();
    assert!(!engine.generate_terrain("bogus", "x", 1.0));
}

#[test]
fn guest_set_camera_moves_the_view() {
    let mut engine = Engine::new_for_test();
    let mut rt = runtime_from_wat(
        r#"(module
            (import "env" "set_camera" (func $set_camera (param f64 f64 f64) (result i32)))
            (func (export "update") (param f64)
                (drop (call $set_camera (f64.const 100.0) (f64.const 200.0) (f64.const 2.5)))))"#,
        &GuestLimits::default(),
    )
    .unwrap();

    rt.update(&mut engine, 0.016).unwrap();

    assert_eq!(engine.get_camera(), (100.0, 200.0, 2.5));
}

#[test]
fn pick_at_returns_entity_under_point() {
    let mut engine = Engine::new_for_test();
    let mut collider = ColliderData::new(Shape::Circle { diameter: 10.0 });
    collider.position = Vec3::new(100.0, 100.0, 0.0);
    engine.register_named_collider("tree", collider);
    engine.physics.begin_frame();

    assert_eq!(engine.pick_at(100.0, 100.0), Some("tree".to_string()));
    assert_eq!(engine.pick_at(500.0, 500.0), None);
}

#[test]
fn guest_set_light_updates_uniforms() {
    let mut engine = Engine::new_for_test();
    let mut rt = runtime_from_wat(
        r#"(module
            (import "env" "set_light" (func $set_light
                (param f64 f64 f64 f64 f64 f64 f64 f64 f64) (result i32)))
            (func (export "update") (param f64)
                (drop (call $set_light
                    (f64.const 0.1) (f64.const 0.2) (f64.const 0.3)
                    (f64.const 0.4) (f64.const 0.5) (f64.const 0.6)
                    (f64.const 0.7) (f64.const 0.8) (f64.const 0.9)))))"#,
        &GuestLimits::default(),
    )
    .unwrap();

    rt.update(&mut engine, 0.016).unwrap();

    assert_eq!(engine.get_light(), ([0.1, 0.2, 0.3], [0.4, 0.5, 0.6], [0.7, 0.8, 0.9]));
}

#[test]
fn guest_mouse_down_and_key_up_trigger_action() {
    let mut engine = Engine::new_for_test();
    engine.input.mouse_down[0] = true;
    engine.input.keys_released.insert("KeyR".to_string(), true);

    let mut rt = runtime_from_wat(
        r#"(module
            (import "env" "mouse_down" (func $md (param i32) (result i32)))
            (import "env" "key_up" (func $ku (param i32 i32) (result i32)))
            (import "env" "spawn" (func $spawn (param i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "KeyR")
            (data (i32.const 16) "marker")
            (func (export "update") (param f64)
                (if (call $md (i32.const 0))
                    (then
                        (if (call $ku (i32.const 0) (i32.const 4))
                            (then (drop (call $spawn (i32.const 16) (i32.const 6)))))))))"#,
        &GuestLimits::default(),
    )
    .unwrap();

    rt.update(&mut engine, 0.016).unwrap();
    assert!(engine.has_name("marker"));
}

#[test]
fn spawn_rect_text_and_set_text() {
    let mut engine = Engine::new_for_test();
    assert!(engine.spawn_rect("bar", 10.0, 20.0, 100.0, 50.0, [1.0, 0.0, 0.0, 1.0]));
    assert!(engine.spawn_text("label", 0.0, 0.0, "hello", 2.0, [1.0, 1.0, 1.0, 1.0]));
    assert!(engine.set_text("label", "world"));

    let label = *engine.names.get("label").unwrap();
    let sdf = engine.world.get::<&classic_core::components::SdfTextRender>(label).unwrap();
    assert_eq!(sdf.text, "world");
    assert_eq!(engine.get_pos("bar"), Some((10.0, 20.0, 0.0)));
}

#[test]
fn guest_spawn_rect_wiring() {
    let mut engine = Engine::new_for_test();
    let mut rt = runtime_from_wat(
        r#"(module
            (import "env" "spawn_rect" (func $spawn_rect
                (param i32 i32 f64 f64 f64 f64 f64 f64 f64 f64) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "bar")
            (func (export "update") (param f64)
                (drop (call $spawn_rect (i32.const 0) (i32.const 3)
                    (f64.const 10.0) (f64.const 20.0) (f64.const 100.0) (f64.const 50.0)
                    (f64.const 1.0) (f64.const 0.0) (f64.const 0.0) (f64.const 1.0)))))"#,
        &GuestLimits::default(),
    )
    .unwrap();

    rt.update(&mut engine, 0.016).unwrap();

    assert!(engine.has_name("bar"));
    assert_eq!(engine.get_pos("bar"), Some((10.0, 20.0, 0.0)));
}

#[test]
fn ui_registration_and_layout_wiring() {
    let mut engine = Engine::new_for_test();
    engine.ui = Some(classic_engine::ui::UIManager::new(800.0, 600.0, &mut engine.world));

    assert!(engine.ui_container("panel", 200.0, 100.0, [0.2, 0.3, 0.5, 1.0]));
    assert!(engine.ui_text(
        "title",
        "Hello",
        1.0,
        200.0,
        [1.0, 1.0, 1.0, 1.0],
        classic_core::components::TextJustify::Left,
    ));
    assert!(engine.ui_button("play", "Play", 120.0, 40.0, [0.1, 0.6, 0.2, 1.0]));
    assert!(engine.ui_add_child(
        "panel",
        "title",
        classic_core::components::UiAnchor::TopLeft,
        classic_core::components::UiAnchor::TopLeft,
    ));

    assert!(engine.has_name("panel"));
    assert!(engine.has_name("title"));
    assert!(engine.has_name("play"));
    assert!(engine.ui_set_size("panel", 300.0, 150.0));
    assert!(engine.ui_set_anchor("panel", classic_core::components::UiAnchor::TopRight));
    assert!(engine.ui_set_color("panel", [1.0, 0.0, 0.0, 1.0]));
    assert!(engine.ui_set_fixed("panel", true));
}

#[test]
fn guest_ui_container_wiring() {
    let mut engine = Engine::new_for_test();
    engine.ui = Some(classic_engine::ui::UIManager::new(800.0, 600.0, &mut engine.world));

    let mut rt = runtime_from_wat(
        r#"(module
            (import "env" "ui_container" (func $ui_container
                (param i32 i32 f64 f64 f64 f64 f64 f64) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "panel")
            (func (export "update") (param f64)
                (drop (call $ui_container (i32.const 0) (i32.const 5)
                    (f64.const 200.0) (f64.const 100.0)
                    (f64.const 0.2) (f64.const 0.3) (f64.const 0.5) (f64.const 1.0)))))"#,
        &GuestLimits::default(),
    )
    .unwrap();

    rt.update(&mut engine, 0.016).unwrap();

    assert!(engine.has_name("panel"));
}
