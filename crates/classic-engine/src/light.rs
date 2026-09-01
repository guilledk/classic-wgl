//! Dynamic light handles + TTL: pooled lights are first-class ECS entities.
//!
//! A stable [`LightHandle`] (a slot index) maps to a spawned `Light` entity so
//! the guest ABI (`light_spawn`/`light_set`/`light_release`) stays stable while
//! lights become attachable world objects — declared in `state.json`, parented
//! to sprites, and driven by animation channels like any other component.

use classic_core::components::Light;

/// Handle to a dynamic light (a slot index into the handle table).
pub type LightHandle = u32;

/// Transient-light lifetime marker.  Present on guest-spawned lights with a
/// finite TTL; the engine decays it each frame and despawns expired lights.
pub struct LightTtl {
    /// Remaining lifetime in seconds.
    pub remaining: f32,
    /// Handle of the light entity, freed on expiry.
    pub handle: LightHandle,
}

/// A fixed-capacity handle→entity table with a free-list allocator.
///
/// Capacity is bounded by `classic_gfx::MAX_LIGHTS` (the UBO block size); a
/// spawn beyond capacity returns `None` rather than reallocating.
pub struct LightHandles {
    slots: Vec<Option<hecs::Entity>>,
    free: Vec<LightHandle>,
}

impl LightHandles {
    pub fn new() -> Self {
        Self { slots: Vec::new(), free: Vec::new() }
    }

    /// Spawn a light entity, returning its handle (or `None` when full).
    pub fn spawn(
        &mut self,
        world: &mut hecs::World,
        light: Light,
        ttl: Option<f32>,
    ) -> Option<LightHandle> {
        let handle = if let Some(h) = self.free.pop() {
            h
        } else if self.slots.len() >= classic_gfx::MAX_LIGHTS {
            return None;
        } else {
            let h = self.slots.len() as LightHandle;
            self.slots.push(None);
            h
        };
        let mut builder = hecs::EntityBuilder::new();
        builder.add(light);
        if let Some(ttl) = ttl {
            builder.add(LightTtl { remaining: ttl, handle });
        }
        let entity = world.spawn(builder.build());
        self.slots[handle as usize] = Some(entity);
        Some(handle)
    }

    /// Overwrite an active light's parameters by handle.
    pub fn set(&mut self, world: &mut hecs::World, handle: LightHandle, light: Light) -> bool {
        match self.slots.get(handle as usize) {
            Some(Some(e)) => world.insert(*e, (light,)).is_ok(),
            _ => false,
        }
    }

    /// Read an active light's parameters by handle.
    pub fn get(&self, world: &hecs::World, handle: LightHandle) -> Option<Light> {
        match self.slots.get(handle as usize) {
            Some(Some(e)) => world.get::<&Light>(*e).ok().map(|r| (*r).clone()),
            _ => None,
        }
    }

    /// Despawn a light entity and release its handle back to the free-list.
    pub fn release(&mut self, world: &mut hecs::World, handle: LightHandle) -> bool {
        if let Some(slot) = self.slots.get_mut(handle as usize) {
            if let Some(e) = slot.take() {
                let _ = world.despawn(e);
                self.free.push(handle);
                return true;
            }
        }
        false
    }

    /// Mark a handle as freed after its entity was despawned elsewhere (e.g. by
    /// TTL decay).
    fn free_handle(&mut self, handle: LightHandle) {
        if let Some(slot) = self.slots.get_mut(handle as usize) {
            if slot.is_some() {
                *slot = None;
                self.free.push(handle);
            }
        }
    }

    /// Advance transient-light TTLs by `dt` seconds, despawning expired lights
    /// and freeing their handles.
    pub fn decay(&mut self, world: &mut hecs::World, dt: f32) {
        if dt <= 0.0 {
            return;
        }
        let mut expired = Vec::new();
        for (e, ttl) in world.query::<&mut LightTtl>().iter() {
            ttl.remaining -= dt;
            if ttl.remaining <= 0.0 {
                expired.push((e, ttl.handle));
            }
        }
        for (e, handle) in expired {
            let _ = world.despawn(e);
            self.free_handle(handle);
        }
    }
}

impl Default for LightHandles {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use classic_core::components::LightKind;

    fn light() -> Light {
        Light {
            kind: LightKind::Point,
            position: glam::Vec3::new(10.0, 20.0, 0.0),
            color: [1.0, 0.5, 0.25],
            intensity: 2.0,
            radius: 100.0,
            dir: glam::Vec3::ZERO,
            cone_angle: 0.0,
            parent: None,
        }
    }

    fn active(world: &hecs::World) -> usize {
        world.query::<&Light>().iter().count()
    }

    #[test]
    fn spawn_reuse_and_capacity() {
        let mut world = hecs::World::new();
        let mut table = LightHandles::new();
        assert_eq!(active(&world), 0);

        let h0 = table.spawn(&mut world, light(), None).unwrap();
        let h1 = table.spawn(&mut world, light(), None).unwrap();
        assert_ne!(h0, h1);
        assert_eq!(active(&world), 2);

        assert!(table.set(&mut world, h1, light()));

        assert!(table.release(&mut world, h0));
        assert_eq!(active(&world), 1);
        assert!(!table.set(&mut world, h0, light()));
        assert!(!table.release(&mut world, h0));

        let h2 = table.spawn(&mut world, light(), None).unwrap();
        assert_eq!(h2, h0);

        assert!(table.release(&mut world, h1));
        assert!(table.release(&mut world, h2));
    }

    #[test]
    fn ttl_decay_releases_transient_lights() {
        let mut world = hecs::World::new();
        let mut table = LightHandles::new();
        let persistent = table.spawn(&mut world, light(), None).unwrap();
        let transient = table.spawn(&mut world, light(), Some(1.0)).unwrap();

        table.decay(&mut world, 0.6);
        assert_eq!(active(&world), 2);

        table.decay(&mut world, 0.5);
        assert_eq!(active(&world), 1);

        let reused = table.spawn(&mut world, light(), None).unwrap();
        assert_eq!(reused, transient);
        assert!(table.release(&mut world, persistent));
        assert!(table.release(&mut world, reused));
    }

    #[test]
    fn capacity_is_bounded() {
        let mut world = hecs::World::new();
        let mut table = LightHandles::new();
        let mut handles = Vec::new();
        for _ in 0..classic_gfx::MAX_LIGHTS {
            handles.push(table.spawn(&mut world, light(), None).unwrap());
        }
        assert_eq!(table.spawn(&mut world, light(), None), None);
        table.release(&mut world, handles[0]);
        assert!(table.spawn(&mut world, light(), None).is_some());
    }
}
