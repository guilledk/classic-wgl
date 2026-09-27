pub mod abi;
pub mod abi_manifest;
pub mod camera;
pub mod collision;
pub mod components;
pub mod fields;
pub mod instrument;
pub mod inventory;
pub mod math;
pub mod model;
pub mod registry;
pub mod sdf_builder;
pub mod terrain;
pub mod tilemap;
pub mod types;

pub mod gjk;
pub mod quadtree;
pub mod simplex_noise;

use components::{
    Animator, IsoAgent, IsoSprite, IsoVehicle, Model, NavMesh, RectRender, Role, SdfTextRender,
    Selectable, Tilemap,
};
use inventory::Inventory;

pub use camera::Camera;
pub use components::{Light, LightKind, RoleKind, SpriteRender, Transform};
pub use types::Rect;

/// Pathfinding lives in the standalone `classic-pathfinder` crate (shared by
/// the native host, the native worker thread, and the web `pathfinder.wasm`
/// module).  Re-exported here so `classic_core::pathfinder::*` keeps resolving.
pub use classic_pathfinder as pathfinder;

/// Install all known component types into the registry.  Idempotent — the
/// first call wins and later calls are no-ops.
pub fn register_all_components() {
    use registry::{dump_as, ComponentReg};

    // Transform — emitted last; subsumed by components that embed position.
    registry::init(vec![
        ComponentReg {
            name: "Transform",
            spawn: |b, v| {
                let tf: Transform = serde_json::from_value(v)?;
                b.add(tf);
                Ok(())
            },
            dump: Some(dump_as::<Transform>),
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
            dump: Some(dump_as::<SpriteRender>),
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
            dump: Some(dump_as::<Tilemap>),
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
            dump: Some(dump_as::<IsoSprite>),
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
                    frame_name: a.frame_name.clone(),
                    tile_set_size: a.tile_set_size,
                    anchor: a.anchor,
                    frame_offset: a.frame_offset,
                    footprint: a.footprint.clone(),
                    ghost_group: 0,
                    color: [1.0, 1.0, 1.0, 1.0],
                });
                b.add(a);
                Ok(())
            },
            dump: Some(dump_as::<IsoAgent>),
            order: 40,
            subsumes: &["IsoSprite", "Transform"],
        },
        ComponentReg {
            name: "Model",
            spawn: |b, v| {
                let m: Model = serde_json::from_value(v)?;
                b.add(Transform::new(m.position, m.scale));
                b.add(m);
                Ok(())
            },
            dump: Some(dump_as::<Model>),
            order: 33,
            subsumes: &["Transform"],
        },
        ComponentReg {
            name: "Animator",
            spawn: |b, v| {
                let a: Animator = serde_json::from_value(v)?;
                b.add(a);
                Ok(())
            },
            dump: Some(dump_as::<Animator>),
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
            dump: Some(dump_as::<IsoVehicle>),
            order: 37,
            subsumes: &[],
        },
        ComponentReg {
            name: "Inventory",
            spawn: |b, v| {
                let inv: Inventory = serde_json::from_value(v)?;
                b.add(inv);
                Ok(())
            },
            dump: Some(dump_as::<Inventory>),
            order: 38,
            subsumes: &[],
        },
        ComponentReg {
            name: "Selectable",
            spawn: |b, v| {
                let s: Selectable = serde_json::from_value(v)?;
                b.add(s);
                Ok(())
            },
            dump: Some(dump_as::<Selectable>),
            order: 39,
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
            dump: Some(dump_as::<NavMesh>),
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
            dump: Some(dump_as::<RectRender>),
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
            dump: Some(dump_as::<SdfTextRender>),
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
            dump: Some(dump_as::<Camera>),
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
            dump: Some(dump_as::<Role>),
            order: 60,
            subsumes: &[],
        },
        ComponentReg {
            name: "Light",
            spawn: |b, v| {
                let l: Light = serde_json::from_value(v)?;
                b.add(l);
                Ok(())
            },
            dump: Some(dump_as::<Light>),
            order: 59,
            subsumes: &[],
        },
    ]);
}
