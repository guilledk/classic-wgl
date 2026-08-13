pub mod camera;
pub mod collision;
pub mod components;
pub mod instrument;
pub mod math;
pub mod pathfinder;
pub mod registry;
pub mod sdf_builder;
pub mod serde_base64;
pub mod terrain;
pub mod tilemap;
pub mod types;

pub mod gjk;
pub mod quadtree;
pub mod simplex_noise;

use components::{
    Animator, IsoAgent, IsoSprite, LightState, NavMesh, RectRender, SdfTextRender, Tilemap,
};

pub use camera::Camera;
pub use components::{SpriteRender, Transform};
pub use types::Rect;

/// Call once at startup to register all known component types.
pub fn register_all_components() {
    use registry::ComponentReg;

    // Transform — emitted last; subsumed by components that embed position.
    registry::register(ComponentReg {
        name: "Transform",
        spawn: |b, v| {
            let tf: Transform = serde_json::from_value(v)?;
            b.add(tf);
            Ok(())
        },
        dump: Some(dumper_transform),
        order: 50,
        subsumes: &[],
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

    registry::register(ComponentReg {
        name: "Rect",
        spawn: |b, v| {
            let r: RectRender = serde_json::from_value(v)?;
            b.add(r);
            Ok(())
        },
        dump: Some(dumper_rect),
        order: 45,
        subsumes: &[],
    });

    registry::register(ComponentReg {
        name: "SdfText",
        spawn: |b, v| {
            let t: SdfTextRender = serde_json::from_value(v)?;
            b.add(t);
            Ok(())
        },
        dump: Some(dumper_sdftext),
        order: 46,
        subsumes: &[],
    });

    registry::register(ComponentReg {
        name: "LightState",
        spawn: |b, v| {
            let l: LightState = serde_json::from_value(v)?;
            b.add(l);
            Ok(())
        },
        dump: Some(dumper_lightstate),
        order: 47,
        subsumes: &[],
    });

    registry::register(ComponentReg {
        name: "Camera",
        spawn: |b, v| {
            let c: Camera = serde_json::from_value(v)?;
            b.add(c);
            Ok(())
        },
        dump: Some(dumper_camera),
        order: 48,
        subsumes: &[],
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
    m.insert("data".into(), serde_base64::encode_u32(&tm.data).into());
    m.insert("heightData".into(), serde_base64::encode_f32(&tm.height_data).into());
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
    m.insert("data".into(), serde_base64::encode_u32(&n.data).into());
    Some(serde_json::Value::Object(m))
}

/// Prepend the `"type"` key to a serde-serialized component body.
fn component_value(type_name: &str, body: serde_json::Value) -> serde_json::Value {
    let mut m = serde_json::Map::new();
    m.insert("type".into(), serde_json::Value::String(type_name.into()));
    if let serde_json::Value::Object(obj) = body {
        for (k, v) in obj {
            m.insert(k, v);
        }
    }
    serde_json::Value::Object(m)
}

fn dumper_transform(world: &hecs::World, entity: hecs::Entity) -> Option<serde_json::Value> {
    let tf = world.get::<&Transform>(entity).ok()?;
    serde_json::to_value(&*tf).ok().map(|v| component_value("Transform", v))
}

fn dumper_rect(world: &hecs::World, entity: hecs::Entity) -> Option<serde_json::Value> {
    let r = world.get::<&RectRender>(entity).ok()?;
    serde_json::to_value(&*r).ok().map(|v| component_value("Rect", v))
}

fn dumper_sdftext(world: &hecs::World, entity: hecs::Entity) -> Option<serde_json::Value> {
    let t = world.get::<&SdfTextRender>(entity).ok()?;
    serde_json::to_value(&*t).ok().map(|v| component_value("SdfText", v))
}

fn dumper_lightstate(world: &hecs::World, entity: hecs::Entity) -> Option<serde_json::Value> {
    let l = world.get::<&LightState>(entity).ok()?;
    serde_json::to_value(&*l).ok().map(|v| component_value("LightState", v))
}

fn dumper_camera(world: &hecs::World, entity: hecs::Entity) -> Option<serde_json::Value> {
    let c = world.get::<&Camera>(entity).ok()?;
    serde_json::to_value(&*c).ok().map(|v| component_value("Camera", v))
}
