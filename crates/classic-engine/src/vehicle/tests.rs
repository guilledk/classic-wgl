use classic_core::components::{IsoSprite, IsoVehicle, NavMesh, Role, RoleKind, Tilemap};
use classic_core::math::{iso_camera_px, iso_world_pos};
use classic_core::types::{FrameTable, VehicleAnchors, VehicleDef, VehiclePartDef};
use glam::{Vec2, Vec3};

use crate::Engine;

use super::kinematics::{
    advance_suspension, derive_wheel_offsets, dir_to_heading, heading_to_dir, lerp_dir2, lookahead,
    pitch_index, should_reverse, soft_deadzone, steer_index, step_spring, step_steer,
    straight_steer, wrap_pi,
};
use super::*;

/// Build a `VehicleAnchors` from `(name, anchors)` pairs — the name→anchors
/// map shape the v2 exporters emit.
fn vehicle_anchors<K: Into<String>>(
    parts: Vec<(K, Vec<[f32; 2]>)>,
    tires: Vec<(K, Vec<[f32; 2]>)>,
) -> VehicleAnchors {
    VehicleAnchors {
        version: 1,
        parts: parts.into_iter().map(|(k, v)| (k.into(), v)).collect(),
        tires: tires.into_iter().map(|(k, v)| (k.into(), v)).collect(),
    }
}

fn test_tilemap() -> Tilemap {
    flat_tilemap(3, 3)
}

/// A flat, fully-walkable-looking tilemap of arbitrary size (heights all 1).
fn flat_tilemap(size_x: i32, size_y: i32) -> Tilemap {
    Tilemap {
        position: Vec3::ZERO,
        scale: Vec3::new(45.0, 45.0, 1.0),
        size_x,
        size_y,
        tile_set: "tileset".into(),
        tile_pixel_size: [32, 32],
        max_tile: 16,
        tiles_grid: None,
        heights_grid: None,
        data: vec![0; (size_x * size_y) as usize],
        height_data: vec![1.0; ((size_x + 1) * (size_y + 1)) as usize],
        height_scale: 14.0,
        tile_set_pixel_size: [0, 0],
        tiles_per_row: 0,
        mouse_iso_pos: Vec3::ZERO,
        selection_iso_begin: Vec3::new(-1.0, -1.0, -1.0),
        selection_iso_end: Vec3::new(-1.0, -1.0, -1.0),
    }
}

/// Submit a vehicle goto under `synchronous_workers` (inline search) and
/// return the poll outcome immediately, mirroring the guest's submit/poll
/// idiom.  Airborne / unknown-vehicle submissions resolve to `NoPath`.
fn goto_sync(engine: &mut Engine, name: &str, tx: i32, ty: i32) -> VehicleGotoPoll {
    engine.set_synchronous_workers(true);
    let id = match engine.vehicle_goto(name, tx, ty) {
        VehicleGotoSubmit::Submitted(id) => id,
        VehicleGotoSubmit::Airborne | VehicleGotoSubmit::NoVehicle => {
            return VehicleGotoPoll::NoPath;
        }
    };
    engine.vehicle_goto_poll(id)
}

#[test]
fn dir_index_maps_all_eight_directions() {
    assert_eq!(dir_index(1, 0), 0);
    assert_eq!(dir_index(1, 1), 1);
    assert_eq!(dir_index(0, 1), 2);
    assert_eq!(dir_index(-1, 1), 3);
    assert_eq!(dir_index(-1, 0), 4);
    assert_eq!(dir_index(-1, -1), 5);
    assert_eq!(dir_index(0, -1), 6);
    assert_eq!(dir_index(1, -1), 7);
    assert_eq!(dir_index(0, 0), 0);
}

#[test]
fn pitch_index_quantizes_five_levels() {
    let max = 20.0f32.to_radians();

    // Single level (flat) always maps to 0.
    assert_eq!(pitch_index(0.0, max, 1), 0);
    assert_eq!(pitch_index(max, max, 1), 0);

    // Five levels: 0 = nose-down, 2 = level, 4 = nose-up.
    assert_eq!(pitch_index(-max, max, 5), 0);
    assert_eq!(pitch_index(-10.0f32.to_radians(), max, 5), 1);
    assert_eq!(pitch_index(0.0, max, 5), 2);
    assert_eq!(pitch_index(10.0f32.to_radians(), max, 5), 3);
    assert_eq!(pitch_index(max, max, 5), 4);
    // Clamps outside the range.
    assert_eq!(pitch_index(-1.5, max, 5), 0);
    assert_eq!(pitch_index(1.5, max, 5), 4);
}

#[test]
fn pitch_spring_overshoots_and_bobs() {
    let dt = 1.0 / 60.0;
    let target = 0.1;
    let (mut pitch, mut vel) = (0.0f32, 0.0f32);
    let mut max_pitch = 0.0f32;
    for _ in 0..600 {
        let (p, v) = step_spring(pitch, vel, target, dt);
        pitch = p;
        vel = v;
        max_pitch = max_pitch.max(pitch);
    }
    // Underdamped: the body overshoots the target before settling near it.
    assert!(max_pitch > target, "expected overshoot, max {max_pitch} vs target {target}");
    assert!((pitch - target).abs() < 0.01, "did not settle: {pitch}");

    // An airborne target (nose-down) drives the angle negative.
    let (mut pitch, mut vel) = (0.0f32, 0.0f32);
    for _ in 0..600 {
        let (p, v) = step_spring(pitch, vel, AIRBORNE_PITCH, dt);
        pitch = p;
        vel = v;
    }
    assert!(pitch < 0.0, "airborne should pitch nose-down, got {pitch}");

    // A zero (level) roll target settles to ~0.
    let (mut roll, mut vel) = (0.5f32, 0.0f32);
    for _ in 0..600 {
        let (r, v) = step_spring(roll, vel, 0.0, dt);
        roll = r;
        vel = v;
    }
    assert!(roll.abs() < 0.01, "roll should settle to level, got {roll}");
}

#[test]
fn pitch_index_quantizes_three_roll_levels() {
    let max = 20.0f32.to_radians();
    // Three roll levels: 0 = right-up (left-down), 1 = level, 2 = left-up.
    assert_eq!(pitch_index(-max, max, 3), 0);
    assert_eq!(pitch_index(0.0, max, 3), 1);
    assert_eq!(pitch_index(max, max, 3), 2);
}

#[test]
fn soft_deadzone_zeroes_sub_threshold_slopes() {
    // Below the dead-zone the slope is zeroed; beyond it the excess passes.
    assert_eq!(soft_deadzone(0.0, 0.1), 0.0);
    assert_eq!(soft_deadzone(0.05, 0.1), 0.0);
    assert_eq!(soft_deadzone(-0.05, 0.1), 0.0);
    assert!((soft_deadzone(0.15, 0.1) - 0.05).abs() < 1e-6);
    assert!((soft_deadzone(-0.15, 0.1) + 0.05).abs() < 1e-6);
    // A zero dead-zone passes the slope through unchanged.
    assert_eq!(soft_deadzone(0.3, 0.0), 0.3);
}

#[test]
fn advance_suspension_settles_to_target() {
    let dt = 1.0 / 60.0;
    let (mut h, mut v) = (0.0f32, 0.0f32);
    for _ in 0..600 {
        let (nh, nv) = advance_suspension(h, v, 10.0, dt);
        h = nh;
        v = nv;
    }
    assert!((h - 10.0).abs() < 0.01, "suspension did not settle: {h}");
}

#[test]
fn steer_index_quantizes_demand_to_five_levels() {
    let max = 30.0f32.to_radians();
    // 0 = full-right, 2 = straight, 4 = full-left (positive demand = left).
    assert_eq!(steer_index(max, max, 5), 4);
    assert_eq!(steer_index(0.0, max, 5), 2);
    assert_eq!(steer_index(-max, max, 5), 0);
    // Half-scale demand lands on the mid frames.
    assert_eq!(steer_index(max * 0.5, max, 5), 3);
    assert_eq!(steer_index(-max * 0.5, max, 5), 1);
    // Clamps beyond the range, and a single level always stays straight.
    assert_eq!(steer_index(2.0, max, 5), 4);
    assert_eq!(steer_index(-2.0, max, 5), 0);
    assert_eq!(steer_index(1.0, max, 1), 0);
}

#[test]
fn straight_steer_is_the_centre_frame() {
    assert_eq!(straight_steer(1), 0);
    assert_eq!(straight_steer(3), 1);
    assert_eq!(straight_steer(5), 2);
}

#[test]
fn step_steer_rate_limits_the_wheel_sweep() {
    let max = 30.0f32.to_radians();
    let rate = 360.0f32.to_radians();
    // From straight to full-left in one 1/60s frame: only `rate · dt` of the
    // way there (a sweep, not a snap).
    let dt = 1.0 / 60.0;
    let stepped = step_steer(0.0, max, max, rate, dt);
    assert!(stepped > 0.0 && stepped < max, "should be between straight and full-left");
    assert!((stepped - rate * dt).abs() < 1e-6, "should move exactly rate·dt");
    // Demand is clamped to the steer envelope.
    assert_eq!(step_steer(0.0, 10.0, max, rate, 1.0), max);
    assert_eq!(step_steer(0.0, -10.0, max, rate, 1.0), -max);
}

#[test]
fn should_reverse_uses_hysteresis() {
    let enter = REVERSE_ENTER;
    let exit = REVERSE_EXIT;
    // Below enter (while driving forward) stays forward.
    assert!(!should_reverse(exit + 0.01, false));
    // Above enter flips into reverse.
    assert!(should_reverse(enter + 0.01, false));
    // While reversing, stays reversed until below exit.
    assert!(should_reverse(exit + 0.01, true));
    assert!(!should_reverse(exit - 0.01, true));
}

#[test]
fn derived_offsets_round_trip_through_iso() {
    let cell = [247.0, 247.0];
    let tile_scale = 45.0;
    let body = [[0.5f32, 0.7352f32]; 8];
    // A synthetic front-left wheel anchor per direction; exact values don't
    // matter, only that the round-trip reproduces the anchor delta.
    let mut wheels = [[[0.5f32, 0.5f32]; 8]; 4];
    wheels[0][0] = [0.566155, 0.618458];
    wheels[1][0] = [0.73363, 0.702169];
    wheels[2][0] = [0.211113, 0.795979];
    wheels[3][0] = [0.378535, 0.879716];

    let offsets = derive_wheel_offsets(&body, &wheels, cell, tile_scale);

    for i in 0..4 {
        let delta = Vec3::new(
            (wheels[i][0][0] - body[0][0]) * cell[0],
            (wheels[i][0][1] - body[0][1]) * cell[1],
            0.0,
        );
        let world = iso_world_pos(offsets[i][0][0], offsets[i][0][1], 0.0);
        let screen = iso_camera_px(world);
        assert!((screen.x - delta.x).abs() < 1e-3, "wheel {i} x {screen} vs {delta}");
        assert!((screen.y - delta.y).abs() < 1e-3, "wheel {i} y {screen} vs {delta}");
    }
}

#[test]
fn spawn_vehicle_creates_body_and_wheels() {
    let mut engine = Engine::new_for_test();
    let tm = engine.world.spawn((test_tilemap(), Role::new(RoleKind::Tilemap)));
    engine.names.insert("tilemap".into(), tm);

    let def = VehicleDef {
        name: "lrv".into(),
        directions: 8,
        anchors: "lrv_anchors".into(),
        columns: 4,
        rows: 2,
        cell: [247.0, 247.0],
        pitch_levels: 1,
        pitch_max_deg: 20.0,
        roll_levels: 1,
        roll_max_deg: 20.0,
        path_footprint: Some(vec![(0, 0)]),
        safe_fall_m: 0.0,
        steer_levels: 1,
        steer_max_deg: 30.0,
        steer_rate_deg_per_sec: 360.0,
        reverse_speed: 1.3,
        turn_cost: 0.0,
        tires: vec![],
        turn_rate_deg_per_sec: 720.0,
        parts: vec![
            VehiclePartDef { name: "body".into(), texture: "lrvBody".into() },
            VehiclePartDef { name: "wheel_fl".into(), texture: "lrvWheelFl".into() },
            VehiclePartDef { name: "wheel_fr".into(), texture: "lrvWheelFr".into() },
            VehiclePartDef { name: "wheel_rl".into(), texture: "lrvWheelRl".into() },
            VehiclePartDef { name: "wheel_rr".into(), texture: "lrvWheelRr".into() },
        ],
    };
    let anchors = vehicle_anchors(
        vec![
            ("body", vec![[0.5, 0.7352]; 8]),
            (
                "wheel_fl",
                vec![
                    [0.566, 0.618],
                    [0.712, 0.676],
                    [0.733, 0.768],
                    [0.618, 0.841],
                    [0.434, 0.852],
                    [0.288, 0.794],
                    [0.267, 0.702],
                    [0.382, 0.629],
                ],
            ),
            (
                "wheel_fr",
                vec![
                    [0.734, 0.702],
                    [0.712, 0.794],
                    [0.566, 0.852],
                    [0.382, 0.841],
                    [0.266, 0.768],
                    [0.288, 0.676],
                    [0.434, 0.618],
                    [0.618, 0.629],
                ],
            ),
            (
                "wheel_rl",
                vec![
                    [0.211, 0.796],
                    [0.210, 0.676],
                    [0.378, 0.591],
                    [0.618, 0.590],
                    [0.789, 0.674],
                    [0.790, 0.794],
                    [0.622, 0.880],
                    [0.382, 0.880],
                ],
            ),
            (
                "wheel_rr",
                vec![
                    [0.379, 0.880],
                    [0.210, 0.794],
                    [0.211, 0.674],
                    [0.382, 0.590],
                    [0.621, 0.591],
                    [0.790, 0.676],
                    [0.789, 0.796],
                    [0.618, 0.880],
                ],
            ),
        ],
        vec![],
    );
    insert_lrv(&mut engine, (def, anchors));

    assert!(engine.spawn_vehicle("lrv", "lrv", 1.0, 1.0));
    assert!(engine.has_name("lrv"));
    for s in ["lrvWheelFl", "lrvWheelFr", "lrvWheelRl", "lrvWheelRr"] {
        assert!(engine.has_name(s), "missing {s}");
    }
    assert_eq!(engine.get_pos("lrv").map(|(x, y, _)| (x, y)), Some((1.0, 1.0)));

    let body = *engine.names.get("lrv").unwrap();
    {
        let veh = engine.world.get::<&IsoVehicle>(body).unwrap();
        assert_eq!(veh.wheel_entities[0], "lrvWheelFl");
        // Wheel offsets derived (non-zero for the front-left wheel at dir 0).
        assert!(veh.wheel_tile_offsets[0][0][0].abs() > 0.0);
        assert!(veh.wheel_tile_offsets[2][0][0].abs() > 0.0);
        assert!(veh.wheel_tile_offsets[0][0][0] != veh.wheel_tile_offsets[2][0][0]);
    }

    // goto/stop still work after spawn.
    let nav = NavMesh {
        position: Vec3::ZERO,
        scale: Vec3::ONE,
        map_entity: "tilemap".into(),
        tile_set: "navTileset".into(),
        data_grid: None,
        data: vec![1; 9],
        size_x: 3,
        size_y: 3,
    };
    let nm = engine.world.spawn((nav, Role::new(RoleKind::NavMesh)));
    engine.names.insert("navmesh".into(), nm);

    assert!(matches!(goto_sync(&mut engine, "lrv", 2, 1), VehicleGotoPoll::Accepted(_)));
    assert!(!engine.world.get::<&IsoVehicle>(body).unwrap().path.is_empty());
    assert!(engine.vehicle_stop("lrv"));
    assert!(engine.world.get::<&IsoVehicle>(body).unwrap().path.is_empty());
}

#[test]
fn spawn_vehicle_sets_frame_name_when_table_exists() {
    let mut engine = Engine::new_for_test();
    let tm = engine.world.spawn((test_tilemap(), Role::new(RoleKind::Tilemap)));
    engine.names.insert("tilemap".into(), tm);
    insert_lrv(&mut engine, lrv_def(720.0));

    // An empty frame table is enough: `frame_name` only keys off the
    // texture name, so the frame resolves to `{texture}_{frame}`.
    let empty =
        FrameTable { version: 1, sheets: vec![], frames: Default::default(), companions: vec![] };
    for texture in ["lrvBody", "lrvWheelFl", "lrvWheelFr", "lrvWheelRl", "lrvWheelRr"] {
        engine.frame_tables.insert(texture.into(), empty.clone());
    }

    assert!(engine.spawn_vehicle("lrv", "lrv", 1.0, 1.0));
    let body = *engine.names.get("lrv").unwrap();
    let wheel = *engine.names.get("lrvWheelFl").unwrap();

    // Initial pose (direction 0): body frame 0, wheel frame 0.
    let body_sprite = engine.world.get::<&IsoSprite>(body).unwrap();
    assert_eq!(body_sprite.frame_name.as_deref(), Some("lrvBody_0"));
    let wheel_sprite = engine.world.get::<&IsoSprite>(wheel).unwrap();
    assert_eq!(wheel_sprite.frame_name.as_deref(), Some("lrvWheelFl_0"));

    // Without a table, `frame_name` stays `None` (grid path takes over).
    let mut bare = Engine::new_for_test();
    let tm = bare.world.spawn((test_tilemap(), Role::new(RoleKind::Tilemap)));
    bare.names.insert("tilemap".into(), tm);
    insert_lrv(&mut bare, lrv_def(720.0));
    assert!(bare.spawn_vehicle("lrv", "lrv", 1.0, 1.0));
    let bare_body = *bare.names.get("lrv").unwrap();
    assert_eq!(bare.world.get::<&IsoSprite>(bare_body).unwrap().frame_name, None);
}

#[test]
fn spawn_vehicle_sizes_body_sheet_by_pitch_levels() {
    let mut engine = Engine::new_for_test();
    let tm = engine.world.spawn((test_tilemap(), Role::new(RoleKind::Tilemap)));
    engine.names.insert("tilemap".into(), tm);

    // Front wheels anchor to the +X side of the frame, rear wheels to -X,
    // and left/right wheels to distinct +Y/-Y, so both the front-rear
    // wheelbase and the left-right track are non-zero.
    let body = [[0.5, 0.5]];
    let fl = [[0.7, 0.4]];
    let fr = [[0.7, 0.6]];
    let rl = [[0.3, 0.4]];
    let rr = [[0.3, 0.6]];
    let def = VehicleDef {
        name: "lrv".into(),
        directions: 8,
        anchors: "lrv_anchors".into(),
        columns: 4,
        rows: 2,
        cell: [247.0, 247.0],
        pitch_levels: 3,
        pitch_max_deg: 20.0,
        roll_levels: 3,
        roll_max_deg: 20.0,
        path_footprint: Some(vec![(0, 0)]),
        safe_fall_m: 0.0,
        steer_levels: 1,
        steer_max_deg: 30.0,
        steer_rate_deg_per_sec: 360.0,
        reverse_speed: 1.3,
        turn_cost: 0.0,
        tires: vec![],
        turn_rate_deg_per_sec: 720.0,
        parts: vec![
            VehiclePartDef { name: "body".into(), texture: "lrvBody".into() },
            VehiclePartDef { name: "wheel_fl".into(), texture: "lrvWheelFl".into() },
            VehiclePartDef { name: "wheel_fr".into(), texture: "lrvWheelFr".into() },
            VehiclePartDef { name: "wheel_rl".into(), texture: "lrvWheelRl".into() },
            VehiclePartDef { name: "wheel_rr".into(), texture: "lrvWheelRr".into() },
        ],
    };
    let anchors = vehicle_anchors(
        vec![
            ("body", body.to_vec()),
            ("wheel_fl", fl.to_vec()),
            ("wheel_fr", fr.to_vec()),
            ("wheel_rl", rl.to_vec()),
            ("wheel_rr", rr.to_vec()),
        ],
        vec![],
    );
    insert_lrv(&mut engine, (def, anchors));

    assert!(engine.spawn_vehicle("lrv", "lrv", 1.0, 1.0));
    let body_entity = *engine.names.get("lrv").unwrap();
    let wheel = *engine.names.get("lrvWheelFl").unwrap();

    let body_sprite = engine.world.get::<&IsoSprite>(body_entity).unwrap();
    let wheel_sprite = engine.world.get::<&IsoSprite>(wheel).unwrap();
    assert_eq!(body_sprite.tile_set_size, Vec2::new(4.0, 18.0));
    assert_eq!(wheel_sprite.tile_set_size, Vec2::new(4.0, 2.0));

    let veh = engine.world.get::<&IsoVehicle>(body_entity).unwrap();
    assert_eq!(veh.pitch_levels, 3);
    assert!((veh.pitch_max - 20.0f32.to_radians()).abs() < 1e-6);
    assert!(veh.wheelbase_m > 0.0);
    assert_eq!(veh.roll_levels, 3);
    assert!((veh.roll_max - 20.0f32.to_radians()).abs() < 1e-6);
    assert!(veh.track_m > 0.0);
}

#[test]
fn goto_rejected_while_airborne() {
    let mut engine = Engine::new_for_test();
    let tm = engine.world.spawn((test_tilemap(), Role::new(RoleKind::Tilemap)));
    engine.names.insert("tilemap".into(), tm);
    let nav = NavMesh {
        position: Vec3::ZERO,
        scale: Vec3::ONE,
        map_entity: "tilemap".into(),
        tile_set: "navTileset".into(),
        data_grid: None,
        data: vec![1; 9],
        size_x: 3,
        size_y: 3,
    };
    let nm = engine.world.spawn((nav, Role::new(RoleKind::NavMesh)));
    engine.names.insert("navmesh".into(), nm);

    let def = VehicleDef {
        name: "lrv".into(),
        directions: 8,
        anchors: "lrv_anchors".into(),
        columns: 4,
        rows: 2,
        cell: [247.0, 247.0],
        pitch_levels: 1,
        pitch_max_deg: 20.0,
        roll_levels: 1,
        roll_max_deg: 20.0,
        path_footprint: Some(vec![(0, 0)]),
        safe_fall_m: 0.0,
        steer_levels: 1,
        steer_max_deg: 30.0,
        steer_rate_deg_per_sec: 360.0,
        reverse_speed: 1.3,
        turn_cost: 0.0,
        tires: vec![],
        turn_rate_deg_per_sec: 720.0,
        parts: vec![
            VehiclePartDef { name: "body".into(), texture: "lrvBody".into() },
            VehiclePartDef { name: "wheel_fl".into(), texture: "lrvWheelFl".into() },
            VehiclePartDef { name: "wheel_fr".into(), texture: "lrvWheelFr".into() },
            VehiclePartDef { name: "wheel_rl".into(), texture: "lrvWheelRl".into() },
            VehiclePartDef { name: "wheel_rr".into(), texture: "lrvWheelRr".into() },
        ],
    };
    let anchors = vehicle_anchors(
        vec![
            ("body", vec![[0.5, 0.7352]; 8]),
            (
                "wheel_fl",
                vec![
                    [0.566, 0.618],
                    [0.712, 0.676],
                    [0.733, 0.768],
                    [0.618, 0.841],
                    [0.434, 0.852],
                    [0.288, 0.794],
                    [0.267, 0.702],
                    [0.382, 0.629],
                ],
            ),
            (
                "wheel_fr",
                vec![
                    [0.734, 0.702],
                    [0.712, 0.794],
                    [0.566, 0.852],
                    [0.382, 0.841],
                    [0.266, 0.768],
                    [0.288, 0.676],
                    [0.434, 0.618],
                    [0.618, 0.629],
                ],
            ),
            (
                "wheel_rl",
                vec![
                    [0.211, 0.796],
                    [0.210, 0.676],
                    [0.378, 0.591],
                    [0.618, 0.590],
                    [0.789, 0.674],
                    [0.790, 0.794],
                    [0.622, 0.880],
                    [0.382, 0.880],
                ],
            ),
            (
                "wheel_rr",
                vec![
                    [0.379, 0.880],
                    [0.210, 0.794],
                    [0.211, 0.674],
                    [0.382, 0.590],
                    [0.621, 0.591],
                    [0.790, 0.676],
                    [0.789, 0.796],
                    [0.618, 0.880],
                ],
            ),
        ],
        vec![],
    );
    insert_lrv(&mut engine, (def, anchors));

    assert!(engine.spawn_vehicle("lrv", "lrv", 1.0, 1.0));
    let body = *engine.names.get("lrv").unwrap();

    // Grounded: goto succeeds.
    assert!(matches!(goto_sync(&mut engine, "lrv", 2, 1), VehicleGotoPoll::Accepted(_)));
    assert!(!engine.world.get::<&IsoVehicle>(body).unwrap().path.is_empty());
    engine.vehicle_stop("lrv");

    // Airborne: goto is rejected and the path stays empty.
    engine.world.get::<&mut IsoVehicle>(body).unwrap().airborne = true;
    assert!(matches!(goto_sync(&mut engine, "lrv", 2, 1), VehicleGotoPoll::NoPath));
    assert!(engine.world.get::<&IsoVehicle>(body).unwrap().path.is_empty());
}

#[test]
fn heading_and_dir_round_trip() {
    for d in 0..8u32 {
        assert_eq!(heading_to_dir(dir_to_heading(d)), d, "direction {d} round-trips");
    }
    // Negative and wrapped headings quantize to the expected frames.
    assert_eq!(heading_to_dir(-std::f32::consts::FRAC_PI_4), 7);
    assert_eq!(heading_to_dir(std::f32::consts::TAU), 0);
    assert_eq!(heading_to_dir(std::f32::consts::FRAC_PI_2), 2);
}

#[test]
fn wrap_pi_bounds_angles() {
    assert!(wrap_pi(0.0).abs() < 1e-6);
    assert!((wrap_pi(0.5) - 0.5).abs() < 1e-6);
    assert!((wrap_pi(std::f32::consts::PI) - std::f32::consts::PI).abs() < 1e-4);
    assert!((wrap_pi(3.0 * std::f32::consts::PI) - std::f32::consts::PI).abs() < 1e-4);
    assert!((wrap_pi(-3.0 * std::f32::consts::PI) + std::f32::consts::PI).abs() < 1e-4);
}

#[test]
fn lerp_dir2_interpolates_between_frames() {
    let vals = [
        [0.0, 0.0],
        [10.0, 0.0],
        [10.0, 10.0],
        [0.0, 10.0],
        [-10.0, 10.0],
        [-10.0, 0.0],
        [-10.0, -10.0],
        [0.0, -10.0],
    ];
    // Midway between East (0) and SouthEast (1) at heading π/8.
    let mid = lerp_dir2(&vals, std::f32::consts::FRAC_PI_8);
    assert!((mid[0] - 5.0).abs() < 1e-6 && mid[1].abs() < 1e-6);
    // Exactly at a frame reproduces that frame's values.
    let at_east = lerp_dir2(&vals, 0.0);
    assert_eq!(at_east, [0.0, 0.0]);
}

#[test]
fn bounded_turn_steers_forward_only() {
    // Start heading East, waypoint due South — the vehicle must arc forward
    // (never reverse) and turn no faster than the configured rate.
    let mut heading = 0.0f32;
    let turn_rate = 45.0f32.to_radians();
    let dt = 1.0 / 60.0;
    let (tx, ty) = (1.5f32, 5.0f32);
    let (mut x, mut y) = (1.0f32, 1.0f32);
    let mut prev_heading = heading;

    for _ in 0..120 {
        let desired = (ty - y).atan2(tx - x);
        let err = wrap_pi(desired - heading);
        let max_turn = turn_rate * dt;
        let dh = err.clamp(-max_turn, max_turn);
        assert!(dh.abs() <= max_turn + 1e-6, "turn rate bounded");
        heading = wrap_pi(heading + dh);
        assert!(heading >= prev_heading - 1e-6, "no reverse — heading monotonic");
        prev_heading = heading;

        let step = 1.0 * dt;
        x += heading.cos() * step;
        y += heading.sin() * step;
    }

    assert!(
        (heading - std::f32::consts::FRAC_PI_2).abs() < 0.4,
        "heading should converge toward south: {heading}"
    );
}

/// Build a minimal LRV vehicle definition (body + 4 wheels, flat anchors)
/// with a configurable turn rate for movement tests, plus its anchors data
/// artifact.
fn lrv_def(turn_rate_deg: f32) -> (VehicleDef, VehicleAnchors) {
    let spec: Vec<(&str, &str, [f32; 2])> = vec![
        ("body", "lrvBody", [0.5, 0.7352]),
        ("wheel_fl", "lrvWheelFl", [0.566, 0.618]),
        ("wheel_fr", "lrvWheelFr", [0.734, 0.702]),
        ("wheel_rl", "lrvWheelRl", [0.211, 0.796]),
        ("wheel_rr", "lrvWheelRr", [0.379, 0.880]),
    ];
    let def = VehicleDef {
        name: "lrv".into(),
        directions: 8,
        anchors: "lrv_anchors".into(),
        columns: 4,
        rows: 2,
        cell: [247.0, 247.0],
        pitch_levels: 1,
        pitch_max_deg: 20.0,
        roll_levels: 1,
        roll_max_deg: 20.0,
        path_footprint: None,
        safe_fall_m: 0.0,
        steer_levels: 1,
        steer_max_deg: 30.0,
        steer_rate_deg_per_sec: 360.0,
        reverse_speed: 1.3,
        turn_cost: 0.0,
        tires: vec![],
        turn_rate_deg_per_sec: turn_rate_deg,
        parts: spec
            .iter()
            .map(|(n, t, _)| VehiclePartDef { name: (*n).into(), texture: (*t).into() })
            .collect(),
    };
    let anchors = vehicle_anchors(spec.iter().map(|(n, _t, a)| (*n, vec![*a])).collect(), vec![]);
    (def, anchors)
}

/// Register a test vehicle's def + anchors data artifact in the engine.
fn insert_lrv(engine: &mut Engine, (def, anchors): (VehicleDef, VehicleAnchors)) {
    engine.vehicles.insert("lrv".into(), def);
    engine.vehicle_anchors.insert("lrv_anchors".into(), anchors);
}

#[test]
fn spawn_vehicle_auto_derives_footprint() {
    let mut engine = Engine::new_for_test();
    let tm = engine.world.spawn((test_tilemap(), Role::new(RoleKind::Tilemap)));
    engine.names.insert("tilemap".into(), tm);
    insert_lrv(&mut engine, lrv_def(90.0));

    assert!(engine.spawn_vehicle("lrv", "lrv", 1.0, 1.0));
    let body = *engine.names.get("lrv").unwrap();
    let v = engine.world.get::<&IsoVehicle>(body).unwrap();

    // Auto-derived footprint is a full rectangle covering the wheel extent
    // plus a 1-tile margin.
    assert!(!v.path_footprint.is_empty());
    let half_x = v.path_footprint.iter().map(|p| p.0.abs()).max().unwrap();
    let half_y = v.path_footprint.iter().map(|p| p.1.abs()).max().unwrap();
    assert!(half_x >= 1 && half_y >= 1, "footprint should cover the wheel extent");
    assert_eq!(
        v.path_footprint.len() as i32,
        (half_x * 2 + 1) * (half_y * 2 + 1),
        "footprint must be a full rectangle"
    );
}

#[test]
fn lookahead_targets_a_point_ahead() {
    // Straight east path, vehicle at (1, 1): with lookahead 2 the target
    // lands two tiles east of the vehicle on the first segment.
    let path = [[1, 1], [5, 1], [9, 1]];
    let (gx, gy, idx) = lookahead(&path, 1, 1.5, 1.5, 2.0);
    assert_eq!(idx, 1);
    assert!((gx - 3.5).abs() < 1e-4 && (gy - 1.5).abs() < 1e-4, "got ({gx}, {gy})");

    // A longer look-ahead crosses into the next segment and advances idx.
    let (gx, gy, idx) = lookahead(&path, 1, 1.5, 1.5, 6.0);
    assert_eq!(idx, 2);
    assert!((gx - 7.5).abs() < 1e-4 && (gy - 1.5).abs() < 1e-4, "got ({gx}, {gy})");

    // Exhausting the path returns the final waypoint center.
    let (gx, gy, idx) = lookahead(&path, 1, 1.5, 1.5, 100.0);
    assert_eq!(idx, path.len());
    assert!((gx - 9.5).abs() < 1e-4 && (gy - 1.5).abs() < 1e-4, "got ({gx}, {gy})");
}

#[test]
fn lookahead_skips_a_passed_waypoint() {
    // Vehicle past the corner: path goes east then south, but the vehicle is
    // already east of the corner.  It must skip the corner waypoint rather
    // than target a point behind it.
    let path = [[1, 1], [5, 1], [5, 5]];
    let (gx, gy, idx) = lookahead(&path, 1, 5.5, 1.5, 2.0);
    // The corner (5,1) is behind the vehicle; look-ahead continues south.
    assert!(idx >= 2, "corner waypoint should be skipped, got idx {idx}");
    assert!((gx - 5.5).abs() < 1e-4 && gy > 1.5, "got ({gx}, {gy})");
}

#[test]
fn pure_pursuit_reaches_goal_without_orbiting() {
    let mut engine = Engine::new_for_test();
    let size = 20i32;
    let tm_e = engine.world.spawn((flat_tilemap(size, size), Role::new(RoleKind::Tilemap)));
    engine.names.insert("tilemap".into(), tm_e);
    insert_lrv(&mut engine, lrv_def(90.0));

    assert!(engine.spawn_vehicle("lrv", "lrv", 1.0, 1.0));
    let body = *engine.names.get("lrv").unwrap();

    // L-shaped path with a 90-degree corner: (1,1) -> (5,1) -> (5,5).
    {
        let mut v = engine.world.get::<&mut IsoVehicle>(body).unwrap();
        v.path = vec![[1, 1], [5, 1], [5, 5]];
        v.path_idx = 1;
    }

    let mut last_idx = 0usize;
    let mut reached = false;
    for _ in 0..600 {
        engine.time.delta = 1.0 / 60.0;
        engine.update_vehicles();
        let v = engine.world.get::<&IsoVehicle>(body).unwrap();
        if v.path.is_empty() {
            reached = true;
            break;
        }
        assert!(v.path_idx >= last_idx, "path_idx went backwards");
        last_idx = v.path_idx;
    }
    assert!(reached, "vehicle never reached the goal after 600 frames");

    let (gx, gy, _) = engine.get_pos("lrv").unwrap();
    assert!(
        (gx - 5.5).abs() < 0.01 && (gy - 5.5).abs() < 0.01,
        "not snapped to goal: ({gx}, {gy})"
    );
}

/// A rectangular footprint `[-hx..=hx] × [-hy..=hy]`.
fn rect_footprint(hx: i32, hy: i32) -> Vec<(i32, i32)> {
    let mut fp = Vec::new();
    for dy in -hy..=hy {
        for dx in -hx..=hx {
            fp.push((dx, dy));
        }
    }
    fp
}

/// The real LRV sidecar anchors (8 directions per part) from `lrv.json`,
/// returned as a `(def, anchors-artifact)` pair.
#[allow(clippy::type_complexity)]
fn lrv_def_real() -> (VehicleDef, VehicleAnchors) {
    let part = |name: &str, texture: &str, anchors: Vec<[f32; 2]>| {
        (VehiclePartDef { name: name.into(), texture: texture.into() }, (name.to_string(), anchors))
    };
    let pairs = vec![
        part("body", "lrvBody", vec![[0.5, 0.6618]; 8]),
        part(
            "wheel_fl",
            "lrvWheelFl",
            vec![
                [0.549373, 0.574673],
                [0.658127, 0.617648],
                [0.674253, 0.686486],
                [0.588304, 0.740864],
                [0.450627, 0.748927],
                [0.341873, 0.705952],
                [0.325747, 0.637114],
                [0.411696, 0.582736],
            ],
        ),
        part(
            "wheel_fr",
            "lrvWheelFr",
            vec![
                [0.674361, 0.637148],
                [0.658154, 0.706015],
                [0.549303, 0.74898],
                [0.411571, 0.740877],
                [0.325639, 0.686452],
                [0.341846, 0.617585],
                [0.450697, 0.57462],
                [0.588429, 0.582723],
            ],
        ),
        part(
            "wheel_rl",
            "lrvWheelRl",
            vec![
                [0.284399, 0.70716],
                [0.283399, 0.617648],
                [0.40928, 0.554],
                [0.588304, 0.553499],
                [0.715601, 0.61644],
                [0.716601, 0.705952],
                [0.59072, 0.7696],
                [0.411696, 0.770101],
            ],
        ),
        part(
            "wheel_rr",
            "lrvWheelRr",
            vec![
                [0.409349, 0.769654],
                [0.283371, 0.706014],
                [0.284292, 0.616474],
                [0.411571, 0.553486],
                [0.590651, 0.553946],
                [0.716629, 0.617586],
                [0.715708, 0.707126],
                [0.588429, 0.770114],
            ],
        ),
    ];
    let (parts, anchors_parts): (Vec<VehiclePartDef>, Vec<(String, Vec<[f32; 2]>)>) =
        pairs.into_iter().unzip();
    let def = VehicleDef {
        name: "lrv".into(),
        directions: 8,
        anchors: "lrv_anchors".into(),
        columns: 4,
        rows: 2,
        cell: [331.0, 331.0],
        pitch_levels: 5,
        pitch_max_deg: 20.0,
        roll_levels: 3,
        roll_max_deg: 20.0,
        path_footprint: None,
        turn_rate_deg_per_sec: 90.0,
        safe_fall_m: 1.5,
        steer_levels: 1,
        steer_max_deg: 30.0,
        steer_rate_deg_per_sec: 360.0,
        reverse_speed: 1.3,
        turn_cost: 0.0,
        tires: vec![],
        parts,
    };
    let anchors = vehicle_anchors(anchors_parts, vec![]);
    (def, anchors)
}

/// `lrv_def_real()` plus two front steering tires (5 steer levels), matching
/// the exporter's front-wheel-steering sidecar shape.  Tires reuse their
/// wheel's anchors (the steering yaw is about the axle's vertical axis, so
/// the ground-origin anchor is steer-invariant).
fn lrv_def_steering() -> (VehicleDef, VehicleAnchors) {
    let (mut def, mut anchors) = lrv_def_real();
    def.steer_levels = 5;
    def.steer_max_deg = 30.0;
    def.tires = vec![
        VehiclePartDef { name: "tire_fl".into(), texture: "lrvTireFl".into() },
        VehiclePartDef { name: "tire_fr".into(), texture: "lrvTireFr".into() },
    ];
    anchors.tires = [
        ("tire_fl", anchors.parts.get("wheel_fl").unwrap().clone()),
        ("tire_fr", anchors.parts.get("wheel_fr").unwrap().clone()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    (def, anchors)
}

/// Build an engine with a 48×48 tilemap + all-walkable nav (the lrvtest
/// shape), spawn a vehicle with a 7×5 footprint + safe_fall, and assert a
/// flat click-to-move finds a path.
fn engine_with_lrvtest_like_map(heights: Option<Vec<f32>>) -> Engine {
    let mut engine = Engine::new_for_test();
    let size = 48;
    let mut tm = flat_tilemap(size, size);
    tm.height_scale = 64.0; // matches commit_terrain(64.0) in lrv-guest
    if let Some(h) = heights {
        tm.height_data = h;
    }
    let tm_e = engine.world.spawn((tm, Role::new(RoleKind::Tilemap)));
    engine.names.insert("tilemap".into(), tm_e);

    let nav = NavMesh {
        position: Vec3::ZERO,
        scale: Vec3::ONE,
        map_entity: "tilemap".into(),
        tile_set: "navTileset".into(),
        data_grid: None,
        data: vec![1; (size * size) as usize],
        size_x: size,
        size_y: size,
    };
    let nm = engine.world.spawn((nav, Role::new(RoleKind::NavMesh)));
    engine.names.insert("navmesh".into(), nm);

    let (mut def, anchors) = lrv_def(90.0);
    def.path_footprint = Some(rect_footprint(3, 2)); // 7x5, matches auto-derive
    def.safe_fall_m = 1.5;
    insert_lrv(&mut engine, (def, anchors));

    engine
}

#[test]
fn vehicle_goto_paths_across_flat_lrvtest_like_map() {
    let mut engine = engine_with_lrvtest_like_map(None);
    assert!(engine.spawn_vehicle("lrv", "lrv", 5.0, 5.0));
    assert!(
        matches!(goto_sync(&mut engine, "lrv", 24, 24), VehicleGotoPoll::Accepted(_)),
        "flat click-to-move should path"
    );
    let body = *engine.names.get("lrv").unwrap();
    assert!(!engine.world.get::<&IsoVehicle>(body).unwrap().path.is_empty());
}

#[test]
fn vehicle_goto_paths_around_features() {
    // A gentle ramp band (0.375 units/tile) and a steep curb (1.0 unit/tile)
    // like the lrvtest map: the flat base and ramp faces must stay reachable.
    let size = 48;
    let n = (size + 1) as usize;
    let mut heights = vec![0.5f32; n * n];
    // East ramp: x in 33..=41, y in 20..=29, rise 1.5 m over 8 tiles.
    for y in 20..=29 {
        for x in 33..=41 {
            heights[y * n + x] = heights[y * n + x].max(0.5 + 1.5 * (x - 33) as f32 / 8.0);
        }
    }
    // Curb: x in 20..=36, y in 42..=47, +0.5 m.
    for x in 20..=36 {
        for y in 42..=47 {
            heights[y * n + x] = heights[y * n + x].max(1.0);
        }
    }

    let mut engine = engine_with_lrvtest_like_map(Some(heights));
    assert!(engine.spawn_vehicle("lrv", "lrv", 5.0, 5.0));
    // Flat destination far from features.
    assert!(
        matches!(goto_sync(&mut engine, "lrv", 10, 10), VehicleGotoPoll::Accepted(_)),
        "flat destination should path"
    );
    // Ramp face (gentle) should be reachable.
    assert!(
        matches!(goto_sync(&mut engine, "lrv", 33, 25), VehicleGotoPoll::Accepted(_)),
        "gentle ramp face should path"
    );
}

#[test]
fn real_lrv_def_auto_derives_and_paths() {
    let mut engine = engine_with_lrvtest_like_map(None);
    insert_lrv(&mut engine, lrv_def_real());

    assert!(engine.spawn_vehicle("lrv", "lrv", 5.0, 5.0));
    let body = *engine.names.get("lrv").unwrap();
    let (half_x, half_y) = {
        let v = engine.world.get::<&IsoVehicle>(body).unwrap();
        let half_x = v.path_footprint.iter().map(|p| p.0.abs()).max().unwrap();
        let half_y = v.path_footprint.iter().map(|p| p.1.abs()).max().unwrap();
        (half_x, half_y)
    };
    let footprint_len = {
        let v = engine.world.get::<&IsoVehicle>(body).unwrap();
        v.path_footprint.len() as i32
    };
    assert_eq!(footprint_len, (half_x * 2 + 1) * (half_y * 2 + 1));

    assert!(
        matches!(goto_sync(&mut engine, "lrv", 24, 24), VehicleGotoPoll::Accepted(_)),
        "goto with real def + auto footprint"
    );
}

#[test]
fn spawn_vehicle_derives_travel_limits() {
    let mut engine = engine_with_lrvtest_like_map(None);
    insert_lrv(&mut engine, lrv_def_real());

    assert!(engine.spawn_vehicle("lrv", "lrv", 5.0, 5.0));
    let body = *engine.names.get("lrv").unwrap();
    let v = engine.world.get::<&IsoVehicle>(body).unwrap();

    assert!(v.wheel_travel_down > 0.0, "droop must be derived positive");
    assert!(
        v.wheel_travel_up > 0.0 && v.wheel_travel_up < v.wheel_travel_down,
        "compression must be a positive fraction of droop"
    );
    // pitch_levels = 5 for the real def, so the dead-zone is non-zero.
    assert!(v.tilt_dead_zone > 0.0, "dead-zone must be derived from frame quantization");
}

#[test]
fn vehicle_probe_reports_reachability_and_caches_result() {
    let mut engine = engine_with_lrvtest_like_map(None);
    insert_lrv(&mut engine, lrv_def_real());
    assert!(engine.spawn_vehicle("lrv", "lrv", 5.0, 5.0));
    engine.set_synchronous_workers(true);

    // A reachable target stores its waypoints and reports 1.
    assert_eq!(engine.vehicle_probe("lrv", 24, 24), 1);
    assert!(engine.preview_paths.contains_key("lrv"));

    // The resolved result is cached as `Done { reachable: true }` (not
    // cleared), so a repeat identical call returns the cached answer and
    // keeps the path (previously it cleared the probe and re-ran A*, which
    // made the preview line vanish and flicker every frame).
    assert!(matches!(
        engine.preview_probe.as_ref().map(|p| p.state),
        Some(PreviewProbeState::Done { reachable: true })
    ));
    assert_eq!(engine.vehicle_probe("lrv", 24, 24), 1);
    assert!(engine.preview_paths.contains_key("lrv"));
    assert!(matches!(
        engine.preview_probe.as_ref().map(|p| p.state),
        Some(PreviewProbeState::Done { reachable: true })
    ));

    // An unknown vehicle is reported distinctly.
    assert_eq!(engine.vehicle_probe("nope", 5, 5), -2);
}

#[test]
fn spawn_vehicle_spawns_steering_tires() {
    let mut engine = engine_with_lrvtest_like_map(None);
    insert_lrv(&mut engine, lrv_def_steering());

    assert!(engine.spawn_vehicle("lrv", "lrv", 5.0, 5.0));
    let body = *engine.names.get("lrv").unwrap();
    let v = engine.world.get::<&IsoVehicle>(body).unwrap();

    assert_eq!(v.steer_levels, 5);
    assert!((v.steer_max - 30.0f32.to_radians()).abs() < 1e-6);
    assert_eq!(v.tire_entities[0], "lrvTireFl");
    assert_eq!(v.tire_entities[1], "lrvTireFr");
    // The tire sheet stacks 5 steer levels over the 2-row wheel grid.
    let tire = engine.names.get("lrvTireFl").copied().unwrap();
    let s = engine.world.get::<&IsoSprite>(tire).unwrap();
    assert_eq!(s.tile_set_size, Vec2::new(4.0, 10.0));
    // Both tires share the front wheels' ghost group (never ghost through
    // their own body/arm).
    assert_eq!(s.ghost_group, engine.world.get::<&IsoSprite>(body).unwrap().ghost_group);
}

#[test]
fn front_tires_steer_into_a_turn() {
    let mut engine = engine_with_lrvtest_like_map(None);
    insert_lrv(&mut engine, lrv_def_steering());
    assert!(engine.spawn_vehicle("lrv", "lrv", 1.0, 1.0));
    let body = *engine.names.get("lrv").unwrap();
    {
        let mut v = engine.world.get::<&mut IsoVehicle>(body).unwrap();
        v.path = vec![[1, 1], [5, 1], [5, 5]];
        v.path_idx = 1;
    }

    // Idle before the first update: steering is straight.
    assert_eq!(engine.world.get::<&IsoVehicle>(body).unwrap().steer_index, straight_steer(5));

    let mut ever_steered = false;
    for _ in 0..600 {
        engine.time.delta = 1.0 / 60.0;
        engine.update_vehicles();
        let v = engine.world.get::<&IsoVehicle>(body).unwrap();
        if v.steer_index != straight_steer(v.steer_levels) {
            ever_steered = true;
        }
        if v.path.is_empty() {
            break;
        }
    }
    assert!(ever_steered, "front tires never steered into the turn");
}

#[test]
fn body_lifts_and_tilts_without_wheels_riding_over() {
    let mut engine = engine_with_lrvtest_like_map(Some(lrvtest_heights()));
    insert_lrv(&mut engine, lrv_def_real());

    assert!(engine.spawn_vehicle("lrv", "lrv", 5.0, 5.0));
    // Teleport onto the east ramp face (rising +x), then let the suspension
    // settle over many frames.
    assert!(engine.vehicle_teleport("lrv", 37.0, 25.0));
    for _ in 0..300 {
        engine.time.delta = 1.0 / 60.0;
        engine.update_vehicles();
    }

    let body = *engine.names.get("lrv").unwrap();
    let v = engine.world.get::<&IsoVehicle>(body).unwrap();

    // The body plane lifts with the wheels (well above the flat 0.5-metre
    // base) and noses up on the east ramp.
    assert!(v.altitude > 0.75, "body did not lift on the ramp: {}", v.altitude);
    assert!(v.pitch > 0.0, "body did not nose up on the east ramp: {}", v.pitch);

    // Every wheel stays within the travel envelope around the body plane, so
    // none rides over the body or hangs beyond droop.
    let tol = 1.0 / PPM_TARGET;
    for i in 0..4 {
        let pw = if i < 2 { 0.5 } else { -0.5 };
        let rw = if i % 2 == 0 { 0.5 } else { -0.5 };
        let plane = v.altitude + pw * v.pitch * v.wheelbase_m + rw * v.roll * v.track_m;
        assert!(
            v.wheel_h[i] <= plane + v.wheel_travel_up + tol,
            "wheel {i} rode over the body: {} vs {}",
            v.wheel_h[i],
            plane + v.wheel_travel_up
        );
        assert!(
            v.wheel_h[i] >= plane - v.wheel_travel_down - tol,
            "wheel {i} over-drooped: {} vs {}",
            v.wheel_h[i],
            plane - v.wheel_travel_down
        );
    }
}

/// The full `lrvtest` ramp-course height field, mirroring `gen_lrvtest_map`
/// in classic-roms (already re-expressed to metres): base 0.5 m, a central
/// hill, four cardinal + four diagonal ramps, and a raised curb.
fn lrvtest_heights() -> Vec<f32> {
    let size = 48usize;
    let n = size + 1;
    let mut h = vec![0.5f32; n * n];
    let idx = |x: usize, y: usize| y * n + x;

    // Central hill: +1 m peak at (24,24), radius 5.
    for y in 0..n {
        for x in 0..n {
            let dx = x as f32 - 24.0;
            let dy = y as f32 - 24.0;
            let d = (dx * dx + dy * dy).sqrt();
            if d < 5.0 {
                let bump = 1.0 * (1.0 - d / 5.0);
                h[idx(x, y)] = h[idx(x, y)].max(0.5 + bump);
            }
        }
    }
    // Cardinal ramps.
    for y in 20..=29 {
        for x in 33..=41 {
            h[idx(x, y)] = h[idx(x, y)].max(0.5 + 1.5 * (x - 33) as f32 / 8.0);
        }
        for x in 7..=15 {
            h[idx(x, y)] = h[idx(x, y)].max(0.5 + 1.5 * (15 - x) as f32 / 8.0);
        }
    }
    for x in 20..=29 {
        for y in 33..=41 {
            h[idx(x, y)] = h[idx(x, y)].max(0.5 + 1.5 * (y - 33) as f32 / 8.0);
        }
        for y in 7..=15 {
            h[idx(x, y)] = h[idx(x, y)].max(0.5 + 1.5 * (15 - y) as f32 / 8.0);
        }
    }
    // Diagonal ramps.
    for x in 33..=41 {
        for y in 33..=41 {
            h[idx(x, y)] = h[idx(x, y)].max(0.5 + 1.5 * ((x + y) as f32 - 66.0) / 16.0);
        }
        for y in 7..=15 {
            h[idx(x, y)] =
                h[idx(x, y)].max(0.5 + 1.5 * (x as f32 + (15.0 - y as f32) - 40.0) / 16.0);
        }
    }
    for x in 7..=15 {
        for y in 7..=15 {
            h[idx(x, y)] = h[idx(x, y)].max(0.5 + 1.5 * (30.0 - (x + y) as f32) / 16.0);
        }
        for y in 33..=41 {
            h[idx(x, y)] = h[idx(x, y)].max(0.5 + 1.5 * (15.0 - x as f32 + y as f32 - 40.0) / 16.0);
        }
    }
    // Curb.
    for x in 20..=36 {
        for y in 42..=47 {
            h[idx(x, y)] = h[idx(x, y)].max(1.0);
        }
    }
    h
}

#[test]
fn vehicle_goto_paths_on_full_lrvtest_map() {
    let mut engine = engine_with_lrvtest_like_map(Some(lrvtest_heights()));
    insert_lrv(&mut engine, lrv_def_real());

    assert!(engine.spawn_vehicle("lrv", "lrv", 5.0, 5.0));

    // Flat base, the ramp faces (gentle slopes), and the central hill must
    // be reachable.
    assert!(
        matches!(goto_sync(&mut engine, "lrv", 20, 20), VehicleGotoPoll::Accepted(_)),
        "flat base should path"
    );
    assert!(
        matches!(goto_sync(&mut engine, "lrv", 11, 25), VehicleGotoPoll::Accepted(_)),
        "west ramp face should path"
    );
    assert!(
        matches!(goto_sync(&mut engine, "lrv", 33, 25), VehicleGotoPoll::Accepted(_)),
        "east ramp face should path"
    );
    assert!(
        matches!(goto_sync(&mut engine, "lrv", 24, 24), VehicleGotoPoll::Accepted(_)),
        "central hill should path"
    );
    assert!(
        matches!(goto_sync(&mut engine, "lrv", 16, 29), VehicleGotoPoll::Accepted(_)),
        "flat tile beside the ramp should path"
    );
    assert!(
        matches!(goto_sync(&mut engine, "lrv", 10, 10), VehicleGotoPoll::Accepted(_)),
        "nw diagonal ramp face should path"
    );
    // The 0.5 m curb is a ~14.5° pitch over the wheelbase — within the
    // 20° limit, so the vehicle can drive over it.
    assert!(
        matches!(goto_sync(&mut engine, "lrv", 28, 45), VehicleGotoPoll::Accepted(_)),
        "0.5 m curb should be traversable"
    );

    // The ramp's 1.5 m cliff sides are a ~38° pitch over the wheelbase —
    // beyond the 20° limit, so the vehicle can't stand on them.
    assert!(
        matches!(goto_sync(&mut engine, "lrv", 7, 29), VehicleGotoPoll::NoPath),
        "ramp cliff edge should not path"
    );
}
