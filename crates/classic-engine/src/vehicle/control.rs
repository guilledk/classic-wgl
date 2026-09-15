//! Vehicle control API: teleport, path goto/poll, stop/speed, and reachability
//! probes.

use classic_core::components::{IsoVehicle, Transform};
use classic_core::pathfinder::PathPoll;

use crate::Engine;

use super::kinematics::{dir_to_heading, pitch_index, straight_steer};
use super::{
    PreviewProbe, PreviewProbeState, VehicleGotoPoll, VehicleGotoSubmit, VehicleWrite, DIRECTIONS,
    JUMP_COST,
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
            let direction = v.direction as usize;
            v.vel_z = 0.0;
            v.airborne = false;
            v.heading = dir_to_heading(v.direction);
            v.path.clear();
            v.path_idx = 0;
            v.steer_index = straight_steer(v.steer_levels);
            v.steer = 0.0;
            v.reversing = false;

            let mut wheel_xy = [[0.0f32; 2]; 4];
            let mut wheel_z = [0.0f32; 4];
            for i in 0..4 {
                let (ox, oy) =
                    (v.wheel_tile_offsets[i][direction][0], v.wheel_tile_offsets[i][direction][1]);
                let wx = x + ox;
                let wy = y + oy;
                wheel_xy[i] = [wx, wy];
                let gh = terrain.height(wx, wy);
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
                *a = v.wheel_anchors[i][direction];
            }
            let body_frame =
                ((v.pitch_index * v.roll_levels + v.roll_index) * DIRECTIONS + v.direction) as f32;

            VehicleWrite {
                body: [x, y, v.altitude],
                body_altitude: v.altitude - terrain.height(x, y),
                body_anchor: v.body_anchors[direction],
                direction: v.direction,
                body_frame,
                wheel_xy,
                wheel_altitude: [0.0; 4],
                wheel_z,
                wheel_anchors,
                steer_index: v.steer_index,
            }
        };

        self.apply_pose(ve, &wheel_handles, &tire_handles, &write);
        true
    }

    /// Set a vehicle's destination (integer tile coordinates).  Submits a
    /// footprint-, slope- and jump-aware A* request off-thread (or computes it
    /// inline under `synchronous_workers`) and returns a request id to poll
    /// with [`Engine::vehicle_goto_poll`].
    pub fn vehicle_goto(&mut self, name: &str, tx: i32, ty: i32) -> VehicleGotoSubmit {
        let Some(&ve) = self.names.get(name) else {
            classic_core::cl_info!(
                classic_core::instrument::Chan::Path,
                "vehicle_goto: no entity named {name}"
            );
            return VehicleGotoSubmit::NoVehicle;
        };
        let (footprint, pitch_max, roll_max, wheelbase_m, track_m, safe_fall_m, turn_cost) = {
            let Ok(v) = self.world.get::<&IsoVehicle>(ve) else {
                return VehicleGotoSubmit::NoVehicle;
            };
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
                v.path_footprint.clone(),
                v.pitch_max,
                v.roll_max,
                v.wheelbase_m,
                v.track_m,
                v.safe_fall_m,
                v.turn_cost,
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

        if !self.synchronous_workers {
            self.ensure_pathfinder();
            if let Some(worker) = self.pathfinder.as_mut() {
                worker.request_vehicle_path(
                    id,
                    from,
                    (tx, ty),
                    footprint,
                    pitch_max,
                    roll_max,
                    wheelbase_m,
                    track_m,
                    safe_fall_m,
                    JUMP_COST,
                    turn_cost,
                );
            }
        } else {
            let poll = match self.find_vehicle_path(
                from,
                (tx, ty),
                &footprint,
                pitch_max,
                roll_max,
                wheelbase_m,
                track_m,
                safe_fall_m,
                JUMP_COST,
                turn_cost,
            ) {
                Some(path) => {
                    VehicleGotoPoll::Accepted(path.into_iter().map(|(x, y)| [x, y]).collect())
                }
                None => VehicleGotoPoll::NoPath,
            };
            self.sync_vehicle_paths.insert(id, poll);
        }

        self.vehicle_path_entities.insert(id, ve);
        VehicleGotoSubmit::Submitted(id)
    }

    /// Poll a previously submitted vehicle path request (non-blocking).  On
    /// acceptance, the route is installed on the vehicle (`v.path` /
    /// `v.path_idx`) here — the single mutation point shared by the sync and
    /// async paths.
    pub fn vehicle_goto_poll(&mut self, id: u64) -> VehicleGotoPoll {
        let poll = if let Some(poll) = self.sync_vehicle_paths.remove(&id) {
            poll
        } else if let Some(worker) = self.pathfinder.as_mut() {
            match worker.poll_vehicle_path(id) {
                PathPoll::Pending => VehicleGotoPoll::Pending,
                PathPoll::NoPath => VehicleGotoPoll::NoPath,
                PathPoll::Path(cells) => {
                    VehicleGotoPoll::Accepted(cells.into_iter().map(|(x, y)| [x, y]).collect())
                }
            }
        } else {
            VehicleGotoPoll::Pending
        };

        match &poll {
            VehicleGotoPoll::Accepted(path) => {
                if let Some(ve) = self.vehicle_path_entities.remove(&id) {
                    if let Ok(mut v) = self.world.get::<&mut IsoVehicle>(ve) {
                        v.path = path.clone();
                        v.path_idx = if v.path.len() > 1 { 1 } else { 0 };
                    }
                }
            }
            VehicleGotoPoll::NoPath => {
                self.vehicle_path_entities.remove(&id);
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
            return true;
        }
        false
    }

    /// Set a vehicle's speed (tiles per second), mutating its `IsoVehicle.speed`
    /// (e.g. slow a loaded LRV).  Returns `false` for an unknown vehicle.
    pub fn vehicle_set_speed(&mut self, name: &str, speed: f32) -> bool {
        let Some(&ve) = self.names.get(name) else { return false };
        if let Ok(mut v) = self.world.get::<&mut IsoVehicle>(ve) {
            v.speed = speed;
            return true;
        }
        false
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
                v.path_footprint.clone(),
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
        if self.synchronous_workers {
            let path = self.find_vehicle_path(
                from,
                target,
                &footprint,
                pitch_max,
                roll_max,
                wheelbase_m,
                track_m,
                safe_fall_m,
                JUMP_COST,
                turn_cost,
            );
            let found = path.is_some();
            self.preview_probe = Some(PreviewProbe {
                name: name.to_string(),
                target,
                state: PreviewProbeState::Done { reachable: found },
            });
            if let Some(path) = path {
                self.preview_paths
                    .insert(name.to_string(), path.into_iter().map(|(x, y)| [x, y]).collect());
                return 1;
            }
            self.preview_paths.remove(name);
            return -1;
        }

        self.ensure_pathfinder();
        if let Some(worker) = self.pathfinder.as_mut() {
            worker.request_vehicle_path(
                id,
                from,
                target,
                footprint,
                pitch_max,
                roll_max,
                wheelbase_m,
                track_m,
                safe_fall_m,
                JUMP_COST,
                turn_cost,
            );
        }
        self.preview_probe = Some(PreviewProbe {
            name: name.to_string(),
            target,
            state: PreviewProbeState::Pending { id },
        });
        0
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
