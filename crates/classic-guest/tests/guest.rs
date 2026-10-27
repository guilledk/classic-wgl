//! Integration tests for the WASM guest runtime, driving every backend (wasmi
//! always; wasmtime on native) with small hand-written WAT guest modules.

use classic_core::components::{
    Animator, ColliderData, Disabled, Emitter, IsoSprite, IsoVehicle, Light, Model, NavMesh, Role,
    Shape, Tilemap,
};
use classic_core::types::{AnimationData, SdfFontMetrics};
use classic_core::RoleKind;
use classic_engine::Engine;
#[cfg(not(target_arch = "wasm32"))]
use classic_guest::WasmtimeRuntime;
use classic_guest::{GuestError, GuestLimits, GuestRuntime, WasmiRuntime};
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

/// Every `native` entry in the ABI table links, with its table signature, on
/// every native backend.
#[test]
fn native_backends_link_every_table_import() {
    use classic_core::abi_manifest::{imports_for, Backend};

    let imports: Vec<String> =
        imports_for(Backend::Native).map(|import| import.wat_import("env")).collect();
    let wat = format!(
        "(module {} (memory (export \"memory\") 1) (func (export \"update\") (param f64)))",
        imports.join(" ")
    );
    let runtimes =
        runtimes_from_wat(&wat, &GuestLimits::default()).expect("every native table import links");
    assert!(!runtimes.is_empty());
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
fn guest_can_set_sprite_frame_and_color() {
    with_each_runtime(
        r#"(module
            (import "env" "set_sprite_frame" (func $set_frame (param i32 i32 f64) (result i32)))
            (import "env" "set_sprite_color" (func $set_color (param i32 i32 f64 f64 f64 f64) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "unit")
            (func (export "update") (param f64)
                (drop (call $set_frame (i32.const 0) (i32.const 4) (f64.const 42.0)))
                (drop (call $set_color (i32.const 0) (i32.const 4) (f64.const 0.2) (f64.const 0.4) (f64.const 0.6) (f64.const 1.0)))))"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            engine.spawn_named("unit");
            let entity = *engine.names.get("unit").unwrap();
            engine
                .world
                .insert_one(
                    entity,
                    IsoSprite {
                        position: Vec3::ZERO,
                        scale: Vec3::ONE,
                        texture: "shippingContainerBody".into(),
                        tilemap: "tilemap".into(),
                        frame: 0.0,
                        frame_name: None,
                        tile_set_size: glam::Vec2::ONE,
                        anchor: glam::Vec2::new(0.5, 0.67),
                        frame_offset: Vec3::ZERO,
                        footprint: vec![],
                        ghost_group: 0,
                        color: [1.0, 1.0, 1.0, 1.0],
                        dir_blend: None,
                        no_ghost: false,
                        team_tint: None,
                        emissive: None,
                    },
                )
                .unwrap();

            rt.update(&mut engine, 0.016).unwrap();

            let sprite = engine.world.get::<&IsoSprite>(entity).unwrap();
            assert_eq!(sprite.frame, 42.0);
            assert_eq!(sprite.color, [0.2, 0.4, 0.6, 1.0]);
        },
    );
}

#[test]
fn guest_can_spawn_sprite_clone() {
    with_each_runtime(
        r#"(module
            (import "env" "spawn_sprite_clone" (func $clone (param i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "template")
            (data (i32.const 16) "clone")
            (func (export "update") (param f64)
                (drop (call $clone (i32.const 0) (i32.const 8) (i32.const 16) (i32.const 5)))))"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            engine.spawn_named("template");
            let template = *engine.names.get("template").unwrap();
            engine
                .world
                .insert_one(
                    template,
                    IsoSprite {
                        position: Vec3::ZERO,
                        scale: Vec3::ONE,
                        texture: "shippingContainerBody".into(),
                        tilemap: "tilemap".into(),
                        frame: 7.0,
                        frame_name: Some("shippingContainerBody_7".into()),
                        tile_set_size: glam::Vec2::ONE,
                        anchor: glam::Vec2::new(0.5, 0.67),
                        frame_offset: Vec3::ZERO,
                        footprint: vec![],
                        ghost_group: 0,
                        color: [0.55, 0.65, 0.95, 1.0],
                        dir_blend: None,
                        no_ghost: false,
                        team_tint: None,
                        emissive: None,
                    },
                )
                .unwrap();

            rt.update(&mut engine, 0.016).unwrap();

            assert!(engine.has_name("clone"));
            let clone = *engine.names.get("clone").unwrap();
            let sprite = engine.world.get::<&IsoSprite>(clone).unwrap();
            assert_eq!(sprite.texture, "shippingContainerBody");
            assert_eq!(sprite.frame, 7.0);
            assert_eq!(sprite.color, [0.55, 0.65, 0.95, 1.0]);
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
        blend: None,
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
fn request_poll_path_sync_roundtrip() {
    let mut engine = Engine::new_for_test();
    install_test_navmesh(&mut engine);
    engine.set_nav_bulk(&[1u32; 9]);
    engine.set_synchronous_workers(true);

    let id = engine.request_path((0, 0), (2, 0));
    match engine.poll_path(id) {
        classic_core::pathfinder::PathPoll::Path(path) => {
            assert_eq!(path, vec![(0, 0), (1, 0), (2, 0)]);
        }
        other => panic!("expected a path, got {other:?}"),
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn request_poll_path_async_roundtrip() {
    let mut engine = Engine::new_for_test();
    install_test_navmesh(&mut engine);
    engine.set_nav_bulk(&[1u32; 9]);

    let id = engine.request_path((0, 0), (2, 2));
    let mut result = classic_core::pathfinder::PathPoll::Pending;
    for _ in 0..1000 {
        result = engine.poll_path(id);
        if result != classic_core::pathfinder::PathPoll::Pending {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    match result {
        classic_core::pathfinder::PathPoll::Path(path) => {
            assert_eq!(path, vec![(0, 0), (1, 1), (2, 2)]);
        }
        other => panic!("expected a path, got {other:?}"),
    }
}

#[test]
fn guest_request_poll_path_imports_wired() {
    with_each_runtime(
        r#"(module
            (import "env" "request_path" (func $request_path (param i32 i32 i32 i32) (result i32)))
            (import "env" "poll_path" (func $poll_path (param i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (func (export "update") (param f64)
                (drop (call $poll_path
                    (call $request_path (i32.const 0) (i32.const 0) (i32.const 2) (i32.const 0))
                    (i32.const 64) (i32.const 256)))))"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            install_test_navmesh(&mut engine);
            engine.set_nav_bulk(&[1u32; 9]);
            engine.set_synchronous_workers(true);
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
            (import "env" "vehicle_goto_poll" (func $gp (param i32) (result i32)))
            (import "env" "vehicle_stop" (func $s (param i32 i32) (result i32)))
            (import "env" "vehicle_spawn" (func $sp (param i32 i32 i32 i32 f64 f64) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "lrv")
            (data (i32.const 16) "rover")
            (func (export "update") (param f64) (local $id i32)
                (drop (call $sp (i32.const 0) (i32.const 3) (i32.const 16) (i32.const 5)
                    (f64.const 1.0) (f64.const 1.0)))
                (drop (call $t (i32.const 16) (i32.const 5) (f64.const 1.0) (f64.const 1.0)))
                (local.set $id (call $g (i32.const 16) (i32.const 5) (i32.const 2) (i32.const 0)))
                (drop (call $gp (local.get $id)))
                (drop (call $s (i32.const 16) (i32.const 5)))))"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            rt.update(&mut engine, 0.016).unwrap();
        },
    );
}

#[test]
fn guest_vehicle_face_and_reverse_imports_wired() {
    // Writes the `vehicle_face` / `vehicle_reverse_to` results for an unknown
    // vehicle (0 each) into `r.x` / `r.y`: both link and reach the engine.
    with_each_runtime(
        r#"(module
            (import "env" "vehicle_face" (func $face (param i32 i32 f64) (result i32)))
            (import "env" "vehicle_reverse_to" (func $rev (param i32 i32 f64 f64) (result i32)))
            (import "env" "set_pos" (func $set (param i32 i32 f64 f64 f64) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "mule")
            (data (i32.const 8) "r")
            (func (export "update") (param f64)
                (drop (call $set (i32.const 8) (i32.const 1)
                    (f64.convert_i32_s (call $face (i32.const 0) (i32.const 4) (f64.const 90)))
                    (f64.convert_i32_s (call $rev (i32.const 0) (i32.const 4)
                        (f64.const 3) (f64.const 4)))
                    (f64.const 7)))))"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            engine.spawn_named("r");
            rt.update(&mut engine, 0.016).unwrap();
            assert_eq!(engine.get_pos("r"), Some((0.0, 0.0, 7.0)), "unknown vehicle: 0, 0");
        },
    );
}

#[test]
fn guest_floor_rect_imports_wired() {
    // Sets floor 3 at 7.5 m (over the 1 m test map) on tiles (1..2) x (1..2), writes the ground
    // height there to `g.x`, then clears it (twice: 1, then 0) into `g.y/z`.
    with_each_runtime(
        r#"(module
            (import "env" "set_floor_rect" (func $set_floor (param i32 f64 f64 f64 f64 f64) (result i32)))
            (import "env" "clear_floor_rect" (func $clear (param i32) (result i32)))
            (import "env" "set_pos" (func $set (param i32 i32 f64 f64 f64) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "g")
            (func (export "update") (param f64)
                (drop (call $set_floor (i32.const 3) (f64.const 2) (f64.const 2)
                    (f64.const 1) (f64.const 1) (f64.const 7.5)))
                (drop (call $set (i32.const 0) (i32.const 1)
                    (f64.convert_i32_s (call $clear (i32.const 3)))
                    (f64.convert_i32_s (call $clear (i32.const 3)))
                    (f64.const 0)))
                (drop (call $set_floor (i32.const 3) (f64.const 2) (f64.const 2)
                    (f64.const 1) (f64.const 1) (f64.const 7.5)))))"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            install_test_tilemap(&mut engine);
            engine.spawn_named("g");
            rt.update(&mut engine, 0.016).unwrap();
            assert_eq!(engine.get_pos("g"), Some((1.0, 0.0, 0.0)), "cleared once, then gone");
            assert_eq!(engine.ground_at(1.5, 1.5), 7.5, "set again, either corner order");
            assert!(engine.height_at(1.5, 1.5) < 7.5, "the terrain is unchanged");
        },
    );
}

#[test]
fn guest_vehicle_goto_pose_import_wired() {
    // An unknown vehicle and a non-finite pose both answer `-1`.
    with_each_runtime(
        r#"(module
            (import "env" "vehicle_goto_pose" (func $pose (param i32 i32 f64 f64 f64) (result i32)))
            (import "env" "set_pos" (func $set (param i32 i32 f64 f64 f64) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "g")
            (data (i32.const 8) "nope")
            (func (export "update") (param f64)
                (drop (call $set (i32.const 0) (i32.const 1)
                    (f64.convert_i32_s (call $pose (i32.const 8) (i32.const 4)
                        (f64.const 3) (f64.const 4) (f64.const 90)))
                    (f64.convert_i32_s (call $pose (i32.const 0) (i32.const 1)
                        (f64.const nan) (f64.const 4) (f64.const 90)))
                    (f64.const 7)))))"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            install_test_tilemap(&mut engine);
            engine.spawn_named("g");
            rt.update(&mut engine, 0.016).unwrap();
            assert_eq!(engine.get_pos("g"), Some((-1.0, -1.0, 7.0)));
        },
    );
}

#[test]
fn guest_vehicle_block_owner_import_wired() {
    // No block 4: `0`; set it, then own it (`1`) and disown it (`1`).
    with_each_runtime(
        r#"(module
            (import "env" "set_vehicle_block" (func $set_block (param i32 i32 i32) (result i32)))
            (import "env" "set_vehicle_block_owner" (func $own (param i32 i32 i32) (result i32)))
            (import "env" "set_pos" (func $set (param i32 i32 f64 f64 f64) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "g")
            (data (i32.const 8) "mule")
            (data (i32.const 16) "\01\00\00\00\01\00\00\00\02\00\00\00\02\00\00\00")
            (func (export "update") (param f64)
                (local $none i32)
                (local.set $none (call $own (i32.const 4) (i32.const 8) (i32.const 4)))
                (drop (call $set_block (i32.const 4) (i32.const 16) (i32.const 16)))
                (drop (call $set (i32.const 0) (i32.const 1)
                    (f64.convert_i32_s (local.get $none))
                    (f64.convert_i32_s (call $own (i32.const 4) (i32.const 8) (i32.const 4)))
                    (f64.const 0)))))"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            install_test_tilemap(&mut engine);
            engine.spawn_named("g");
            rt.update(&mut engine, 0.016).unwrap();
            assert_eq!(engine.get_pos("g"), Some((0.0, 1.0, 0.0)));
            assert_eq!(engine.vehicle_block_owners.get(&4).map(String::as_str), Some("mule"));
        },
    );
}

#[test]
fn guest_vehicle_heading_and_reverse_speed_imports_wired() {
    // No vehicle `nope`: the heading reads NaN (stored as `x != x`) and the
    // reverse speed is refused (`0`).
    with_each_runtime(
        r#"(module
            (import "env" "vehicle_heading" (func $heading (param i32 i32) (result f64)))
            (import "env" "vehicle_set_reverse_speed" (func $rev (param i32 i32 f64) (result i32)))
            (import "env" "set_pos" (func $set (param i32 i32 f64 f64 f64) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "g")
            (data (i32.const 8) "nope")
            (func (export "update") (param f64)
                (local $h f64)
                (local.set $h (call $heading (i32.const 8) (i32.const 4)))
                (drop (call $set (i32.const 0) (i32.const 1)
                    (f64.convert_i32_s (f64.ne (local.get $h) (local.get $h)))
                    (f64.convert_i32_s (call $rev (i32.const 8) (i32.const 4) (f64.const 1.6)))
                    (f64.const 3)))))"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            install_test_tilemap(&mut engine);
            engine.spawn_named("g");
            rt.update(&mut engine, 0.016).unwrap();
            assert_eq!(engine.get_pos("g"), Some((1.0, 0.0, 3.0)));
        },
    );
}

#[test]
fn guest_vehicle_block_imports_wired() {
    // Sets block 4 to two rects (the second in reverse corner order), writes
    // the rect count, a ragged list's `-1` and two clears (1, then 0) to `g`,
    // then sets it again.
    with_each_runtime(
        r#"(module
            (import "env" "set_vehicle_block" (func $set_block (param i32 i32 i32) (result i32)))
            (import "env" "clear_vehicle_block" (func $clear (param i32) (result i32)))
            (import "env" "set_pos" (func $set (param i32 i32 f64 f64 f64) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "g")
            (data (i32.const 16) "\01\00\00\00\01\00\00\00\02\00\00\00\02\00\00\00\05\00\00\00\05\00\00\00\03\00\00\00\03\00\00\00")
            (func (export "update") (param f64)
                (drop (call $set (i32.const 0) (i32.const 1)
                    (f64.convert_i32_s (call $set_block (i32.const 4) (i32.const 16) (i32.const 32)))
                    (f64.convert_i32_s (call $set_block (i32.const 4) (i32.const 16) (i32.const 12)))
                    (f64.convert_i32_s (i32.add
                        (i32.mul (call $clear (i32.const 4)) (i32.const 10))
                        (call $clear (i32.const 4))))))
                (drop (call $set_block (i32.const 4) (i32.const 16) (i32.const 32)))))"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            install_test_tilemap(&mut engine);
            engine.spawn_named("g");
            rt.update(&mut engine, 0.016).unwrap();
            assert_eq!(
                engine.get_pos("g"),
                Some((2.0, -1.0, 10.0)),
                "2 rects, ragged, cleared once"
            );
            assert_eq!(engine.vehicle_blocks.get(&4), Some(&vec![[1, 1, 2, 2], [3, 3, 5, 5]]));
        },
    );
}

#[test]
fn guest_selection_and_speed_imports_wired() {
    with_each_runtime(
        r#"(module
            (import "env" "vehicle_set_speed" (func $vss (param i32 i32 f64) (result i32)))
            (import "env" "vehicle_probe" (func $vprobe (param i32 i32 i32 i32) (result i32)))
            (import "env" "vehicle_probe_clear" (func $vclear (param i32 i32) (result i32)))
            (import "env" "vehicle_footprint_radius" (func $vfpr (param i32 i32) (result f64)))
            (import "env" "selected_names" (func $sel (param i32 i32) (result i32)))
            (import "env" "selection_clear" (func $clear (result i32)))
            (import "env" "set_sprite_offset" (func $soff (param i32 i32 f64 f64 f64) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "lrv")
            (func (export "update") (param f64)
                (drop (call $vss (i32.const 0) (i32.const 3) (f64.const 1.3)))
                (drop (call $vprobe (i32.const 0) (i32.const 3) (i32.const 2) (i32.const 0)))
                (drop (call $vclear (i32.const 0) (i32.const 3)))
                (drop (call $vfpr (i32.const 0) (i32.const 3)))
                (drop (call $sel (i32.const 64) (i32.const 64)))
                (drop (call $clear))
                (drop (call $soff (i32.const 0) (i32.const 3) (f64.const 0.0) (f64.const -448.0) (f64.const 0.0)))))"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            rt.update(&mut engine, 0.016).unwrap();
        },
    );
}

#[test]
fn guest_inventory_imports_wired() {
    with_each_runtime(
        r#"(module
            (import "env" "inventory_dump" (func $idump (param i32 i32 i32 i32) (result i32)))
            (import "env" "inventory_add" (func $iadd (param i32 i32 i32 i32 i32) (result i32)))
            (import "env" "inventory_remove" (func $irem (param i32 i32 i32 i32 i32) (result i32)))
            (import "env" "inventory_transfer" (func $itrf (param i32 i32 i32 i32 i32 i32 i32) (result i32)))
            (import "env" "item_def" (func $idef (param i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "lrv")
            (data (i32.const 16) "ore")
            (data (i32.const 32) "rocket")
            (func (export "update") (param f64)
                (drop (call $iadd (i32.const 0) (i32.const 3) (i32.const 16) (i32.const 3) (i32.const 1)))
                (drop (call $irem (i32.const 0) (i32.const 3) (i32.const 16) (i32.const 3) (i32.const 1)))
                (drop (call $itrf (i32.const 32) (i32.const 6) (i32.const 0) (i32.const 3) (i32.const 16) (i32.const 3) (i32.const 1)))
                (drop (call $idump (i32.const 0) (i32.const 3) (i32.const 64) (i32.const 64)))
                (drop (call $idef (i32.const 16) (i32.const 3) (i32.const 128) (i32.const 128)))))"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            rt.update(&mut engine, 0.016).unwrap();
        },
    );
}

#[test]
fn guest_get_sprite_frame_and_inventory_capacity_wired() {
    with_each_runtime(
        r#"(module
            (import "env" "get_sprite_frame" (func $gsf (param i32 i32) (result f64)))
            (import "env" "inventory_capacity" (func $icap (param i32 i32) (result i32)))
            (import "env" "set_sprite_frame" (func $ssf (param i32 i32 f64) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "unit")
            (data (i32.const 16) "rocket")
            (data (i32.const 32) "copy_a")
            (data (i32.const 64) "copy_b")
            (func (export "update") (param f64)
                (drop (call $ssf (i32.const 32) (i32.const 6)
                         (call $gsf (i32.const 0) (i32.const 4))))
                (drop (call $ssf (i32.const 64) (i32.const 6)
                         (f64.convert_i32_s (call $icap (i32.const 16) (i32.const 6)))))))"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            let sprite = IsoSprite {
                position: Vec3::ZERO,
                scale: Vec3::ONE,
                texture: "shippingContainerBody".into(),
                tilemap: "tilemap".into(),
                frame: 0.0,
                frame_name: None,
                tile_set_size: glam::Vec2::ONE,
                anchor: glam::Vec2::new(0.5, 0.67),
                frame_offset: Vec3::ZERO,
                footprint: vec![],
                ghost_group: 0,
                color: [1.0, 1.0, 1.0, 1.0],
                dir_blend: None,
                no_ghost: false,
                team_tint: None,
                emissive: None,
            };

            engine.spawn_named("unit");
            let unit = *engine.names.get("unit").unwrap();
            let mut unit_sprite = sprite.clone();
            unit_sprite.frame = 42.0;
            engine.world.insert_one(unit, unit_sprite).unwrap();

            engine.spawn_named("copy_a");
            let copy_a = *engine.names.get("copy_a").unwrap();
            engine.world.insert_one(copy_a, sprite.clone()).unwrap();

            engine.spawn_named("copy_b");
            let copy_b = *engine.names.get("copy_b").unwrap();
            engine.world.insert_one(copy_b, sprite.clone()).unwrap();

            engine.spawn_named("rocket");
            let rocket = *engine.names.get("rocket").unwrap();
            engine
                .world
                .insert_one(
                    rocket,
                    classic_core::inventory::Inventory {
                        capacity: 7,
                        kind: "cargo_bay".into(),
                        ..Default::default()
                    },
                )
                .unwrap();

            rt.update(&mut engine, 0.016).unwrap();

            assert_eq!(engine.world.get::<&IsoSprite>(copy_a).unwrap().frame, 42.0);
            assert_eq!(engine.world.get::<&IsoSprite>(copy_b).unwrap().frame, 7.0);
        },
    );
}

#[test]
fn guest_inventory_ui_show_wired() {
    with_each_runtime(
        r#"(module
            (import "env" "inventory_ui_show" (func $show (param i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "unit")
            (func (export "update") (param f64)
                (drop (call $show (i32.const 0) (i32.const 4)))
                (drop (call $show (i32.const 0) (i32.const 0)))))"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            // Show "unit" then hide with an empty name; the import must not
            // trap (neither entity needs an Inventory for the intent itself).
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

/// A minimal worker guest module (the "second instance") exporting an `entry`
/// that returns the two bytes "OK".
fn worker_ok_wasm() -> Vec<u8> {
    wat::parse_str(
        r#"(module
            (import "env" "task_return" (func $task_return (param i32 i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "OK")
            (func (export "entry")
                (call $task_return (i32.const 0) (i32.const 2))))"#,
    )
    .unwrap()
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn engine_spawn_poll_task_roundtrip() {
    let mut engine = Engine::new_for_test();
    engine.install_guest_worker(&worker_ok_wasm()).unwrap();

    let id = engine.spawn_task("entry", vec![1, 2, 3]);
    let mut result = None;
    for _ in 0..1000 {
        result = engine.poll_task(id);
        if result.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(result, Some(Ok(b"OK".to_vec())));
}

#[test]
fn guest_spawn_poll_task_imports_wired() {
    with_each_runtime(
        r#"(module
            (import "env" "spawn_task" (func $spawn_task (param i32 i32 i32 i32) (result i32)))
            (import "env" "poll_task" (func $poll_task (param i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "entry")
            (func (export "update") (param f64)
                (drop (call $poll_task
                    (call $spawn_task (i32.const 0) (i32.const 5) (i32.const 64) (i32.const 0))
                    (i32.const 128) (i32.const 256)))))"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            engine.install_guest_worker(&worker_ok_wasm()).unwrap();
            rt.update(&mut engine, 0.016).unwrap();
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

    assert_eq!(engine.pick_at(100.0, 100.0, ""), Some("tree".to_string()));
    assert_eq!(engine.pick_at(500.0, 500.0, ""), None);
}

#[test]
fn pick_at_filters_by_component() {
    let mut engine = Engine::new_for_test();
    engine.spawn_named("crate");
    assert!(engine.spawn_collider("crate", 50.0, 60.0, 20.0, 10.0));

    engine.spawn_named("container");
    let entity = *engine.names.get("container").unwrap();
    engine.world.insert_one(entity, classic_core::inventory::Inventory::default()).unwrap();
    assert!(engine.spawn_collider("container", 50.0, 60.0, 20.0, 10.0));

    engine.physics.begin_frame();

    // No filter: top collider (crate registered first → lowest pid).
    assert_eq!(engine.pick_at(50.0, 60.0, ""), Some("crate".to_string()));
    // Inventory filter: skips crate, returns the inventory-bearing container.
    assert_eq!(engine.pick_at(50.0, 60.0, "Inventory"), Some("container".to_string()));
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
fn guest_light_pool_spawn_set_release() {
    with_each_runtime(
        r#"(module
            (import "env" "light_spawn" (func $spawn (param i32 f64 f64 f64 f64 f64 f64 f64 f64 f64) (result i32)))
            (import "env" "light_set" (func $set (param i32 f64 f64 f64 f64 f64 f64 f64 f64) (result i32)))
            (import "env" "light_release" (func $release (param i32) (result i32)))
            (global $handle (mut i32) (i32.const -1))
            (global $frame (mut i32) (i32.const 0))
            (func (export "update") (param f64)
                (local $f i32)
                (local.set $f (global.get $frame))
                (if (i32.eqz (local.get $f))
                    (then
                        (global.set $handle
                            (call $spawn
                                (i32.const 0)
                                (f64.const 10) (f64.const 20) (f64.const 0)
                                (f64.const 1) (f64.const 0.5) (f64.const 0.25)
                                (f64.const 2) (f64.const 100) (f64.const 0)))))
                (if (i32.eq (local.get $f) (i32.const 1))
                    (then
                        (drop (call $set
                            (global.get $handle)
                            (f64.const 30) (f64.const 40) (f64.const 5)
                            (f64.const 0) (f64.const 0) (f64.const 1)
                            (f64.const 3) (f64.const 50)))))
                (if (i32.eq (local.get $f) (i32.const 2))
                    (then
                        (drop (call $release (global.get $handle)))))
                (global.set $frame (i32.add (local.get $f) (i32.const 1))))
        )"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            let lights = |engine: &Engine| {
                engine.world.query::<&Light>().iter().map(|(_, l)| l.clone()).collect::<Vec<_>>()
            };
            rt.update(&mut engine, 0.016).unwrap();
            assert_eq!(lights(&engine).len(), 1);
            assert_eq!(lights(&engine)[0].position.x, 10.0);
            assert_eq!(lights(&engine)[0].color, [1.0, 0.5, 0.25]);
            assert_eq!(lights(&engine)[0].intensity, 2.0);

            rt.update(&mut engine, 0.016).unwrap();
            let active = lights(&engine);
            assert_eq!(active[0].position.x, 30.0);
            assert_eq!(active[0].color, [0.0, 0.0, 1.0]);
            assert_eq!(active[0].intensity, 3.0);

            rt.update(&mut engine, 0.016).unwrap();
            assert_eq!(lights(&engine).len(), 0);
        },
    );
}

/// A test engine with a tiny effect library registered under `t`.
fn engine_with_effects() -> Engine {
    let mut engine = Engine::new_for_test();
    let lib = r#"{"effects":[
      {"name":"pop","layers":[{"type":"particles","name":"p","max":64,"spawn":{"burst":8},"life":[0.5,0.5]}]},
      {"name":"stream","layers":[{"type":"particles","name":"p","max":256,"spawn":{"rate":60},"life":[1,1]}]}
    ]}"#;
    engine.register_effects(
        "t",
        classic_core::effects::EffectLibrary::from_json(lib.as_bytes()).unwrap(),
    );
    engine
}

#[test]
fn guest_fx_emit_and_emitter_pool_spawn_set_release() {
    with_each_runtime(
        r#"(module
            (import "env" "fx_emit" (func $fx (param i32 i32 f64 f64 f64 f64 f64 f64 f64) (result i32)))
            (import "env" "emitter_spawn" (func $espawn (param i32 i32 f64 f64 f64 f64 f64 f64 f64) (result i32)))
            (import "env" "emitter_set" (func $eset (param i32 f64 f64 f64 f64 f64 f64 f64 f64 f64) (result i32)))
            (import "env" "emitter_release" (func $erelease (param i32) (result i32)))
            (import "env" "spawn" (func $spawn (param i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "pop")
            (data (i32.const 8) "nope")
            (data (i32.const 16) "t::stream")
            (data (i32.const 32) "bad")
            (global $handle (mut i32) (i32.const -1))
            (global $frame (mut i32) (i32.const 0))
            (func $flag_if (param $cond i32)
                (if (local.get $cond) (then (drop (call $spawn (i32.const 32) (i32.const 3))))))
            (func (export "update") (param f64)
                (local $f i32)
                (local.set $f (global.get $frame))
                (if (i32.eqz (local.get $f))
                    (then
                        ;; a bare name resolves; an unknown one returns 0
                        (call $flag_if (i32.ne (call $fx (i32.const 0) (i32.const 3)
                            (f64.const 5) (f64.const -5) (f64.const 1)
                            (f64.const 0) (f64.const 0) (f64.const 0) (f64.const 1)) (i32.const 1)))
                        (call $flag_if (call $fx (i32.const 8) (i32.const 4)
                            (f64.const 0) (f64.const 0) (f64.const 0)
                            (f64.const 0) (f64.const 0) (f64.const 1) (f64.const 1)))
                        (call $flag_if (i32.ne (call $espawn (i32.const 8) (i32.const 4)
                            (f64.const 0) (f64.const 0) (f64.const 0)
                            (f64.const 0) (f64.const 0) (f64.const 1) (f64.const 1)) (i32.const -1)))
                        (global.set $handle (call $espawn (i32.const 16) (i32.const 9)
                            (f64.const 10) (f64.const 20) (f64.const 0)
                            (f64.const 1) (f64.const 0) (f64.const 0) (f64.const 2)))))
                (if (i32.eq (local.get $f) (i32.const 1))
                    (then
                        (call $flag_if (i32.ne (call $eset (global.get $handle)
                            (f64.const 30) (f64.const 40) (f64.const 5)
                            (f64.const 0) (f64.const 0) (f64.const 0)
                            (f64.const 3) (f64.const 0.5) (f64.const 4)) (i32.const 1)))
                        (call $flag_if (call $eset (i32.const -1)
                            (f64.const 0) (f64.const 0) (f64.const 0)
                            (f64.const 0) (f64.const 0) (f64.const 0)
                            (f64.const 1) (f64.const 1) (f64.const 1)))))
                (if (i32.eq (local.get $f) (i32.const 2))
                    (then
                        (call $flag_if (i32.ne (call $erelease (global.get $handle)) (i32.const 1)))
                        (call $flag_if (call $erelease (global.get $handle)))))
                (global.set $frame (i32.add (local.get $f) (i32.const 1))))
        )"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = engine_with_effects();
            let emitters = |engine: &Engine| {
                engine.world.query::<&Emitter>().iter().map(|(_, e)| e.clone()).collect::<Vec<_>>()
            };
            rt.update(&mut engine, 0.016).unwrap();
            engine.update_particles(0.016);
            assert_eq!(engine.particle_stats().live, 8, "the one-shot burst");
            let spawned = emitters(&engine);
            assert_eq!(spawned.len(), 1);
            assert_eq!(spawned[0].effect, "t::stream");
            assert_eq!(spawned[0].position, Vec3::new(10.0, 20.0, 0.0));
            assert_eq!(spawned[0].dir, Vec3::X);
            assert_eq!(spawned[0].scale, 2.0);

            rt.update(&mut engine, 0.016).unwrap();
            let set = emitters(&engine);
            assert_eq!(set[0].position, Vec3::new(30.0, 40.0, 5.0));
            assert_eq!(set[0].dir, Vec3::Z, "a zero direction means up");
            assert_eq!((set[0].scale, set[0].rate, set[0].intensity), (3.0, 0.5, 4.0));
            assert_eq!(set[0].effect, "t::stream", "the effect is kept");

            rt.update(&mut engine, 0.016).unwrap();
            assert!(emitters(&engine).is_empty());
            assert!(!engine.has_name("bad"), "every return code matched");
        },
    );
}

/// A guest that keeps a light/emitter handle past its release must get `0`
/// from `*_set`/`*_release` once the slot is reused, never drive the reuser.
#[test]
fn guest_stale_handles_after_release_are_rejected() {
    with_each_runtime(
        r#"(module
            (import "env" "light_spawn" (func $lspawn (param i32 f64 f64 f64 f64 f64 f64 f64 f64 f64) (result i32)))
            (import "env" "light_set" (func $lset (param i32 f64 f64 f64 f64 f64 f64 f64 f64) (result i32)))
            (import "env" "light_release" (func $lrelease (param i32) (result i32)))
            (import "env" "emitter_spawn" (func $espawn (param i32 i32 f64 f64 f64 f64 f64 f64 f64) (result i32)))
            (import "env" "emitter_set" (func $eset (param i32 f64 f64 f64 f64 f64 f64 f64 f64 f64) (result i32)))
            (import "env" "emitter_release" (func $erelease (param i32) (result i32)))
            (import "env" "spawn" (func $spawn (param i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "t::stream")
            (data (i32.const 32) "bad")
            (func $flag_if (param $cond i32)
                (if (local.get $cond) (then (drop (call $spawn (i32.const 32) (i32.const 3))))))
            (func $light (param $i f64) (result i32)
                (call $lspawn (i32.const 0)
                    (f64.const 1) (f64.const 2) (f64.const 0)
                    (f64.const 1) (f64.const 1) (f64.const 1)
                    (local.get $i) (f64.const 10) (f64.const 0)))
            (func $emitter (param $x f64) (result i32)
                (call $espawn (i32.const 0) (i32.const 9)
                    (local.get $x) (f64.const 0) (f64.const 0)
                    (f64.const 0) (f64.const 0) (f64.const 1) (f64.const 1)))
            (func (export "update") (param f64)
                (local $stale i32) (local $fresh i32)
                ;; lights: release, respawn into the same slot, poke the stale handle
                (local.set $stale (call $light (f64.const 1)))
                (call $flag_if (i32.ne (call $lrelease (local.get $stale)) (i32.const 1)))
                (local.set $fresh (call $light (f64.const 9)))
                (call $flag_if (i32.lt_s (local.get $fresh) (i32.const 0)))
                (call $flag_if (i32.eq (local.get $fresh) (local.get $stale)))
                (call $flag_if (call $lset (local.get $stale)
                    (f64.const 0) (f64.const 0) (f64.const 0)
                    (f64.const 0) (f64.const 0) (f64.const 0)
                    (f64.const 0) (f64.const 0)))
                (call $flag_if (call $lrelease (local.get $stale)))
                ;; emitters: the same dance
                (local.set $stale (call $emitter (f64.const 1)))
                (call $flag_if (i32.ne (call $erelease (local.get $stale)) (i32.const 1)))
                (local.set $fresh (call $emitter (f64.const 7)))
                (call $flag_if (i32.lt_s (local.get $fresh) (i32.const 0)))
                (call $flag_if (i32.eq (local.get $fresh) (local.get $stale)))
                (call $flag_if (call $eset (local.get $stale)
                    (f64.const 0) (f64.const 0) (f64.const 0)
                    (f64.const 0) (f64.const 0) (f64.const 0)
                    (f64.const 1) (f64.const 0) (f64.const 0)))
                (call $flag_if (call $erelease (local.get $stale))))
        )"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = engine_with_effects();
            rt.update(&mut engine, 0.016).unwrap();
            assert!(!engine.has_name("bad"), "every stale call was refused");
            let lights: Vec<Light> =
                engine.world.query::<&Light>().iter().map(|(_, l)| l.clone()).collect();
            assert_eq!(lights.len(), 1);
            assert_eq!(lights[0].intensity, 9.0, "the reusing light is untouched");
            let emitters: Vec<Emitter> =
                engine.world.query::<&Emitter>().iter().map(|(_, e)| e.clone()).collect();
            assert_eq!(emitters.len(), 1);
            assert_eq!(emitters[0].position, Vec3::new(7.0, 0.0, 0.0));
            assert_eq!(emitters[0].rate, 1.0, "the reusing emitter is untouched");
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

    assert_eq!(engine.pick_at(50.0, 60.0, ""), Some("unit".to_string()));
    assert_eq!(engine.pick_at(500.0, 500.0, ""), None);
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
fn guest_start_model_clip_import_is_wired() {
    with_each_runtime(
        r#"(module
            (import "env" "start_model_clip" (func $smc (param i32 i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "rocket")
            (data (i32.const 16) "launch")
            (func (export "update") (param f64)
                (drop (call $smc (i32.const 0) (i32.const 6) (i32.const 16) (i32.const 6) (i32.const 1)))))"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            engine.spawn_named("rocket");
            let entity = *engine.names.get("rocket").unwrap();
            let mut model = Model::new("landing", "tilemap");
            model.time = 3.0;
            engine.world.insert_one(entity, model).unwrap();
            rt.update(&mut engine, 0.016).unwrap();
            let m = engine.world.get::<&Model>(entity).unwrap();
            assert_eq!(m.model, "launch");
            assert_eq!(m.clip.as_deref(), Some("launch"));
            assert!(m.playing && m.repeat);
            assert_eq!(m.time, 0.0);
        },
    );
}

#[test]
fn guest_set_enabled_import_is_wired() {
    with_each_runtime(
        r#"(module
            (import "env" "set_enabled" (func $se (param i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "unit")
            (func (export "update") (param f64)
                (drop (call $se (i32.const 0) (i32.const 4) (i32.const 0)))))"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            engine.spawn_named("unit");
            let entity = *engine.names.get("unit").unwrap();
            rt.update(&mut engine, 0.016).unwrap();
            assert!(engine.world.get::<&Disabled>(entity).is_ok());
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
        blend: None,
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
    engine.texture_names.insert("tree".into());
    engine.sdf_fonts.insert(
        "font".into(),
        SdfFontMetrics {
            name: "font".into(),
            family: "test".into(),
            atlas_size: [128.0, 128.0],
            glyph_size: 16.0,
            spread: 2.0,
            baseline: 0.0,
            line_height: 1.0,
            glyphs: Default::default(),
        },
    );
    engine.animations.insert(
        "anim".into(),
        AnimationData {
            name: "anim".into(),
            src: String::new(),
            rate: 1.0,
            sequence: vec![],
            offsets: vec![],
            offset_keyframes: vec![],
            channels: vec![],
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
fn guest_field_registry_kernels_wired() {
    with_each_runtime(
        r#"(module
            (import "env" "alloc_field" (func $alloc (param i32 i32 i32 i32 i32) (result i32)))
            (import "env" "write_field" (func $write (param i32 i32 i32 i32) (result i32)))
            (import "env" "map_scalar" (func $map_scalar (param i32 i32 i32 f64) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "h")
            (data (i32.const 64) "\00\00\80\3f\00\00\00\40\00\00\40\40\00\00\80\40")
            (func (export "update") (param f64)
                (drop (call $alloc (i32.const 0) (i32.const 1) (i32.const 2) (i32.const 2) (i32.const 0)))
                (drop (call $write (i32.const 0) (i32.const 1) (i32.const 64) (i32.const 16)))
                (drop (call $map_scalar (i32.const 0) (i32.const 0) (i32.const 1) (f64.const 10.0)))))"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            rt.update(&mut engine, 0.016).unwrap();
            let (data, w, h) = engine.fields.f32("h").unwrap();
            assert_eq!((w, h), (2, 2));
            assert_eq!(data, &[11.0, 12.0, 13.0, 14.0]);
        },
    );
}

#[test]
fn guest_field_kernel_imports_link() {
    with_each_runtime(
        r#"(module
            (import "env" "alloc_field" (func $alloc (param i32 i32 i32 i32 i32) (result i32)))
            (import "env" "write_field" (func $write (param i32 i32 i32 i32) (result i32)))
            (import "env" "read_field" (func $read (param i32 i32 i32 i32) (result i32)))
            (import "env" "map_field" (func $map_field (param i32 i32 i32 i32 i32) (result i32)))
            (import "env" "blur_box_field" (func $blur (param i32 i32 i32) (result i32)))
            (import "env" "relax_slopes_field" (func $relax (param i32 i32 f64 i32 f64 i32 i32) (result f64)))
            (import "env" "gradient_magnitude_field" (func $grad (param i32 i32 i32 i32) (result i32)))
            (import "env" "threshold_le_field" (func $thresh (param i32 i32 i32 i32 f64) (result i32)))
            (import "env" "prune_components_field" (func $prune (param i32 i32) (result i32)))
            (import "env" "reduce_field" (func $reduce (param i32 i32 i32) (result f64)))
            (import "env" "free_field" (func $free (param i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "h")
            (data (i32.const 16) "s")
            (data (i32.const 64) "\00\00\80\3f\00\00\00\40\00\00\40\40\00\00\80\40")
            (func (export "update") (param f64)
                (drop (call $alloc (i32.const 0) (i32.const 1) (i32.const 2) (i32.const 2) (i32.const 0)))
                (drop (call $alloc (i32.const 16) (i32.const 1) (i32.const 2) (i32.const 2) (i32.const 0)))
                (drop (call $write (i32.const 0) (i32.const 1) (i32.const 64) (i32.const 16)))
                (drop (call $map_field (i32.const 0) (i32.const 16) (i32.const 1) (i32.const 0) (i32.const 1)))
                (drop (call $blur (i32.const 0) (i32.const 1) (i32.const 1)))
                (drop (call $relax (i32.const 0) (i32.const 1) (f64.const 1.0) (i32.const 10) (f64.const 0.01) (i32.const 0) (i32.const 0)))
                (drop (call $grad (i32.const 0) (i32.const 1) (i32.const 16) (i32.const 1)))
                (drop (call $thresh (i32.const 0) (i32.const 1) (i32.const 16) (i32.const 1) (f64.const 1.0)))
                (drop (call $prune (i32.const 16) (i32.const 1)))
                (drop (call $reduce (i32.const 0) (i32.const 1) (i32.const 0)))
                (drop (call $read (i32.const 0) (i32.const 1) (i32.const 256) (i32.const 256)))
                (drop (call $free (i32.const 0) (i32.const 1)))))"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            rt.update(&mut engine, 0.016).unwrap();
        },
    );
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

#[test]
fn guest_bench_imports_wired() {
    with_each_runtime(
        r#"(module
            (import "env" "bench_mode" (func $mode (result i32)))
            (import "env" "bench_section" (func $section (param i32 i32) (result i32)))
            (import "env" "bench_end" (func $end (result i32)))
            (import "env" "spawn" (func $spawn (param i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "models")
            (data (i32.const 16) "interactive")
            (func (export "update") (param f64)
                (if (i32.eqz (call $mode))
                    (then (drop (call $spawn (i32.const 16) (i32.const 11)))))
                (drop (call $section (i32.const 0) (i32.const 6)))
                (drop (call $end))))"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            rt.update(&mut engine, 0.016).unwrap();
            // No CLASSIC_BENCH under `cargo test`: interactive (0), and
            // `bench_end` records the end without closing the app.
            assert!(engine.has_name("interactive"));
            assert!(engine.bench.ended);
            assert!(engine.bench.section.is_none());
            assert!(!engine.test_should_close);
        },
    );
}

/// A v2 vehicle the layer imports act on: hull, a slewing turret with fire
/// frames, and a crane tool with a clip.
fn spawn_test_tank(engine: &mut Engine) {
    install_test_tilemap(engine);
    let tm = engine.names["tilemap"];
    engine
        .world
        .insert_one(tm, classic_core::components::Transform::new(Vec3::ZERO, Vec3::ONE))
        .unwrap();
    let def: classic_core::types::VehicleDef = serde_json::from_value(serde_json::json!({
        "version": 2, "name": "tank", "directions": 16,
        "layers": [
            { "name": "hull", "kind": "hull", "texture": "tankHull" },
            { "name": "turret", "kind": "turret", "texture": "tankTurret",
              "offset": [0.0, 0.0, 1.2], "fire_frames": 3, "yaw_rate_deg_per_sec": 90.0,
              "pitch": { "levels": 4, "min_deg": -5.0, "max_deg": 25.0 } },
            { "name": "crane", "kind": "tool", "texture": "tankCrane", "parent": "turret",
              "clips": { "swing": { "start": 0, "frames": 6 } } }
        ]
    }))
    .unwrap();
    engine.vehicles.insert("tank".into(), def);
    assert!(engine.spawn_vehicle("tank", "tank", 1.0, 1.0));
}

#[test]
fn guest_vehicle_layer_imports_drive_layers() {
    with_each_runtime(
        r#"(module
            (import "env" "vehicle_aim" (func $aim (param i32 i32 f64 f64) (result i32)))
            (import "env" "vehicle_aim_angles" (func $aa (param i32 i32 f64 f64) (result i32)))
            (import "env" "vehicle_play" (func $play (param i32 i32 i32 i32 i32 i32 f64 i32) (result i32)))
            (import "env" "vehicle_layer_visible" (func $vis (param i32 i32 i32 i32 i32) (result i32)))
            (import "env" "vehicle_layer_color" (func $col (param i32 i32 i32 i32 f64 f64 f64 f64) (result i32)))
            (import "env" "set_pos" (func $sp (param i32 i32 f64 f64 f64) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "tank")
            (data (i32.const 16) "turret")
            (data (i32.const 32) "fire")
            (data (i32.const 48) "crane")
            (data (i32.const 64) "swing")
            (data (i32.const 80) "ok")
            (func (export "update") (param f64) (local $n i32)
                (local.set $n (call $aim (i32.const 0) (i32.const 4) (f64.const 2.0) (f64.const 2.0)))
                (local.set $n (i32.add (local.get $n)
                    (call $aa (i32.const 0) (i32.const 4) (f64.const 45.0) (f64.const 10.0))))
                (local.set $n (i32.add (local.get $n)
                    (call $play (i32.const 0) (i32.const 4) (i32.const 48) (i32.const 5)
                        (i32.const 64) (i32.const 5) (f64.const -1.0) (i32.const 2))))
                (local.set $n (i32.add (local.get $n)
                    (call $play (i32.const 0) (i32.const 4) (i32.const 16) (i32.const 6)
                        (i32.const 32) (i32.const 4) (f64.const 1.0) (i32.const 1))))
                (local.set $n (i32.add (local.get $n)
                    (call $vis (i32.const 0) (i32.const 4) (i32.const 48) (i32.const 5) (i32.const 0))))
                (local.set $n (i32.add (local.get $n)
                    (call $col (i32.const 0) (i32.const 4) (i32.const 16) (i32.const 6)
                        (f64.const 1.0) (f64.const 0.0) (f64.const 0.0) (f64.const 1.0))))
                ;; Report the success count through an entity position.
                (drop (call $sp (i32.const 80) (i32.const 2)
                    (f64.convert_i32_s (local.get $n)) (f64.const 0.0) (f64.const 0.0)))))"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            spawn_test_tank(&mut engine);
            engine.spawn_named("ok");
            rt.update(&mut engine, 0.016).unwrap();
            assert_eq!(engine.get_pos("ok").map(|p| p.0), Some(6.0), "every import succeeded");
            let tank = engine.names["tank"];
            let v = engine.world.get::<&IsoVehicle>(tank).unwrap();
            let layer = |n: &str| v.layers.iter().find(|s| s.name == n).unwrap().clone();
            let turret = layer("turret");
            assert!(turret.aim.is_none(), "aim_angles drops the aim point");
            assert!((turret.yaw_target - 45f32.to_radians()).abs() < 1e-5);
            assert!((turret.pitch - 10f32.to_radians()).abs() < 1e-5);
            assert_eq!(turret.color, [1.0, 0.0, 0.0, 1.0]);
            assert_eq!(turret.clip.as_ref().map(|c| c.clip.as_str()), Some("fire"));
            let crane = layer("crane");
            assert!(!crane.visible);
            let c = crane.clip.unwrap();
            assert_eq!((c.clip.as_str(), c.speed, c.repeat), ("swing", -1.0, 2));
            drop(v);
            assert!(engine.is_disabled(engine.names["tank.crane"]));
        },
    );
}

#[test]
fn guest_vehicle_team_and_emissive_reach_every_part() {
    with_each_runtime(
        r#"(module
            (import "env" "vehicle_team" (func $team (param i32 i32 f64 f64 f64) (result i32)))
            (import "env" "vehicle_emissive" (func $em (param i32 i32 f64) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "tank")
            (func (export "update") (param f64)
                (drop (call $team (i32.const 0) (i32.const 4) (f64.const 0.2) (f64.const 0.4) (f64.const 1.0)))
                (drop (call $em (i32.const 0) (i32.const 4) (f64.const 2.5)))))"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            spawn_test_tank(&mut engine);
            rt.update(&mut engine, 0.016).unwrap();
            for part in ["tank", "tank.turret", "tank.crane"] {
                let s = engine.world.get::<&IsoSprite>(engine.names[part]).unwrap();
                assert_eq!(s.team_tint, Some([0.2, 0.4, 1.0]), "{part}");
                assert_eq!(s.emissive, Some(2.5), "{part}");
                assert_eq!(s.color, [1.0; 4], "team is separate from color");
            }
        },
    );
}

#[test]
fn guest_vehicle_socket_imports_wired() {
    with_each_runtime(
        r#"(module
            (import "env" "vehicle_socket_pos" (func $sp (param i32 i32 i32 i32 i32) (result i32)))
            (import "env" "vehicle_socket_fx" (func $fx (param i32 i32 i32 i32 i32 i32) (result i32)))
            (import "env" "set_pos" (func $set (param i32 i32 f64 f64 f64) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "tank")
            (data (i32.const 16) "lamp")
            (data (i32.const 32) "spark")
            (data (i32.const 48) "ok")
            (func (export "update") (param f64)
                (drop (call $fx (i32.const 0) (i32.const 4) (i32.const 16) (i32.const 4)
                    (i32.const 32) (i32.const 5)))
                (if (call $sp (i32.const 0) (i32.const 4) (i32.const 16) (i32.const 4) (i32.const 64))
                    (then (drop (call $set (i32.const 48) (i32.const 2)
                        (f64.load (i32.const 64)) (f64.load (i32.const 72))
                        (f64.load (i32.const 80))))))))"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            spawn_test_tank(&mut engine);
            let tank = engine.vehicles.get_mut("tank").unwrap();
            tank.sockets.insert(
                "lamp".into(),
                classic_core::vehicle_def::SocketDef {
                    layer: "hull".into(),
                    offset: [1.0, 0.0, 0.5],
                },
            );
            engine.spawn_named("ok");
            rt.update(&mut engine, 0.016).unwrap();
            let expect = engine.vehicle_socket_pos("tank", "lamp").expect("socket resolves");
            let got = engine.get_pos("ok").expect("the guest wrote the socket position");
            assert!((glam::Vec3::new(got.0, got.1, got.2) - expect).length() < 1e-4);
        },
    );
}

/// A two-node model asset: a `rig` root a clip named `fly` lifts from glTF
/// `(0, 0, 0)` to `(0, 10, 0)` (world +Z) over 1 s, with a `nozzle` child 1 m
/// below it.
fn flying_model_asset() -> classic_core::model::ModelAsset {
    use classic_core::model::{
        Interpolation, ModelAsset, ModelChannel, ModelClip, ModelNode, TrsProperty,
    };
    use glam::{Quat, Vec4};
    let node = |name: &str, parent, children, translation| ModelNode {
        name: name.into(),
        parent,
        children,
        translation,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
        mesh: None,
    };
    ModelAsset {
        nodes: vec![
            node("rig", None, vec![1], Vec3::ZERO),
            node("nozzle", Some(0), vec![], Vec3::new(0.0, -1.0, 0.0)),
        ],
        meshes: vec![],
        clips: vec![ModelClip {
            name: "fly".into(),
            duration: 1.0,
            channels: vec![ModelChannel {
                node: 0,
                property: TrsProperty::Translation,
                interpolation: Interpolation::Linear,
                times: vec![0.0, 1.0],
                values: vec![Vec4::ZERO, Vec4::new(0.0, 10.0, 0.0, 0.0)],
            }],
            node_channels: vec![],
        }],
        images: vec![],
        root: Some(0),
    }
}

#[test]
fn guest_model_imports_wired() {
    // Writes the nozzle position to `np`, the clip time to `ct.x`, and the
    // unknown-node result to `miss.x`; tints and powers the rocket up.
    with_each_runtime(
        r#"(module
            (import "env" "model_node_pos" (func $np (param i32 i32 i32 i32 i32) (result i32)))
            (import "env" "model_tint" (func $tint (param i32 i32 f64 f64 f64) (result i32)))
            (import "env" "model_emissive" (func $em (param i32 i32 f64) (result i32)))
            (import "env" "model_clip_time" (func $ct (param i32 i32) (result f64)))
            (import "env" "set_pos" (func $set (param i32 i32 f64 f64 f64) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "rocket")
            (data (i32.const 16) "nozzle")
            (data (i32.const 32) "np")
            (data (i32.const 40) "ct")
            (data (i32.const 48) "miss")
            (data (i32.const 56) "nope")
            (func (export "update") (param f64)
                (drop (call $tint (i32.const 0) (i32.const 6)
                    (f64.const 0.25) (f64.const 0.5) (f64.const 1)))
                (drop (call $em (i32.const 0) (i32.const 6) (f64.const 2.5)))
                (drop (call $set (i32.const 40) (i32.const 2)
                    (call $ct (i32.const 0) (i32.const 6)) (f64.const 0) (f64.const 0)))
                (drop (call $set (i32.const 48) (i32.const 4)
                    (f64.convert_i32_s (call $np (i32.const 0) (i32.const 6)
                        (i32.const 56) (i32.const 4) (i32.const 64)))
                    (f64.const 0) (f64.const 0)))
                (if (call $np (i32.const 0) (i32.const 6) (i32.const 16) (i32.const 6) (i32.const 64))
                    (then (drop (call $set (i32.const 32) (i32.const 2)
                        (f64.load (i32.const 64)) (f64.load (i32.const 72))
                        (f64.load (i32.const 80))))))))"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            install_test_tilemap(&mut engine);
            let tm = engine.names["tilemap"];
            engine
                .world
                .insert_one(tm, classic_core::components::Transform::new(Vec3::ZERO, Vec3::ONE))
                .unwrap();
            engine.models.insert("fly".into(), flying_model_asset());
            engine.spawn_named("rocket");
            let rocket = engine.names["rocket"];
            engine
                .world
                .insert_one(
                    rocket,
                    classic_core::components::Transform::new(Vec3::new(1.0, 1.0, 0.0), Vec3::ONE),
                )
                .unwrap();
            engine.world.insert_one(rocket, Model::new("fly", "tilemap")).unwrap();
            for n in ["np", "ct", "miss"] {
                engine.spawn_named(n);
            }
            assert!(engine.start_model_clip("rocket", "fly", false));
            // Mid-clip, as the model system would have advanced it this frame.
            engine.update_models(0.5);
            rt.update(&mut engine, 0.016).unwrap();

            let expect = engine.model_node_pos("rocket", "nozzle").expect("node resolves");
            let got = engine.get_pos("np").expect("the guest wrote the node position");
            assert!((Vec3::new(got.0, got.1, got.2) - expect).length() < 1e-4);
            // Rig lifted 5 m, nozzle 1 m below it, over the tile's ground point.
            assert!((expect.z - (1.0 + 4.0)).abs() < 1e-4, "{expect}");
            assert_eq!(engine.get_pos("ct").unwrap().0, 0.5);
            assert_eq!(engine.get_pos("miss").unwrap().0, 0.0, "unknown node is none");
            let m = engine.world.get::<&Model>(rocket).unwrap();
            assert_eq!(m.tint, Some([0.25, 0.5, 1.0]));
            assert_eq!(m.emissive, Some(2.5));
        },
    );
}

#[test]
fn guest_sprite_pose_imports_wired() {
    // `a` is posed by heading + tilt, `b` fitted to the (flat) terrain.
    with_each_runtime(
        r#"(module
            (import "env" "sprite_pose" (func $pose (param i32 i32 f64 f64 f64) (result i32)))
            (import "env" "sprite_fit_terrain" (func $fit (param i32 i32 f64 f64) (result i32)))
            (import "env" "set_pos" (func $set (param i32 i32 f64 f64 f64) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "a")
            (data (i32.const 8) "b")
            (data (i32.const 16) "ok")
            (func (export "update") (param f64)
                (drop (call $set (i32.const 16) (i32.const 2)
                    (f64.convert_i32_s (call $pose (i32.const 0) (i32.const 1)
                        (f64.const 100) (f64.const 10) (f64.const -20)))
                    (f64.convert_i32_s (call $fit (i32.const 8) (i32.const 1)
                        (f64.const 180) (f64.const 2.44)))
                    (f64.const 0)))))"#,
        &GuestLimits::default(),
        |rt| {
            let mut engine = Engine::new_for_test();
            install_test_tilemap(&mut engine);
            let frames: serde_json::Map<String, serde_json::Value> = (0..240)
                .map(|i| {
                    (format!("box_{i}"), serde_json::json!({"sheet": 0, "rect": [0, 0, 1, 1]}))
                })
                .collect();
            let table: classic_core::types::FrameTable =
                serde_json::from_value(serde_json::json!({
                    "version": 2, "frames": frames,
                    "axes": { "directions": 16, "pitch": { "levels": 5, "max_deg": 20 },
                              "roll": { "levels": 3, "max_deg": 20 } },
                    "sheets": [{ "name": "s", "src": "/s.png", "size": [4, 4] }]
                }))
                .unwrap();
            engine.frame_tables.insert("box".into(), table);
            for n in ["a", "b", "ok"] {
                engine.spawn_named(n);
            }
            for n in ["a", "b"] {
                let e = engine.names[n];
                let sprite = IsoSprite {
                    position: Vec3::new(1.5, 1.5, 0.0),
                    scale: Vec3::ONE,
                    texture: "box".into(),
                    tilemap: "tilemap".into(),
                    frame: 0.0,
                    frame_name: None,
                    tile_set_size: glam::Vec2::ONE,
                    anchor: glam::Vec2::new(0.5, 0.5),
                    frame_offset: Vec3::ZERO,
                    footprint: vec![],
                    ghost_group: 0,
                    color: [1.0; 4],
                    dir_blend: None,
                    no_ghost: false,
                    team_tint: None,
                    emissive: None,
                };
                engine.world.insert_one(e, sprite).unwrap();
                engine.set_pos(n, 1.5, 1.5, 0.0);
            }
            rt.update(&mut engine, 0.016).unwrap();
            assert_eq!(engine.get_pos("ok").unwrap(), (1.0, 1.0, 0.0), "both imports succeed");
            let frame = |n: &str| engine.world.get::<&IsoSprite>(engine.names[n]).unwrap().frame;
            // 100° -> dir 4; pitch 10° -> level 3; roll -20° -> level 0.
            assert_eq!(frame("a"), ((3 * 3) * 16 + 4) as f32);
            // Flat ground, facing West (dir 8): the level frame.
            assert_eq!(frame("b"), ((2 * 3 + 1) * 16 + 8) as f32);
        },
    );
}
