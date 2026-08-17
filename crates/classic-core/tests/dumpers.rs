use classic_core::components::{Animator, IsoAgent, IsoSprite, NavMesh, Tilemap};
use classic_core::{register_all_components, SpriteRender, Transform};

#[test]
fn sprite_dumper_round_trip_keys() {
    register_all_components();

    let s = SpriteRender {
        position: [0.0, 0.0, -20000.0].into(),
        scale: [1.0, 1.0, 1.0].into(),
        texture: "cursor".into(),
        ignore_cam: true,
        frame: 0.0,
        tile_set_size: [1.0, 1.0].into(),
        anchor: [0.5, 0.5].into(),
    };

    let mut world = hecs::World::new();
    let e = world.spawn((Transform::new(s.position, s.scale), s));

    let regs = classic_core::registry::ordered_regs();
    let sprite_reg = regs.iter().find(|r| r.name == "Sprite").unwrap();
    let val = sprite_reg.dump.unwrap()(&world, e).unwrap();

    assert_eq!(val["type"], "Sprite");
    assert_eq!(val["texture"], "cursor");
    assert_eq!(val["ignore_cam"], true);
    assert_eq!(val["tile_set_size"], serde_json::json!([1.0, 1.0]));
}

#[test]
fn tilemap_dumper_round_trip_keys() {
    register_all_components();

    let tm = Tilemap {
        position: [1.0, 2.0, 3.0].into(),
        scale: [45.0, 45.0, 1.0].into(),
        size_x: 200,
        size_y: 200,
        tile_set: "tileSet".into(),
        tile_pixel_size: [32, 32],
        max_tile: 32,
        tiles_grid: None,
        heights_grid: None,
        data: vec![1, 2, 3],
        height_data: vec![1.0; 9],
        height_scale: 32.0,
        tile_set_pixel_size: [0, 0],
        tiles_per_row: 0,
        mouse_iso_pos: [0.0, 0.0, 0.0].into(),
        selection_iso_begin: [-1.0, -1.0, -1.0].into(),
        selection_iso_end: [-1.0, -1.0, -1.0].into(),
    };

    let mut world = hecs::World::new();
    let e = world.spawn((Transform::new(tm.position, tm.scale), tm.clone()));

    // Find the Tilemap dumper from the registry
    let regs = classic_core::registry::ordered_regs();
    let tilemap_reg = regs.iter().find(|r| r.name == "Tilemap").unwrap();
    let val = tilemap_reg.dump.unwrap()(&world, e).unwrap();

    assert_eq!(val["type"], "Tilemap");
    assert_eq!(val["position"], serde_json::json!([1.0, 2.0, 3.0]));
    assert_eq!(val["scale"], serde_json::json!([45.0, 45.0, 1.0]));
    assert_eq!(val["size_x"], 200);
    assert_eq!(val["size_y"], 200);
    assert_eq!(val["tile_set"], "tileSet");
    assert_eq!(val["tile_pixel_size"], serde_json::json!([32, 32]));
    assert_eq!(val["max_tile"], 32);
    assert_eq!(val["height_scale"], 32.0);

    // Bulk grids are persisted as named ROM resources, not inlined in state.
    assert!(val.get("data").is_none());
    assert!(val.get("height_data").is_none());
    assert!(val.get("tiles_grid").is_none());
    assert!(val.get("heights_grid").is_none());
}

#[test]
fn isoagent_subsume_skips_transform_and_isosprite() {
    register_all_components();

    let agent = IsoAgent {
        position: [12.5, 12.5, 0.0].into(),
        scale: [1.0, 1.0, 1.0].into(),
        texture: "humanoid".into(),
        tilemap: "tilemap".into(),
        frame: 333.0,
        tile_set_size: [32.0, 16.0].into(),
        anchor: [0.5, 0.98].into(),
        frame_offset: glam::Vec3::ZERO,
        footprint: vec![[0.5, -0.5].into(), [0.5, 0.5].into()],
        speed: 2.5,
        anim_speed: 1.0,
        anim_prefix: "".into(),
    };

    let mut world = hecs::World::new();
    let e = world.spawn((
        Transform::new(agent.position, agent.scale),
        IsoSprite {
            position: agent.position,
            scale: agent.scale,
            texture: agent.texture.clone(),
            tilemap: agent.tilemap.clone(),
            frame: agent.frame,
            tile_set_size: agent.tile_set_size,
            anchor: agent.anchor,
            frame_offset: glam::Vec3::ZERO,
            footprint: agent.footprint.clone(),
        },
        agent.clone(),
    ));

    let regs = classic_core::registry::ordered_regs();
    let agent_reg = regs.iter().find(|r| r.name == "IsoAgent").unwrap();
    let val = agent_reg.dump.unwrap()(&world, e).unwrap();

    assert_eq!(val["type"], "IsoAgent");
    assert_eq!(val["texture"], "humanoid");
    assert_eq!(val["tilemap"], "tilemap");
    assert_eq!(val["speed"], 2.5);
    assert_eq!(val["anim_speed"], 1.0);

    // "type" is emitted first by `component_value`.
    let keys: Vec<&str> = val.as_object().unwrap().keys().map(|s| s.as_str()).collect();
    assert_eq!(keys[0], "type");
}

#[test]
fn animator_dumper_works() {
    register_all_components();

    let anim = Animator {
        target: "navAgent.IsoAgent".into(),
        speed: 1.0,
        animation: None,
        counter: 0.0,
        frame: 0.0,
        offset: glam::Vec3::ZERO,
        repeat: true,
        playing: true,
    };

    let mut world = hecs::World::new();
    let e = world.spawn((anim,));

    let regs = classic_core::registry::ordered_regs();
    let anim_reg = regs.iter().find(|r| r.name == "Animator").unwrap();
    let val = anim_reg.dump.unwrap()(&world, e).unwrap();

    assert_eq!(val["type"], "Animator");
    assert_eq!(val["target"], "navAgent.IsoAgent");
    assert_eq!(val["speed"], 1.0);
}

#[test]
fn animator_state_round_trips() {
    register_all_components();

    // A ROM can declare a starting one-shot animation directly in `state.json`.
    let anim = Animator {
        target: "rocket.IsoSprite".into(),
        speed: 1.0,
        animation: Some("rocketLanding".into()),
        counter: 0.0,
        frame: 0.0,
        offset: glam::Vec3::ZERO,
        repeat: false,
        playing: true,
    };

    let mut world = hecs::World::new();
    let e = world.spawn((anim,));

    let regs = classic_core::registry::ordered_regs();
    let anim_reg = regs.iter().find(|r| r.name == "Animator").unwrap();
    let val = anim_reg.dump.unwrap()(&world, e).unwrap();

    assert_eq!(val["animation"], "rocketLanding");
    assert_eq!(val["repeat"], false);
    assert_eq!(val["playing"], true);

    // Spawn it back from the dumped value (minus the "type" key).
    let mut fields = val.as_object().unwrap().clone();
    fields.remove("type");
    let mut builder = hecs::EntityBuilder::new();
    (anim_reg.spawn)(&mut builder, serde_json::Value::Object(fields)).unwrap();
    let spawned = world.spawn(builder.build());
    let round_tripped = world.get::<&Animator>(spawned).unwrap();
    assert_eq!(round_tripped.animation.as_deref(), Some("rocketLanding"));
    assert!(!round_tripped.repeat);
    assert!(round_tripped.playing);
}

#[test]
fn animation_offsets_default_empty() {
    let animation: classic_core::types::AnimationData = serde_json::from_value(serde_json::json!({
        "name": "rocketLanding",
        "src": "rocketLanding",
        "rate": 24.0,
        "sequence": [0, 1, 2]
    }))
    .unwrap();

    assert!(animation.offsets.is_empty());
}

#[test]
fn navmesh_dumper_uses_map_entity_key() {
    register_all_components();

    let nav = NavMesh {
        position: [0.0, 0.0, 0.0].into(),
        scale: [45.0, 45.0, 1.0].into(),
        map_entity: "tilemap".into(),
        tile_set: "".into(),
        data_grid: None,
        data: vec![0, 1, 1, 0],
        size_x: 200,
        size_y: 200,
    };

    let mut world = hecs::World::new();
    let e = world.spawn((Transform::new(nav.position, nav.scale), nav));

    let regs = classic_core::registry::ordered_regs();
    let nav_reg = regs.iter().find(|r| r.name == "IsometricNavMesh").unwrap();
    let val = nav_reg.dump.unwrap()(&world, e).unwrap();

    assert_eq!(val["type"], "IsometricNavMesh");
    assert_eq!(val["map_entity"], "tilemap");
    assert!(val.get("data").is_none());
    assert!(val.get("data_grid").is_none());
    assert_eq!(val["position"], serde_json::json!([0.0, 0.0, 0.0]));
    assert_eq!(val["scale"], serde_json::json!([45.0, 45.0, 1.0]));
}

#[test]
fn rect_and_transform_dumpers_round_trip() {
    register_all_components();

    let rect =
        classic_core::components::RectRender { color: [1.0, 0.0, 0.0, 1.0], ignore_cam: true };
    let mut world = hecs::World::new();
    let e = world.spawn((Transform::new([10.0, 20.0, 0.0].into(), [2.0, 3.0, 1.0].into()), rect));

    let regs = classic_core::registry::ordered_regs();
    let rect_val =
        regs.iter().find(|r| r.name == "Rect").unwrap().dump.unwrap()(&world, e).unwrap();
    assert_eq!(rect_val["type"], "Rect");
    assert_eq!(rect_val["color"], serde_json::json!([1.0, 0.0, 0.0, 1.0]));
    assert_eq!(rect_val["ignore_cam"], true);

    let tf_val =
        regs.iter().find(|r| r.name == "Transform").unwrap().dump.unwrap()(&world, e).unwrap();
    assert_eq!(tf_val["type"], "Transform");
    assert_eq!(tf_val["position"], serde_json::json!([10.0, 20.0, 0.0]));
}

#[test]
fn camera_dumper() {
    register_all_components();

    let cam = classic_core::Camera::new([5.0, 6.0, 0.0].into(), [0.5, 0.5, 1.0].into());

    let mut world = hecs::World::new();
    let ce = world.spawn((cam,));

    let regs = classic_core::registry::ordered_regs();

    let cam_val =
        regs.iter().find(|r| r.name == "Camera").unwrap().dump.unwrap()(&world, ce).unwrap();
    assert_eq!(cam_val["type"], "Camera");
    assert_eq!(cam_val["position"], serde_json::json!([5.0, 6.0, 0.0]));
    assert_eq!(cam_val["scale"], serde_json::json!([0.5, 0.5, 1.0]));
    // `size` is runtime-derived and must not be serialized.
    assert!(cam_val.get("size").is_none());
}

#[test]
fn role_dumper_round_trips() {
    register_all_components();

    let role = classic_core::components::Role::new(classic_core::RoleKind::Tilemap);
    let mut world = hecs::World::new();
    let e = world.spawn((role,));

    let regs = classic_core::registry::ordered_regs();
    let role_reg = regs.iter().find(|r| r.name == "Role").unwrap();
    let val = role_reg.dump.unwrap()(&world, e).unwrap();
    assert_eq!(val["type"], "Role");
    assert_eq!(val["value"], "tilemap");

    // Spawn it back from the dumped value.
    let mut builder = hecs::EntityBuilder::new();
    let fields = serde_json::json!({ "value": "tilemap" });
    (role_reg.spawn)(&mut builder, fields).unwrap();
    let spawned = world.spawn(builder.build());
    assert_eq!(
        world.get::<&classic_core::components::Role>(spawned).unwrap().value,
        classic_core::RoleKind::Tilemap
    );
}
