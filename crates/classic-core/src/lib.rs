pub mod camera;
pub mod collision;
pub mod components;
pub mod instrument;
pub mod math;
pub mod pathfinder;
pub mod registry;
pub mod sdf_builder;
pub mod tilemap;
pub mod types;

pub mod gjk;
pub mod quadtree;
pub mod simplex_noise;

use components::{Animator, IsoAgent, IsoSprite, NavMesh, Tilemap};

pub use camera::Camera;
pub use components::{SpriteRender, Transform};
pub use types::Rect;

/// Call once at startup to register all known component types.
pub fn register_all_components() {
    use registry::ComponentReg;

    // Transform — bare spawner, no dumper (subsumed by all other components).
    registry::register_spawner("Transform", |b, v| {
        let tf: Transform = serde_json::from_value(v)?;
        b.add(tf);
        Ok(())
    });

    registry::register(ComponentReg {
        name: "Sprite",
        spawn: |b, v| {
            let s: SpriteRender = serde_json::from_value(v)?;
            b.add(Transform::new(s.position, s.scale));
            b.add(s);
            Ok(())
        },
        dump: Some(dumper_sprite),
        order: 20,
        subsumes: &["Transform"],
    });

    registry::register(ComponentReg {
        name: "Tilemap",
        spawn: |b, v| {
            let tm: Tilemap = serde_json::from_value(v)?;
            b.add(Transform::new(tm.position, tm.scale));
            b.add(tm);
            Ok(())
        },
        dump: Some(dumper_tilemap),
        order: 10,
        subsumes: &["Transform"],
    });

    registry::register(ComponentReg {
        name: "IsoSprite",
        spawn: |b, v| {
            let s: IsoSprite = serde_json::from_value(v)?;
            b.add(Transform::new(s.position, s.scale));
            b.add(s);
            Ok(())
        },
        dump: Some(dumper_isosprite),
        order: 30,
        subsumes: &["Transform"],
    });

    registry::register(ComponentReg {
        name: "IsoAgent",
        spawn: |b, v| {
            let a: IsoAgent = serde_json::from_value(v)?;
            b.add(Transform::new(a.position, a.scale));
            b.add(IsoSprite {
                position: a.position,
                scale: a.scale,
                texture: a.texture.clone(),
                tilemap: a.tilemap.clone(),
                frame: a.frame,
                tile_set_size: a.tile_set_size,
                anchor: a.anchor,
                footprint: a.footprint.clone(),
            });
            b.add(a);
            Ok(())
        },
        dump: Some(dumper_isoagent),
        order: 40,
        subsumes: &["IsoSprite", "Transform"],
    });

    registry::register(ComponentReg {
        name: "Animator",
        spawn: |b, v| {
            let a: Animator = serde_json::from_value(v)?;
            b.add(a);
            Ok(())
        },
        dump: Some(dumper_animator),
        order: 35,
        subsumes: &[],
    });

    registry::register(ComponentReg {
        name: "IsometricNavMesh",
        spawn: |b, v| {
            let n: NavMesh = serde_json::from_value(v)?;
            b.add(Transform::new(n.position, n.scale));
            b.add(n);
            Ok(())
        },
        dump: Some(dumper_navmesh),
        order: 15,
        subsumes: &["Transform"],
    });
}

// Dumper helpers

fn dumper_sprite(world: &hecs::World, entity: hecs::Entity) -> Option<serde_json::Value> {
    let s = world.get::<&SpriteRender>(entity).ok()?;
    let mut m = serde_json::Map::new();
    m.insert("type".into(), serde_json::Value::String("Sprite".into()));
    m.insert("position".into(), serde_json::json!([s.position.x, s.position.y, s.position.z]));
    m.insert("scale".into(), serde_json::json!([s.scale.x, s.scale.y, s.scale.z]));
    m.insert("texture".into(), s.texture.as_str().into());
    m.insert("ignoreCam".into(), s.ignore_cam.into());
    m.insert("frame".into(), s.frame.into());
    m.insert("tileSetSize".into(), serde_json::json!([s.tile_set_size.x, s.tile_set_size.y]));
    m.insert("anchor".into(), serde_json::json!([s.anchor.x, s.anchor.y]));
    Some(serde_json::Value::Object(m))
}

fn dumper_tilemap(world: &hecs::World, entity: hecs::Entity) -> Option<serde_json::Value> {
    let tm = world.get::<&Tilemap>(entity).ok()?;
    let mut m = serde_json::Map::new();
    m.insert("type".into(), serde_json::Value::String("Tilemap".into()));
    m.insert("position".into(), serde_json::json!([tm.position.x, tm.position.y, tm.position.z]));
    m.insert("scale".into(), serde_json::json!([tm.scale.x, tm.scale.y, tm.scale.z]));
    m.insert("sizeX".into(), tm.size_x.into());
    m.insert("sizeY".into(), tm.size_y.into());
    m.insert("tileSet".into(), tm.tile_set.as_str().into());
    m.insert(
        "tilePixelSize".into(),
        serde_json::json!([tm.tile_pixel_size[0], tm.tile_pixel_size[1]]),
    );
    m.insert("maxTile".into(), tm.max_tile.into());
    m.insert("data".into(), tm.data_url.as_deref().unwrap_or("").into());
    // heightScale is only emitted if non-zero (TS parity: it's optional)
    if tm.height_scale != 0.0 {
        m.insert("heightScale".into(), tm.height_scale.into());
    }
    Some(serde_json::Value::Object(m))
}

fn dumper_isosprite(world: &hecs::World, entity: hecs::Entity) -> Option<serde_json::Value> {
    let s = world.get::<&IsoSprite>(entity).ok()?;
    let mut m = serde_json::Map::new();
    m.insert("type".into(), serde_json::Value::String("IsoSprite".into()));
    m.insert("position".into(), serde_json::json!([s.position.x, s.position.y, s.position.z]));
    m.insert("scale".into(), serde_json::json!([s.scale.x, s.scale.y, s.scale.z]));
    m.insert("texture".into(), s.texture.as_str().into());
    m.insert("tilemap".into(), s.tilemap.as_str().into());
    m.insert("frame".into(), s.frame.into());
    m.insert("tileSetSize".into(), serde_json::json!([s.tile_set_size.x, s.tile_set_size.y]));
    m.insert("anchor".into(), serde_json::json!([s.anchor.x, s.anchor.y]));
    m.insert(
        "footprint".into(),
        serde_json::json!(s.footprint.iter().map(|v| [v.x, v.y]).collect::<Vec<[f32; 2]>>()),
    );
    Some(serde_json::Value::Object(m))
}

fn dumper_isoagent(world: &hecs::World, entity: hecs::Entity) -> Option<serde_json::Value> {
    let a = world.get::<&IsoAgent>(entity).ok()?;
    let mut m = serde_json::Map::new();
    m.insert("type".into(), serde_json::Value::String("IsoAgent".into()));
    m.insert("position".into(), serde_json::json!([a.position.x, a.position.y, a.position.z]));
    m.insert("scale".into(), serde_json::json!([a.scale.x, a.scale.y, a.scale.z]));
    m.insert("texture".into(), a.texture.as_str().into());
    m.insert("tilemap".into(), a.tilemap.as_str().into());
    m.insert("frame".into(), a.frame.into());
    m.insert("tileSetSize".into(), serde_json::json!([a.tile_set_size.x, a.tile_set_size.y]));
    m.insert("anchor".into(), serde_json::json!([a.anchor.x, a.anchor.y]));
    m.insert(
        "footprint".into(),
        serde_json::json!(a.footprint.iter().map(|v| [v.x, v.y]).collect::<Vec<[f32; 2]>>()),
    );
    m.insert("speed".into(), a.speed.into());
    m.insert("animSpeed".into(), a.anim_speed.into());
    Some(serde_json::Value::Object(m))
}

fn dumper_animator(world: &hecs::World, entity: hecs::Entity) -> Option<serde_json::Value> {
    let a = world.get::<&Animator>(entity).ok()?;
    let mut m = serde_json::Map::new();
    m.insert("type".into(), serde_json::Value::String("Animator".into()));
    m.insert("target".into(), a.target.as_str().into());
    m.insert("speed".into(), a.speed.into());
    Some(serde_json::Value::Object(m))
}

fn dumper_navmesh(world: &hecs::World, entity: hecs::Entity) -> Option<serde_json::Value> {
    let n = world.get::<&NavMesh>(entity).ok()?;
    let mut m = serde_json::Map::new();
    m.insert("type".into(), serde_json::Value::String("IsometricNavMesh".into()));
    m.insert("map".into(), n.map_entity.as_str().into());
    m.insert("sizeX".into(), n.size_x.into());
    m.insert("sizeY".into(), n.size_y.into());
    m.insert("data".into(), n.data_url.as_deref().unwrap_or("").into());
    Some(serde_json::Value::Object(m))
}
