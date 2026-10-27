//! Lights and emitters a vehicle def carries (`VehicleDef::lights` /
//! `emitters`): ordinary `Light` / `Emitter` entities, parented to the
//! vehicle on a socket, spawned with every vehicle of the def (a scene's or
//! one spawned at runtime) and removed with it.
//!
//! They behave as a scene's socket lights and emitters do (the same
//! `gather_lights` / particle attachment), with one addition shared by both:
//! a light or emitter on a vehicle socket is scaled by the vehicle's
//! emissive strength (`vehicle_emissive`) up to 1, so `0` puts the lamps
//! out and a bloom strength above 1 leaves them at their authored
//! brightness.  Day and night are the guest's own lighting (`set_light`),
//! which local lights do not follow, here as in a scene.

use classic_core::components::{DebugName, IsoSprite, IsoVehicle};

use crate::Engine;

impl Engine {
    /// Spawn the def's lights and emitters for vehicle `name` (see the
    /// module docs); a name already taken is skipped.
    pub(super) fn spawn_vehicle_attachments(&mut self, def_name: &str, name: &str) {
        let Some(def) = self.vehicles.get(def_name) else { return };
        let (lights, emitters) = (def.lights.clone(), def.emitters.clone());
        let mut spawned = Vec::new();
        for (i, mut light) in lights.into_iter().enumerate() {
            let entity = format!("{name}__light{i}");
            if self.names.contains_key(&entity) {
                continue;
            }
            light.parent = Some(name.to_string());
            let e = self.world.spawn((light, DebugName(entity.clone())));
            self.names.insert(entity.clone(), e);
            self.name_order.push(entity.clone());
            spawned.push(entity);
        }
        for (i, mut emitter) in emitters.into_iter().enumerate() {
            let entity = format!("{name}__emitter{i}");
            if self.names.contains_key(&entity) {
                continue;
            }
            emitter.parent = Some(name.to_string());
            let e = self.world.spawn((emitter, DebugName(entity.clone())));
            self.names.insert(entity.clone(), e);
            self.name_order.push(entity.clone());
            spawned.push(entity);
        }
        if let Some(&ve) = self.names.get(name) {
            if let Ok(mut v) = self.world.get::<&mut IsoVehicle>(ve) {
                v.attachments = spawned;
            }
        }
    }

    /// The def-attachment names of vehicle entity `ve` (empty for anything
    /// else), for [`Engine::despawn_named`].
    pub(crate) fn vehicle_attachments(&self, ve: hecs::Entity) -> Vec<String> {
        self.world.get::<&IsoVehicle>(ve).map(|v| v.attachments.clone()).unwrap_or_default()
    }

    /// The factor a light or emitter on `parent`'s socket `node` is scaled
    /// by: the vehicle's emissive strength up to 1 (`1` off a socket, for
    /// anything but a vehicle, or a vehicle never given one; wheel dust,
    /// parented to the body without a socket, is not a lamp).
    pub(crate) fn vehicle_lamp_scale(&self, parent: Option<&str>, node: Option<&str>) -> f32 {
        let (Some(parent), Some(_)) = (parent, node) else { return 1.0 };
        // `vehicle_emissive` sets every part alike: the first that has one
        self.vehicle_part_entities(parent)
            .into_iter()
            .find_map(|e| self.world.get::<&IsoSprite>(e).ok().and_then(|s| s.emissive))
            .map_or(1.0, |s| s.clamp(0.0, 1.0))
    }
}
