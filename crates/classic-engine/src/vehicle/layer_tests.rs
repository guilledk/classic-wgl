//! v2 layered vehicles: spawn, layer placement and frames, clips, turrets,
//! spin and pivot turns.

use classic_core::components::{ClipPlay, IsoSprite, IsoVehicle, Role, RoleKind, Transform};
use classic_core::tilemap::TILE_M;
use classic_core::types::{FrameTable, VehicleDef};
use classic_core::vehicle_def::ClipDef;

use super::layers::{chassis_to_tile, clip_frame, contact_offsets, layer_poses};
use crate::Engine;

fn tank_def() -> VehicleDef {
    serde_json::from_value(serde_json::json!({
        "version": 2, "name": "tank", "directions": 16, "pivot_turn": true,
        "turn_rate_deg_per_sec": 90.0,
        "layers": [
            { "name": "hull", "kind": "hull", "texture": "tankHull",
              "pitch": { "levels": 3, "max_deg": 12.0 } },
            { "name": "track_l", "kind": "gear_side", "texture": "tankTrackL", "side": "left",
              "offset": [0.0, 1.0, 0.3], "spin_frames": 4, "radius_m": 0.5 },
            { "name": "track_r", "kind": "gear_side", "texture": "tankTrackR", "side": "right",
              "offset": [0.0, -1.0, 0.3], "spin_frames": 4, "radius_m": 0.5 },
            { "name": "turret", "kind": "turret", "texture": "tankTurret", "directions": 24,
              "offset": [-0.5, 0.0, 1.2], "fire_frames": 3, "yaw_rate_deg_per_sec": 90.0,
              "pitch": { "levels": 4, "min_deg": -5.0, "max_deg": 25.0 } },
            { "name": "crane", "kind": "tool", "texture": "tankCrane", "parent": "turret",
              "offset": [1.0, 0.0, 0.5],
              "clips": { "swing": { "start": 2, "frames": 6, "fps": 10.0 } } },
            { "name": "crate", "kind": "cargo", "texture": "tankCrate", "visible": false,
              "ghost": false }
        ]
    }))
    .unwrap()
}

fn tables(engine: &mut Engine) {
    for t in ["tankHull", "tankTrackL", "tankTrackR", "tankTurret", "tankCrane", "tankCrate"] {
        engine.frame_tables.insert(
            t.into(),
            FrameTable {
                version: 2,
                sheets: vec![],
                frames: Default::default(),
                dir_interp: false,
                companions: vec![],
            },
        );
    }
}

fn spawn_tank(x: f32, y: f32) -> Engine {
    let mut engine = Engine::new_for_test();
    let tm =
        engine.world.spawn((super::tests::flat_tilemap_pub(24, 24), Role::new(RoleKind::Tilemap)));
    engine.names.insert("tilemap".into(), tm);
    engine.vehicles.insert("tank".into(), tank_def());
    tables(&mut engine);
    assert!(engine.spawn_vehicle("tank", "tank", x, y));
    engine
}

fn frame_name(engine: &Engine, entity: &str) -> String {
    engine.world.get::<&IsoSprite>(engine.names[entity]).unwrap().frame_name.clone().unwrap()
}

fn pos(engine: &Engine, entity: &str) -> glam::Vec3 {
    engine.world.get::<&Transform>(engine.names[entity]).unwrap().position
}

#[test]
fn pure_layer_geometry() {
    let def = tank_def();
    let poses = layer_poses(&def, &[0.0, 0.0, 0.0, std::f32::consts::FRAC_PI_2, 0.0, 0.0]);
    // The crane rides the turret: its offset turns with the turret yaw (a
    // quarter turn clockwise on screen = toward chassis -y).
    let crane = poses[4].pos;
    assert!((crane.x - -0.5).abs() < 1e-5 && (crane.y - -1.0).abs() < 1e-5, "{crane:?}");
    assert!((crane.z - 1.7).abs() < 1e-5);
    assert!((poses[4].yaw - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
    // Contacts: the tracks' footprint, a radius ahead / behind.
    let c = contact_offsets(&def);
    assert_eq!(c, [[0.5, 1.0], [0.5, -1.0], [-0.5, 1.0], [-0.5, -1.0]]);
    // Chassis -> tile: forward runs along the heading, left is 90° anticlockwise
    // on screen (heading East: left is North, tile -y).
    let f = chassis_to_tile([TILE_M, 0.0], 0.0);
    let l = chassis_to_tile([0.0, TILE_M], 0.0);
    assert!((f[0] - 1.0).abs() < 1e-6 && f[1].abs() < 1e-6);
    assert!(l[0].abs() < 1e-6 && (l[1] + 1.0).abs() < 1e-6);
    // Clip frames: forward, backward, loop and hold.
    let cd = ClipDef { start: 2, frames: 6, fps: 10.0 };
    let play = |t: f32, speed: f32, repeat: i32| {
        clip_frame(&cd, &ClipPlay { clip: "swing".into(), time: t, speed, repeat })
    };
    assert_eq!(play(0.0, 1.0, 0), (2, false));
    assert_eq!(play(0.35, 1.0, 0), (5, false));
    assert_eq!(play(0.65, 1.0, 0), (2, false), "loops");
    assert_eq!(play(0.65, 1.0, 1), (7, true), "holds the last frame");
    assert_eq!(play(0.0, -1.0, 0), (7, false), "backward starts at the end");
    assert_eq!(play(0.65, -1.0, 1), (2, true), "backward holds the first frame");
    assert_eq!(play(0.2, 2.0, 0), (6, false), "speed scales the fps");
}

#[test]
fn v2_vehicle_spawns_and_places_its_layers() {
    let engine = spawn_tank(8.0, 8.0);
    let tank = engine.names["tank"];
    let v = engine.world.get::<&IsoVehicle>(tank).unwrap();
    assert_eq!(v.directions, 16);
    assert_eq!(v.layers.len(), 6);
    assert_eq!(v.contact_offsets_m.len(), 4);
    assert!((v.wheelbase_m - 1.0).abs() < 1e-5 && (v.track_m - 2.0).abs() < 1e-5);
    assert!(v.pivot_turn);
    drop(v);
    // Layers are entities named `{vehicle}.{layer}`; the hull is the body.
    for l in ["tank.track_l", "tank.track_r", "tank.turret", "tank.crane", "tank.crate"] {
        assert!(engine.names.contains_key(l), "{l}");
    }
    assert!(engine.is_disabled(engine.names["tank.crate"]), "an invisible layer starts hidden");
    assert!(engine.world.get::<&IsoSprite>(engine.names["tank.crate"]).unwrap().no_ghost);
    // Heading 0 (East): the left track sits a metre north (tile -y).
    let body = pos(&engine, "tank");
    let left = pos(&engine, "tank.track_l");
    assert!((left.x - body.x).abs() < 1e-4);
    assert!((left.y - (body.y - 1.0 / TILE_M)).abs() < 1e-4, "{left:?} vs {body:?}");
    // Frames: hull level pitch (index 1 of 3) facing 0 -> 1 * 16 + 0.
    assert_eq!(frame_name(&engine, "tank"), "tankHull_16");
    assert_eq!(frame_name(&engine, "tank.turret"), "tankTurret_0");
    assert_eq!(frame_name(&engine, "tank.crane"), "tankCrane_0");
    // All layers share the vehicle's ghost group.
    let g = |n: &str| engine.world.get::<&IsoSprite>(engine.names[n]).unwrap().ghost_group;
    assert!(g("tank") != 0 && g("tank.turret") == g("tank") && g("tank.crane") == g("tank"));
}

#[test]
fn turret_slews_and_frames_independently() {
    let mut engine = spawn_tank(8.0, 8.0);
    assert!(engine.vehicle_aim_angles("tank", 90.0, 25.0));
    engine.time.delta = 0.5;
    engine.update_vehicles();
    // 90°/s for 0.5 s: 45° = 3 of 24 turret directions; gun pitch at the top.
    let turret = frame_name(&engine, "tank.turret");
    // index = (pitch 3 * fire 3 + fire 0) * 24 + dir 3
    assert_eq!(turret, format!("tankTurret_{}", 9 * 24 + 3));
    engine.update_vehicles();
    assert_eq!(frame_name(&engine, "tank.turret"), format!("tankTurret_{}", 9 * 24 + 6));
    // The crane rides the turret: after 90° it sits on the chassis right.
    let body = pos(&engine, "tank");
    let crane = pos(&engine, "tank.crane");
    let expect = chassis_to_tile([-0.5, -1.0], 0.0);
    assert!((crane.x - body.x - expect[0]).abs() < 1e-3, "{crane:?}");
    assert!((crane.y - body.y - expect[1]).abs() < 1e-3, "{crane:?}");
    // Aiming at a point turns the turret toward it (due south = tile +y).
    assert!(engine.vehicle_aim("tank", 8.0 - 0.5 / TILE_M, 16.0));
    for _ in 0..8 {
        engine.update_vehicles();
    }
    let tank = engine.names["tank"];
    let yaw = engine.world.get::<&IsoVehicle>(tank).unwrap().layers[3].yaw;
    assert!((yaw - std::f32::consts::FRAC_PI_2).abs() < 0.02, "yaw {yaw}");
}

#[test]
fn clips_play_and_layers_toggle() {
    let mut engine = spawn_tank(8.0, 8.0);
    assert!(engine.vehicle_play("tank", "crane", "swing", 1.0, 1));
    assert!(!engine.vehicle_play("tank", "crane", "nope", 1.0, 0));
    assert!(engine.vehicle_play("tank", "turret", super::FIRE_CLIP, 1.0, 1));
    assert!(engine.vehicle_aim_angles("tank", 0.0, 15.0));
    engine.time.delta = 0.25;
    engine.update_vehicles();
    // Crane: 10 fps for 0.25 s = local frame 2 -> clip frame 4, dir 0.
    assert_eq!(frame_name(&engine, "tank.crane"), format!("tankCrane_{}", 4 * 16));
    // Turret fire: 12 fps -> fire frame 2 (of 3); 15° is pitch index 2 of
    // the -5..25 axis; dir 0: (2 * 3 + 2) * 24.
    assert_eq!(frame_name(&engine, "tank.turret"), "tankTurret_192");
    engine.time.delta = 2.0;
    engine.update_vehicles();
    assert_eq!(frame_name(&engine, "tank.crane"), format!("tankCrane_{}", 7 * 16), "holds");
    assert!(engine.vehicle_layer_visible("tank", "crate", true));
    assert!(!engine.is_disabled(engine.names["tank.crate"]));
    assert!(engine.vehicle_layer_color("tank", "hull", [0.5, 0.5, 0.5, 1.0]));
    engine.update_vehicles();
    assert_eq!(
        engine.world.get::<&IsoSprite>(engine.names["tank"]).unwrap().color,
        [0.5, 0.5, 0.5, 1.0]
    );
}

#[test]
fn tracks_spin_with_distance_and_pivot_turns() {
    let mut engine = spawn_tank(8.0, 8.0);
    let tank = engine.names["tank"];
    // Drive a quarter revolution of a 0.5 m track: 2π·0.5/4 metres.
    {
        let mut v = engine.world.get::<&mut IsoVehicle>(tank).unwrap();
        v.odometer_m = std::f32::consts::TAU * 0.5 / 4.0 + 1e-4;
    }
    engine.time.delta = 1e-6;
    engine.update_vehicles();
    assert_eq!(frame_name(&engine, "tank.track_l"), "tankTrackL_16", "spin 1 * 16 dirs");
    assert_eq!(frame_name(&engine, "tank.track_r"), "tankTrackR_16");
    // A pivot turn runs the sides opposite ways.
    {
        let mut v = engine.world.get::<&mut IsoVehicle>(tank).unwrap();
        v.odometer_m = 0.0;
        v.turned = (std::f32::consts::TAU * 0.5 / 4.0 + 1e-4) / 1.0;
    }
    engine.update_vehicles();
    assert_eq!(frame_name(&engine, "tank.track_l"), "tankTrackL_16", "spin 1 * 16 dirs");
    // The right side ran just over a quarter turn backward: spin 2.
    assert_eq!(frame_name(&engine, "tank.track_r"), "tankTrackR_32");
    // A target far behind: the tank turns in place (no translation).
    let start = pos(&engine, "tank");
    {
        let mut v = engine.world.get::<&mut IsoVehicle>(tank).unwrap();
        v.path = vec![[2, 8]];
        v.path_idx = 0;
        v.reverse_speed = 0.0;
    }
    engine.time.delta = 0.1;
    engine.update_vehicles();
    let after = pos(&engine, "tank");
    assert!(
        (after.x - start.x).abs() < 1e-5 && (after.y - start.y).abs() < 1e-5,
        "pivoted in place"
    );
    assert!(engine.world.get::<&IsoVehicle>(tank).unwrap().heading.abs() > 0.1);
}
