use classic_core::components::{IsoSprite, Role, RoleKind, Transform};
use classic_core::types::FrameTable;
use classic_core::vehicle_def::{AngleAxis, FrameSelect, SpriteAxes};
use glam::{Vec2, Vec3};

use super::*;

/// A frame table of `count` frames named `{texture}_{i}`, each a 10 × 20
/// trimmed rect in a 40 × 40 source cell of one 256² sheet.
fn table(texture: &str, count: u32, axes: Option<SpriteAxes>, interp: bool) -> FrameTable {
    let frames: serde_json::Map<String, serde_json::Value> = (0..count)
        .map(|i| {
            let x = (i % 16) * 12;
            let y = (i / 16) * 22;
            let v = serde_json::json!({"sheet": 0, "rect": [x, y, 10, 20],
                "source_size": [40, 40], "trim_offset": [15, 12]});
            (format!("{texture}_{i}"), v)
        })
        .collect();
    let mut t: FrameTable = serde_json::from_value(serde_json::json!({
        "version": 2, "dir_interp": interp, "axes": axes,
        "sheets": [{"name": format!("{texture}_sheet"), "src": "/res/s.png", "size": [256, 256]}],
        "frames": frames,
    }))
    .unwrap();
    t.precompute_companions();
    t
}

fn box_axes() -> SpriteAxes {
    SpriteAxes {
        directions: 16,
        pitch: Some(AngleAxis::symmetric(5, 20.0)),
        roll: Some(AngleAxis::symmetric(3, 20.0)),
    }
}

fn box_engine(interp: bool) -> Engine {
    let mut e = Engine::new_for_test();
    let axes = box_axes();
    e.frame_tables.insert("box".into(), table("box", axes.frame_count(), Some(axes), interp));
    let sprite = IsoSprite {
        position: Vec3::new(4.0, 4.0, 0.0),
        scale: Vec3::ONE,
        texture: "box".into(),
        tilemap: "tilemap".into(),
        frame: 0.0,
        frame_name: None,
        tile_set_size: Vec2::ONE,
        anchor: Vec2::new(0.5, 0.75),
        frame_offset: Vec3::ZERO,
        footprint: vec![],
        ghost_group: 0,
        color: [1.0; 4],
        dir_blend: None,
        no_ghost: false,
        team_tint: None,
        emissive: None,
    };
    let ent = e.world.spawn((sprite, Transform::new(Vec3::new(4.0, 4.0, 0.0), Vec3::ONE)));
    e.names.insert("box".into(), ent);
    e
}

fn live_frame(e: &Engine, name: &str) -> Option<String> {
    e.world.get::<&IsoSprite>(e.names[name]).unwrap().frame_name.clone()
}

#[test]
fn sprite_frame_wraps_the_packed_lookup() {
    let e = box_engine(false);
    let f = e.sprite_frame("box", "box_17", Vec2::new(0.5, 0.75)).unwrap();
    assert_eq!(f.texture, "box_sheet");
    assert_eq!(f.frame_name.as_deref(), Some("box_17"));
    assert_eq!(f.cell, Vec2::new(40.0, 40.0), "the quad is the source cell");
    assert_eq!(f.content_min, Vec2::new(15.0, 12.0));
    assert_eq!(f.content_size, Vec2::new(10.0, 20.0));
    // The source-cell anchor ratio, translated through the trim and back.
    assert!((f.anchor_px - Vec2::new(20.0, 30.0)).abs().max_element() < 1e-4);
    let FrameRegion::Uv { uv_rect, .. } = f.region else { panic!("packed") };
    // Frame 17: column 1, row 1 of the 12 × 22 grid.
    let want = [12.0 / 256.0, 22.0 / 256.0, 22.0 / 256.0, 42.0 / 256.0];
    assert!(uv_rect.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-6), "{uv_rect:?}");
    assert!(e.sprite_frame("box", "nope", Vec2::ZERO).is_none());
    assert!(e.sprite_frame("ghost", "box_0", Vec2::ZERO).is_none());
}

#[test]
fn pose_of_inverts_frame_index() {
    let axes = box_axes();
    for f in 0..axes.frame_count() {
        let (d, p, r) = axes.pose_of(f);
        assert_eq!(axes.frame_index(d, p, r), f);
    }
    assert_eq!(SpriteAxes::directions(8).pose_of(13), (5, 0, 0), "past the sheet: the last level");
}

#[test]
fn a_posed_sprite_previews_the_frame_sprite_pose_draws() {
    for interp in [false, true] {
        let mut e = box_engine(interp);
        let axes = e.pose_axes("box").expect("a sheet with axes");
        assert_eq!(axes, box_axes());
        for dir in 0..16 {
            for pitch in 0..5 {
                for roll in 0..3 {
                    let pose = Pose { dir, pitch, roll };
                    let parts = e.posed_parts("box", Some(pose));
                    assert_eq!(parts.len(), 1);
                    let (h, p, r) = pose.angles(&axes);
                    assert!(e.sprite_pose("box", h, p, r));
                    let ctx = format!("interp {interp} {pose:?}");
                    assert_eq!(parts[0].frame.frame_name, live_frame(&e, "box"), "{ctx}");
                    assert_eq!(e.current_pose("box"), Some(pose), "{ctx}");
                }
            }
        }
    }
}

#[test]
fn previewing_leaves_the_entity_alone() {
    let mut e = box_engine(true);
    assert!(e.sprite_pose("box", 1.0, 0.1, -0.2));
    let before = live_frame(&e, "box");
    let parts = e.posed_parts("box", Some(Pose { dir: 9, pitch: 4, roll: 0 }));
    assert_eq!(parts[0].frame.frame_name.as_deref(), Some("box_201"), "(4·3+0)·16+9");
    assert_eq!(live_frame(&e, "box"), before);
    // `None`: the frame it draws now.
    assert_eq!(e.posed_parts("box", None)[0].frame.frame_name, before);
}

#[test]
fn an_unposed_sprite_has_no_axes_and_previews_its_frame() {
    let mut e = box_engine(false);
    e.frame_tables.get_mut("box").unwrap().axes = None;
    assert_eq!(e.pose_axes("box"), None);
    assert_eq!(e.current_pose("box"), None);
    e.world.get::<&mut IsoSprite>(e.names["box"]).unwrap().frame_name = Some("box_5".into());
    let parts = e.posed_parts("box", Some(Pose::default()));
    assert_eq!(parts[0].frame.frame_name.as_deref(), Some("box_5"));
    assert!(e.posed_parts("ghost", None).is_empty());
}

/// A layered vehicle: a 16-direction hull tilting 3 × 3 levels and a wheel.
fn cart() -> Engine {
    let mut e = Engine::new_for_test();
    let tm = classic_core::components::Tilemap {
        position: Vec3::ZERO,
        scale: Vec3::ONE,
        size_x: 16,
        size_y: 16,
        tile_set: "tileset".into(),
        tile_pixel_size: [32, 32],
        max_tile: 16,
        tiles_grid: None,
        heights_grid: None,
        data: vec![0; 256],
        height_data: vec![0.0; 17 * 17],
        height_scale: 64.0,
        tile_set_pixel_size: [0, 0],
        tiles_per_row: 0,
        mouse_iso_pos: Vec3::ZERO,
        selection_iso_begin: Vec3::splat(-1.0),
        selection_iso_end: Vec3::splat(-1.0),
        blend: None,
    };
    let tm = e.world.spawn((tm, Role::new(RoleKind::Tilemap)));
    e.names.insert("tilemap".into(), tm);
    let def: classic_core::types::VehicleDef = serde_json::from_value(serde_json::json!({
        "version": 2, "name": "cart", "directions": 16,
        "layers": [
            { "name": "hull", "kind": "hull", "texture": "cartHull",
              "pitch": { "levels": 3, "max_deg": 20.0 }, "roll": { "levels": 3, "max_deg": 20.0 } },
            { "name": "wheel", "kind": "wheel", "texture": "cartWheel", "offset": [1.0, 1.0, 0.3] }
        ]
    }))
    .unwrap();
    e.vehicles.insert("cart".into(), def);
    e.frame_tables.insert("cartHull".into(), table("cartHull", 144, None, true));
    e.frame_tables.insert("cartWheel".into(), table("cartWheel", 16, None, true));
    assert!(e.spawn_vehicle("cart", "cart", 8.0, 8.0));
    e
}

#[test]
fn a_layered_vehicle_steps_its_hull_axes_and_stacks_its_layers() {
    let e = cart();
    let axes = e.pose_axes("cart").unwrap();
    assert_eq!(axes.directions, 16);
    assert_eq!((axes.pitch.unwrap().count(), axes.roll.unwrap().count()), (3, 3));
    // Spawned level, facing East.
    let now = e.current_pose("cart").unwrap();
    assert_eq!(now, Pose { dir: 0, pitch: 1, roll: 1 });
    let parts = e.posed_parts("cart", Some(now));
    assert_eq!(parts[0].frame.frame_name, live_frame(&e, "cart"), "the drawn pose");
    let hull = e.vehicles["cart"].layers[0].clone();
    for (dir, pitch, roll) in [(0, 0, 0), (5, 2, 1), (15, 1, 2), (8, 0, 2)] {
        let parts = e.posed_parts("cart", Some(Pose { dir, pitch, roll }));
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].entity, "cart");
        let sel = FrameSelect { dir, pitch, roll, ..Default::default() };
        let want = format!("cartHull_{}", hull.frame_index(16, &sel));
        assert_eq!(
            parts[0].frame.frame_name.as_deref(),
            Some(want.as_str()),
            "{dir} {pitch} {roll}"
        );
        // The wheel sits off the body's anchor; the hull at it.
        assert_eq!(parts[0].offset, Vec2::ZERO);
        assert!(parts[1].offset.length() > 10.0);
        let wheel = format!("cartWheel_{dir}");
        assert_eq!(parts[1].frame.frame_name.as_deref(), Some(wheel.as_str()));
    }
    // Facing East the wheel (a metre forward and left: North-East of the
    // body, straight up the screen in the iso view) draws above the hull's
    // anchor, and farther from the camera.
    let off = parts[1].offset;
    assert!(off.x.abs() < 0.1 && off.y < 0.0, "{off:?}");
    assert!(parts[1].depth < parts[0].depth);
}
