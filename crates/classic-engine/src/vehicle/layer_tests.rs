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
                axes: None,
                tilt_interp: false,
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

fn heading(engine: &Engine) -> f32 {
    engine.world.get::<&IsoVehicle>(engine.names["tank"]).unwrap().heading
}

#[test]
fn vehicle_face_pivots_in_place_or_snaps() {
    use std::f32::consts::FRAC_PI_2;
    let mut engine = spawn_tank(8.0, 8.0);
    let tank = engine.names["tank"];
    engine.world.get::<&mut IsoVehicle>(tank).unwrap().heading = 0.0;
    let start = pos(&engine, "tank");
    // Tracked (`pivot_turn`, 90 deg/s): South (90 tile degrees) in 10 steps
    // of 0.1 s, standing still.
    assert_eq!(engine.vehicle_face("tank", 90.0), 1);
    engine.time.delta = 0.1;
    engine.update_vehicles();
    assert!((heading(&engine) - FRAC_PI_2 / 10.0).abs() < 1e-5, "{}", heading(&engine));
    for _ in 0..9 {
        engine.update_vehicles();
    }
    assert!((heading(&engine) - FRAC_PI_2).abs() < 1e-5, "{}", heading(&engine));
    assert!(engine.world.get::<&IsoVehicle>(tank).unwrap().face.is_none(), "arrived");
    let after = pos(&engine, "tank");
    assert!((after.x - start.x).abs() < 1e-5 && (after.y - start.y).abs() < 1e-5);
    // The short way round: from South to North-East (-45) turns back.
    assert_eq!(engine.vehicle_face("tank", -45.0), 1);
    engine.update_vehicles();
    assert!((heading(&engine) - (FRAC_PI_2 - FRAC_PI_2 / 10.0)).abs() < 1e-5);
    // Wheeled (no pivot turn): it cannot turn standing still, so it snaps.
    engine.world.get::<&mut IsoVehicle>(tank).unwrap().pivot_turn = false;
    assert_eq!(engine.vehicle_face("tank", 180.0), 1);
    engine.update_vehicles();
    assert!((heading(&engine).abs() - std::f32::consts::PI).abs() < 1e-5);
    // Any other command drops a pending face; bad input is refused.
    engine.world.get::<&mut IsoVehicle>(tank).unwrap().pivot_turn = true;
    assert_eq!(engine.vehicle_face("tank", 0.0), 1);
    assert!(engine.vehicle_stop("tank"));
    assert!(engine.world.get::<&IsoVehicle>(tank).unwrap().face.is_none());
    assert_eq!(engine.vehicle_face("tank", f32::NAN), -1);
    assert_eq!(engine.vehicle_face("ghost", 0.0), 0);
}

#[test]
fn vehicle_reverse_to_backs_straight_keeping_heading() {
    // The mule backing over a box: lined up East, the box 3 tiles behind.
    let mut engine = spawn_tank(8.0, 8.0);
    let tank = engine.names["tank"];
    {
        let mut v = engine.world.get::<&mut IsoVehicle>(tank).unwrap();
        v.heading = 0.0;
        v.reverse_speed = 1.0;
    }
    let start = pos(&engine, "tank");
    // Ahead is refused; behind is accepted (its 0.3 sideways offset is not
    // steered for: the heading is kept).
    assert_eq!(engine.vehicle_reverse_to("tank", start.x + 3.0, start.y), -1);
    assert_eq!(engine.vehicle_reverse_to("tank", start.x - 3.0, start.y + 0.3), 1);
    engine.time.delta = 0.1;
    engine.update_vehicles();
    let v = engine.world.get::<&IsoVehicle>(tank).unwrap().reversing;
    assert!(v, "reversing");
    assert!((pos(&engine, "tank").x - (start.x - 0.1)).abs() < 1e-4, "reverse_speed 1 tile/s");
    let mut steps = 1;
    while engine.world.get::<&IsoVehicle>(tank).unwrap().reverse_to.is_some() {
        engine.update_vehicles();
        steps += 1;
        assert!(steps < 100, "never arrived");
    }
    assert_eq!(steps, 30, "3 tiles at 1 tile/s, no overshoot");
    let end = pos(&engine, "tank");
    assert!((end.x - (start.x - 3.0)).abs() < 1e-4, "{end:?}");
    assert!((end.y - start.y).abs() < 1e-5, "straight: no sideways drift");
    assert_eq!(heading(&engine), 0.0, "heading kept");
    // Stopped: the next step does not move it.
    engine.update_vehicles();
    assert!((pos(&engine, "tank").x - end.x).abs() < 1e-6);
    assert!(!engine.world.get::<&IsoVehicle>(tank).unwrap().reversing);
    assert_eq!(engine.vehicle_reverse_to("ghost", 0.0, 0.0), 0);
}

fn socket_tank() -> Engine {
    let mut engine = Engine::new_for_test();
    let tm = engine.world.spawn((
        super::tests::flat_tilemap_pub(24, 24),
        Role::new(RoleKind::Tilemap),
        Transform::new(glam::Vec3::ZERO, glam::Vec3::ONE),
    ));
    engine.names.insert("tilemap".into(), tm);
    let mut def = tank_def();
    def.layers[3].pitch_pivot = Some([0.5, 0.0, 0.6]);
    def.sockets.insert(
        "muzzle".into(),
        classic_core::vehicle_def::SocketDef { layer: "turret".into(), offset: [2.5, 0.0, 0.6] },
    );
    def.sockets.insert(
        "lamp".into(),
        classic_core::vehicle_def::SocketDef { layer: "hull".into(), offset: [1.0, 0.5, 0.8] },
    );
    engine.vehicles.insert("tank".into(), def);
    tables(&mut engine);
    assert!(engine.spawn_vehicle("tank", "tank", 8.0, 8.0));
    engine
}

#[test]
fn sockets_follow_heading_turret_yaw_and_gun_pitch() {
    let mut engine = socket_tank();
    let base = {
        let tank = engine.names["tank"];
        let tf = engine.world.get::<&Transform>(tank).unwrap().position;
        let alt = engine.world.get::<&IsoVehicle>(tank).unwrap().altitude;
        engine.iso_to_world(tf.x, tf.y, alt - engine.height_at(tf.x, tf.y)).unwrap()
    };
    // Heading 0 (East = world +X): a hull socket 1 m forward, 0.5 m left
    // (world +Y), 0.8 m up.
    let lamp = engine.vehicle_socket_pos("tank", "lamp").unwrap();
    assert!((lamp - base - glam::Vec3::new(1.0, 0.5, 0.8)).length() < 1e-4, "{lamp:?}");
    // Turret at (-0.5, 0, 1.2); muzzle 2.5 m forward of the turret origin.
    let muzzle = engine.vehicle_socket_pos("tank", "muzzle").unwrap();
    assert!((muzzle - base - glam::Vec3::new(2.0, 0.0, 1.8)).length() < 1e-4, "{muzzle:?}");
    // Gun pitch 25° about the trunnion (0.5 m forward, 0.6 m up): the muzzle
    // swings on a 2 m arm.
    assert!(engine.vehicle_aim_angles("tank", 0.0, 25.0));
    engine.time.delta = 1e-6;
    engine.update_vehicles();
    let p = 25f32.to_radians();
    let expect = glam::Vec3::new(-0.5 + 0.5 + 2.0 * p.cos(), 0.0, 1.2 + 0.6 + 2.0 * p.sin());
    let muzzle = engine.vehicle_socket_pos("tank", "muzzle").unwrap();
    assert!((muzzle - base - expect).length() < 1e-3, "{muzzle:?} vs {expect:?}");
    // Turret yaw 90° (clockwise on screen: toward the chassis right, world
    // -Y at heading 0): the arm now points right.
    assert!(engine.vehicle_aim_angles("tank", 90.0, 0.0));
    engine.time.delta = 1.0;
    engine.update_vehicles();
    let muzzle = engine.vehicle_socket_pos("tank", "muzzle").unwrap();
    assert!((muzzle - base - glam::Vec3::new(-0.5, -2.5, 1.8)).length() < 1e-3, "{muzzle:?}");
    // The socket frame's forward axis points along the gun.
    let (_, basis) = engine.vehicle_socket_frame("tank", "muzzle").unwrap();
    assert!((basis.x_axis - glam::Vec3::new(0.0, -1.0, 0.0)).length() < 1e-3);
    assert!(engine.vehicle_socket_pos("tank", "nope").is_none());
}

#[test]
fn lights_ride_vehicle_sockets() {
    let mut engine = socket_tank();
    let light: classic_core::components::Light = serde_json::from_value(serde_json::json!({
        "kind": "point", "position": [0.0, 0.0, 0.1], "color": [1, 1, 1], "intensity": 1,
        "radius": 3, "parent": "tank", "socket": "lamp"
    }))
    .unwrap();
    assert_eq!(light.node.as_deref(), Some("lamp"), "`socket` is the node key");
    engine.world.spawn((light,));
    let lamp = engine.vehicle_socket_pos("tank", "lamp").unwrap();
    let lights = engine.gather_lights();
    assert_eq!(lights.len(), 1);
    assert!((lights[0].position - lamp - glam::Vec3::new(0.0, 0.0, 0.1)).length() < 1e-4);
}

/// A 16-direction vehicle teleports at a direction past the first 8 (the
/// mule's scripted reverse teleports every frame): every per-direction table
/// is sized to the direction count, and the teleport reads them by its own
/// length.
#[test]
fn sixteen_direction_vehicles_teleport_past_direction_eight() {
    let mut engine = spawn_tank(8.0, 8.0);
    let tank = engine.names["tank"];
    {
        let v = engine.world.get::<&IsoVehicle>(tank).unwrap();
        assert_eq!(v.body_anchors.len(), 16);
        assert!(v.wheel_anchors.iter().all(|a| a.len() == 16));
        assert!(v.wheel_tile_offsets.iter().all(|a| a.len() == 16));
    }
    for dir in [8u32, 10, 15] {
        engine.world.get::<&mut IsoVehicle>(tank).unwrap().direction = dir;
        assert!(engine.vehicle_teleport("tank", 9.0, 9.0), "teleport at direction {dir}");
        let v = engine.world.get::<&IsoVehicle>(tank).unwrap();
        let step = std::f32::consts::TAU / 16.0;
        assert!((v.heading - dir as f32 * step).abs() < 1e-5);
        drop(v);
        assert_eq!(frame_name(&engine, "tank"), format!("tankHull_{}", 16 + dir));
    }
}

/// Tables of another length (an anchors artifact shorter than the count)
/// wrap instead of panicking.
#[test]
fn dir_entry_wraps_on_the_table_length() {
    use super::kinematics::dir_entry;
    let t = [[0.1, 0.2], [0.3, 0.4]];
    assert_eq!(dir_entry(&t, 0), [0.1, 0.2]);
    assert_eq!(dir_entry(&t, 3), [0.3, 0.4]);
    assert_eq!(dir_entry(&[], 5), [0.0, 0.0]);
}

/// A spot light on a vehicle socket turns its beam with the socket frame:
/// the authored direction is in the socket layer's local frame.
#[test]
fn socket_spot_lights_turn_with_the_vehicle() {
    let mut engine = socket_tank();
    let light: classic_core::components::Light = serde_json::from_value(serde_json::json!({
        "kind": "spot", "position": [0.0, 0.0, 0.0], "color": [1, 1, 1], "intensity": 1,
        "radius": 6, "dir": [1.0, 0.0, -0.25], "cone_angle": 0.4,
        "parent": "tank", "socket": "lamp"
    }))
    .unwrap();
    engine.world.spawn((light,));
    let beam = |engine: &Engine| engine.gather_lights()[0].dir.normalize();
    // Heading 0 (East): forward is world +X.
    let d = beam(&engine);
    assert!((d - glam::Vec3::new(1.0, 0.0, -0.25).normalize()).length() < 1e-4, "{d:?}");
    // A quarter turn clockwise on screen (tile +y, world -Y).
    let tank = engine.names["tank"];
    engine.world.get::<&mut IsoVehicle>(tank).unwrap().heading = std::f32::consts::FRAC_PI_2;
    let d = beam(&engine);
    assert!((d - glam::Vec3::new(0.0, -1.0, -0.25).normalize()).length() < 1e-4, "{d:?}");
}

/// The tank with a rolling hull (pitch and roll 3 × ±12°) and a crate cargo
/// layer baked like the tilted container (pitch 5 × roll 3, ±20°), on a
/// heightfield rising `rise` m per tile toward East.
fn spawn_tilting_tank_on_slope(rise: f32) -> Engine {
    let mut engine = Engine::new_for_test();
    let mut tm = super::tests::flat_tilemap_pub(24, 24);
    tm.height_data = (0..25 * 25).map(|i| (i % 25) as f32 * rise).collect();
    let tm = engine.world.spawn((tm, Role::new(RoleKind::Tilemap)));
    engine.names.insert("tilemap".into(), tm);
    let mut def = tank_def();
    let hull = def.layers.iter_mut().find(|l| l.name == "hull").unwrap();
    hull.roll = Some(classic_core::vehicle_def::AngleAxis::symmetric(3, 12.0));
    let crate_layer = def.layers.iter_mut().find(|l| l.name == "crate").unwrap();
    crate_layer.pitch = Some(classic_core::vehicle_def::AngleAxis::symmetric(5, 20.0));
    crate_layer.roll = Some(classic_core::vehicle_def::AngleAxis::symmetric(3, 20.0));
    classic_core::vehicle_def::validate(&def).expect("cargo may carry pitch / roll");
    engine.vehicles.insert("tank".into(), def);
    tables(&mut engine);
    assert!(engine.spawn_vehicle("tank", "tank", 12.0, 12.0));
    engine
}

fn face(engine: &mut Engine, dir: u32) {
    engine.world.get::<&mut IsoVehicle>(engine.names["tank"]).unwrap().direction = dir;
    assert!(engine.vehicle_teleport("tank", 12.0, 12.0));
}

#[test]
fn cargo_tilt_follows_the_carriers_drawn_frame() {
    // ~8° slope: the hull draws its 12° level, the crate its nearest (10°,
    // then 20° on its roll axis, which has no 10° level).
    let mut engine = spawn_tilting_tank_on_slope(0.1);
    let slope = (0.1 / TILE_M).atan();
    assert!((slope.to_degrees() - 8.1).abs() < 0.1);
    // Facing East (uphill): hull pitch level 2 (+12°), roll level 1.
    face(&mut engine, 0);
    let v = IsoVehicle::clone(&engine.world.get::<&IsoVehicle>(engine.names["tank"]).unwrap());
    assert!((v.pitch - slope).abs() < 1e-3 && v.roll.abs() < 1e-3);
    assert_eq!(frame_name(&engine, "tank"), format!("tankHull_{}", (2 * 3 + 1) * 16));
    // Crate: hull 12° -> its pitch level 3 (10°) of 5, roll level 1.
    assert_eq!(frame_name(&engine, "tank.crate"), format!("tankCrate_{}", (3 * 3 + 1) * 16));
    // Facing South (dir 4): East is left, so the hull rolls left-up.
    face(&mut engine, 4);
    let v = IsoVehicle::clone(&engine.world.get::<&IsoVehicle>(engine.names["tank"]).unwrap());
    assert!(v.pitch.abs() < 1e-3 && (v.roll - slope).abs() < 1e-3, "{} {}", v.pitch, v.roll);
    assert_eq!(v.roll_levels, 3, "a v2 hull's roll axis reaches the chassis");
    assert_eq!(frame_name(&engine, "tank"), format!("tankHull_{}", (3 + 2) * 16 + 4));
    // Crate: hull roll 12° -> its roll level 2 (20°), pitch level 2 (0°).
    assert_eq!(frame_name(&engine, "tank.crate"), format!("tankCrate_{}", (2 * 3 + 2) * 16 + 4));
    // Facing West (downhill, dir 8): nose down.
    face(&mut engine, 8);
    assert_eq!(frame_name(&engine, "tank"), format!("tankHull_{}", 16 + 8));
    assert_eq!(frame_name(&engine, "tank.crate"), format!("tankCrate_{}", (3 + 1) * 16 + 8));
    // Flat ground: every level frame.
    let mut flat = spawn_tilting_tank_on_slope(0.0);
    face(&mut flat, 2);
    assert_eq!(frame_name(&flat, "tank"), format!("tankHull_{}", (3 + 1) * 16 + 2));
    assert_eq!(frame_name(&flat, "tank.crate"), format!("tankCrate_{}", (2 * 3 + 1) * 16 + 2));
}

#[test]
fn a_rolling_hull_takes_its_dead_zone_from_its_finest_axis() {
    // Pitch 3 × ±12° (half-step 6°) and roll 5 × ±12° (half-step 3°).
    let mut engine = Engine::new_for_test();
    let tm =
        engine.world.spawn((super::tests::flat_tilemap_pub(24, 24), Role::new(RoleKind::Tilemap)));
    engine.names.insert("tilemap".into(), tm);
    let mut def = tank_def();
    def.layers[0].roll = Some(classic_core::vehicle_def::AngleAxis::symmetric(5, 12.0));
    engine.vehicles.insert("tank".into(), def);
    tables(&mut engine);
    assert!(engine.spawn_vehicle("tank", "tank", 12.0, 12.0));
    let v = engine.world.get::<&IsoVehicle>(engine.names["tank"]).unwrap();
    assert_eq!((v.pitch_levels, v.roll_levels), (3, 5));
    assert!((v.tilt_dead_zone - 3f32.to_radians()).abs() < 1e-6);
    // Roll only: its own half-step (a pitch-less hull used to get 0).
    let mut engine = Engine::new_for_test();
    let tm =
        engine.world.spawn((super::tests::flat_tilemap_pub(24, 24), Role::new(RoleKind::Tilemap)));
    engine.names.insert("tilemap".into(), tm);
    let mut def = tank_def();
    def.layers[0].pitch = None;
    def.layers[0].roll = Some(classic_core::vehicle_def::AngleAxis::symmetric(3, 12.0));
    engine.vehicles.insert("tank".into(), def);
    tables(&mut engine);
    assert!(engine.spawn_vehicle("tank", "tank", 12.0, 12.0));
    let v = engine.world.get::<&IsoVehicle>(engine.names["tank"]).unwrap();
    assert!((v.tilt_dead_zone - 6f32.to_radians()).abs() < 1e-6);
}

#[test]
fn tilt_interp_draws_the_exact_chassis_tilt_on_hull_and_cargo() {
    // ~8.1° slope: the hull's 3 × ±12° levels snap to 12°, the warp draws 8.1°.
    let mut engine = spawn_tilting_tank_on_slope(0.1);
    for t in ["tankHull", "tankCrate"] {
        let table = engine.frame_tables.get_mut(t).unwrap();
        table.dir_interp = true;
        table.tilt_interp = true;
    }
    face(&mut engine, 0);
    let slope = (0.1 / TILE_M).atan();
    let blend = |e: &Engine, n: &str| {
        e.world.get::<&IsoSprite>(e.names[n]).unwrap().dir_blend.expect("dir_interp blends")
    };
    // Hull: level frame pitch 2 (+12°), warped down to the slope.
    assert_eq!(frame_name(&engine, "tank"), format!("tankHull_{}", (2 * 3 + 1) * 16));
    let want = classic_core::vehicle_def::tilt_residual(0.0, slope, 0.0, 12f32.to_radians(), 0.0);
    let got = blend(&engine, "tank").tilt;
    assert!(got.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-5), "{got:?} vs {want:?}");
    // The crate rides the exact (warped) hull tilt: level 3 (10°), warped to
    // 8.1°, not the hull's 12° level.
    assert_eq!(frame_name(&engine, "tank.crate"), format!("tankCrate_{}", (3 * 3 + 1) * 16));
    let want = classic_core::vehicle_def::tilt_residual(0.0, slope, 0.0, 10f32.to_radians(), 0.0);
    let got = blend(&engine, "tank.crate").tilt;
    assert!(got.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-5), "{got:?} vs {want:?}");
}

#[test]
fn layer_origins_ride_the_full_body_tilt_like_sockets() {
    // On the ~8° slope facing uphill, a layer 1.2 m up (the turret) sits back
    // by ~z·sin(p) along the heading; its sprite anchor and a zero-offset
    // socket on it are the same world point.
    let mut engine = spawn_tilting_tank_on_slope(0.1);
    let tm = engine.names["tilemap"];
    engine.world.insert_one(tm, Transform::new(glam::Vec3::ZERO, glam::Vec3::ONE)).unwrap();
    for (layer, socket) in [("turret", "t0"), ("crate", "c0"), ("track_l", "l0")] {
        let tank = engine.vehicles.get_mut("tank").unwrap();
        tank.sockets.insert(
            socket.into(),
            classic_core::vehicle_def::SocketDef { layer: layer.into(), offset: [0.0; 3] },
        );
    }
    for dir in [0, 4, 6] {
        face(&mut engine, dir);
        for (entity, socket) in
            [("tank.turret", "t0"), ("tank.crate", "c0"), ("tank.track_l", "l0")]
        {
            let e = engine.names[entity];
            let p = engine.world.get::<&Transform>(e).unwrap().position;
            let alt = engine.world.get::<&IsoSprite>(e).unwrap().frame_offset.z;
            let anchor = engine.iso_to_world(p.x, p.y, alt).unwrap();
            let sock = engine.vehicle_socket_pos("tank", socket).unwrap();
            assert!((anchor - sock).length() < 1e-3, "dir {dir} {entity}: {anchor} vs {sock}");
        }
    }
}
