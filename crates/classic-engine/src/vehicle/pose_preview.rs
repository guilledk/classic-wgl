//! A vehicle framed at a chosen pose without touching it, for tools (the
//! sprite preview in classic-demo's inspector): every layer of a v2 def is
//! framed and placed through the same [`LayerFrame`] / [`layer_offset`]
//! path [`Engine::apply_layers`] runs each frame, and a v1 body through the
//! simulation's [`v1_body_frame`] and direction blend.

use classic_core::components::{IsoSprite, IsoVehicle};
use glam::{Vec2, Vec3};

use super::kinematics::v1_body_frame;
use super::layers::{hull_drawn_tilt, layer_offset, layer_poses, Chassis, LayerFrame};
use crate::Engine;

/// One part of a vehicle framed at a preview pose: its entity name, the
/// framed sprite (a scratch copy of the live one) and its offset from the
/// body's ground point (tile `x`, `y`; metres `z`).
pub(crate) struct PosedPart {
    pub(crate) entity: String,
    pub(crate) sprite: IsoSprite,
    pub(crate) offset: Vec3,
}

impl Engine {
    /// Every visible layer of the layered (v2) vehicle `ve`, framed and
    /// placed for tile heading `heading` and body tilt `(pitch, roll)`
    /// (radians); the other drivers (wheel spin and steer, turret yaw, clips)
    /// keep their current state.  Empty for a v1 vehicle.
    pub(crate) fn layered_pose(
        &self,
        ve: hecs::Entity,
        heading: f32,
        pitch: f32,
        roll: f32,
    ) -> Vec<PosedPart> {
        let Ok(v) = self.world.get::<&IsoVehicle>(ve) else { return Vec::new() };
        let Some(def) = self.vehicles.get(&v.def_name) else { return Vec::new() };
        let anchors = self.vehicle_anchors.get(&def.anchors);
        let chassis = Chassis { heading, pitch, roll, ..Chassis::of(&v, heading) };
        let hull_tilt = hull_drawn_tilt(&self.frame_tables, def, &chassis);
        let poses = layer_poses(def, &v.layers.iter().map(|s| s.yaw).collect::<Vec<_>>());
        let mut out = Vec::with_capacity(v.layers.len());
        for s in v.layers.iter().filter(|s| s.visible) {
            let Some(l) = def.layers.get(s.def_index) else { continue };
            let Some(&e) = self.names.get(&s.entity) else { continue };
            let Ok(live) = self.world.get::<&IsoSprite>(e) else { continue };
            let mut sprite = IsoSprite::clone(&live);
            let pose = poses[s.def_index];
            let frame = LayerFrame { def, layer: l, state: s, pose, hull_tilt };
            frame.apply(&self.frame_tables, anchors, &chassis, &mut sprite);
            let offset = layer_offset(pose, &chassis);
            out.push(PosedPart { entity: s.entity.clone(), sprite, offset });
        }
        out
    }

    /// The body sprite of the v1 (sheet) vehicle `ve` framed at direction
    /// `dir` and tilt levels `(pitch, roll)`: the frame the simulation writes
    /// for that pose, the nearer direction of its blend on a `dir_interp`
    /// sheet.  `None` for a layered vehicle.
    pub(crate) fn v1_body_pose(
        &self,
        ve: hecs::Entity,
        dir: u32,
        pitch: u32,
        roll: u32,
    ) -> Option<IsoSprite> {
        let v = self.world.get::<&IsoVehicle>(ve).ok()?;
        if !v.layers.is_empty() {
            return None;
        }
        let dirs = v.directions.max(1);
        let (dir, pitch, roll) =
            (dir % dirs, pitch.min(v.pitch_levels.max(1) - 1), roll.min(v.roll_levels.max(1) - 1));
        let mut s = IsoSprite::clone(&*self.world.get::<&IsoSprite>(ve).ok()?);
        let frame = v1_body_frame(&v, dir, pitch, roll);
        s.frame = frame as f32;
        s.frame_name = Self::frame_name(&self.frame_tables, &s.texture, s.frame);
        if let Some(a) = v.body_anchors.get(dir as usize) {
            s.anchor = Vec2::from(*a);
        }
        s.dir_blend = None;
        Some(s)
    }
}

#[cfg(test)]
mod tests {
    use classic_core::components::{IsoSprite, IsoVehicle, Role, RoleKind, Tilemap, Transform};
    use classic_core::tilemap::TILE_M;
    use classic_core::types::FrameTable;

    use crate::Engine;

    fn table(dir_interp: bool, tilt_interp: bool) -> FrameTable {
        FrameTable {
            version: 2,
            sheets: vec![],
            frames: Default::default(),
            dir_interp,
            axes: None,
            tilt_interp,
            companions: vec![],
        }
    }

    /// A map rising `deg` toward East, under a vehicle def'd by `setup`,
    /// spawned facing `dir` and settled for three seconds.
    fn settle_on_ramp(deg: f32, dir: u32, name: &str, setup: impl FnOnce(&mut Engine)) -> Engine {
        let mut engine = Engine::new_for_test();
        let mut tm: Tilemap = super::super::tests::flat_tilemap_pub(24, 24);
        let rise = deg.to_radians().tan() * TILE_M;
        tm.height_data = (0..25 * 25).map(|i| (i % 25) as f32 * rise).collect();
        let tm = engine.world.spawn((tm, Role::new(RoleKind::Tilemap)));
        engine.names.insert("tilemap".into(), tm);
        setup(&mut engine);
        assert!(engine.spawn_vehicle(name, name, 12.0, 12.0));
        engine.world.get::<&mut IsoVehicle>(engine.names[name]).unwrap().direction = dir;
        assert!(engine.vehicle_teleport(name, 12.0, 12.0));
        for _ in 0..90 {
            engine.time.delta = 1.0 / 30.0;
            engine.update_vehicles();
        }
        engine
    }

    fn rover_on_ramp(deg: f32, dir: u32) -> Engine {
        settle_on_ramp(deg, dir, "rover", |e| {
            e.vehicles.insert("rover".into(), super::super::layer_tests::rover_def());
            for (t, tilt) in [("roverHull", true), ("roverTool", false), ("roverWheel", false)] {
                e.frame_tables.insert(t.into(), table(true, tilt));
            }
        })
    }

    #[test]
    fn a_layered_pose_frames_and_places_every_layer_as_the_simulation_does() {
        for (deg, dir) in [(0.0, 0), (7.0, 3), (13.7, 4), (17.0, 9), (10.0, 12)] {
            let e = rover_on_ramp(deg, dir);
            let ve = e.names["rover"];
            let v = IsoVehicle::clone(&e.world.get::<&IsoVehicle>(ve).unwrap());
            let body = e.world.get::<&Transform>(ve).unwrap().position;
            let parts = e.layered_pose(ve, v.heading, v.pitch, v.roll);
            assert_eq!(parts.len(), 6, "every visible layer");
            for p in &parts {
                let pe = e.names[&p.entity];
                let live = IsoSprite::clone(&e.world.get::<&IsoSprite>(pe).unwrap());
                let ctx = format!("{deg}° dir {dir}: {}", p.entity);
                assert_eq!(p.sprite.frame_name, live.frame_name, "{ctx}");
                assert_eq!(p.sprite.frame, live.frame, "{ctx}");
                assert_eq!(p.sprite.anchor, live.anchor, "{ctx}");
                assert_eq!(p.sprite.dir_blend, live.dir_blend, "{ctx}");
                let at = e.world.get::<&Transform>(pe).unwrap().position;
                assert!((body + p.offset - at).abs().max_element() < 1e-5, "{ctx}");
            }
            // The live entities are untouched.
            let after = IsoVehicle::clone(&e.world.get::<&IsoVehicle>(ve).unwrap());
            assert_eq!(after.heading, v.heading);
        }
    }

    #[test]
    fn a_v1_body_pose_is_the_frame_the_simulation_writes() {
        let mut tilted = 0;
        for (deg, dir) in [(0.0, 0), (15.0, 0), (15.0, 2), (18.0, 4), (18.0, 7)] {
            let e = settle_on_ramp(deg, dir, "lrv", |e| {
                let (mut def, anchors) = super::super::tests::lrv_def(720.0);
                (def.pitch_levels, def.roll_levels) = (5, 3);
                super::super::tests::insert_lrv(e, (def, anchors));
                e.frame_tables.insert("lrvBody".into(), table(false, false));
            });
            let ve = e.names["lrv"];
            let v = IsoVehicle::clone(&e.world.get::<&IsoVehicle>(ve).unwrap());
            let live = IsoSprite::clone(&e.world.get::<&IsoSprite>(ve).unwrap());
            let s = e.v1_body_pose(ve, v.direction, v.pitch_index, v.roll_index).unwrap();
            assert_eq!(s.frame_name, live.frame_name, "{deg}° dir {dir}");
            assert_eq!(s.frame, live.frame);
            tilted += usize::from(v.pitch_index != 2 || v.roll_index != 1);
        }
        assert!(tilted >= 1, "a ramp tilts the body");
    }
}
