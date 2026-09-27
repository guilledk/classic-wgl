//! Vehicle spawn and the per-frame chassis/suspension simulation.

use classic_core::components::{
    DebugName, IsoSprite, IsoVehicle, RoleKind, Selectable, Tilemap, Transform,
};
use classic_core::tilemap::{PPM_TARGET, TILE_M};
use glam::{Vec2, Vec3};

use crate::Engine;

use super::kinematics::{
    advance_suspension, anchors8, derive_wheel_offsets, heading_to_dir, lerp_dir2, lookahead,
    pitch_index, should_reverse, soft_deadzone, steer_index, step_spring, step_steer,
    straight_steer, wrap_pi,
};
use super::{
    TerrainSnapshot, VehicleWrite, AIRBORNE_PITCH, COMPRESS_RATIO, DIRECTIONS, GOAL_TOLERANCE,
    MOON_GRAVITY, WHEEL_SUFFIXES,
};

impl Engine {
    /// Resolve a vehicle sprite's packed-atlas frame name from its texture and
    /// flat frame index, mirroring the animator's `{texture}_{frame}` naming.
    /// Returns `None` when the texture has no frame table, so the uniform-grid
    /// `frame`/`tile_set_size` path takes over.
    fn frame_name(
        tables: &std::collections::HashMap<String, classic_core::types::FrameTable>,
        texture: &str,
        frame: f32,
    ) -> Option<String> {
        tables.contains_key(texture).then(|| format!("{texture}_{}", frame as u32))
    }

    /// Snapshot the tilemap's terrain data (cloned once per frame).
    pub(super) fn vehicle_terrain(&self) -> Option<TerrainSnapshot> {
        let tm_entity = self.entity_by_role(RoleKind::Tilemap)?;
        let tm = self.world.get::<&Tilemap>(tm_entity).ok()?;
        Some(TerrainSnapshot {
            size_x: tm.size_x,
            size_y: tm.size_y,
            heights: tm.height_data.clone(),
        })
    }

    /// Resolve the four wheel entity handles referenced by a vehicle.
    pub(super) fn vehicle_wheel_handles(&self, ve: hecs::Entity) -> [Option<hecs::Entity>; 4] {
        let Ok(v) = self.world.get::<&IsoVehicle>(ve) else {
            return [None; 4];
        };
        let mut handles = [None; 4];
        for (i, name) in v.wheel_entities.iter().enumerate() {
            handles[i] = self.names.get(name).copied();
        }
        handles
    }

    /// Resolve the steering-tire entity handles referenced by a vehicle.
    pub(super) fn vehicle_tire_handles(&self, ve: hecs::Entity) -> [Option<hecs::Entity>; 2] {
        let Ok(v) = self.world.get::<&IsoVehicle>(ve) else {
            return [None; 2];
        };
        let mut handles = [None; 2];
        for (i, name) in v.tire_entities.iter().enumerate() {
            handles[i] = self.names.get(name).copied();
        }
        handles
    }

    /// Spawn a vehicle of a declared type: a body entity (with `IsoVehicle`) plus
    /// four wheel `IsoSprite`s, positioned at `(x, y)` with anchors/offsets taken
    /// from the definition sidecar.
    pub fn spawn_vehicle(&mut self, def_name: &str, entity_name: &str, x: f32, y: f32) -> bool {
        let Some(def) = self.vehicles.get(def_name).cloned() else { return false };
        let Some(tm_entity) = self.entity_by_role(RoleKind::Tilemap) else { return false };
        let tile_scale = self.world.get::<&Transform>(tm_entity).map(|t| t.scale.x).unwrap_or(45.0);
        if self.names.contains_key(entity_name) {
            return false;
        }

        let Some(body_part) = def.parts.first().cloned() else { return false };
        if def.parts.len() != 5 {
            return false;
        }

        // Resolve each part's per-direction anchors from the vehicle's anchors
        // data artifact (referenced by `def.anchors`); absent parts fall back to
        // the neutral 0.5/0.5 anchor.
        let anchors_map: std::collections::HashMap<String, [[f32; 2]; 8]> = self
            .vehicle_anchors
            .get(&def.anchors)
            .map(|a| {
                a.parts
                    .iter()
                    .chain(a.tires.iter())
                    .map(|(name, anchors)| (name.clone(), anchors8(anchors)))
                    .collect()
            })
            .unwrap_or_default();
        let body_anchors =
            anchors_map.get(&body_part.name).copied().unwrap_or([[0.5f32, 0.5f32]; 8]);
        let mut wheel_anchors = [[[0.5f32, 0.5f32]; 8]; 4];
        for (i, wheel_anchors) in wheel_anchors.iter_mut().enumerate() {
            *wheel_anchors =
                anchors_map.get(&def.parts[i + 1].name).copied().unwrap_or([[0.5f32, 0.5f32]; 8]);
        }
        let tile_set_size = Vec2::new(def.columns as f32, def.rows as f32);
        let body_tile_set_size = Vec2::new(
            def.columns as f32,
            def.rows as f32 * def.pitch_levels.max(1) as f32 * def.roll_levels.max(1) as f32,
        );
        let cell = if def.cell[0] > 0.0 && def.cell[1] > 0.0 { def.cell } else { [1.0, 1.0] };
        let wheel_tile_offsets =
            derive_wheel_offsets(&body_anchors, &wheel_anchors, cell, tile_scale);

        // Front-rear axle distance in world metres, from the derived wheel
        // offsets (front = fl/fr, rear = rl/rr) scaled by the tile metre length.
        let wheelbase_m = {
            let front = Vec2::new(
                (wheel_tile_offsets[0][0][0] + wheel_tile_offsets[1][0][0]) * 0.5,
                (wheel_tile_offsets[0][0][1] + wheel_tile_offsets[1][0][1]) * 0.5,
            );
            let rear = Vec2::new(
                (wheel_tile_offsets[2][0][0] + wheel_tile_offsets[3][0][0]) * 0.5,
                (wheel_tile_offsets[2][0][1] + wheel_tile_offsets[3][0][1]) * 0.5,
            );
            (front - rear).length() * TILE_M
        };
        // Left-right axle distance in world metres (left = fl/rl, right = fr/rr).
        let track_m = {
            let left = Vec2::new(
                (wheel_tile_offsets[0][0][0] + wheel_tile_offsets[2][0][0]) * 0.5,
                (wheel_tile_offsets[0][0][1] + wheel_tile_offsets[2][0][1]) * 0.5,
            );
            let right = Vec2::new(
                (wheel_tile_offsets[1][0][0] + wheel_tile_offsets[3][0][0]) * 0.5,
                (wheel_tile_offsets[1][0][1] + wheel_tile_offsets[3][0][1]) * 0.5,
            );
            (left - right).length() * TILE_M
        };

        // Suspension travel limits derived from the def geometry (not hand-tuned
        // per vehicle): the downward droop is the vertical reach the baked tilt
        // frames provide at a wheel (`0.5 · axle · tan(max tilt)`), and the
        // upward compression is a fixed fraction of it, so a single-wheel bump
        // compresses the wheel more than it lifts the body.  The tilt dead-zone
        // is half the smallest representable pitch-frame step, so the body only
        // tilts on slope it can visibly show.
        let pitch_max = def.pitch_max_deg.to_radians();
        let roll_max = def.roll_max_deg.to_radians();
        let tilt_reach = 0.5 * wheelbase_m.max(track_m) * pitch_max.max(roll_max).tan();
        let wheel_travel_down = tilt_reach.max(1.0 / PPM_TARGET);
        let wheel_travel_up = wheel_travel_down * COMPRESS_RATIO;
        let tilt_dead_zone =
            if def.pitch_levels > 1 { pitch_max / (def.pitch_levels - 1) as f32 } else { 0.0 };

        // Collision footprint for pathfinding: explicit def override, else
        // auto-derived from the wheelbase/track — half the wheelbase
        // (front-rear) along the heading axis and half the track (left-right)
        // across it, rounded out.  This matches the vehicle's actual footprint
        // in a single heading rather than the union of all 8 (which
        // over-estimates on diagonal headings).
        let footprint: Vec<(i32, i32)> = match &def.path_footprint {
            Some(fp) => fp.clone(),
            None => {
                let half_x = ((wheelbase_m / TILE_M) * 0.5).ceil() as i32;
                let half_y = ((track_m / TILE_M) * 0.5).ceil() as i32;
                let mut fp = Vec::with_capacity(((half_x * 2 + 1) * (half_y * 2 + 1)) as usize);
                for dy in -half_y..=half_y {
                    for dx in -half_x..=half_x {
                        fp.push((dx, dy));
                    }
                }
                fp
            }
        };

        let wheel_names: [String; 4] = WHEEL_SUFFIXES.map(|s| format!("{entity_name}Wheel{s}"));
        // Reference the actual tilemap entity by its (possibly namespace-qualified)
        // name rather than the hardcoded bare `"tilemap"`, so a vehicle spawned by
        // a namespaced ROM's guest still resolves its terrain under multi-ROM load.
        let tilemap_name = self.debug_name(tm_entity);
        // Assign a unique per-instance stencil ghost-group id to the body + all
        // four wheels, so the parts never ghost through each other (but still
        // ghost through terrain and other entities).  Ids live in 1..=255; 0 is
        // reserved for ungrouped sprites.
        let ghost_group = self.next_ghost_group;
        // Groups cycle through 1..255; `MODEL_GHOST_GROUP` (255) is reserved for
        // the 3D-model composite.
        self.next_ghost_group = (self.next_ghost_group % (classic_gfx::MODEL_GHOST_GROUP - 1)) + 1;

        // Spawn the four wheel sprites.
        for i in 0..4 {
            let sprite = IsoSprite {
                position: Vec3::new(x, y, 0.0),
                scale: Vec3::ONE,
                texture: def.parts[i + 1].texture.clone(),
                tilemap: tilemap_name.clone(),
                frame: 0.0,
                frame_name: None,
                tile_set_size,
                anchor: Vec2::from(wheel_anchors[i][0]),
                frame_offset: Vec3::ZERO,
                footprint: vec![],
                ghost_group,
                color: [1.0, 1.0, 1.0, 1.0],
            };
            let we = self.world.spawn((sprite, Transform::new(Vec3::new(x, y, 0.0), Vec3::ONE)));
            let _ = self.world.insert_one(we, DebugName(wheel_names[i].clone()));
            self.names.insert(wheel_names[i].clone(), we);
            self.name_order.push(wheel_names[i].clone());
        }

        // Spawn the steering tires (rotating disks over the wheels, matched by
        // index: `tires[0]` steers `wheel_fl`, `tires[1]` steers `wheel_fr`).
        // A tire shares its wheel's ground-origin anchor — steering yaws the disk
        // about the axle's vertical axis, so the anchor is steer-invariant.
        let steer_levels = def.steer_levels.max(1);
        let tire_tile_set_size =
            Vec2::new(def.columns as f32, def.rows as f32 * steer_levels as f32);
        let mut tire_entities = [String::new(), String::new()];
        for (i, tire) in def.tires.iter().take(2).enumerate() {
            let name = format!("{entity_name}Tire{}", WHEEL_SUFFIXES[i]);
            let sprite = IsoSprite {
                position: Vec3::new(x, y, 0.0),
                scale: Vec3::ONE,
                texture: tire.texture.clone(),
                tilemap: tilemap_name.clone(),
                frame: 0.0,
                frame_name: None,
                tile_set_size: tire_tile_set_size,
                anchor: Vec2::from(wheel_anchors[i][0]),
                frame_offset: Vec3::ZERO,
                footprint: vec![],
                ghost_group,
                color: [1.0, 1.0, 1.0, 1.0],
            };
            let te = self.world.spawn((sprite, Transform::new(Vec3::new(x, y, 0.0), Vec3::ONE)));
            let _ = self.world.insert_one(te, DebugName(name.clone()));
            self.names.insert(name.clone(), te);
            self.name_order.push(name.clone());
            tire_entities[i] = name;
        }

        // Spawn the body entity with the IsoVehicle component.
        let body_sprite = IsoSprite {
            position: Vec3::new(x, y, 0.0),
            scale: Vec3::ONE,
            texture: body_part.texture.clone(),
            tilemap: tilemap_name.clone(),
            frame: 0.0,
            frame_name: None,
            tile_set_size: body_tile_set_size,
            anchor: Vec2::from(body_anchors[0]),
            frame_offset: Vec3::ZERO,
            footprint: vec![],
            ghost_group,
            color: [1.0, 1.0, 1.0, 1.0],
        };
        let vehicle = IsoVehicle {
            tilemap: tilemap_name,
            wheel_entities: [
                wheel_names[0].clone(),
                wheel_names[1].clone(),
                wheel_names[2].clone(),
                wheel_names[3].clone(),
            ],
            tire_entities,
            body_anchors,
            wheel_anchors,
            speed: 2.6,
            direction: 0,
            wheel_tile_offsets,
            pitch_levels: def.pitch_levels,
            pitch_max,
            wheelbase_m,
            roll_levels: def.roll_levels,
            roll_max,
            track_m,
            path_footprint: footprint,
            turn_rate: def.turn_rate_deg_per_sec.to_radians(),
            safe_fall_m: def.safe_fall_m,
            wheel_travel_up,
            wheel_travel_down,
            tilt_dead_zone,
            steer_index: straight_steer(steer_levels),
            steer_levels,
            steer_max: def.steer_max_deg.to_radians(),
            steer_rate: def.steer_rate_deg_per_sec.to_radians(),
            reverse_speed: def.reverse_speed,
            turn_cost: def.turn_cost,
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

    /// Advance every `IsoVehicle` one step: follow the movement path, sample
    /// terrain under each wheel, integrate the point-mass jump + suspension,
    /// and write the five sprites.
    pub fn update_vehicles(&mut self) {
        let delta = self.time.delta;
        let vehicles: Vec<hecs::Entity> =
            self.world.query::<&IsoVehicle>().iter().map(|(e, _)| e).collect();
        if vehicles.is_empty() {
            return;
        }
        let Some(terrain) = self.vehicle_terrain() else {
            return;
        };
        for ve in vehicles {
            self.update_one_vehicle(ve, delta, &terrain);
        }
    }

    fn update_one_vehicle(&mut self, ve: hecs::Entity, delta: f32, terrain: &TerrainSnapshot) {
        let (body_x, body_y) = match self.world.get::<&Transform>(ve) {
            Ok(t) => (t.position.x, t.position.y),
            Err(_) => return,
        };
        let wheel_handles = self.vehicle_wheel_handles(ve);
        let tire_handles = self.vehicle_tire_handles(ve);

        let write = {
            let Some(mut v) = self.world.get::<&mut IsoVehicle>(ve).ok() else { return };

            // -- movement along the A* path (kinematic bicycle, forward+reverse) --
            let mut x = body_x;
            let mut y = body_y;
            let mut following = false;
            if !v.path.is_empty() {
                // Arrive once the final waypoint is within tolerance; snap to
                // its center and stop.
                let goal = (
                    v.path[v.path.len() - 1][0] as f32 + 0.5,
                    v.path[v.path.len() - 1][1] as f32 + 0.5,
                );
                let dx = goal.0 - x;
                let dy = goal.1 - y;
                if dx * dx + dy * dy <= GOAL_TOLERANCE * GOAL_TOLERANCE {
                    x = goal.0;
                    y = goal.1;
                    v.path.clear();
                    v.path_idx = 0;
                    v.reversing = false;
                } else {
                    // Pure-pursuit: lead the target by ~1.5× the minimum
                    // turning radius (speed / turn_rate) so the vehicle arcs
                    // through corners rather than orbiting a waypoint inside
                    // its turning circle.
                    let turn_radius = v.speed / v.turn_rate.max(1e-3);
                    let lookahead_dist = (turn_radius * 1.5).max(1.0);
                    let (gx, gy, next_idx) = lookahead(&v.path, v.path_idx, x, y, lookahead_dist);
                    v.path_idx = next_idx;

                    // Heading error to the look-ahead target.  When it's
                    // substantially behind, reverse and reorient instead of
                    // arcing wide off-traversable.
                    let desired = (gy - y).atan2(gx - x);
                    let err = wrap_pi(desired - v.heading);
                    v.reversing = v.reverse_speed > 0.0 && should_reverse(err, v.reversing);

                    // Front wheels steer into the turn (rate-limited state, so
                    // the tires sweep through their steer frames rather than
                    // snapping); the body rotates toward the target at the max
                    // turn rate.
                    v.steer = step_steer(v.steer, err, v.steer_max, v.steer_rate, delta);
                    let max_turn = v.turn_rate * delta;
                    v.heading = wrap_pi(v.heading + err.clamp(-max_turn, max_turn));

                    // Forward drives ahead; reverse backs up along the heading.
                    let dir: f32 = if v.reversing { -1.0 } else { 1.0 };
                    let drive_speed = if v.reversing { v.reverse_speed } else { v.speed };
                    let step = drive_speed * delta;
                    x += dir * v.heading.cos() * step;
                    y += dir * v.heading.sin() * step;
                    following = true;
                }
            }
            if !following {
                // Stopped: return the steering wheel to straight.
                v.steer = step_steer(v.steer, 0.0, v.steer_max, v.steer_rate, delta);
                v.reversing = false;
            }

            // Quantize the continuous heading for the 8-way sprite sheets; the
            // tires follow the integrated steering angle, not the raw error.
            v.direction = heading_to_dir(v.heading);
            v.steer_index = steer_index(v.steer, v.steer_max, v.steer_levels);

            // -- wheel ground contacts --------------------------------------
            let mut wheel_xy = [[0.0f32; 2]; 4];
            let mut wheel_ground = [0.0f32; 4];
            for i in 0..4 {
                let [ox, oy] = lerp_dir2(&v.wheel_tile_offsets[i], v.heading);
                let wx = x + ox;
                let wy = y + oy;
                wheel_xy[i] = [wx, wy];
                wheel_ground[i] = terrain.height(wx, wy);
            }

            // -- body plane (chassis) ---------------------------------------
            // The body pose `(altitude, pitch, roll)` is one rigid plane fit to
            // the four wheel contacts and spring-smoothed.  Wheels are placed
            // relative to that plane within a travel envelope, so they can never
            // ride over the body.  The body centre is the mean of the four wheel
            // grounds (not the terrain under the body centre), so it lifts as
            // the wheels climb.
            let alt_target =
                (wheel_ground[0] + wheel_ground[1] + wheel_ground[2] + wheel_ground[3]) * 0.25;

            // Body point-mass vertical physics (moon gravity), snapping to the
            // wheel-plane centre when grounded and going airborne when the
            // ground beneath it drops away.
            let body_terrain = terrain.height(x, y);
            v.vel_z -= MOON_GRAVITY * delta;
            v.altitude += v.vel_z * delta;
            let airborne = v.altitude > alt_target + 0.5 / PPM_TARGET;
            if !airborne {
                v.altitude = alt_target;
                if v.vel_z < 0.0 {
                    v.vel_z = 0.0;
                }
            }
            v.airborne = airborne;
            // `frame_offset.z` is the altitude above the sprite's own terrain in
            // world metres (the sprite model re-adds terrain height at its x/y).
            let body_altitude = v.altitude - body_terrain;

            // -- body pitch (angular spring-damper) --------------------------
            // Target = terrain slope along the heading from the instantaneous
            // wheel grounds, dead-zoned so sub-frame slopes stay wheel
            // compression rather than body tilt (OpenRA-style tilt margin).
            // Airborne, the body noses down instead of tracking.
            let pitch_target = if airborne {
                AIRBORNE_PITCH
            } else {
                let front = (wheel_ground[0] + wheel_ground[1]) * 0.5;
                let rear = (wheel_ground[2] + wheel_ground[3]) * 0.5;
                soft_deadzone(((front - rear) / v.wheelbase_m.max(1e-4)).atan(), v.tilt_dead_zone)
            };
            let (pitch, pitch_vel) = step_spring(v.pitch, v.pitch_vel, pitch_target, delta);
            v.pitch = pitch;
            v.pitch_vel = pitch_vel;
            v.pitch_index = pitch_index(v.pitch, v.pitch_max, v.pitch_levels);

            // -- body roll (side slope, left-up positive) --------------------
            // Target = terrain slope across the vehicle; level when airborne.
            let roll_target = if airborne {
                0.0
            } else {
                let left = (wheel_ground[0] + wheel_ground[2]) * 0.5;
                let right = (wheel_ground[1] + wheel_ground[3]) * 0.5;
                soft_deadzone(((left - right) / v.track_m.max(1e-4)).atan(), v.tilt_dead_zone)
            };
            let (roll, roll_vel) = step_spring(v.roll, v.roll_vel, roll_target, delta);
            v.roll = roll;
            v.roll_vel = roll_vel;
            v.roll_index = pitch_index(v.roll, v.roll_max, v.roll_levels);

            // Actual body-plane height at each wheel (small-angle sin ≈ angle).
            // Airborne the plane is flat so wheels hang uniformly.
            let (plane_pitch, plane_roll) = if airborne { (0.0, 0.0) } else { (pitch, roll) };
            let plane = [
                v.altitude + 0.5 * plane_pitch * v.wheelbase_m + 0.5 * plane_roll * v.track_m,
                v.altitude + 0.5 * plane_pitch * v.wheelbase_m - 0.5 * plane_roll * v.track_m,
                v.altitude - 0.5 * plane_pitch * v.wheelbase_m + 0.5 * plane_roll * v.track_m,
                v.altitude - 0.5 * plane_pitch * v.wheelbase_m - 0.5 * plane_roll * v.track_m,
            ];

            // -- per-wheel suspension ---------------------------------------
            // Each wheel follows its terrain, but only within a travel envelope
            // around the body plane: it droops at most `wheel_travel_down` below
            // and compresses at most `wheel_travel_up` above.  Anything beyond
            // that is absorbed by the body plane (lift + tilt) instead.
            let mut wheel_altitude = [0.0f32; 4];
            let mut wheel_z = [0.0f32; 4];
            for i in 0..4 {
                let target = wheel_ground[i]
                    .clamp(plane[i] - v.wheel_travel_down, plane[i] + v.wheel_travel_up);
                let (h, hv) = advance_suspension(v.wheel_h[i], v.wheel_v[i], target, delta);
                v.wheel_h[i] = h;
                v.wheel_v[i] = hv;
                // Never sink below the terrain, and never compress above the
                // travel cap (so wheels never ride over the body).
                let final_h = v.wheel_h[i].max(wheel_ground[i]).min(plane[i] + v.wheel_travel_up);
                wheel_altitude[i] = final_h - wheel_ground[i];
                wheel_z[i] = final_h;
            }

            let mut wheel_anchors = [[0.0f32; 2]; 4];
            for (i, a) in wheel_anchors.iter_mut().enumerate() {
                *a = lerp_dir2(&v.wheel_anchors[i], v.heading);
            }

            let body_frame =
                ((v.pitch_index * v.roll_levels + v.roll_index) * DIRECTIONS + v.direction) as f32;

            VehicleWrite {
                body: [x, y, v.altitude],
                body_altitude,
                body_anchor: lerp_dir2(&v.body_anchors, v.heading),
                direction: v.direction,
                body_frame,
                wheel_xy,
                wheel_altitude,
                wheel_z,
                wheel_anchors,
                steer_index: v.steer_index,
            }
        };

        self.apply_pose(ve, &wheel_handles, &tire_handles, &write);
    }

    /// Write a computed pose into the body + wheel `Transform`/`IsoSprite`s.
    pub(super) fn apply_pose(
        &mut self,
        ve: hecs::Entity,
        wheel_handles: &[Option<hecs::Entity>; 4],
        tire_handles: &[Option<hecs::Entity>; 2],
        write: &VehicleWrite,
    ) {
        if let Ok(mut tf) = self.world.get::<&mut Transform>(ve) {
            tf.position.x = write.body[0];
            tf.position.y = write.body[1];
            tf.position.z = write.body[2];
        }
        if let Ok(mut s) = self.world.get::<&mut IsoSprite>(ve) {
            s.frame = write.body_frame;
            s.frame_name = Self::frame_name(&self.frame_tables, &s.texture, write.body_frame);
            s.frame_offset.z = write.body_altitude;
            s.anchor = Vec2::from(write.body_anchor);
        }

        for (i, handle) in wheel_handles.iter().enumerate() {
            let Some(we) = handle else { continue };
            if let Ok(mut tf) = self.world.get::<&mut Transform>(*we) {
                tf.position.x = write.wheel_xy[i][0];
                tf.position.y = write.wheel_xy[i][1];
                tf.position.z = write.wheel_z[i];
            }
            if let Ok(mut s) = self.world.get::<&mut IsoSprite>(*we) {
                s.frame = write.direction as f32;
                s.frame_name = Self::frame_name(&self.frame_tables, &s.texture, s.frame);
                s.frame_offset.z = write.wheel_altitude[i];
                s.anchor = Vec2::from(write.wheel_anchors[i]);
            }
        }

        // Steering tires ride the same axle/vertical offset as their wheel but
        // select a frame by steer-major order (`steer_index · 8 + direction`).
        for (i, handle) in tire_handles.iter().enumerate() {
            let Some(te) = handle else { continue };
            if let Ok(mut tf) = self.world.get::<&mut Transform>(*te) {
                tf.position.x = write.wheel_xy[i][0];
                tf.position.y = write.wheel_xy[i][1];
                tf.position.z = write.wheel_z[i];
            }
            if let Ok(mut s) = self.world.get::<&mut IsoSprite>(*te) {
                let frame = (write.steer_index * DIRECTIONS + write.direction) as f32;
                s.frame = frame;
                s.frame_name = Self::frame_name(&self.frame_tables, &s.texture, frame);
                s.frame_offset.z = write.wheel_altitude[i];
                s.anchor = Vec2::from(write.wheel_anchors[i]);
            }
        }
    }
}
