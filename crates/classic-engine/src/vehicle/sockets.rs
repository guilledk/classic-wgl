//! Vehicle sockets (`VehicleDef::sockets`): named attachment frames on a
//! layer, for lights, emitters and one-shot effects.

use classic_core::components::IsoVehicle;
use classic_core::vehicle_def::LayerKind;
use glam::{Mat3, Vec3};

use super::layers::layer_poses;
use crate::Engine;

/// Rotation about the chassis `y` axis by `a` (nose-up positive: `x` toward
/// `z`).
pub(super) fn rot_pitch(a: f32) -> Mat3 {
    let (s, c) = a.sin_cos();
    Mat3::from_cols(Vec3::new(c, 0.0, s), Vec3::Y, Vec3::new(-s, 0.0, c))
}

/// Rotation about the chassis `x` axis by `a` (left-up positive: `y` toward
/// `z`).
pub(super) fn rot_roll(a: f32) -> Mat3 {
    let (s, c) = a.sin_cos();
    Mat3::from_cols(Vec3::X, Vec3::new(0.0, c, s), Vec3::new(0.0, -s, c))
}

/// Rotation about the chassis `z` axis by a tile-space yaw (clockwise on
/// screen = negative about +Z).
fn rot_yaw(tile_yaw: f32) -> Mat3 {
    Mat3::from_rotation_z(-tile_yaw)
}

/// Chassis axes (`x` forward, `y` left, `z` up) in world metres for a tile
/// heading (`world = (tx, -ty) · TILE_M`).
fn chassis_to_world(heading: f32) -> Mat3 {
    let (s, c) = heading.sin_cos();
    Mat3::from_cols(Vec3::new(c, -s, 0.0), Vec3::new(s, c, 0.0), Vec3::Z)
}

impl Engine {
    /// A vehicle socket's world frame: its origin (world metres, +Z up, the
    /// space of lights and effects) and its axes (`x` forward, `y` left, `z`
    /// up).  A hull socket turns with the heading, pitch and roll; a turret
    /// socket also with the turret yaw and its gun pitch about `pitch_pivot`.
    pub fn vehicle_socket_frame(&self, vehicle: &str, socket: &str) -> Option<(Vec3, Mat3)> {
        let &ve = self.names.get(vehicle)?;
        self.vehicle_socket_frame_of(ve, socket)
    }

    pub(crate) fn vehicle_socket_frame_of(
        &self,
        ve: hecs::Entity,
        socket: &str,
    ) -> Option<(Vec3, Mat3)> {
        let v = self.world.get::<&IsoVehicle>(ve).ok()?;
        let def = self.vehicles.get(&v.def_name)?;
        let s = def.sockets.get(socket)?;
        let li = def.layers.iter().position(|l| l.name == s.layer)?;
        let layer = &def.layers[li];
        let state = v.layers.iter().find(|st| st.def_index == li);
        let yaws: Vec<f32> = (0..def.layers.len())
            .map(|i| v.layers.iter().find(|st| st.def_index == i).map_or(0.0, |st| st.yaw))
            .collect();
        let pose = layer_poses(def, &yaws)[li];
        // In the layer frame: a turret pitches its sockets about the pivot.
        let o = Vec3::from_array(s.offset);
        let (local, local_basis) = if layer.kind == LayerKind::Turret {
            let p = state.map_or(0.0, |st| st.pitch);
            let pivot = Vec3::from_array(layer.pitch_pivot.unwrap_or([0.0; 3]));
            let r = rot_pitch(p);
            (pivot + r * (o - pivot), r)
        } else {
            (o, Mat3::IDENTITY)
        };
        let yaw = rot_yaw(pose.yaw);
        let chassis = pose.pos + yaw * local;
        let tilt = rot_pitch(v.pitch) * rot_roll(v.roll);
        let to_world = chassis_to_world(v.heading);
        let tf = self.world.get::<&classic_core::components::Transform>(ve).ok()?;
        // `iso_to_world` adds the terrain under the point: pass the lift above it.
        let lift = v.altitude - self.height_at(tf.position.x, tf.position.y);
        let base = self.iso_to_world(tf.position.x, tf.position.y, lift)?;
        let basis = to_world * tilt * yaw * local_basis;
        Some((base + to_world * tilt * chassis, basis))
    }

    /// A vehicle socket's world position (metres, +Z up).
    pub fn vehicle_socket_pos(&self, vehicle: &str, socket: &str) -> Option<Vec3> {
        self.vehicle_socket_frame(vehicle, socket).map(|f| f.0)
    }

    /// Fire a one-shot effect at a vehicle socket, oriented by its full frame
    /// (heading, pitch / roll, turret yaw, gun pitch): the effect's local `+x`
    /// is the socket's forward axis.
    pub fn vehicle_socket_fx(&mut self, vehicle: &str, socket: &str, effect: &str) -> bool {
        let Some((pos, basis)) = self.vehicle_socket_frame(vehicle, socket) else {
            return false;
        };
        self.fx_emit_oriented(effect, pos, basis, 1.0)
    }
}
