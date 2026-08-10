use classic_core::components::{Animator, IsoAgent, IsoSprite, NavMesh, Tilemap};
use classic_core::{register_all_components, SpriteRender, Transform};

#[test]
fn sprite_dumper_round_trip_keys() {
    classic_core::registry::clear();
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
    assert_eq!(val["ignoreCam"], true);
    assert_eq!(val["tileSetSize"], serde_json::json!([1.0, 1.0]));
}

#[test]
fn tilemap_dumper_round_trip_keys() {
    classic_core::registry::clear();
    register_all_components();

    let tm = Tilemap {
        position: [1.0, 2.0, 3.0].into(),
        scale: [45.0, 45.0, 1.0].into(),
        size_x: 200,
        size_y: 200,
        tile_set: "tileSet".into(),
        tile_pixel_size: [32, 32],
        max_tile: 32,
        data_url: Some("map001.txt".into()),
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
    assert_eq!(val["sizeX"], 200);
    assert_eq!(val["sizeY"], 200);
    assert_eq!(val["tileSet"], "tileSet");
    assert_eq!(val["tilePixelSize"], serde_json::json!([32, 32]));
    assert_eq!(val["maxTile"], 32);
    assert_eq!(val["data"], "map001.txt");
    assert_eq!(val["heightScale"], 32.0);

    // height_data should NOT be in the dump (it's a sidecar)
    assert!(val.get("heightData").is_none());
}

#[test]
fn isoagent_subsume_skips_transform_and_isosprite() {
    classic_core::registry::clear();
    register_all_components();

    let agent = IsoAgent {
        position: [12.5, 12.5, 0.0].into(),
        scale: [1.0, 1.0, 1.0].into(),
        texture: "humanoid".into(),
        tilemap: "tilemap".into(),
        frame: 333.0,
        tile_set_size: [32.0, 16.0].into(),
        anchor: [0.5, 0.98].into(),
        footprint: vec![[0.5, -0.5].into(), [0.5, 0.5].into()],
        speed: 2.5,
        anim_speed: 1.0,
        anim_prefix: "".into(),
        path: vec![],
        target_index: 1,
        delta: 0.0,
        init_dist: 0.0,
        direction: 0.0,
        anim_index: 2,
        state: classic_core::components::AgentState::Idle,
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
    assert_eq!(val["animSpeed"], 1.0);

    // "type" must be the first key (critical for TS positional loader)
    let keys: Vec<&str> = val.as_object().unwrap().keys().map(|s| s.as_str()).collect();
    assert_eq!(keys[0], "type");
}

#[test]
fn animator_dumper_works() {
    classic_core::registry::clear();
    register_all_components();

    let anim = Animator {
        target: "navAgent.IsoAgent".into(),
        speed: 1.0,
        animation: None,
        counter: 0.0,
        frame: 0.0,
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
fn navmesh_dumper_uses_map_key() {
    classic_core::registry::clear();
    register_all_components();

    let nav = NavMesh {
        position: [0.0, 0.0, 0.0].into(),
        scale: [45.0, 45.0, 1.0].into(),
        map_entity: "tilemap".into(),
        tile_set: "".into(),
        data: vec![0, 1, 1, 0],
        data_url: Some("map001.nav.txt".into()),
        size_x: 200,
        size_y: 200,
    };

    let mut world = hecs::World::new();
    let e = world.spawn((Transform::new(nav.position, nav.scale), nav));

    let regs = classic_core::registry::ordered_regs();
    let nav_reg = regs.iter().find(|r| r.name == "IsometricNavMesh").unwrap();
    let val = nav_reg.dump.unwrap()(&world, e).unwrap();

    assert_eq!(val["type"], "IsometricNavMesh");
    assert_eq!(val["map"], "tilemap");
    assert_eq!(val["data"], "map001.nav.txt");
    // NavMesh dumper should NOT emit position/scale (matches TS)
    assert!(val.get("position").is_none());
    assert!(val.get("scale").is_none());
}
