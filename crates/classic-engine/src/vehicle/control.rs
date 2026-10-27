//! Vehicle control API: teleport, path goto/poll, stop/speed, and reachability
//! probes.

use classic_core::components::{IsoVehicle, Transform};
use classic_core::pathfinder::{unpack_waypoint, PathPoll, PoseGoal};
use classic_worker::{VehiclePathQuery, VehicleSnapshot};

use crate::hooks::BlockView;
use crate::Engine;

use super::kinematics::{dir_entry, dir_to_heading, pitch_index, search_footprint, straight_steer};
use super::{
    PreviewProbe, PreviewProbeState, VehicleGotoPoll, VehicleGotoSubmit, VehicleWrite, JUMP_COST,
    POSE_GEAR_COST, POSE_REVERSE_PENALTY,
};

impl Engine {
    /// Reposition a vehicle (body + 4 wheels) and zero its transient physics.
    /// Used on spawn and whenever the map re-rolls.
    pub fn vehicle_teleport(&mut self, name: &str, x: f32, y: f32) -> bool {
        let Some(&ve) = self.names.get(name) else { return false };
        let Some(terrain) = self.vehicle_terrain() else { return false };
        let wheel_handles = self.vehicle_wheel_handles(ve);
        let tire_handles = self.vehicle_tire_handles(ve);

        let write = {
            let Some(mut v) = self.world.get::<&mut IsoVehicle>(ve).ok() else { return false };
            v.vel_z = 0.0;
            v.airborne = false;
            v.heading = dir_to_heading(v.direction, v.directions);
            v.path.clear();
            v.path_idx = 0;
            v.path_reverse.clear();
            v.goal_heading = None;
            v.gear_wait = 0.0;
            v.steer_index = straight_steer(v.steer_levels);
            v.steer = 0.0;
            v.reversing = false;
            v.face = None;
            v.reverse_to = None;
            // A teleport is not a speed change: no dust burst next step.
            v.dust_speed = 0.0;

            let mut wheel_xy = [[0.0f32; 2]; 4];
            let mut wheel_z = [0.0f32; 4];
            for i in 0..4 {
                let [ox, oy] = match v.contact_offsets_m.get(i) {
                    Some(c) => super::layers::chassis_to_tile(*c, v.heading),
                    None => dir_entry(&v.wheel_tile_offsets[i], v.direction),
                };
                let wx = x + ox;
                let wy = y + oy;
                wheel_xy[i] = [wx, wy];
                let gh = terrain.ground(wx, wy);
                v.wheel_h[i] = gh;
                v.wheel_v[i] = 0.0;
                wheel_z[i] = gh;
            }
            // The body sits on the wheel-plane centre (mean of the four wheel
            // grounds), so a teleport onto a slope settles without flickering
            // through the level frame.
            v.altitude = (v.wheel_h[0] + v.wheel_h[1] + v.wheel_h[2] + v.wheel_h[3]) * 0.25;
            // Settle the body pitch/roll from the freshly-sampled wheel heights
            // so a teleport onto a slope doesn't flicker through the level frame.
            let front = (v.wheel_h[0] + v.wheel_h[1]) * 0.5;
            let rear = (v.wheel_h[2] + v.wheel_h[3]) * 0.5;
            v.pitch = ((front - rear) / v.wheelbase_m.max(1e-4)).atan();
            v.pitch_vel = 0.0;
            v.pitch_index = pitch_index(v.pitch, v.pitch_max, v.pitch_levels);
            let left = (v.wheel_h[0] + v.wheel_h[2]) * 0.5;
            let right = (v.wheel_h[1] + v.wheel_h[3]) * 0.5;
            v.roll = ((left - right) / v.track_m.max(1e-4)).atan();
            v.roll_vel = 0.0;
            v.roll_index = pitch_index(v.roll, v.roll_max, v.roll_levels);
            let mut wheel_anchors = [[0.0f32; 2]; 4];
            for (i, a) in wheel_anchors.iter_mut().enumerate() {
                *a = dir_entry(&v.wheel_anchors[i], v.direction);
            }
            let body_frame = ((v.pitch_index * v.roll_levels + v.roll_index) * v.directions
                + v.direction) as f32;

            VehicleWrite {
                body: [x, y, v.altitude],
                directions: v.directions,
                layered: !v.layers.is_empty(),
                body_altitude: v.altitude - terrain.height(x, y),
                body_anchor: dir_entry(&v.body_anchors, v.direction),
                direction: v.direction,
                body_frame,
                wheel_xy,
                wheel_altitude: [0.0; 4],
                wheel_z,
                wheel_anchors,
                steer_index: v.steer_index,
                heading: v.heading,
                motion_sign: 0.0,
                speed_mps: 0.0,
                dspeed_mps: 0.0,
                grounded: [true; 4],
                touchdown: None,
            }
        };

        self.apply_pose(ve, &wheel_handles, &tire_handles, &write);
        self.apply_layers(ve, &write, &terrain, 0.0);
        true
    }

    /// Set a vehicle's destination (integer tile coordinates).  Submits a
    /// footprint-, slope- and jump-aware A* request off-thread (or computes it
    /// inline under `synchronous_workers`) and returns a request id to poll
    /// with [`Engine::vehicle_goto_poll`].
    pub fn vehicle_goto(&mut self, name: &str, tx: i32, ty: i32) -> VehicleGotoSubmit {
        self.submit_goto(name, (tx, ty), None)
    }

    /// Drive a vehicle to tile point `(x, y)` arriving facing tile heading
    /// `heading_deg` (`0` = East, `90` = South).  The route (the
    /// `pathfinder` pose search) turns at the vehicle's turning radius
    /// (`speed / turn_rate`, or the def's wider `turn_radius_m`) and, when it can reverse, backs
    /// up where a forward arc has no room: a three-point or K-turn.  Poll
    /// with [`Engine::vehicle_goto_poll`]; the follower drives each forward
    /// and backward run in turn, steering, and stops at the point (a
    /// `pivot_turn` vehicle then turns in place onto the heading).
    /// `NoVehicle` for an unknown vehicle or a non-finite pose.
    pub fn vehicle_goto_pose(
        &mut self,
        name: &str,
        x: f32,
        y: f32,
        heading_deg: f32,
    ) -> VehicleGotoSubmit {
        if !(x.is_finite() && y.is_finite() && heading_deg.is_finite()) {
            return VehicleGotoSubmit::NoVehicle;
        }
        let heading = super::kinematics::wrap_pi(heading_deg.to_radians());
        self.submit_goto(name, (x.floor() as i32, y.floor() as i32), Some((x, y, heading)))
    }

    /// Submit a route to `to`, or to the pose `(x, y, heading)`.
    fn submit_goto(
        &mut self,
        name: &str,
        to: (i32, i32),
        pose: Option<(f32, f32, f32)>,
    ) -> VehicleGotoSubmit {
        let (tx, ty) = to;
        let Some(&ve) = self.names.get(name) else {
            classic_core::cl_info!(
                classic_core::instrument::Chan::Path,
                "vehicle_goto: no entity named {name}"
            );
            return VehicleGotoSubmit::NoVehicle;
        };
        let (footprint, pitch_max, roll_max, wheelbase_m, track_m, safe_fall_m, turn_cost, pose) = {
            let Ok(v) = self.world.get::<&IsoVehicle>(ve) else {
                return VehicleGotoSubmit::NoVehicle;
            };
            // The pose goal: turning at the follower's own radius (its turn
            // rate at speed; the follower steers onto the arcs, see
            // `POSE_LOOKAHEAD`), or the def's wider `turn_radius_m`.  Costs
            // are in time: a backward tile at the reverse speed (times a
            // penalty), a gear change as its stop and pull-away.
            let pose = pose.map(|(x, y, heading)| PoseGoal {
                x,
                y,
                heading,
                start_heading: v.heading,
                turn_radius: pose_turn_radius(&v),
                reverse: v.reverse_speed > 0.0,
                reverse_cost: POSE_REVERSE_PENALTY * v.speed / v.reverse_speed.max(1e-3),
                gear_cost: POSE_GEAR_COST + v.gear_change_s * v.speed,
            });
            // Reject new paths while airborne: the vehicle keeps its current
            // trajectory until its wheels touch down again (issue #40).
            if v.airborne {
                classic_core::cl_info!(
                    classic_core::instrument::Chan::Path,
                    "vehicle_goto {name}: airborne, rejecting"
                );
                return VehicleGotoSubmit::Airborne;
            }
            (
                search_footprint(&v),
                v.pitch_max,
                v.roll_max,
                v.wheelbase_m,
                v.track_m,
                v.safe_fall_m,
                v.turn_cost,
                pose,
            )
        };
        let from = {
            let Ok(tf) = self.world.get::<&Transform>(ve) else {
                return VehicleGotoSubmit::NoVehicle;
            };
            (tf.position.x.floor() as i32, tf.position.y.floor() as i32)
        };

        // Allocate a request id.  Shares the `next_path_id` counter with the
        // humanoid `request_path` so ids stay unique in the worker's result map.
        let id = self.next_path_id;
        self.next_path_id = self.next_path_id.wrapping_add(1);

        let query = VehiclePathQuery {
            from,
            to: (tx, ty),
            footprint,
            pitch_max,
            roll_max,
            wheelbase_m,
            track_m,
            safe_fall_m,
            jump_cost: JUMP_COST,
            turn_cost,
            pose,
            closed: Vec::new(),
        };
        self.submit_vehicle_path(id, query, name);

        self.vehicle_path_entities.insert(id, ve);
        if let Some(goal) = pose {
            self.vehicle_pose_goals.insert(id, goal.heading);
        }
        if let Ok(mut v) = self.world.get::<&mut IsoVehicle>(ve) {
            v.face = None;
            v.reverse_to = None;
        }
        VehicleGotoSubmit::Submitted(id)
    }

    /// Poll a previously submitted vehicle path request (non-blocking).  On
    /// acceptance, the route is installed on the vehicle (`v.path` /
    /// `v.path_idx`) here — the single mutation point shared by the sync and
    /// async paths.
    pub fn vehicle_goto_poll(&mut self, id: u64) -> VehicleGotoPoll {
        // a pose route's waypoints carry their gear (`unpack_waypoint`)
        let mut gears = Vec::new();
        let poll = if let Some(worker) = self.pathfinder.as_mut() {
            match worker.poll_vehicle_path(id) {
                PathPoll::Pending => VehicleGotoPoll::Pending,
                PathPoll::NoPath => VehicleGotoPoll::NoPath,
                PathPoll::Path(cells) => VehicleGotoPoll::Accepted(
                    cells
                        .into_iter()
                        .map(|c| {
                            let ((x, y), back) = unpack_waypoint(c);
                            gears.push(back);
                            [x, y]
                        })
                        .collect(),
                ),
            }
        } else {
            VehicleGotoPoll::Pending
        };

        match &poll {
            VehicleGotoPoll::Accepted(path) => {
                let goal_heading = self.vehicle_pose_goals.remove(&id);
                if let Some(ve) = self.vehicle_path_entities.remove(&id) {
                    if let Ok(mut v) = self.world.get::<&mut IsoVehicle>(ve) {
                        v.path = path.clone();
                        v.path_idx = if v.path.len() > 1 { 1 } else { 0 };
                        v.goal_heading = goal_heading;
                        v.gear_wait = 0.0;
                        v.path_reverse = if goal_heading.is_some() { gears } else { Vec::new() };
                        if let Some(h) = goal_heading {
                            classic_core::cl_info!(
                                classic_core::instrument::Chan::Path,
                                "pose route {id}: {} waypoints, {} backward, facing {:.0} deg: {:?}",
                                v.path.len(),
                                v.path_reverse.iter().filter(|&&b| b).count(),
                                h.to_degrees(),
                                v.path.iter().zip(&v.path_reverse).collect::<Vec<_>>()
                            );
                        }
                    }
                }
            }
            VehicleGotoPoll::NoPath => {
                self.vehicle_path_entities.remove(&id);
                self.vehicle_pose_goals.remove(&id);
            }
            VehicleGotoPoll::Pending => {}
        }

        poll
    }

    /// Stop a vehicle, clearing its movement path.
    pub fn vehicle_stop(&mut self, name: &str) -> bool {
        let Some(&ve) = self.names.get(name) else { return false };
        if let Ok(mut v) = self.world.get::<&mut IsoVehicle>(ve) {
            v.path.clear();
            v.path_idx = 0;
            v.path_reverse.clear();
            v.goal_heading = None;
            v.face = None;
            v.reverse_to = None;
            return true;
        }
        false
    }

    /// Turn a vehicle to face tile heading `heading_deg` (tile degrees: `0` =
    /// East, `90` = South), dropping its path.  A `pivot_turn` (tracked)
    /// vehicle turns in place at its `turn_rate`; any other snaps to the
    /// heading on the next step (it cannot turn without driving).  Returns
    /// `1` when accepted, `0` for an unknown vehicle, `-1` while airborne or
    /// for a non-finite heading.
    pub fn vehicle_face(&mut self, name: &str, heading_deg: f32) -> i32 {
        let Some(&ve) = self.names.get(name) else { return 0 };
        let Ok(mut v) = self.world.get::<&mut IsoVehicle>(ve) else { return 0 };
        if v.airborne || !heading_deg.is_finite() {
            return -1;
        }
        v.path.clear();
        v.path_idx = 0;
        v.path_reverse.clear();
        v.goal_heading = None;
        v.reverse_to = None;
        v.face = Some(super::kinematics::wrap_pi(heading_deg.to_radians()));
        1
    }

    /// Back a vehicle up in a straight line to tile point `(x, y)`, keeping
    /// its current heading (no steering): it drives along `-heading` at
    /// `reverse_speed` (its `speed` when reversing is off) until it has
    /// covered the point's distance along that line, then stops.  The point's
    /// sideways offset from the line is not corrected (line the vehicle up
    /// with [`Engine::vehicle_face`] first).  Drops its path.  Returns `1`
    /// when accepted, `0` for an unknown vehicle, `-1` while airborne, for a
    /// non-finite point or one that is not behind the vehicle.
    pub fn vehicle_reverse_to(&mut self, name: &str, x: f32, y: f32) -> i32 {
        let Some(&ve) = self.names.get(name) else { return 0 };
        let Some((bx, by)) =
            self.world.get::<&Transform>(ve).ok().map(|t| (t.position.x, t.position.y))
        else {
            return 0;
        };
        let Ok(mut v) = self.world.get::<&mut IsoVehicle>(ve) else { return 0 };
        if v.airborne || !x.is_finite() || !y.is_finite() {
            return -1;
        }
        let behind = -((x - bx) * v.heading.cos() + (y - by) * v.heading.sin());
        if behind <= 0.0 {
            return -1;
        }
        v.path.clear();
        v.path_idx = 0;
        v.path_reverse.clear();
        v.goal_heading = None;
        v.face = None;
        v.reverse_to = Some([x, y]);
        1
    }

    /// Set a vehicle's forward speed (tiles per second), mutating its
    /// `IsoVehicle.speed` (e.g. slow a loaded LRV); the reverse speed is
    /// [`Self::vehicle_set_reverse_speed`].  Returns `false` for an unknown
    /// vehicle.
    pub fn vehicle_set_speed(&mut self, name: &str, speed: f32) -> bool {
        let Some(&ve) = self.names.get(name) else { return false };
        if let Ok(mut v) = self.world.get::<&mut IsoVehicle>(ve) {
            v.speed = speed;
            return true;
        }
        false
    }

    /// Set a vehicle's reverse speed (tiles per second; `0` stops it
    /// reversing, a pose route then turns round instead of backing up).
    /// `vehicle_set_speed` sets the forward speed only: a guest that slows a
    /// loaded vehicle sets both.  `false` for an unknown vehicle or a
    /// non-finite speed.
    pub fn vehicle_set_reverse_speed(&mut self, name: &str, speed: f32) -> bool {
        let Some(&ve) = self.names.get(name) else { return false };
        if !speed.is_finite() {
            return false;
        }
        if let Ok(mut v) = self.world.get::<&mut IsoVehicle>(ve) {
            v.reverse_speed = speed.max(0.0);
            return true;
        }
        false
    }

    /// A vehicle's heading in tile degrees, `[0, 360)` (`0` East, `90`
    /// South): the simulation's continuous heading.  The hull's sprite frame
    /// shows only the direction at or below it (the blend to the next one
    /// is drawn, not stored), so a guest reading the frame is off by up to a
    /// direction step.  `None` for an unknown vehicle.
    pub fn vehicle_heading(&self, name: &str) -> Option<f32> {
        let &ve = self.names.get(name)?;
        let v = self.world.get::<&IsoVehicle>(ve).ok()?;
        Some(v.heading.to_degrees().rem_euclid(360.0) % 360.0)
    }

    /// Non-mutating vehicle reachability probe: run the same footprint-, slope-
    /// and jump-aware A* as [`Engine::vehicle_goto`] to `(tx, ty)` but do **not**
    /// install the route on the vehicle.  On success the waypoints are stored in
    /// `Engine::preview_paths[name]` for the demo overlay to draw.
    ///
    /// Return codes: `1` reachable, `-1` no path, `0` pending (search still
    /// running — call again), `-2` unknown vehicle.  Results are cached per
    /// target: a repeated call with the same `(name, tx, ty)` returns the cached
    /// answer without re-running A*, and a target change resubmits.
    pub fn vehicle_probe(&mut self, name: &str, tx: i32, ty: i32) -> i32 {
        let Some(&ve) = self.names.get(name) else { return -2 };
        let (footprint, pitch_max, roll_max, wheelbase_m, track_m, safe_fall_m, turn_cost) = {
            let Ok(v) = self.world.get::<&IsoVehicle>(ve) else { return -2 };
            (
                search_footprint(&v),
                v.pitch_max,
                v.roll_max,
                v.wheelbase_m,
                v.track_m,
                v.safe_fall_m,
                v.turn_cost,
            )
        };
        let from = {
            let Ok(tf) = self.world.get::<&Transform>(ve) else { return -2 };
            (tf.position.x.floor() as i32, tf.position.y.floor() as i32)
        };
        let target = (tx, ty);

        // Same target: return the cached answer, or poll the in-flight request.
        if let Some(p) = &self.preview_probe {
            if p.name == name && p.target == target {
                return match p.state {
                    PreviewProbeState::Done { reachable } => {
                        if reachable {
                            1
                        } else {
                            -1
                        }
                    }
                    PreviewProbeState::Pending { id } => self.poll_preview_probe(id),
                };
            }
        }

        // Different target (or none): drain any in-flight probe before
        // submitting a fresh one, so at most one worker result is outstanding at
        // a time (no orphaned results accumulate on rapid target changes).
        if let Some(id) = self.preview_probe.as_ref().and_then(|p| match p.state {
            PreviewProbeState::Pending { id } => Some(id),
            PreviewProbeState::Done { .. } => None,
        }) {
            if self.poll_preview_probe(id) == 0 {
                return 0; // old probe still running; defer the new target
            }
        }
        self.preview_paths.remove(name);

        // New target (or first call): submit a fresh probe.
        let id = self.next_path_id;
        self.next_path_id = self.next_path_id.wrapping_add(1);
        let query = VehiclePathQuery {
            from,
            to: target,
            footprint,
            pitch_max,
            roll_max,
            wheelbase_m,
            track_m,
            safe_fall_m,
            jump_cost: JUMP_COST,
            turn_cost,
            pose: None,
            closed: Vec::new(),
        };
        self.submit_vehicle_path(id, query, name);
        self.preview_probe = Some(PreviewProbe {
            name: name.to_string(),
            target,
            state: PreviewProbeState::Pending { id },
        });
        // A synchronous pathfinder has already answered (`1`/`-1`); a background
        // one is still searching (`0`).
        self.poll_preview_probe(id)
    }

    /// Submit `vehicle`'s path request to the pathfinder.  A synchronous
    /// pathfinder searches a snapshot freshly built from the live world (so the
    /// deterministic path sees current heights and `blocks_nav` obstacles),
    /// with the vehicle blocks as `vehicle` sees them.  A background one
    /// searches the snapshot last shared, plus the query's `closed` rects (the
    /// blocks other vehicles own): the main thread builds no grid per request.
    fn submit_vehicle_path(&mut self, id: u64, mut query: VehiclePathQuery, vehicle: &str) {
        self.ensure_pathfinder();
        let synchronous = self.pathfinder.as_ref().is_some_and(|w| w.is_synchronous());
        let snapshot = if synchronous {
            let obstacles = self.compute_nav_obstacles();
            VehicleSnapshot::Fresh(
                self.build_vehicle_nav_snapshot_for(&obstacles, BlockView::Vehicle(vehicle)),
            )
        } else {
            query.closed = self.closed_vehicle_blocks(vehicle);
            VehicleSnapshot::Current
        };
        if let Some(worker) = self.pathfinder.as_mut() {
            worker.request_vehicle(id, query, snapshot);
        }
    }

    /// Poll an in-flight preview probe by id, finalising `preview_probe` (and
    /// `preview_paths`) when the worker resolves it.  The resolved outcome is
    /// cached as `Done { reachable }` so a repeated identical call returns the
    /// answer without re-running A* (see `vehicle_probe`'s per-target cache).
    fn poll_preview_probe(&mut self, id: u64) -> i32 {
        let poll = if let Some(worker) = self.pathfinder.as_mut() {
            worker.poll_vehicle_path(id)
        } else {
            PathPoll::Pending
        };
        let (name, target) = match self.preview_probe.as_ref() {
            Some(p) => (p.name.clone(), p.target),
            None => (String::new(), (0, 0)),
        };
        match poll {
            PathPoll::Pending => 0,
            PathPoll::NoPath => {
                classic_core::cl_debug!(
                    classic_core::instrument::Chan::Path,
                    "vehicle_probe {name} -> {target:?}: no path"
                );
                self.preview_paths.remove(&name);
                self.preview_probe = Some(PreviewProbe {
                    name,
                    target,
                    state: PreviewProbeState::Done { reachable: false },
                });
                -1
            }
            PathPoll::Path(cells) => {
                classic_core::cl_debug!(
                    classic_core::instrument::Chan::Path,
                    "vehicle_probe {name} -> {target:?}: {} waypoints",
                    cells.len()
                );
                let path: Vec<[i32; 2]> = cells.into_iter().map(|(x, y)| [x, y]).collect();
                self.preview_paths.insert(name.clone(), path);
                self.preview_probe = Some(PreviewProbe {
                    name,
                    target,
                    state: PreviewProbeState::Done { reachable: true },
                });
                1
            }
        }
    }

    /// Clear a vehicle's drop-preview state (candidate path + cached/in-flight
    /// probe), e.g. when the guest leaves preview mode.  Returns `true` when
    /// anything was cleared.  An in-flight probe is best-effort drained first so
    /// a ready result doesn't linger.
    pub fn vehicle_probe_clear(&mut self, name: &str) -> bool {
        let pending_id = match self.preview_probe.as_ref() {
            Some(p) if p.name == name => match p.state {
                PreviewProbeState::Pending { id } => Some(id),
                PreviewProbeState::Done { .. } => None,
            },
            _ => None,
        };
        let had_probe = self.preview_probe.as_ref().map(|p| p.name == name).unwrap_or(false);
        if let Some(id) = pending_id {
            let _ = self.poll_preview_probe(id);
        }
        if self.preview_probe.as_ref().map(|p| p.name == name).unwrap_or(false) {
            self.preview_probe = None;
        }
        let had_path = self.preview_paths.remove(name).is_some();
        had_probe || had_path
    }

    /// The max tile radius of a vehicle's collision `path_footprint` (the
    /// Chebyshev extent `max(|dx|, |dy|)` over its integer cell offsets).
    /// Guests use it to derive a pick-up/drop clearance from the vehicle's real
    /// footprint instead of guessing a fixed offset.  Returns `-1.0` when the
    /// vehicle is unknown.
    pub fn vehicle_footprint_radius(&self, name: &str) -> f64 {
        let Some(&ve) = self.names.get(name) else { return -1.0 };
        let Ok(v) = self.world.get::<&IsoVehicle>(ve) else { return -1.0 };
        v.path_footprint.iter().map(|(dx, dy)| dx.abs().max(dy.abs())).max().unwrap_or(0) as f64
    }
}

/// The radius (tiles) a pose route plans `v`'s arcs at: the follower's own
/// (`speed / turn_rate`), or the def's `turn_radius_m` when wider.
pub(super) fn pose_turn_radius(v: &IsoVehicle) -> f32 {
    let own = v.speed / v.turn_rate.max(1e-3);
    v.turn_radius.map_or(own, |r| r.max(own)).max(1.0)
}
