//! Wheel dust and touchdown puffs for vehicles that opt in
//! (`IsoVehicle::dust_effect` / `touchdown_effect`, from the `VehicleDef`).
//!
//! Each wheel owns an ordinary `Emitter` entity, `<body>Dust{Fl,Fr,Rl,Rr}`,
//! parented to the body (so a hidden or removed vehicle goes quiet), spawned
//! the first frame the vehicle has a dust effect (which is read then; later
//! changes to `dust_effect` do not swap a live emitter's effect).  Every
//! simulation step moves
//! it to the wheel's ground contact and drives it from the chassis state:
//!
//! - **rate + launch speed ∝ ground speed** (`speed / DUST_REF_SPEED`), so a
//!   crawling rover trickles and a fast one throws long arcs;
//! - **bursts ∝ |Δspeed|** through the cumulative `burst` counter, so pulling
//!   away, braking and speed steps kick up a puff on top of the rate;
//! - **direction**: up, leaning back against the travel direction — the
//!   world velocity of a rolling tyre's trailing edge is forward (inherited by
//!   the effect's layers) plus the tangential spin, which there points up;
//! - only wheels touching the ground emit; airborne, none do.
//!
//! On the step the body lands from an airborne arc, the touchdown effect fires
//! once under the body, scaled by the impact speed.  Everything derives from
//! the simulation state, so it is as deterministic as the sim under `fixed_dt`.

use classic_core::components::{DebugName, Emitter, IsoVehicle};
use glam::Vec3;

use crate::Engine;

use super::{
    VehicleWrite, DUST_BACK_LEAN, DUST_BURSTS_PER_MPS, DUST_MIN_SPEED_MUL, DUST_REF_SPEED,
    TOUCHDOWN_MIN_IMPACT, TOUCHDOWN_REF_IMPACT, WHEEL_SUFFIXES,
};

impl Engine {
    /// Drive a vehicle's wheel-dust emitters and touchdown puff from this
    /// step's pose (see the module docs).  A no-op for vehicles without either
    /// effect, so scenes that never opt in are untouched.
    pub(super) fn drive_vehicle_fx(&mut self, ve: hecs::Entity, w: &VehicleWrite) {
        let (dust, touchdown) = match self.world.get::<&IsoVehicle>(ve) {
            Ok(v) => (v.dust_effect.clone(), v.touchdown_effect.clone()),
            Err(_) => return,
        };
        if dust.is_none() && touchdown.is_none() {
            return;
        }
        let Some(body_ground) = self.iso_to_world(w.body[0], w.body[1], 0.0) else { return };

        if let (Some(fx), Some(impact)) = (touchdown, w.touchdown) {
            if impact >= TOUCHDOWN_MIN_IMPACT {
                let scale = (impact / TOUCHDOWN_REF_IMPACT).clamp(0.5, 2.0);
                self.fx_emit(&fx, body_ground, Vec3::Z, scale);
            }
        }

        let Some(fx) = dust else { return };
        let body = self.debug_name(ve);
        // A parented emitter's `position` is an offset from the parent's ground
        // point plus its frame offset (the chassis lift, written by
        // `apply_pose` just before this).
        let base = body_ground + self.parent_frame_offset(ve);
        let travel = {
            let (s, c) = w.heading.sin_cos();
            let ahead = self.iso_to_world(w.body[0] + c, w.body[1] + s, 0.0);
            ahead
                .map(|a| Vec3::new(a.x - body_ground.x, a.y - body_ground.y, 0.0))
                .and_then(|d| d.try_normalize())
                .unwrap_or(Vec3::X)
                * w.motion_sign
        };
        let dir = (Vec3::Z - travel * DUST_BACK_LEAN).normalize();
        let speed_mul = w.speed_mps / DUST_REF_SPEED;
        for (i, suffix) in WHEEL_SUFFIXES.iter().enumerate() {
            let [wx, wy] = w.wheel_xy[i];
            // On a raised floor the dust rises from the floor, not the terrain.
            let lift = self.ground_at(wx, wy) - self.height_at(wx, wy);
            let Some(contact) = self.iso_to_world(wx, wy, lift) else {
                continue;
            };
            let name = format!("{body}Dust{suffix}");
            let entity = match self.names.get(&name) {
                Some(&e) => e,
                None => {
                    let mut em = Emitter::new(fx.clone(), contact - base);
                    em.parent = Some(body.clone());
                    em.rate = 0.0;
                    let e = self.world.spawn((em, DebugName(name.clone())));
                    self.names.insert(name.clone(), e);
                    self.name_order.push(name);
                    e
                }
            };
            let Ok(mut em) = self.world.get::<&mut Emitter>(entity) else { continue };
            em.position = contact - base;
            em.dir = dir;
            em.speed = speed_mul.max(DUST_MIN_SPEED_MUL);
            if w.grounded[i] {
                em.rate = speed_mul;
                em.burst += w.dspeed_mps * DUST_BURSTS_PER_MPS;
            } else {
                em.rate = 0.0;
            }
        }
    }
}
