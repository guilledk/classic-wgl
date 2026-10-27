use classic_core::components::{IsoSprite, IsoVehicle, NavMesh, Role, RoleKind, Tilemap};
use classic_core::math::{iso_camera_px, iso_world_pos};
use classic_core::types::{FrameTable, VehicleAnchors, VehicleDef, VehiclePartDef};
use glam::{Vec2, Vec3};

use crate::Engine;

use super::kinematics::{
    advance_suspension, derive_wheel_offsets, dir_to_heading, heading_to_dir, lerp_dir2, lookahead,
    pitch_index, search_footprint, should_reverse, soft_deadzone, steer_index, step_spring,
    step_steer, straight_steer, wrap_pi, MAX_SPRING_STEP,
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
pub(super) fn flat_tilemap_pub(size_x: i32, size_y: i32) -> Tilemap {
    flat_tilemap(size_x, size_y)
}

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
        blend: None,
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
fn springs_stay_stable_on_long_steps() {
    // Semi-implicit Euler diverges past `2 / damping` (0.133 s for the
    // suspension): a slow web frame stream (dt 0.6-0.8 s) grew the state to
    // `inf`, then `NaN`.  Sub-stepped, any frame length settles.
    for dt in [0.2f32, 0.7, 2.0, 100.0] {
        let (mut p, mut pv) = (0.0f32, 0.0f32);
        let (mut h, mut hv) = (0.0f32, 0.0f32);
        for _ in 0..200 {
            (p, pv) = step_spring(p, pv, AIRBORNE_PITCH, dt);
            (h, hv) = advance_suspension(h, hv, 1.0, dt);
            assert!(p.is_finite() && pv.is_finite() && h.is_finite() && hv.is_finite());
        }
        assert!((p - AIRBORNE_PITCH).abs() < 0.01, "dt {dt}: pitch {p}");
        assert!((h - 1.0).abs() < 0.01, "dt {dt}: suspension {h}");
    }
    // At or below the sub-step length it is the original single step, bit
    // for bit (the fixed-dt goldens run at 1/60 and 0.05).
    for dt in [1.0f32 / 60.0, MAX_SPRING_STEP] {
        let (x, v, target) = (0.3f32, -0.7f32, 0.05f32);
        let old = |k: f32, c: f32| {
            let nv = v + (k * (target - x) - c * v) * dt;
            (x + nv * dt, nv)
        };
        assert_eq!(step_spring(x, v, target, dt), old(SPRING_STIFFNESS, SPRING_DAMPING));
        assert_eq!(advance_suspension(x, v, target, dt), old(SUSP_STIFFNESS, SUSP_DAMPING));
    }
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
    let mut wheels: [Vec<[f32; 2]>; 4] = std::array::from_fn(|_| vec![[0.5f32, 0.5f32]; 8]);
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
        path_body_m: None,
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
        dust_effect: None,
        touchdown_effect: None,
        ..Default::default()
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
    let empty = FrameTable {
        version: 1,
        sheets: vec![],
        frames: Default::default(),
        dir_interp: false,
        axes: None,
        tilt_interp: false,
        companions: vec![],
    };
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
        path_body_m: None,
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
        dust_effect: None,
        touchdown_effect: None,
        ..Default::default()
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
        path_body_m: None,
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
        dust_effect: None,
        touchdown_effect: None,
        ..Default::default()
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
    for n in [8u32, 16, 24] {
        for d in 0..n {
            assert_eq!(heading_to_dir(dir_to_heading(d, n), n), d, "direction {d}/{n} round-trips");
        }
    }
    // Negative and wrapped headings quantize to the expected frames.
    assert_eq!(heading_to_dir(-std::f32::consts::FRAC_PI_4, 8), 7);
    assert_eq!(heading_to_dir(std::f32::consts::TAU, 8), 0);
    assert_eq!(heading_to_dir(std::f32::consts::FRAC_PI_2, 8), 2);
    assert_eq!(heading_to_dir(std::f32::consts::FRAC_PI_2, 16), 4);
    assert_eq!(heading_to_dir(-std::f32::consts::FRAC_PI_8, 16), 15);
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
        path_body_m: None,
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
        dust_effect: None,
        touchdown_effect: None,
        ..Default::default()
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
        path_body_m: None,
        turn_rate_deg_per_sec: 90.0,
        safe_fall_m: 1.5,
        steer_levels: 1,
        steer_max_deg: 30.0,
        steer_rate_deg_per_sec: 360.0,
        reverse_speed: 1.3,
        turn_cost: 0.0,
        tires: vec![],
        parts,
        dust_effect: None,
        touchdown_effect: None,
        ..Default::default()
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
fn the_path_body_is_the_defs_rectangle_else_the_wheel_contacts() {
    use classic_core::pathfinder::{BodyRect, VehicleFootprint};
    use classic_core::tilemap::TILE_M;

    let mut engine = engine_with_lrvtest_like_map(None);
    let (mut def, anchors) = lrv_def_real();
    insert_lrv(&mut engine, (def.clone(), anchors.clone()));
    assert!(engine.spawn_vehicle("lrv", "a", 5.0, 5.0));
    let body = |engine: &Engine, name: &str| {
        let v = engine.world.get::<&IsoVehicle>(engine.names[name]).unwrap();
        (v.path_body, search_footprint(&v), v.wheelbase_m, v.track_m)
    };
    // auto: the wheel contacts' rectangle, turned by the search
    let (b, fp, wb, tr) = body(&engine, "a");
    let want = BodyRect::centred(wb / TILE_M, tr / TILE_M);
    let b = b.expect("an auto footprint turns");
    for (got, want) in [(b.rear, want.rear), (b.front, want.front), (b.right, want.right)] {
        assert!((got - want).abs() < 1e-4, "{b:?} vs {want:?}");
    }
    assert!(matches!(fp, VehicleFootprint::Oriented { pivot: false, .. }));

    // the def's own rectangle wins
    def.path_body_m = Some([-1.83, 1.64, -2.07, 2.07]);
    insert_lrv(&mut engine, (def.clone(), anchors.clone()));
    assert!(engine.spawn_vehicle("lrv", "b", 15.0, 5.0));
    let b = body(&engine, "b").0.unwrap();
    assert!((b.rear + 1.83 / TILE_M).abs() < 1e-4 && (b.left - 2.07 / TILE_M).abs() < 1e-4);

    // an authored cell footprint is searched as authored
    def.path_footprint = Some(rect_footprint(1, 1));
    insert_lrv(&mut engine, (def, anchors));
    assert!(engine.spawn_vehicle("lrv", "c", 25.0, 5.0));
    let (b, fp, ..) = body(&engine, "c");
    assert_eq!(b, None);
    assert_eq!(fp, VehicleFootprint::Cells(rect_footprint(1, 1)));
}

#[test]
fn vehicle_goto_turns_the_body_to_each_step() {
    use classic_core::tilemap::TILE_M;

    // 3 cells nose to tail, 5 across: facing East its axis box spans rows
    // -2..=2, so a goal one row in from the map's north edge is out of the
    // box's reach; driving north, the turned body spans rows -1..=1
    let mut engine = engine_with_lrvtest_like_map(None);
    let (mut def, anchors) = lrv_def_real();
    def.path_footprint = None;
    def.path_body_m = Some([-0.95 * TILE_M, 0.95 * TILE_M, -1.9 * TILE_M, 1.9 * TILE_M]);
    def.safe_fall_m = 0.0;
    insert_lrv(&mut engine, (def, anchors));
    assert!(engine.spawn_vehicle("lrv", "lrv", 24.5, 20.5));

    match goto_sync(&mut engine, "lrv", 24, 1) {
        VehicleGotoPoll::Accepted(path) => {
            assert!(path.iter().all(|c| c[0] == 24), "straight north: {path:?}")
        }
        _ => panic!("the body turned north fits by the edge"),
    }
    // the same body as its East-facing axis box cannot
    let (mut def, anchors) = lrv_def_real();
    def.path_footprint = Some(rect_footprint(1, 2));
    def.safe_fall_m = 0.0;
    insert_lrv(&mut engine, (def, anchors));
    assert!(engine.spawn_vehicle("lrv", "boxed", 24.5, 20.5));
    assert!(matches!(goto_sync(&mut engine, "boxed", 24, 1), VehicleGotoPoll::NoPath));
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

// -- wheel dust + touchdown puffs ------------------------------------------

const DUST_LIB: &str = r#"{"effects":[
  {"name":"dust","layers":[{"type":"particles","name":"p","max":2048,
     "spawn":{"rate":40,"burst":4},"velocity":{"speed":[1,2]},"life":[1,1]}]},
  {"name":"puff","layers":[{"type":"particles","name":"p","max":64,
     "spawn":{"burst":16},"life":[0.5,0.5]}]}
]}"#;

/// A flat 20×20 map (with the tilemap transform `iso_to_world` needs), the
/// dust library under `t`, and an LRV def carrying `dust`/`touchdown`.
fn dusty_engine(dust: Option<&str>, touchdown: Option<&str>) -> Engine {
    let mut engine = Engine::new_for_test();
    let tm_e = engine.world.spawn((
        flat_tilemap(20, 20),
        Role::new(RoleKind::Tilemap),
        classic_core::Transform::new(Vec3::ZERO, Vec3::new(45.0, 45.0, 1.0)),
    ));
    engine.names.insert("tilemap".into(), tm_e);
    engine.register_effects(
        "t",
        classic_core::effects::EffectLibrary::from_json(DUST_LIB.as_bytes()).unwrap(),
    );
    let (mut def, anchors) = lrv_def(90.0);
    def.dust_effect = dust.map(Into::into);
    def.touchdown_effect = touchdown.map(Into::into);
    insert_lrv(&mut engine, (def, anchors));
    assert!(engine.spawn_vehicle("lrv", "lrv", 2.0, 2.0));
    engine
}

fn step(engine: &mut Engine, frames: usize) {
    for _ in 0..frames {
        engine.time.delta = 1.0 / 60.0;
        engine.update_vehicles();
        engine.update_particles(1.0 / 60.0);
    }
}

fn dust_emitters(engine: &Engine) -> Vec<classic_core::components::Emitter> {
    ["Fl", "Fr", "Rl", "Rr"]
        .iter()
        .map(|s| {
            let e = engine.names[&format!("lrvDust{s}")];
            engine
                .world
                .get::<&classic_core::components::Emitter>(e)
                .map(|em| (*em).clone())
                .unwrap()
        })
        .collect()
}

fn drive_to(engine: &mut Engine, tx: i32, ty: i32) {
    let body = engine.names["lrv"];
    let mut v = engine.world.get::<&mut IsoVehicle>(body).unwrap();
    v.path = vec![[2, 2], [tx, ty]];
    v.path_idx = 1;
}

#[test]
fn vehicles_without_dust_spawn_no_emitters() {
    let mut engine = dusty_engine(None, None);
    drive_to(&mut engine, 15, 2);
    step(&mut engine, 60);
    assert_eq!(engine.world.query::<&classic_core::components::Emitter>().iter().count(), 0);
    assert_eq!(engine.particle_stats().instances, 0);
}

#[test]
fn wheel_dust_follows_speed_and_bursts_on_speed_changes() {
    let mut engine = dusty_engine(Some("t::dust"), None);
    step(&mut engine, 1);
    let parked = dust_emitters(&engine);
    assert!(parked.iter().all(|e| e.rate == 0.0 && e.burst == 0.0), "parked: no dust");
    assert!(parked.iter().all(|e| e.parent.as_deref() == Some("lrv")));

    drive_to(&mut engine, 15, 2);
    step(&mut engine, 30);
    let body = engine.names["lrv"];
    let speed_mps =
        engine.world.get::<&IsoVehicle>(body).unwrap().speed * classic_core::tilemap::TILE_M;
    let driving = dust_emitters(&engine);
    for em in &driving {
        assert!((em.rate - speed_mps / DUST_REF_SPEED).abs() < 1e-4, "rate ∝ speed");
        assert!(em.burst > 0.0, "pulling away bursts");
        assert!(em.dir.z > 0.5, "dust leaves the ground upward");
    }
    // Driving +x in tile space: the dust leans back against the travel
    // direction in world space.
    let travel =
        engine.iso_to_world(3.0, 2.0, 0.0).unwrap() - engine.iso_to_world(2.0, 2.0, 0.0).unwrap();
    assert!(driving[0].dir.x * travel.x + driving[0].dir.y * travel.y < 0.0, "leans back");
    // Each emitter sits on its wheel's ground contact.
    let wheel = engine.names["lrvWheelFl"];
    let wp = engine.world.get::<&classic_core::Transform>(wheel).unwrap().position;
    let contact = engine.iso_to_world(wp.x, wp.y, 0.0).unwrap();
    let frame =
        engine.attachment_frame(Some("lrv"), None, driving[0].position, driving[0].dir).unwrap();
    assert!((frame.0 - contact).length() < 1e-3, "{:?} vs {contact:?}", frame.0);
    assert!(engine.particle_stats().live > 0);

    // Stopping is a speed change too.
    let before = driving[0].burst;
    engine.world.get::<&mut IsoVehicle>(body).unwrap().path.clear();
    step(&mut engine, 2);
    let stopped = dust_emitters(&engine);
    assert!(stopped.iter().all(|e| e.rate == 0.0));
    assert!(stopped[0].burst > before, "braking bursts");
}

#[test]
fn wheel_dust_is_deterministic() {
    let run = || {
        let mut engine = dusty_engine(Some("t::dust"), None);
        drive_to(&mut engine, 15, 9);
        step(&mut engine, 90);
        (format!("{:?}", dust_emitters(&engine)), engine.particle_stats())
    };
    assert_eq!(run(), run());
}

#[test]
fn touchdown_puff_fires_once_on_landing() {
    let mut engine = dusty_engine(None, Some("t::puff"));
    step(&mut engine, 1);
    let body = engine.names["lrv"];
    {
        let mut v = engine.world.get::<&mut IsoVehicle>(body).unwrap();
        v.altitude += 2.0; // drop from 2 m
    }
    let mut landed_at = None;
    for f in 0..240 {
        step(&mut engine, 1);
        let stats = engine.particle_stats();
        if stats.instances > 0 && landed_at.is_none() {
            landed_at = Some(f);
            assert_eq!(stats.live, 16, "one puff");
            assert!(!engine.world.get::<&IsoVehicle>(body).unwrap().airborne);
        }
    }
    assert!(landed_at.is_some_and(|f| f > 10), "the puff waits for the landing");
    assert_eq!(engine.particle_stats().instances, 0, "the one-shot retired, none re-fired");
}

#[test]
fn long_frames_through_hops_stay_finite() {
    // The fx gallery's rover on a ~1 fps software-GL page: it drives over
    // sloped ground, hops and lands, at dt 0.7 s.  It used to reach `NaN`
    // within a few frames and panic in the wheel travel clamp.
    let dt = 0.7;
    let mut engine = engine_with_lrvtest_like_map(Some(lrvtest_heights()));
    insert_lrv(&mut engine, lrv_def_real());
    assert!(engine.spawn_vehicle("lrv", "lrv", 5.0, 5.0));
    // Start on the east ramp face and drive back across the hill.
    assert!(engine.vehicle_teleport("lrv", 37.0, 25.0));
    let body = engine.names["lrv"];
    {
        let mut v = engine.world.get::<&mut IsoVehicle>(body).unwrap();
        v.path = vec![[37, 25], [12, 25]];
        v.path_idx = 1;
    }
    let mut hops = 0;
    for frame in 0..120 {
        if frame % 9 == 4 {
            let mut v = engine.world.get::<&mut IsoVehicle>(body).unwrap();
            if !v.airborne {
                v.altitude += 0.5;
                v.vel_z = 0.4;
                hops += 1;
            }
        }
        engine.time.delta = dt;
        engine.update_vehicles();
        let v = engine.world.get::<&IsoVehicle>(body).unwrap();
        let state = [v.altitude, v.vel_z, v.pitch, v.pitch_vel, v.roll, v.roll_vel, v.heading];
        assert!(state.iter().all(|s| s.is_finite()), "frame {frame}: {state:?}");
        assert!(v.wheel_h.iter().chain(&v.wheel_v).all(|s| s.is_finite()), "frame {frame}");
        assert!(v.pitch.abs() < 1.0 && v.roll.abs() < 1.0, "frame {frame}: tipped over");
    }
    assert!(hops >= 3, "the rover hopped and landed repeatedly ({hops})");
}

/// A part whose texture opts into direction interpolation draws the two
/// baked directions straddling the continuous heading, each at its anchor;
/// the others keep the quantized frame and no blend.
#[test]
fn dir_interp_parts_blend_the_straddling_directions() {
    let mut engine = Engine::new_for_test();
    let tm = engine.world.spawn((test_tilemap(), Role::new(RoleKind::Tilemap)));
    engine.names.insert("tilemap".into(), tm);
    insert_lrv(&mut engine, lrv_def(720.0));
    let table = |interp: bool| FrameTable {
        version: 2,
        sheets: vec![],
        frames: Default::default(),
        dir_interp: interp,
        axes: None,
        tilt_interp: false,
        companions: vec![],
    };
    engine.frame_tables.insert("lrvBody".into(), table(true));
    engine.frame_tables.insert("lrvWheelFl".into(), table(false));
    assert!(engine.spawn_vehicle("lrv", "lrv", 4.0, 4.0));
    let body = *engine.names.get("lrv").unwrap();
    let wheel = *engine.names.get("lrvWheelFl").unwrap();
    // 0.3 of the way from direction 2 to 3.
    let step = std::f32::consts::FRAC_PI_4;
    engine.world.get::<&mut IsoVehicle>(body).unwrap().heading = 2.3 * step;
    engine.time.delta = 1.0 / 60.0;
    engine.update_vehicles();

    let s = engine.world.get::<&IsoSprite>(body).unwrap();
    let blend = s.dir_blend.expect("body blends");
    let base = s.frame as u32 - 2;
    assert_eq!(s.frame_name.as_deref(), Some(format!("lrvBody_{}", base + 2).as_str()));
    assert_eq!(blend.frame_b as u32, base + 3);
    assert!((blend.dtheta_a - 0.3 * step).abs() < 1e-4);
    assert!((blend.dtheta_b + 0.7 * step).abs() < 1e-4);
    let w = engine.world.get::<&IsoSprite>(wheel).unwrap();
    assert!(w.dir_blend.is_none(), "a part without dir_interp keeps one frame");
    assert_eq!(w.frame, 2.0, "quantized to the nearer direction");
}

/// `directions` sizes every per-direction array and drives the heading
/// quantization and the frame stride of a non-interpolated sheet.
#[test]
fn sixteen_direction_vehicles_quantize_to_sixteen_frames() {
    let mut engine = Engine::new_for_test();
    let tm = engine.world.spawn((test_tilemap(), Role::new(RoleKind::Tilemap)));
    engine.names.insert("tilemap".into(), tm);
    let (mut def, mut anchors) = lrv_def(720.0);
    def.directions = 16;
    def.pitch_levels = 3;
    def.pitch_max_deg = 12.0;
    // Distinct body anchors per direction, to see which one is used.
    anchors.parts.insert("body".into(), (0..16).map(|d| [0.5, 0.6 + 0.01 * d as f32]).collect());
    insert_lrv(&mut engine, (def, anchors));
    let empty = FrameTable {
        version: 2,
        sheets: vec![],
        frames: Default::default(),
        dir_interp: false,
        axes: None,
        tilt_interp: false,
        companions: vec![],
    };
    engine.frame_tables.insert("lrvBody".into(), empty.clone());
    engine.frame_tables.insert("lrvWheelFl".into(), empty);
    assert!(engine.spawn_vehicle("lrv", "lrv", 4.0, 4.0));
    let body = *engine.names.get("lrv").unwrap();
    {
        let v = engine.world.get::<&IsoVehicle>(body).unwrap();
        assert_eq!(v.directions, 16);
        assert_eq!(v.body_anchors.len(), 16);
        assert!(v.wheel_anchors.iter().all(|a| a.len() == 16));
        assert!(v.wheel_tile_offsets.iter().all(|a| a.len() == 16));
    }
    // 5.2 sixteenths of a turn -> direction 5.
    let step = std::f32::consts::TAU / 16.0;
    engine.world.get::<&mut IsoVehicle>(body).unwrap().heading = 5.2 * step;
    engine.time.delta = 1.0 / 60.0;
    engine.update_vehicles();
    let v = engine.world.get::<&IsoVehicle>(body).unwrap();
    assert_eq!(v.direction, 5);
    let s = engine.world.get::<&IsoSprite>(body).unwrap();
    // Level pitch is index 1 of 3: frame = 1 * 16 + 5.
    assert_eq!(s.frame_name.as_deref(), Some("lrvBody_21"));
    assert!((s.anchor.y - (0.6 + 0.01 * 5.2)).abs() < 1e-4, "anchor lerps by heading");
    let wheel = *engine.names.get("lrvWheelFl").unwrap();
    assert_eq!(
        engine.world.get::<&IsoSprite>(wheel).unwrap().frame_name.as_deref(),
        Some("lrvWheelFl_5")
    );
}

/// The lrvtest-like map with a vehicle 3 cells nose to tail and 5 across
/// (`path_body_m`) spawned at `(5.5, 10.5)` as `lrv`.
fn engine_with_wide_body() -> Engine {
    use classic_core::tilemap::TILE_M;
    let mut engine = engine_with_lrvtest_like_map(None);
    let (mut def, anchors) = lrv_def_real();
    def.path_footprint = None;
    def.path_body_m = Some([-0.95 * TILE_M, 0.95 * TILE_M, -1.9 * TILE_M, 1.9 * TILE_M]);
    def.safe_fall_m = 0.0;
    insert_lrv(&mut engine, (def, anchors));
    assert!(engine.spawn_vehicle("lrv", "lrv", 5.5, 10.5));
    engine
}

#[test]
fn a_vehicle_block_stops_the_vehicle_search_and_clearing_it_reopens_it() {
    let mut engine = engine_with_wide_body();
    // a wall across the map at x = 24, and a second block sharing a cell
    engine.set_vehicle_block(1, &[[24, 0, 24, 47]]);
    engine.set_vehicle_block(2, &[[24, 20, 30, 20]]);
    assert!(matches!(goto_sync(&mut engine, "lrv", 40, 10), VehicleGotoPoll::NoPath));
    // human pathing never sees it
    assert!(engine.find_path((5, 10), (40, 10)).is_some());
    // clearing the wall reopens the route; block 2 keeps its own cells
    assert!(engine.clear_vehicle_block(1));
    assert!(!engine.clear_vehicle_block(1), "already gone");
    assert!(matches!(goto_sync(&mut engine, "lrv", 40, 10), VehicleGotoPoll::Accepted(_)));
    assert_eq!(engine.vehicle_nav_snapshot.structural[20 * 48 + 24], 0);
    assert_eq!(engine.vehicle_nav_snapshot.structural[10 * 48 + 24], 1);
    // an empty list clears too
    engine.set_vehicle_block(2, &[]);
    assert!(engine.vehicle_blocks.is_empty());
    assert_eq!(engine.vehicle_nav_snapshot.structural[20 * 48 + 24], 1);
}

#[test]
fn a_lane_of_vehicle_blocks_is_driven_along_and_refused_across() {
    // blocked but for a 5-tall east-west lane (rows 8..=12) and a 3-wide
    // north-south one (columns 23..=25) crossing it
    let mut engine = engine_with_wide_body();
    let walls = [[0, 0, 22, 7], [26, 0, 47, 7], [0, 13, 22, 47], [26, 13, 47, 47]];
    engine.set_vehicle_block(7, &walls);
    match goto_sync(&mut engine, "lrv", 40, 10) {
        VehicleGotoPoll::Accepted(path) => {
            assert!(path.iter().all(|c| c[1] == 10), "straight along the lane: {path:?}")
        }
        _ => panic!("the 5-tall lane fits the body across it"),
    }
    // the body turned south is 5 wide: not down the 3-wide lane
    assert!(matches!(goto_sync(&mut engine, "lrv", 24, 40), VehicleGotoPoll::NoPath));
    // a body 3 wide is
    let (mut def, anchors) = lrv_def_real();
    def.path_footprint = None;
    def.path_body_m = Some([-0.6, 0.6, -0.6, 0.6]);
    def.safe_fall_m = 0.0;
    insert_lrv(&mut engine, (def, anchors));
    assert!(engine.spawn_vehicle("lrv", "small", 5.5, 10.5));
    assert!(matches!(goto_sync(&mut engine, "small", 24, 40), VehicleGotoPoll::Accepted(_)));
}

#[test]
fn a_vehicle_block_across_a_route_makes_the_vehicle_replan() {
    use classic_core::pathfinder::route_clear;

    let mut engine = engine_with_wide_body();
    let VehicleGotoPoll::Accepted(first) = goto_sync(&mut engine, "lrv", 40, 10) else {
        panic!("open map");
    };
    assert!(first.iter().all(|c| c[1] == 10));
    for _ in 0..30 {
        engine.time.delta = 1.0 / 60.0;
        engine.update_vehicles();
    }
    let ve = engine.names["lrv"];
    // a block off the route changes nothing
    engine.set_vehicle_block(1, &[[10, 30, 12, 32]]);
    assert_eq!(engine.world.get::<&IsoVehicle>(ve).unwrap().path, first);
    // a wall across it, with a way round to the south
    engine.set_vehicle_block(2, &[[30, 0, 30, 20]]);
    assert!(engine.world.get::<&IsoVehicle>(ve).unwrap().path.is_empty(), "stopped");
    engine.time.delta = 1.0 / 60.0;
    engine.update_vehicles();
    let path = engine.world.get::<&IsoVehicle>(ve).unwrap().path.clone();
    assert_eq!(path.last(), Some(&[40, 10]), "re-planned to the same goal: {path:?}");
    assert_ne!(path, first);
    let cells: Vec<(i32, i32)> = path.iter().map(|c| (c[0], c[1])).collect();
    let fp = search_footprint(&engine.world.get::<&IsoVehicle>(ve).unwrap());
    let snap = &engine.vehicle_nav_snapshot;
    assert!(route_clear(&snap.structural, 48, 48, &cells, &fp), "the new route is clear");
    // a wall with no way round: the vehicle stays stopped
    engine.set_vehicle_block(3, &[[34, 0, 34, 47]]);
    for _ in 0..3 {
        engine.time.delta = 1.0 / 60.0;
        engine.update_vehicles();
    }
    assert!(engine.world.get::<&IsoVehicle>(ve).unwrap().path.is_empty());
}

/// The lrvtest-like map with the mule-like rover (16 directions, front
/// steer, reversing at 0.9 tiles/s) spawned facing East at `(12.5, 24.5)`.
fn engine_with_rover() -> Engine {
    let mut engine = engine_with_lrvtest_like_map(None);
    engine.vehicles.insert("rover".into(), super::layer_tests::rover_def());
    assert!(engine.spawn_vehicle("rover", "rover", 12.5, 24.5));
    engine.set_synchronous_workers(true);
    engine
}

/// Submit a pose goto and install it (synchronous workers).
fn goto_pose_sync(engine: &mut Engine, x: f32, y: f32, deg: f32) -> VehicleGotoPoll {
    match engine.vehicle_goto_pose("rover", x, y, deg) {
        VehicleGotoSubmit::Submitted(id) => engine.vehicle_goto_poll(id),
        _ => VehicleGotoPoll::NoPath,
    }
}

/// Drive until the route is done: the biggest per-frame heading change
/// (radians), whether it backed up, and the frames it took.
fn drive_route(engine: &mut Engine) -> (f32, bool, u32) {
    let ve = engine.names["rover"];
    let (mut max_turn, mut backed) = (0.0f32, false);
    for frame in 0..3000 {
        let h0 = engine.world.get::<&IsoVehicle>(ve).unwrap().heading;
        engine.time.delta = 1.0 / 30.0;
        engine.update_vehicles();
        let v = engine.world.get::<&IsoVehicle>(ve).unwrap();
        max_turn = max_turn.max(wrap_pi(v.heading - h0).abs());
        backed |= v.reversing;
        if v.path.is_empty() && v.face.is_none() {
            return (max_turn, backed, frame);
        }
    }
    panic!("route not done in 100 s");
}

/// How far (degrees) the rover's heading is off `deg`, and its distance
/// (tiles) from `(x, y)`.
fn pose_error(engine: &Engine, x: f32, y: f32, deg: f32) -> (f32, f32) {
    let ve = engine.names["rover"];
    let v = engine.world.get::<&IsoVehicle>(ve).unwrap();
    let p = engine.world.get::<&classic_core::components::Transform>(ve).unwrap().position;
    (wrap_pi(v.heading - deg.to_radians()).abs().to_degrees(), (p.x - x).hypot(p.y - y))
}

#[test]
fn a_pose_goto_arrives_on_the_heading_in_open_space() {
    // Half the rover's 16-direction step: what its frame can show.
    let tolerance = 11.25;
    for (x, y, deg) in [(30.5, 24.5, 180.0), (28.5, 34.5, 90.0), (30.5, 16.5, 315.0)] {
        let mut engine = engine_with_rover();
        assert!(
            matches!(goto_pose_sync(&mut engine, x, y, deg), VehicleGotoPoll::Accepted(_)),
            "pose ({x}, {y}) facing {deg}"
        );
        let turn_rate = engine.world.get::<&IsoVehicle>(engine.names["rover"]).unwrap().turn_rate;
        let (max_turn, _, frames) = drive_route(&mut engine);
        let (dh, dp) = pose_error(&engine, x, y, deg);
        assert!(dh <= tolerance, "facing {deg}: {dh:.1} deg off after {frames} frames");
        assert!(dp <= 0.01, "at ({x}, {y}): {dp} tiles off");
        assert!(max_turn <= turn_rate / 30.0 + 1e-4, "no heading jump: {max_turn}");
    }
}

#[test]
fn a_narrow_lane_mouth_takes_a_k_turn_clear_of_the_vehicle_blocks() {
    use classic_core::pathfinder::route_clear_pose;
    // A corridor (rows 17..=31) along x, the rest stamped as vehicle blocks:
    // facing East at its west end, reach (32.5, 24.5) facing West.  Too
    // narrow to U-turn forward, so the route backs up.
    let mut engine = engine_with_rover();
    engine.set_vehicle_block(1, &[[0, 0, 47, 16], [0, 32, 47, 47]]);
    let VehicleGotoPoll::Accepted(path) = goto_pose_sync(&mut engine, 32.5, 24.5, 180.0) else {
        panic!("a K-turn route");
    };
    let ve = engine.names["rover"];
    let (reverse, fp) = {
        let v = engine.world.get::<&IsoVehicle>(ve).unwrap();
        (v.path_reverse.clone(), search_footprint(&v))
    };
    assert!(reverse.iter().any(|&b| b), "backs up: {path:?}");
    let cells: Vec<(i32, i32)> = path.iter().map(|c| (c[0], c[1])).collect();
    let s = &engine.vehicle_nav_snapshot;
    assert!(route_clear_pose(&s.structural, 48, 48, &cells, &reverse, &fp), "every chord clear");
    let (_, backed, _) = drive_route(&mut engine);
    assert!(backed, "the follower drove the reverse run");
    let (dh, dp) = pose_error(&engine, 32.5, 24.5, 180.0);
    assert!(dh <= 11.25 && dp <= 0.01, "arrives facing West: {dh:.1} deg, {dp} tiles");
}

#[test]
fn an_unreachable_pose_is_refused() {
    let mut engine = engine_with_rover();
    engine.set_vehicle_block(1, &[[30, 0, 30, 47]]);
    assert!(matches!(goto_pose_sync(&mut engine, 40.5, 24.5, 0.0), VehicleGotoPoll::NoPath));
    // no room for the body at the goal, and off the map
    assert!(matches!(goto_pose_sync(&mut engine, 1.5, 1.5, 0.0), VehicleGotoPoll::NoPath));
    assert!(matches!(goto_pose_sync(&mut engine, -4.0, 9.0, 0.0), VehicleGotoPoll::NoPath));
    assert!(matches!(
        engine.vehicle_goto_pose("rover", f32::NAN, 9.0, 0.0),
        VehicleGotoSubmit::NoVehicle
    ));
}

#[test]
fn a_block_owner_drives_through_its_block_and_no_one_else_does() {
    // A wall across the map at x = 24, owned by `lrv`.
    let mut engine = engine_with_wide_body();
    assert!(engine.spawn_vehicle("lrv", "other", 5.5, 30.5));
    assert!(!engine.set_vehicle_block_owner(1, Some("lrv")), "no block 1 yet");
    engine.set_vehicle_block(1, &[[24, 0, 24, 47]]);
    assert!(engine.set_vehicle_block_owner(1, Some("lrv")));
    assert!(matches!(goto_sync(&mut engine, "lrv", 40, 10), VehicleGotoPoll::Accepted(_)));
    assert!(matches!(goto_sync(&mut engine, "other", 40, 30), VehicleGotoPoll::NoPath));
    // replacing the rects keeps the owner
    engine.set_vehicle_block(1, &[[24, 0, 24, 47], [25, 0, 25, 3]]);
    assert!(matches!(goto_sync(&mut engine, "lrv", 40, 12), VehicleGotoPoll::Accepted(_)));
    // the shared grid leaves an owned block out; every other vehicle's
    // queries close it on top
    assert_eq!(engine.vehicle_nav_snapshot.structural[10 * 48 + 24], 1);
    assert_eq!(engine.closed_vehicle_blocks("other"), vec![[24, 0, 24, 47], [25, 0, 25, 3]]);
    assert!(engine.closed_vehicle_blocks("lrv").is_empty());
    // clearing the block reopens it for everyone, and forgets the owner
    assert!(engine.clear_vehicle_block(1));
    assert!(engine.vehicle_block_owners.is_empty());
    assert!(matches!(goto_sync(&mut engine, "other", 40, 30), VehicleGotoPoll::Accepted(_)));
}

/// Submit a goto on the background pathfinder thread and wait for it.
fn goto_threaded(engine: &mut Engine, name: &str, tx: i32, ty: i32) -> VehicleGotoPoll {
    engine.set_synchronous_workers(false);
    let VehicleGotoSubmit::Submitted(id) = engine.vehicle_goto(name, tx, ty) else {
        return VehicleGotoPoll::NoPath;
    };
    for _ in 0..5000 {
        match engine.vehicle_goto_poll(id) {
            VehicleGotoPoll::Pending => std::thread::sleep(std::time::Duration::from_millis(1)),
            done => return done,
        }
    }
    panic!("the background search never answered");
}

#[test]
fn the_background_search_opens_a_block_to_its_owner_only() {
    // the worker's shared grid leaves the owned wall out; `other`'s query
    // closes it on top, `lrv`'s does not
    let mut engine = engine_with_wide_body();
    assert!(engine.spawn_vehicle("lrv", "other", 5.5, 30.5));
    engine.set_vehicle_block(1, &[[24, 0, 24, 47]]);
    assert!(matches!(goto_threaded(&mut engine, "lrv", 40, 10), VehicleGotoPoll::NoPath));
    engine.set_vehicle_block_owner(1, Some("lrv"));
    assert!(matches!(goto_threaded(&mut engine, "lrv", 40, 10), VehicleGotoPoll::Accepted(_)));
    assert!(matches!(goto_threaded(&mut engine, "other", 40, 30), VehicleGotoPoll::NoPath));
    // given away, it closes to lrv and opens to its new owner
    engine.set_vehicle_block_owner(1, Some("other"));
    assert!(matches!(goto_threaded(&mut engine, "lrv", 40, 12), VehicleGotoPoll::NoPath));
    assert!(matches!(goto_threaded(&mut engine, "other", 40, 30), VehicleGotoPoll::Accepted(_)));
}

#[test]
fn losing_a_blocks_ownership_replans_the_route_through_it() {
    let mut engine = engine_with_wide_body();
    engine.set_vehicle_block(1, &[[24, 0, 24, 47]]);
    engine.set_vehicle_block_owner(1, Some("lrv"));
    let VehicleGotoPoll::Accepted(route) = goto_sync(&mut engine, "lrv", 40, 10) else {
        panic!("the owner's route");
    };
    let ve = engine.names["lrv"];
    for _ in 0..10 {
        engine.time.delta = 1.0 / 60.0;
        engine.update_vehicles();
    }
    // handing it to another vehicle cuts lrv's route: it stops, re-plans,
    // and (the wall spans the map) stays stopped
    assert!(engine.set_vehicle_block_owner(1, Some("other")));
    assert!(engine.world.get::<&IsoVehicle>(ve).unwrap().path.is_empty(), "stopped");
    for _ in 0..3 {
        engine.time.delta = 1.0 / 60.0;
        engine.update_vehicles();
    }
    assert!(engine.world.get::<&IsoVehicle>(ve).unwrap().path.is_empty(), "no way round");
    // given back, a new goto drives through again
    assert!(engine.set_vehicle_block_owner(1, Some("lrv")));
    assert!(
        matches!(goto_sync(&mut engine, "lrv", 40, 10), VehicleGotoPoll::Accepted(r) if r.last() == route.last())
    );
}
