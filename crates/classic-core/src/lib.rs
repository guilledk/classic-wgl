pub mod camera;
pub mod collision;
pub mod components;
pub mod instrument;
pub mod math;
pub mod pathfinder;
pub mod registry;
pub mod sdf_builder;
pub mod terrain;
pub mod tilemap;
pub mod types;

pub mod gjk;
pub mod quadtree;
pub mod simplex_noise;

use components::{
    Animator, IsoAgent, IsoSprite, IsoVehicle, NavMesh, RectRender, Role, SdfTextRender, Tilemap,
};

pub use camera::Camera;
pub use components::{RoleKind, SpriteRender, Transform};
pub use types::Rect;

/// Install all known component types into the registry.  Idempotent — the
/// first call wins and later calls are no-ops.
pub fn register_all_components() {
    use registry::ComponentReg;

    // Transform — emitted last; subsumed by components that embed position.
    registry::init(vec![
        ComponentReg {
            name: "Transform",
            spawn: |b, v| {
                let tf: Transform = serde_json::from_value(v)?;
                b.add(tf);
                Ok(())
            },
            dump: Some(dumper_transform),
            order: 50,
            subsumes: &[],
        },
        ComponentReg {
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
        },
        ComponentReg {
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
        },
        ComponentReg {
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
        },
        ComponentReg {
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
                    frame_offset: a.frame_offset,
                    footprint: a.footprint.clone(),
                });
                b.add(a);
                Ok(())
            },
            dump: Some(dumper_isoagent),
            order: 40,
            subsumes: &["IsoSprite", "Transform"],
        },
        ComponentReg {
            name: "Animator",
            spawn: |b, v| {
                let a: Animator = serde_json::from_value(v)?;
                b.add(a);
                Ok(())
            },
            dump: Some(dumper_animator),
            order: 35,
            subsumes: &[],
        },
        ComponentReg {
            name: "IsoVehicle",
            spawn: |b, v| {
                let veh: IsoVehicle = serde_json::from_value(v)?;
                b.add(veh);
                Ok(())
            },
            dump: Some(dumper_isovehicle),
            order: 37,
            subsumes: &[],
        },
        ComponentReg {
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
        },
        ComponentReg {
            name: "Rect",
            spawn: |b, v| {
                let r: RectRender = serde_json::from_value(v)?;
                b.add(r);
                Ok(())
            },
            dump: Some(dumper_rect),
            order: 45,
            subsumes: &[],
        },
        ComponentReg {
            name: "SdfText",
            spawn: |b, v| {
                let t: SdfTextRender = serde_json::from_value(v)?;
                b.add(t);
                Ok(())
            },
            dump: Some(dumper_sdftext),
            order: 46,
            subsumes: &[],
        },
        ComponentReg {
            name: "Camera",
            spawn: |b, v| {
                let c: Camera = serde_json::from_value(v)?;
                b.add(c);
                Ok(())
            },
            dump: Some(dumper_camera),
            order: 48,
            subsumes: &[],
        },
        ComponentReg {
            name: "Role",
            spawn: |b, v| {
                let r: Role = serde_json::from_value(v)?;
                b.add(r);
                Ok(())
            },
            dump: Some(dumper_role),
            order: 60,
            subsumes: &[],
        },
    ]);
}

// Dumper helpers

fn dumper_sprite(world: &hecs::World, entity: hecs::Entity) -> Option<serde_json::Value> {
    let s = world.get::<&SpriteRender>(entity).ok()?;
    serde_json::to_value(&*s).ok().map(|v| component_value("Sprite", v))
}

fn dumper_tilemap(world: &hecs::World, entity: hecs::Entity) -> Option<serde_json::Value> {
    let tm = world.get::<&Tilemap>(entity).ok()?;
    serde_json::to_value(&*tm).ok().map(|v| component_value("Tilemap", v))
}

fn dumper_isosprite(world: &hecs::World, entity: hecs::Entity) -> Option<serde_json::Value> {
    let s = world.get::<&IsoSprite>(entity).ok()?;
    serde_json::to_value(&*s).ok().map(|v| component_value("IsoSprite", v))
}

fn dumper_isoagent(world: &hecs::World, entity: hecs::Entity) -> Option<serde_json::Value> {
    let a = world.get::<&IsoAgent>(entity).ok()?;
    serde_json::to_value(&*a).ok().map(|v| component_value("IsoAgent", v))
}

fn dumper_animator(world: &hecs::World, entity: hecs::Entity) -> Option<serde_json::Value> {
    let a = world.get::<&Animator>(entity).ok()?;
    serde_json::to_value(&*a).ok().map(|v| component_value("Animator", v))
}

fn dumper_isovehicle(world: &hecs::World, entity: hecs::Entity) -> Option<serde_json::Value> {
    let v = world.get::<&IsoVehicle>(entity).ok()?;
    serde_json::to_value(&*v).ok().map(|v| component_value("IsoVehicle", v))
}

fn dumper_navmesh(world: &hecs::World, entity: hecs::Entity) -> Option<serde_json::Value> {
    let n = world.get::<&NavMesh>(entity).ok()?;
    serde_json::to_value(&*n).ok().map(|v| component_value("IsometricNavMesh", v))
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

fn dumper_camera(world: &hecs::World, entity: hecs::Entity) -> Option<serde_json::Value> {
    let c = world.get::<&Camera>(entity).ok()?;
    serde_json::to_value(&*c).ok().map(|v| component_value("Camera", v))
}

fn dumper_role(world: &hecs::World, entity: hecs::Entity) -> Option<serde_json::Value> {
    let r = world.get::<&Role>(entity).ok()?;
    serde_json::to_value(*r).ok().map(|v| component_value("Role", v))
}
