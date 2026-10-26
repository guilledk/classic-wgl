//! Vehicle definition v2 runtime: layered units (`classic_core::vehicle_def`).
//!
//! A v2 def spawns one sprite entity per layer (the hull is the body entity,
//! which carries `IsoVehicle`); the chassis simulation (path following,
//! contacts, suspension plane, pitch / roll) is the v1 one, fed with four
//! virtual ground contacts taken from the wheel / gear layers.  Each frame
//! [`Engine::apply_layers`] places every layer from its chassis offset (and
//! its parent's, e.g. a tool on a turret), and picks its frame from its axes:
//!
//! - hull: direction × body pitch × roll;
//! - wheel / gear side: direction × spin (from the odometer and the turn
//!   differential, `distance / (2π · radius_m)` revolutions) × steer;
//! - turret: its own slewed yaw over its own directions × gun pitch × fire;
//! - tool / legs / cargo / lift: direction × the playing clip frame, and
//!   (when the layer declares them) the hull's drawn pitch / roll turned
//!   into the layer's yaw, so carried cargo leans with its carrier.
//!
//! Direction interpolation (`dir_interp` sheets) applies to every layer.

use classic_core::components::{
    ClipPlay, DebugName, DirBlend, DirThird, IsoSprite, IsoVehicle, RoleKind, Selectable,
    Transform, VehicleLayerState,
};
use classic_core::tilemap::{PPM_TARGET, TILE_M};
use classic_core::types::{FrameTable, VehicleDef};
use classic_core::vehicle_def::{
    heading_neighbours, heading_to_dir_n, pick_tilt, third_tilt, tilt_for_yaw, AngleAxis, ClipDef,
    FrameSelect, GhostGroupMode, LayerKind, TiltPick, VehicleLayerDef, DEFAULT_CLIP_FPS,
};
use glam::{Vec2, Vec3};
use std::collections::HashMap;

use super::kinematics::{anchors_n, straight_steer, wrap_pi};
use super::{TerrainSnapshot, VehicleWrite, COMPRESS_RATIO};
use crate::Engine;

/// The clip name that plays a turret's fire frames (`vehicle_play`).
pub const FIRE_CLIP: &str = "fire";

/// A layer's tilt framing for [`Engine::set_layer_frame`]: the levels and
/// residual picked ([`pick_tilt`]), its axes, and whether its sheet warps
/// tilt (`tilt_interp`, which also enables the third source).
#[derive(Clone, Copy, Debug)]
pub(crate) struct LayerTilt {
    pub(crate) pick: TiltPick,
    pub(crate) pitch: Option<AngleAxis>,
    pub(crate) roll: Option<AngleAxis>,
    pub(crate) interp: bool,
}

/// A layer's pose in the vehicle frame: position (metres, `x` forward, `y`
/// left, `z` up) and yaw relative to the hull (tile radians).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct LayerPose {
    pub pos: Vec3,
    pub yaw: f32,
}

/// Rotate a chassis vector (`x` forward, `y` left, metres) by a tile heading
/// into a tile-space delta.
pub(super) fn chassis_to_tile(v: [f32; 2], heading: f32) -> [f32; 2] {
    let (s, c) = heading.sin_cos();
    [(v[0] * c + v[1] * s) / TILE_M, (v[0] * s - v[1] * c) / TILE_M]
}

/// Every layer's pose in the vehicle frame, composing parents (a child's
/// offset turns with its parent's yaw).  `yaws` are the layers' own yaws
/// (turrets), indexed like `def.layers`.
pub(super) fn layer_poses(def: &VehicleDef, yaws: &[f32]) -> Vec<LayerPose> {
    let n = def.layers.len();
    let mut poses: Vec<Option<LayerPose>> = vec![None; n];
    fn resolve(
        i: usize,
        def: &VehicleDef,
        yaws: &[f32],
        poses: &mut Vec<Option<LayerPose>>,
        depth: usize,
    ) -> LayerPose {
        if let Some(p) = poses[i] {
            return p;
        }
        let l = &def.layers[i];
        let own_yaw = yaws.get(i).copied().unwrap_or(0.0);
        let parent = l
            .parent
            .as_deref()
            .and_then(|p| def.layers.iter().position(|x| x.name == p))
            .or_else(|| {
                (l.kind != LayerKind::Hull)
                    .then(|| def.layers.iter().position(|x| x.kind == LayerKind::Hull))
                    .flatten()
            })
            .filter(|&p| p != i && depth < def.layers.len());
        let base = match parent {
            Some(p) => resolve(p, def, yaws, poses, depth + 1),
            None => LayerPose::default(),
        };
        let o = l.offset.unwrap_or([0.0; 3]);
        // The offset is in the parent's frame: turn it by the parent's yaw.
        // Tile radians turn the opposite way to the chassis frame's x→y.
        let (s, c) = (-base.yaw).sin_cos();
        let pos = base.pos + Vec3::new(o[0] * c - o[1] * s, o[0] * s + o[1] * c, o[2]);
        let p = LayerPose { pos, yaw: base.yaw + own_yaw };
        poses[i] = Some(p);
        p
    }
    (0..n).map(|i| resolve(i, def, yaws, &mut poses, 0)).collect()
}

/// The four ground contacts `[fl, fr, rl, rr]` (chassis metres) from the
/// wheel and gear-side layers' footprint, else a 1 m square.
pub(super) fn contact_offsets(def: &VehicleDef) -> [[f32; 2]; 4] {
    let poses = layer_poses(def, &[]);
    let mut lo = [f32::MAX; 2];
    let mut hi = [f32::MIN; 2];
    for (l, p) in def.layers.iter().zip(&poses) {
        if matches!(l.kind, LayerKind::Wheel | LayerKind::GearSide) {
            // A track runs a radius ahead of and behind its centre.
            let r = if l.kind == LayerKind::GearSide { l.radius_m.max(0.0) } else { 0.0 };
            lo = [lo[0].min(p.pos.x - r), lo[1].min(p.pos.y)];
            hi = [hi[0].max(p.pos.x + r), hi[1].max(p.pos.y)];
        }
    }
    if lo[0] > hi[0] || (hi[0] - lo[0]) < 1e-3 || (hi[1] - lo[1]) < 1e-3 {
        lo = [lo[0].min(-0.5), lo[1].min(-0.5)];
        hi = [hi[0].max(0.5), hi[1].max(0.5)];
        if lo[0] > hi[0] {
            lo = [-0.5, -0.5];
            hi = [0.5, 0.5];
        }
    }
    [[hi[0], hi[1]], [hi[0], lo[1]], [lo[0], hi[1]], [lo[0], lo[1]]]
}

/// The clip frame (absolute on the layer's clip axis) of a playback, and
/// whether it is finished.
pub(super) fn clip_frame(def: &ClipDef, play: &ClipPlay) -> (u32, bool) {
    let n = def.frames.max(1);
    let fps = if def.fps > 0.0 { def.fps } else { DEFAULT_CLIP_FPS };
    let k = (play.time.max(0.0) * fps * play.speed.abs()).floor() as u64;
    let plays = k / n as u64;
    let (local, done) = if play.repeat > 0 && plays >= play.repeat as u64 {
        (n - 1, true)
    } else {
        ((k % n as u64) as u32, false)
    };
    let local = if play.speed < 0.0 { n - 1 - local } else { local };
    (def.start + local, done)
}

/// A layer's clip by name; `fire` on a turret is its fire frames.
fn layer_clip(l: &VehicleLayerDef, name: &str) -> Option<ClipDef> {
    if l.kind == LayerKind::Turret && name == FIRE_CLIP && l.fire_frames > 1 {
        return Some(ClipDef { start: 0, frames: l.fire_frames, fps: DEFAULT_CLIP_FPS });
    }
    l.clips.get(name).copied()
}

impl Engine {
    /// The entity name of a vehicle's layer (`{vehicle}.{layer}`; the hull is
    /// the vehicle itself).
    pub fn vehicle_layer_entity(vehicle: &str, layer: &VehicleLayerDef) -> String {
        if layer.kind == LayerKind::Hull {
            vehicle.to_string()
        } else {
            format!("{vehicle}.{}", layer.name)
        }
    }

    /// Spawn a v2 (layered) vehicle.
    pub(super) fn spawn_vehicle_v2(
        &mut self,
        def_name: &str,
        def: &VehicleDef,
        entity_name: &str,
        x: f32,
        y: f32,
    ) -> bool {
        let Some(tm_entity) = self.entity_by_role(RoleKind::Tilemap) else { return false };
        let Some(hull_index) = def.layers.iter().position(|l| l.kind == LayerKind::Hull) else {
            return false;
        };
        let tilemap_name = self.debug_name(tm_entity);
        let dirs = def.directions.max(1);
        let ghost_group = match def.ghost_group {
            GhostGroupMode::Instance => {
                let g = self.next_ghost_group;
                self.next_ghost_group =
                    (self.next_ghost_group % (classic_gfx::MODEL_GHOST_GROUP - 1)) + 1;
                g
            }
            GhostGroupMode::None => 0,
        };
        let anchors = self.vehicle_anchors.get(&def.anchors).cloned();
        let layer_anchors = |l: &VehicleLayerDef| -> Vec<[f32; 2]> {
            let n = l.dirs(dirs);
            match anchors.as_ref().and_then(|a| a.parts.get(&l.name)) {
                Some(a) => anchors_n(a, n),
                // No anchors entry (e.g. cargo): the frame's packer anchor
                // wins in `effective_anchor`, else the cell centre.
                None => vec![[0.5, 0.5]; n as usize],
            }
        };

        // Chassis geometry from the wheel / gear layers.
        let contacts = contact_offsets(def);
        let wheelbase_m = (contacts[0][0] - contacts[2][0]).abs();
        let track_m = (contacts[0][1] - contacts[1][1]).abs();
        let hull = &def.layers[hull_index];
        let (pitch_levels, pitch_max) =
            hull.pitch.map_or((1, 0.0), |a| (a.count(), a.max_deg.to_radians()));
        let (roll_levels, roll_max) =
            hull.roll.map_or((1, 0.0), |a| (a.count(), a.max_deg.to_radians()));
        let steer = def.layers.iter().find_map(|l| l.steer);
        let (steer_levels, steer_max) =
            steer.map_or((1, 30f32.to_radians()), |a| (a.count(), a.max_deg.to_radians()));
        let tilt_reach = 0.5 * wheelbase_m.max(track_m) * pitch_max.max(roll_max).tan();
        let wheel_travel_down = tilt_reach.max(1.0 / PPM_TARGET);
        // Half a tilt step: the finer of the hull's pitch and roll axes (a
        // hull may bake either, or both at different levels).
        let half_step = |levels: u32, max: f32| (levels > 1).then(|| max / (levels - 1) as f32);
        let tilt_dead_zone =
            match (half_step(pitch_levels, pitch_max), half_step(roll_levels, roll_max)) {
                (Some(p), Some(r)) => p.min(r),
                (p, r) => p.or(r).unwrap_or(0.0),
            };
        let footprint: Vec<(i32, i32)> = match &def.path_footprint {
            Some(fp) => fp.clone(),
            None => {
                let half_x = ((wheelbase_m / TILE_M) * 0.5).ceil() as i32;
                let half_y = ((track_m / TILE_M) * 0.5).ceil() as i32;
                (-half_y..=half_y)
                    .flat_map(|dy| (-half_x..=half_x).map(move |dx| (dx, dy)))
                    .collect()
            }
        };

        let mut states = Vec::with_capacity(def.layers.len());
        let mut body_sprite = None;
        for (i, l) in def.layers.iter().enumerate() {
            let entity = Self::vehicle_layer_entity(entity_name, l);
            let a = layer_anchors(l);
            let sprite = IsoSprite {
                position: Vec3::new(x, y, 0.0),
                scale: Vec3::ONE,
                texture: l.texture.clone(),
                tilemap: tilemap_name.clone(),
                frame: 0.0,
                frame_name: None,
                tile_set_size: Vec2::ONE,
                anchor: Vec2::from(a[0]),
                frame_offset: Vec3::ZERO,
                footprint: vec![],
                ghost_group,
                color: [1.0; 4],
                dir_blend: None,
                no_ghost: !l.ghost,
                team_tint: None,
                emissive: None,
            };
            if i == hull_index {
                body_sprite = Some(sprite);
            } else {
                if self.names.contains_key(&entity) {
                    return false;
                }
                let e = self.world.spawn((sprite, Transform::new(Vec3::new(x, y, 0.0), Vec3::ONE)));
                let _ = self.world.insert_one(e, DebugName(entity.clone()));
                self.names.insert(entity.clone(), e);
                self.name_order.push(entity.clone());
                if !l.visible {
                    self.set_enabled(e, false);
                }
            }
            let yaw = 0.0;
            states.push(VehicleLayerState {
                name: l.name.clone(),
                entity,
                def_index: i,
                visible: l.visible,
                color: [1.0; 4],
                yaw,
                yaw_target: yaw,
                pitch: 0.0,
                aim: None,
                clip: None,
            });
        }
        let body_sprite = body_sprite.expect("hull sprite");
        let vehicle = IsoVehicle {
            tilemap: tilemap_name,
            directions: dirs,
            body_anchors: layer_anchors(hull),
            // The v1 wheel tables, sized to the direction count (v2 places
            // contacts from `contact_offsets_m`; these stay neutral).
            wheel_anchors: std::array::from_fn(|_| vec![[0.5, 0.5]; dirs as usize]),
            wheel_tile_offsets: std::array::from_fn(|_| vec![[0.0, 0.0]; dirs as usize]),
            speed: 2.6,
            pitch_levels,
            pitch_max,
            wheelbase_m,
            roll_levels,
            roll_max,
            track_m,
            path_footprint: footprint,
            turn_rate: def.turn_rate_deg_per_sec.to_radians(),
            safe_fall_m: def.safe_fall_m,
            wheel_travel_up: wheel_travel_down * COMPRESS_RATIO,
            wheel_travel_down,
            tilt_dead_zone,
            steer_index: straight_steer(steer_levels),
            steer_levels,
            steer_max,
            steer_rate: def.steer_rate_deg_per_sec.to_radians(),
            reverse_speed: def.reverse_speed,
            turn_cost: def.turn_cost,
            dust_effect: def.dust_effect.clone(),
            touchdown_effect: def.touchdown_effect.clone(),
            def_name: def_name.to_string(),
            layers: states,
            contact_offsets_m: contacts.to_vec(),
            pivot_turn: def.pivot_turn,
            ..Default::default()
        };
        let be = self.world.spawn((
            body_sprite,
            vehicle,
            Transform::new(Vec3::new(x, y, 0.0), Vec3::ONE),
            Selectable { priority: 1, group: 0 },
        ));
        let _ = self.world.insert_one(be, DebugName(entity_name.to_string()));
        self.names.insert(entity_name.to_string(), be);
        self.name_order.push(entity_name.to_string());
        self.vehicle_teleport(entity_name, x, y)
    }

    /// Advance the layers' own state (turret slew and aim, clips) by `dt` and
    /// place + frame every layer sprite from the chassis pose in `write`.
    pub(super) fn apply_layers(
        &mut self,
        ve: hecs::Entity,
        write: &VehicleWrite,
        terrain: &TerrainSnapshot,
        dt: f32,
    ) {
        let (def_name, mut v_state) = match self.world.get::<&IsoVehicle>(ve) {
            Ok(v) if !v.layers.is_empty() => (v.def_name.clone(), IsoVehicle::clone(&v)),
            _ => return,
        };
        let Some(def) = self.vehicles.get(&def_name).cloned() else { return };
        let anchors = self.vehicle_anchors.get(&def.anchors).cloned();
        let heading = write.heading;
        let [bx, by, _] = write.body;

        // Turret slew toward the aim / target, and clip time.
        let poses0 = layer_poses(&def, &v_state.layers.iter().map(|s| s.yaw).collect::<Vec<_>>());
        for s in v_state.layers.iter_mut() {
            let Some(l) = def.layers.get(s.def_index) else { continue };
            if l.kind == LayerKind::Turret {
                if let Some((px, py)) = s.aim {
                    let p = poses0[s.def_index].pos;
                    let t = chassis_to_tile([p.x, p.y], heading);
                    let (tx, ty) = (bx + t[0], by + t[1]);
                    let desired = (py - ty).atan2(px - tx);
                    s.yaw_target = wrap_pi(desired - heading);
                    let dist_m = ((px - tx).powi(2) + (py - ty).powi(2)).sqrt() * TILE_M;
                    let z_turret = write.body[2] + p.z;
                    let dz = terrain.height(px, py) - z_turret;
                    let elev = dz.atan2(dist_m.max(1e-3));
                    s.pitch = match l.pitch {
                        Some(ax) => elev.clamp(ax.min().to_radians(), ax.max_deg.to_radians()),
                        None => 0.0,
                    };
                }
                let rate = l.yaw_rate_deg_per_sec.to_radians();
                let err = wrap_pi(s.yaw_target - s.yaw);
                s.yaw = if rate > 0.0 {
                    wrap_pi(s.yaw + err.clamp(-rate * dt, rate * dt))
                } else {
                    s.yaw_target
                };
            }
            if let Some(c) = s.clip.as_mut() {
                c.time += dt;
            }
        }
        let poses = layer_poses(&def, &v_state.layers.iter().map(|s| s.yaw).collect::<Vec<_>>());
        // Whether a layer's sheet warps its tilt (`tilt_interp`).
        let tilt_interp =
            |l: &VehicleLayerDef| self.frame_tables.get(&l.texture).is_some_and(|t| t.tilt_interp);
        // The hull's drawn tilt: its baked level angles (`0` on an axis it
        // lacks), or the exact chassis tilt when its sheet warps tilt.  What a
        // layer riding it (cargo, a tool) leans by.
        let hull_tilt =
            def.layers.iter().find(|l| l.kind == LayerKind::Hull).map_or((0.0, 0.0), |h| {
                let interp = tilt_interp(h);
                pick_tilt(h.pitch, h.roll, heading, v_state.pitch, v_state.roll, interp).drawn
            });

        for s in &v_state.layers {
            let Some(l) = def.layers.get(s.def_index) else { continue };
            let Some(&e) = self.names.get(&s.entity) else { continue };
            let pose = poses[s.def_index];
            let layer_heading = heading + pose.yaw;
            // Position: the chassis offset tilted by the body pitch / roll
            // about the body's ground point (as `vehicle_socket_frame` does,
            // so a layer origin and a zero-offset socket on it agree), then
            // turned by the heading.
            let tilted = super::sockets::rot_pitch(v_state.pitch)
                * super::sockets::rot_roll(v_state.roll)
                * pose.pos;
            let t = chassis_to_tile([tilted.x, tilted.y], heading);
            let (lx, ly) = (bx + t[0], by + t[1]);
            let lz = write.body[2] + tilted.z;
            if let Ok(mut tf) = self.world.get::<&mut Transform>(e) {
                tf.position = Vec3::new(lx, ly, lz);
            }
            let dirs = l.dirs(def.directions);
            let mut sel = FrameSelect::default();
            let mut tilt: Option<LayerTilt> = None;
            match l.kind {
                LayerKind::Hull => {
                    let interp = tilt_interp(l);
                    let pick = pick_tilt(
                        l.pitch,
                        l.roll,
                        layer_heading,
                        v_state.pitch,
                        v_state.roll,
                        interp,
                    );
                    (sel.pitch, sel.roll) = (pick.pitch, pick.roll);
                    tilt = Some(LayerTilt { pick, pitch: l.pitch, roll: l.roll, interp });
                }
                LayerKind::Wheel | LayerKind::GearSide => {
                    if l.spin_frames > 1 && l.radius_m > 0.0 {
                        // A right turn (tile heading up) runs the left (+y)
                        // side forward.
                        let dist = v_state.odometer_m + v_state.turned * pose.pos.y;
                        let rev = dist / (std::f32::consts::TAU * l.radius_m);
                        sel.spin = (rev.rem_euclid(1.0) * l.spin_frames as f32) as u32;
                    }
                    sel.steer = l.steer.map_or(0, |a| a.index(v_state.steer));
                }
                LayerKind::Turret => {
                    sel.pitch = l.pitch.map_or(0, |a| a.index(s.pitch));
                    if let Some(c) = &s.clip {
                        if let Some(cd) = layer_clip(l, &c.clip) {
                            sel.fire = clip_frame(&cd, c).0;
                        }
                    }
                }
                LayerKind::Tool | LayerKind::Legs | LayerKind::Cargo | LayerKind::Lift => {
                    let interp = tilt_interp(l);
                    if l.pitch.is_some() || l.roll.is_some() || interp {
                        let (p, r) = tilt_for_yaw(hull_tilt.0, hull_tilt.1, pose.yaw);
                        let pick = pick_tilt(l.pitch, l.roll, layer_heading, p, r, interp);
                        (sel.pitch, sel.roll) = (pick.pitch, pick.roll);
                        tilt = Some(LayerTilt { pick, pitch: l.pitch, roll: l.roll, interp });
                    }
                    if let Some(c) = &s.clip {
                        if let Some(cd) = layer_clip(l, &c.clip) {
                            sel.clip = clip_frame(&cd, c).0;
                        }
                    }
                }
            }
            let layer_anchors: Vec<[f32; 2]> =
                match anchors.as_ref().and_then(|a| a.parts.get(&l.name)) {
                    Some(a) => anchors_n(a, dirs),
                    None => vec![[0.5, 0.5]; dirs as usize],
                };
            let frame_index = |dir: u32, levels: Option<(u32, u32)>| {
                let mut s2 = sel;
                s2.dir = dir;
                if let Some((p, r)) = levels {
                    (s2.pitch, s2.roll) = (p, r);
                }
                l.frame_index(def.directions, &s2)
            };
            let altitude = lz - terrain.height(lx, ly);
            if let Ok(mut sp) = self.world.get::<&mut IsoSprite>(e) {
                sp.frame_offset.z = altitude;
                sp.color = s.color;
                Self::set_layer_frame(
                    &self.frame_tables,
                    &mut sp,
                    layer_heading,
                    dirs,
                    &layer_anchors,
                    frame_index,
                    tilt,
                );
            }
        }

        // Keep the layer state (turret yaw, clips) on the component.
        if let Ok(mut v) = self.world.get::<&mut IsoVehicle>(ve) {
            v.layers = std::mem::take(&mut v_state.layers);
        }
    }

    /// Frame one layer sprite for a heading: the quantized direction, or the
    /// two straddling directions when its texture opts into interpolation.
    pub(crate) fn set_layer_frame(
        tables: &HashMap<String, FrameTable>,
        s: &mut IsoSprite,
        heading: f32,
        dirs: u32,
        anchors: &[[f32; 2]],
        frame_index: impl Fn(u32, Option<(u32, u32)>) -> u32,
        tilt: Option<LayerTilt>,
    ) {
        let name = |s: &IsoSprite, f: u32| {
            tables.contains_key(&s.texture).then(|| format!("{}_{f}", s.texture))
        };
        let anchor = |d: u32| Vec2::from(anchors.get(d as usize).copied().unwrap_or([0.5, 0.5]));
        if tables.get(&s.texture).is_some_and(|t| t.dir_interp) {
            let (a, b, t) = heading_neighbours(heading, dirs);
            let step = std::f32::consts::TAU / dirs.max(1) as f32;
            let fa = frame_index(a, None);
            s.frame = fa as f32;
            s.frame_name = name(s, fa);
            s.anchor = anchor(a);
            // The third source: the nearer yaw neighbour at an adjacent tilt
            // level, for a large residual on a tilt-warping sheet.
            let third = tilt
                .filter(|lt| lt.interp)
                .and_then(|lt| third_tilt(lt.pitch, lt.roll, heading, &lt.pick))
                .map(|th| {
                    let from_b = t >= 0.5;
                    let d = if from_b { b } else { a };
                    DirThird {
                        frame: frame_index(d, Some((th.pitch, th.roll))) as f32,
                        anchor: anchor(d),
                        tilt: th.residual,
                        from_b,
                    }
                });
            s.dir_blend = Some(DirBlend {
                frame_b: frame_index(b, None) as f32,
                anchor_b: anchor(b),
                dtheta_a: t * step,
                dtheta_b: (t - 1.0) * step,
                tilt: tilt.map_or([0.0; 3], |lt| lt.pick.residual),
                third,
            });
        } else {
            let d = heading_to_dir_n(heading, dirs);
            let f = frame_index(d, None);
            s.frame = f as f32;
            s.frame_name = name(s, f);
            s.anchor = anchor(d);
            s.dir_blend = None;
        }
    }

    /// The vehicle body entity and one of its layers' index, by name.
    fn vehicle_layer(&self, name: &str, layer: &str) -> Option<(hecs::Entity, usize)> {
        let &ve = self.names.get(name)?;
        let v = self.world.get::<&IsoVehicle>(ve).ok()?;
        let i = v.layers.iter().position(|s| s.name == layer)?;
        Some((ve, i))
    }

    /// Aim every turret layer at the map point `(x, y)` (tile coords; the
    /// gun elevation targets the terrain there).  The yaw slews at the
    /// layer's `yaw_rate_deg_per_sec`.
    pub fn vehicle_aim(&mut self, name: &str, x: f32, y: f32) -> bool {
        self.with_turrets(name, |s| {
            s.aim = Some((x, y));
        })
    }

    /// Set every turret layer's yaw relative to the hull and gun pitch
    /// (degrees), dropping any aim point.
    pub fn vehicle_aim_angles(&mut self, name: &str, yaw_deg: f32, pitch_deg: f32) -> bool {
        let Some(def) = self.vehicle_def_of(name) else { return false };
        self.with_turrets(name, |s| {
            s.aim = None;
            s.yaw_target = wrap_pi(yaw_deg.to_radians());
            let pitch = pitch_deg.to_radians();
            s.pitch = match def.layers.get(s.def_index).and_then(|l| l.pitch) {
                Some(ax) => pitch.clamp(ax.min().to_radians(), ax.max_deg.to_radians()),
                None => 0.0,
            };
        })
    }

    fn vehicle_def_of(&self, name: &str) -> Option<VehicleDef> {
        let &ve = self.names.get(name)?;
        let v = self.world.get::<&IsoVehicle>(ve).ok()?;
        self.vehicles.get(&v.def_name).cloned()
    }

    fn with_turrets(&mut self, name: &str, mut f: impl FnMut(&mut VehicleLayerState)) -> bool {
        let Some(def) = self.vehicle_def_of(name) else { return false };
        let Some(&ve) = self.names.get(name) else { return false };
        let Ok(mut v) = self.world.get::<&mut IsoVehicle>(ve) else { return false };
        let mut any = false;
        for s in v.layers.iter_mut() {
            if def.layers.get(s.def_index).is_some_and(|l| l.kind == LayerKind::Turret) {
                f(s);
                any = true;
            }
        }
        any
    }

    /// Play a layer clip: `speed` multiplies its fps (negative plays it
    /// backward from the last frame), `repeat` `0` loops and `n` plays it
    /// `n` times then holds the end frame.  On a turret, `fire` plays the
    /// fire frames.
    pub fn vehicle_play(
        &mut self,
        name: &str,
        layer: &str,
        clip: &str,
        speed: f32,
        repeat: i32,
    ) -> bool {
        let Some(def) = self.vehicle_def_of(name) else { return false };
        let Some((ve, i)) = self.vehicle_layer(name, layer) else { return false };
        let Ok(mut v) = self.world.get::<&mut IsoVehicle>(ve) else { return false };
        let s = &mut v.layers[i];
        let Some(l) = def.layers.get(s.def_index) else { return false };
        if layer_clip(l, clip).is_none() {
            return false;
        }
        s.clip = Some(ClipPlay { clip: clip.to_string(), time: 0.0, speed, repeat: repeat.max(0) });
        true
    }

    /// Show or hide one layer.
    pub fn vehicle_layer_visible(&mut self, name: &str, layer: &str, visible: bool) -> bool {
        let Some((ve, i)) = self.vehicle_layer(name, layer) else { return false };
        let entity = {
            let Ok(mut v) = self.world.get::<&mut IsoVehicle>(ve) else { return false };
            v.layers[i].visible = visible;
            v.layers[i].entity.clone()
        };
        let Some(&e) = self.names.get(&entity) else { return false };
        self.set_enabled(e, visible);
        true
    }

    /// Tint one layer (RGBA multiplied onto its albedo).
    pub fn vehicle_layer_color(&mut self, name: &str, layer: &str, rgba: [f32; 4]) -> bool {
        let Some((ve, i)) = self.vehicle_layer(name, layer) else { return false };
        let Ok(mut v) = self.world.get::<&mut IsoVehicle>(ve) else { return false };
        v.layers[i].color = rgba;
        true
    }
}

impl Engine {
    /// Every sprite entity of a vehicle: the body, its v1 wheels and tires,
    /// and its v2 layers.
    pub fn vehicle_part_entities(&self, name: &str) -> Vec<hecs::Entity> {
        let Some(&ve) = self.names.get(name) else { return Vec::new() };
        let Ok(v) = self.world.get::<&IsoVehicle>(ve) else { return Vec::new() };
        let mut out = vec![ve];
        let names = v.wheel_entities.iter().chain(v.tire_entities.iter());
        for n in names.chain(v.layers.iter().map(|s| &s.entity)) {
            if let Some(&e) = self.names.get(n) {
                if !out.contains(&e) {
                    out.push(e);
                }
            }
        }
        out
    }

    /// Set a vehicle's team colour: every part whose sheet carries a team
    /// mask multiplies its masked texels by `rgb` (separate from `color`).
    pub fn vehicle_team(&mut self, name: &str, rgb: [f32; 3]) -> bool {
        self.for_vehicle_sprites(name, |s| s.team_tint = Some(rgb))
    }

    /// Set a vehicle's emissive strength: the marker-normal (unlit) pixels
    /// scale by it; above 1 they brighten into the bloom pass.
    pub fn vehicle_emissive(&mut self, name: &str, strength: f32) -> bool {
        let strength = if strength.is_finite() { strength.max(0.0) } else { 1.0 };
        self.for_vehicle_sprites(name, |s| s.emissive = Some(strength))
    }

    fn for_vehicle_sprites(&mut self, name: &str, mut f: impl FnMut(&mut IsoSprite)) -> bool {
        let parts = self.vehicle_part_entities(name);
        for e in &parts {
            if let Ok(mut s) = self.world.get::<&mut IsoSprite>(*e) {
                f(&mut s);
            }
        }
        !parts.is_empty()
    }
}
