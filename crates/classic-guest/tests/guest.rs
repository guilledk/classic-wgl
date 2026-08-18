//! Integration tests for the WASM guest runtime, driving every backend (wasmi
//! always; wasmtime on native) with small hand-written WAT guest modules.

use classic_core::components::{Animator, ColliderData, NavMesh, Role, Shape, Tilemap};
use classic_core::types::AnimationData;
use classic_core::RoleKind;
use classic_engine::Engine;
#[cfg(not(target_arch = "wasm32"))]
use classic_guest::WasmtimeRuntime;
use classic_guest::{GuestError, GuestLimits, GuestRuntime, WasmiRuntime};
use classic_rom::{ResourceKind, ResourceSet};
use glam::Vec3;

/// Build one runtime per available backend for a WAT module.
fn runtimes_from_wat(
    wat: &str,
    limits: &GuestLimits,
) -> Result<Vec<Box<dyn GuestRuntime>>, GuestError> {
    let wasm = wat::parse_str(wat).expect("valid WAT");
    let mut runtimes: Vec<Box<dyn GuestRuntime>> =
        vec![Box::new(WasmiRuntime::new(&wasm, limits)?)];
    #[cfg(not(target_arch = "wasm32"))]
    runtimes.push(Box::new(WasmtimeRuntime::new(&wasm, limits)?));
    Ok(runtimes)
}

/// Run a guest-driven assertion against every backend, with a fresh engine per
/// backend.
fn with_each_runtime(wat: &str, limits: &GuestLimits, f: impl Fn(&mut dyn GuestRuntime)) {
    for mut rt in runtimes_from_wat(wat, limits).expect("valid guest") {
        f(&mut *rt);
    }
}

#[test]
fn noop_guest_runs() {
    with_each_runtime(
        r#"(module (func (export "update") (param f64)))"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            rt.update(&mut engine, 0.016).unwrap();
        },
    );
}

#[test]
fn guest_can_spawn_and_move_entities() {
    with_each_runtime(
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
        |rt| {
            let mut engine = Engine::new_for_test();
            rt.update(&mut engine, 0.016).unwrap();

            assert!(engine.has_name("unit"));
            assert_eq!(engine.get_pos("unit"), Some((10.0, 20.0, 3.0)));
        },
    );
}

#[test]
fn infinite_loop_halts_on_fuel_budget() {
    with_each_runtime(
        r#"(module
            (func (export "update") (param f64)
                (loop $l
                    (i32.const 1)
                    (i32.const 2)
                    (i32.add)
                    (drop)
                    (br $l))))"#,
        &GuestLimits { fuel_per_frame: 10_000, ..GuestLimits::default() },
        |rt| {
            let mut engine = Engine::new_for_test();
            let err = rt.update(&mut engine, 0.016).unwrap_err();
            assert!(
                matches!(err, GuestError::FuelExhausted),
                "expected fuel exhaustion, got {err}"
            );
        },
    );
}

#[test]
fn memory_growth_past_cap_traps() {
    with_each_runtime(
        r#"(module
            (memory (export "memory") 1)
            (func (export "update") (param f64)
                (drop (memory.grow (i32.const 32)))))"#,
        &GuestLimits { max_memory_bytes: 1 << 20, ..GuestLimits::default() },
        |rt| {
            let mut engine = Engine::new_for_test();
            assert!(rt.update(&mut engine, 0.016).is_err());
        },
    );
}

/// Install a small (3x3) fully-walkable nav mesh on a role-tagged entity.
fn install_test_navmesh(engine: &mut Engine) {
    let nav = NavMesh {
        position: Vec3::ZERO,
        scale: Vec3::ONE,
        map_entity: "tilemap".into(),
        tile_set: "navTileset".into(),
        data_grid: None,
        data: vec![1u32; 9],
        size_x: 3,
        size_y: 3,
    };
    let entity = engine.world.spawn((nav, Role::new(RoleKind::NavMesh)));
    engine.names.insert("navmesh".into(), entity);
}

/// Install a small (3x3) flat tilemap on a role-tagged entity.
fn install_test_tilemap(engine: &mut Engine) {
    let tilemap = Tilemap {
        position: Vec3::ZERO,
        scale: Vec3::ONE,
        size_x: 3,
        size_y: 3,
        tile_set: "tileset".into(),
        tile_pixel_size: [32, 32],
        max_tile: 16,
        tiles_grid: None,
        heights_grid: None,
        data: vec![0u32; 9],
        height_data: vec![1.0f32; 16],
        height_scale: 1.0,
        tile_set_pixel_size: [0, 0],
        tiles_per_row: 0,
        mouse_iso_pos: Vec3::ZERO,
        selection_iso_begin: Vec3::new(-1.0, -1.0, -1.0),
        selection_iso_end: Vec3::new(-1.0, -1.0, -1.0),
    };
    let entity = engine.world.spawn((tilemap, Role::new(RoleKind::Tilemap)));
    engine.names.insert("tilemap".into(), entity);
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
    with_each_runtime(
        r#"(module
            (import "env" "find_path" (func $find_path (param i32 i32 i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (func (export "update") (param f64)
                (drop (call $find_path (i32.const 0) (i32.const 0) (i32.const 2) (i32.const 0)
                    (i32.const 64) (i32.const 256)))))"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            install_test_navmesh(&mut engine);
            rt.update(&mut engine, 0.016).unwrap();
        },
    );
}

#[test]
fn guest_vehicle_imports_are_wired() {
    with_each_runtime(
        r#"(module
            (import "env" "vehicle_teleport" (func $t (param i32 i32 f64 f64) (result i32)))
            (import "env" "vehicle_goto" (func $g (param i32 i32 i32 i32) (result i32)))
            (import "env" "vehicle_stop" (func $s (param i32 i32) (result i32)))
            (import "env" "vehicle_spawn" (func $sp (param i32 i32 i32 i32 f64 f64) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "lrv")
            (data (i32.const 16) "rover")
            (func (export "update") (param f64)
                (drop (call $sp (i32.const 0) (i32.const 3) (i32.const 16) (i32.const 5)
                    (f64.const 1.0) (f64.const 1.0)))
                (drop (call $t (i32.const 16) (i32.const 5) (f64.const 1.0) (f64.const 1.0)))
                (drop (call $g (i32.const 16) (i32.const 5) (i32.const 2) (i32.const 0)))
                (drop (call $s (i32.const 16) (i32.const 5)))))"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            rt.update(&mut engine, 0.016).unwrap();
        },
    );
}

#[test]
fn guest_was_key_pressed_triggers_action() {
    with_each_runtime(
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
        |rt| {
            let mut engine = Engine::new_for_test();
            engine.input.keys_pressed.insert("KeyR".to_string(), true);
            rt.update(&mut engine, 0.016).unwrap();
            assert!(engine.has_name("marker"));
        },
    );
}

#[test]
fn guest_set_camera_moves_the_view() {
    with_each_runtime(
        r#"(module
            (import "env" "set_camera" (func $set_camera (param f64 f64 f64) (result i32)))
            (func (export "update") (param f64)
                (drop (call $set_camera (f64.const 100.0) (f64.const 200.0) (f64.const 2.5)))))"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            rt.update(&mut engine, 0.016).unwrap();
            assert_eq!(engine.get_camera(), (100.0, 200.0, 2.5));
        },
    );
}

#[test]
fn iso_to_screen_projects_with_tilemap_scale() {
    let mut engine = Engine::new_for_test();
    install_test_tilemap(&mut engine);
    assert!(engine.iso_to_screen(0.0, 0.0).is_some());
    assert!(engine.iso_to_screen(32.0, 13.0).is_some());
}

#[test]
fn iso_to_screen_returns_none_without_tilemap() {
    let engine = Engine::new_for_test();
    assert_eq!(engine.iso_to_screen(32.0, 13.0), None);
}

#[test]
fn guest_iso_to_screen_and_set_grid_wiring() {
    with_each_runtime(
        r#"(module
            (import "env" "iso_to_screen" (func $iso (param f64 f64 i32) (result i32)))
            (import "env" "set_grid" (func $set_grid (param i32) (result i32)))
            (memory (export "memory") 1)
            (func (export "update") (param f64)
                (drop (call $iso (f64.const 32.0) (f64.const 13.0) (i32.const 64)))
                (drop (call $set_grid (i32.const 1)))))"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            install_test_tilemap(&mut engine);
            rt.update(&mut engine, 0.016).unwrap();
            assert!(engine.show_grid);
        },
    );
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
    with_each_runtime(
        r#"(module
            (import "env" "set_light" (func $set_light
                (param f64 f64 f64 f64 f64 f64 f64 f64 f64) (result i32)))
            (func (export "update") (param f64)
                (drop (call $set_light
                    (f64.const 0.1) (f64.const 0.2) (f64.const 0.3)
                    (f64.const 0.4) (f64.const 0.5) (f64.const 0.6)
                    (f64.const 0.7) (f64.const 0.8) (f64.const 0.9)))))"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            rt.update(&mut engine, 0.016).unwrap();
            assert_eq!(engine.get_light(), ([0.1, 0.2, 0.3], [0.4, 0.5, 0.6], [0.7, 0.8, 0.9]));
        },
    );
}

#[test]
fn guest_mouse_down_and_key_up_trigger_action() {
    with_each_runtime(
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
        |rt| {
            let mut engine = Engine::new_for_test();
            engine.input.mouse_down[0] = true;
            engine.input.keys_released.insert("KeyR".to_string(), true);
            rt.update(&mut engine, 0.016).unwrap();
            assert!(engine.has_name("marker"));
        },
    );
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
    with_each_runtime(
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
        |rt| {
            let mut engine = Engine::new_for_test();
            rt.update(&mut engine, 0.016).unwrap();
            assert!(engine.has_name("bar"));
            assert_eq!(engine.get_pos("bar"), Some((10.0, 20.0, 0.0)));
        },
    );
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
    with_each_runtime(
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
        |rt| {
            let mut engine = Engine::new_for_test();
            engine.ui = Some(classic_engine::ui::UIManager::new(800.0, 600.0, &mut engine.world));
            rt.update(&mut engine, 0.016).unwrap();
            assert!(engine.has_name("panel"));
        },
    );
}

#[test]
fn subscribe_and_poll_event() {
    let mut engine = Engine::new_for_test();
    engine.ui = Some(classic_engine::ui::UIManager::new(800.0, 600.0, &mut engine.world));
    engine.ui_button("play", "Play", 120.0, 40.0, [0.1, 0.6, 0.2, 1.0]);

    // ui_button auto-subscribes; subscribe is idempotent for known entities.
    assert!(engine.subscribe("play"));
    assert!(!engine.subscribe("nope"));

    // No events queued yet.
    assert!(engine.poll_event().is_none());
}

#[test]
fn guest_subscribe_and_poll_wiring() {
    with_each_runtime(
        r#"(module
            (import "env" "subscribe" (func $subscribe (param i32 i32) (result i32)))
            (import "env" "poll_event" (func $poll_event (param i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "play")
            (func (export "update") (param f64)
                (drop (call $subscribe (i32.const 0) (i32.const 4)))
                (drop (call $poll_event (i32.const 64) (i32.const 256)))))"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            engine.ui = Some(classic_engine::ui::UIManager::new(800.0, 600.0, &mut engine.world));
            engine.ui_button("play", "Play", 120.0, 40.0, [0.1, 0.6, 0.2, 1.0]);
            rt.update(&mut engine, 0.016).unwrap();
        },
    );
}

#[test]
fn spawn_collider_and_pick() {
    let mut engine = Engine::new_for_test();
    engine.spawn_named("unit");
    assert!(engine.spawn_collider("unit", 50.0, 60.0, 20.0, 10.0));
    assert!(engine.subscribe("unit"));
    engine.physics.begin_frame();

    assert_eq!(engine.pick_at(50.0, 60.0), Some("unit".to_string()));
    assert_eq!(engine.pick_at(500.0, 500.0), None);
}

#[test]
fn get_anim_reads_animation() {
    let mut engine = Engine::new_for_test();
    engine.spawn_named("unit");
    let entity = *engine.names.get("unit").unwrap();
    engine
        .world
        .insert_one(
            entity,
            Animator {
                target: "unit.IsoAgent".into(),
                speed: 1.0,
                animation: Some("walkEast".into()),
                counter: 0.0,
                frame: 5.0,
                offset: Vec3::ZERO,
                repeat: true,
                playing: true,
            },
        )
        .unwrap();

    assert_eq!(engine.get_anim("unit"), Some(("walkEast".to_string(), 5.0)));
    assert_eq!(engine.get_anim("missing"), None);
}

#[test]
fn start_anim_resets_animator_state() {
    let mut engine = Engine::new_for_test();
    engine.spawn_named("unit");
    let entity = *engine.names.get("unit").unwrap();
    engine
        .world
        .insert_one(
            entity,
            Animator {
                target: "unit.IsoAgent".into(),
                speed: 1.0,
                animation: Some("walkEast".into()),
                counter: 7.0,
                frame: 5.0,
                offset: Vec3::ONE,
                repeat: true,
                playing: false,
            },
        )
        .unwrap();

    assert!(engine.start_anim("unit", "landing", false));
    let a = engine.world.get::<&Animator>(entity).unwrap();
    assert_eq!(a.animation.as_deref(), Some("landing"));
    assert!(!a.repeat);
    assert!(a.playing);
    assert_eq!(a.counter, 0.0);
    assert_eq!(a.frame, 0.0);
    assert_eq!(a.offset, Vec3::ZERO);
}

#[test]
fn guest_start_anim_import_is_wired() {
    with_each_runtime(
        r#"(module
            (import "env" "start_anim" (func $sa (param i32 i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "unit")
            (data (i32.const 16) "landing")
            (func (export "update") (param f64)
                (drop (call $sa (i32.const 0) (i32.const 4) (i32.const 16) (i32.const 7) (i32.const 0)))))"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            engine.spawn_named("unit");
            let entity = *engine.names.get("unit").unwrap();
            engine
                .world
                .insert_one(
                    entity,
                    Animator {
                        target: "unit.IsoAgent".into(),
                        speed: 1.0,
                        animation: None,
                        counter: 9.0,
                        frame: 4.0,
                        offset: Vec3::ONE,
                        repeat: true,
                        playing: false,
                    },
                )
                .unwrap();
            rt.update(&mut engine, 0.016).unwrap();
            let a = engine.world.get::<&Animator>(entity).unwrap();
            assert_eq!(a.animation.as_deref(), Some("landing"));
            assert_eq!(a.counter, 0.0);
        },
    );
}

#[test]
fn guest_init_hook_spawns_once_before_update() {
    with_each_runtime(
        r#"(module
            (import "env" "spawn" (func $spawn (param i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "inited")
            (func (export "init")
                (drop (call $spawn (i32.const 0) (i32.const 6))))
            (func (export "update") (param f64)))"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            rt.init(&mut engine).unwrap();
            assert!(engine.has_name("inited"));
            rt.update(&mut engine, 0.016).unwrap();
        },
    );
}

#[test]
fn guest_start_hook_spawns() {
    with_each_runtime(
        r#"(module
            (import "env" "spawn" (func $spawn (param i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "started")
            (func (export "update") (param f64))
            (func (export "start")
                (drop (call $spawn (i32.const 0) (i32.const 7)))))"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            rt.start(&mut engine).unwrap();
            assert!(engine.has_name("started"));
        },
    );
}

#[test]
fn guest_without_lifecycle_hooks_still_runs() {
    with_each_runtime(
        r#"(module (func (export "update") (param f64)))"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            assert!(rt.init(&mut engine).is_ok());
            assert!(rt.start(&mut engine).is_ok());
            rt.update(&mut engine, 0.016).unwrap();
        },
    );
}

#[test]
fn guest_edits_terrain_tile_and_height() {
    with_each_runtime(
        r#"(module
            (import "env" "set_tile" (func $set_tile (param i32 i32 i32) (result i32)))
            (import "env" "set_height" (func $set_height (param i32 i32 f64) (result i32)))
            (import "env" "rebuild_terrain" (func $rebuild_terrain (result i32)))
            (func (export "update") (param f64)
                (drop (call $set_tile (i32.const 1) (i32.const 2) (i32.const 7)))
                (drop (call $set_height (i32.const 0) (i32.const 0) (f64.const 5.0)))
                (drop (call $rebuild_terrain))))"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            install_test_tilemap(&mut engine);
            rt.update(&mut engine, 0.016).unwrap();

            let tm_entity = engine.entity_by_role(RoleKind::Tilemap).unwrap();
            let tm = engine.world.get::<&Tilemap>(tm_entity).unwrap();
            assert_eq!(tm.data[2 * 3 + 1], 7);
            assert_eq!(tm.height_data[0], 5.0);
        },
    );
}

#[test]
fn guest_terrain_edits_are_bounds_checked() {
    with_each_runtime(
        r#"(module
            (import "env" "set_tile" (func $set_tile (param i32 i32 i32) (result i32)))
            (import "env" "set_height" (func $set_height (param i32 i32 f64) (result i32)))
            (func (export "update") (param f64)
                (drop (call $set_tile (i32.const 99) (i32.const 99) (i32.const 7)))
                (drop (call $set_height (i32.const 99) (i32.const 99) (f64.const 5.0)))))"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            install_test_tilemap(&mut engine);
            rt.update(&mut engine, 0.016).unwrap();

            let tm_entity = engine.entity_by_role(RoleKind::Tilemap).unwrap();
            let tm = engine.world.get::<&Tilemap>(tm_entity).unwrap();
            assert!(tm.data.iter().all(|&t| t == 0));
            assert!(tm.height_data.iter().all(|&h| h == 1.0));
        },
    );
}

#[test]
fn bulk_terrain_upload_roundtrip() {
    let mut engine = Engine::new_for_test();
    install_test_tilemap(&mut engine);
    install_test_navmesh(&mut engine);

    let tm_entity = engine.entity_by_role(RoleKind::Tilemap).unwrap();
    let nav_entity = engine.entity_by_role(RoleKind::NavMesh).unwrap();

    // 3x3 tilemap: 9 tiles, 16 height vertices; 3x3 navmesh: 9 cells.
    assert!(engine.set_tiles_bulk(&[7u32; 9]));
    assert!(engine.set_heights_bulk(&[2.5f32; 16]));
    assert!(engine.set_nav_bulk(&[1u32; 9]));

    assert_eq!(engine.world.get::<&Tilemap>(tm_entity).unwrap().data, vec![7u32; 9]);
    assert_eq!(engine.world.get::<&Tilemap>(tm_entity).unwrap().height_data, vec![2.5f32; 16]);
    assert_eq!(engine.world.get::<&NavMesh>(nav_entity).unwrap().data, vec![1u32; 9]);

    // Length mismatch is rejected.
    assert!(!engine.set_tiles_bulk(&[1u32; 8]));
}

#[test]
fn bulk_terrain_upload_populates_empty_grids() {
    // `state_lunar.json` declares `"data": null` (no `heightData`), so the
    // loaded Tilemap/NavMesh grids are empty until the guest uploads them.
    let mut engine = Engine::new_for_test();

    let tilemap = Tilemap {
        position: Vec3::ZERO,
        scale: Vec3::ONE,
        size_x: 3,
        size_y: 3,
        tile_set: "lunarTileset".into(),
        tile_pixel_size: [32, 32],
        max_tile: 24,
        tiles_grid: None,
        heights_grid: None,
        data: vec![],
        height_data: vec![],
        height_scale: 0.0,
        tile_set_pixel_size: [0, 0],
        tiles_per_row: 0,
        mouse_iso_pos: Vec3::ZERO,
        selection_iso_begin: Vec3::new(-1.0, -1.0, -1.0),
        selection_iso_end: Vec3::new(-1.0, -1.0, -1.0),
    };
    let tm_entity = engine.world.spawn((tilemap, Role::new(RoleKind::Tilemap)));
    engine.names.insert("tilemap".into(), tm_entity);

    let nav = NavMesh {
        position: Vec3::ZERO,
        scale: Vec3::ONE,
        map_entity: "tilemap".into(),
        tile_set: "navTileset".into(),
        data_grid: None,
        data: vec![],
        size_x: 3,
        size_y: 3,
    };
    let nav_entity = engine.world.spawn((nav, Role::new(RoleKind::NavMesh)));
    engine.names.insert("navmesh".into(), nav_entity);

    // Bulk writes key off the component dimensions, not the (empty) buffers.
    assert!(engine.set_tiles_bulk(&[7u32; 9]));
    assert!(engine.set_heights_bulk(&[2.5f32; 16]));
    assert!(engine.set_nav_bulk(&[1u32; 9]));

    assert_eq!(engine.world.get::<&Tilemap>(tm_entity).unwrap().data, vec![7u32; 9]);
    assert_eq!(engine.world.get::<&Tilemap>(tm_entity).unwrap().height_data, vec![2.5f32; 16]);
    assert_eq!(engine.world.get::<&NavMesh>(nav_entity).unwrap().data, vec![1u32; 9]);
}

#[test]
fn guest_noise_field_imports_wired() {
    with_each_runtime(
        r#"(module
            (import "env" "fbm_field" (func $fbm (param i32 i32 i32 i32 i32 f64 f64 f64 i32 i32) (result i32)))
            (import "env" "noise2d" (func $noise2d (param i32 i32 f64 f64) (result f64)))
            (memory (export "memory") 1)
            (data (i32.const 0) "apollo")
            (func (export "update") (param f64)
                (drop (call $fbm (i32.const 8) (i32.const 8) (i32.const 0) (i32.const 6)
                    (i32.const 4) (f64.const 0.03) (f64.const 2.0) (f64.const 0.5)
                    (i32.const 1024) (i32.const 256)))
                (drop (call $noise2d (i32.const 0) (i32.const 6) (f64.const 1.0) (f64.const 2.0)))))"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            rt.update(&mut engine, 0.016).unwrap();
        },
    );
}

fn install_test_resources(engine: &mut Engine) {
    let mut resources = ResourceSet::default();
    resources.insert(ResourceKind::Texture, "tree", vec![0, 1, 2]);
    resources.insert(ResourceKind::Font, "font", vec![b'{']);
    engine.rom_resources = Some(resources);
    engine.animations.insert(
        "anim".into(),
        AnimationData {
            name: "anim".into(),
            src: String::new(),
            rate: 1.0,
            sequence: vec![],
            offsets: vec![],
            metadata: None,
        },
    );
}

#[test]
fn has_resource_and_texture_size_queries() {
    let mut engine = Engine::new_for_test();
    install_test_resources(&mut engine);

    assert!(engine.has_texture("tree"));
    assert!(engine.has_font("font"));
    assert!(engine.has_animation("anim"));
    assert!(!engine.has_texture("nope"));
    // Dimensions are only known once the texture is uploaded to GL.
    assert_eq!(engine.texture_size("tree"), None);
}

#[test]
fn guest_has_resource_wiring() {
    with_each_runtime(
        r#"(module
            (import "env" "has_resource" (func $has (param i32 i32 i32) (result i32)))
            (import "env" "spawn" (func $spawn (param i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "tree")
            (data (i32.const 16) "font")
            (data (i32.const 32) "anim")
            (data (i32.const 48) "marker")
            (func (export "update") (param f64)
                (if (call $has (i32.const 0) (i32.const 0) (i32.const 4))
                    (then
                        (if (call $has (i32.const 1) (i32.const 16) (i32.const 4))
                            (then
                                (if (call $has (i32.const 2) (i32.const 32) (i32.const 4))
                                    (then (drop (call $spawn (i32.const 48) (i32.const 6)))))))))))"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            install_test_resources(&mut engine);
            rt.update(&mut engine, 0.016).unwrap();
            assert!(engine.has_name("marker"));
        },
    );
}
