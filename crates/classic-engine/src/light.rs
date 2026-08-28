//! Dynamic light pool: a fixed-capacity free-list of [`Light`]s with optional
//! per-light TTL decay for transient effects (flashes, explosions, rocket
//! booster burns).

use classic_core::components::Light;

/// Handle to a pooled light (a slot index into the pool).
pub type LightHandle = u32;

struct LightSlot {
    active: bool,
    light: Light,
    /// Remaining lifetime in seconds; `None` = persistent (no auto-release).
    ttl: Option<f32>,
}

/// A fixed-capacity pool of dynamic lights with a free-list allocator.
///
/// Capacity is bounded by `classic_gfx::MAX_LIGHTS` (the UBO block size); a
/// spawn beyond capacity returns `None` rather than reallocating.
pub struct LightPool {
    slots: Vec<LightSlot>,
    free: Vec<LightHandle>,
}

impl LightPool {
    pub fn new() -> Self {
        Self { slots: Vec::new(), free: Vec::new() }
    }

    /// Allocate a light slot, returning its handle (or `None` when full).
    pub fn spawn(&mut self, light: Light, ttl: Option<f32>) -> Option<LightHandle> {
        if let Some(idx) = self.free.pop() {
            let slot = &mut self.slots[idx as usize];
            slot.active = true;
            slot.light = light;
            slot.ttl = ttl;
            return Some(idx);
        }
        if self.slots.len() >= classic_gfx::MAX_LIGHTS {
            return None;
        }
        let idx = self.slots.len() as u32;
        self.slots.push(LightSlot { active: true, light, ttl });
        Some(idx)
    }

    /// Overwrite an active light's parameters by handle.
    pub fn set(&mut self, handle: LightHandle, light: Light) -> bool {
        match self.slots.get_mut(handle as usize) {
            Some(slot) if slot.active => {
                slot.light = light;
                true
            }
            _ => false,
        }
    }

    /// Release a light back to the free-list.
    pub fn release(&mut self, handle: LightHandle) -> bool {
        match self.slots.get_mut(handle as usize) {
            Some(slot) if slot.active => {
                slot.active = false;
                slot.ttl = None;
                self.free.push(handle);
                true
            }
            _ => false,
        }
    }

    /// Advance transient-light TTLs by `dt` seconds, releasing expired lights.
    pub fn decay(&mut self, dt: f32) {
        if dt <= 0.0 {
            return;
        }
        let mut expired = Vec::new();
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if !slot.active {
                continue;
            }
            if let Some(ttl) = slot.ttl.as_mut() {
                *ttl -= dt;
                if *ttl <= 0.0 {
                    slot.active = false;
                    slot.ttl = None;
                    expired.push(i as LightHandle);
                }
            }
        }
        self.free.extend(expired);
    }

    /// Collect the active lights in stable slot order (matches the UBO order).
    pub fn gather(&self) -> Vec<Light> {
        self.slots.iter().filter(|s| s.active).map(|s| s.light).collect()
    }

    /// Number of currently active lights.
    pub fn active_count(&self) -> usize {
        self.slots.iter().filter(|s| s.active).count()
    }
}

impl Default for LightPool {
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
        }
    }

    #[test]
    fn spawn_reuse_and_capacity() {
        let mut pool = LightPool::new();
        assert_eq!(pool.active_count(), 0);

        let h0 = pool.spawn(light(), None).unwrap();
        let h1 = pool.spawn(light(), None).unwrap();
        assert_ne!(h0, h1);
        assert_eq!(pool.active_count(), 2);

        // Set on an active handle succeeds.
        assert!(pool.set(h1, light()));

        // Release h0; a released handle rejects further set/release.
        assert!(pool.release(h0));
        assert_eq!(pool.active_count(), 1);
        assert!(!pool.set(h0, light()));
        assert!(!pool.release(h0));

        // Re-spawn reuses the freed slot.
        let h2 = pool.spawn(light(), None).unwrap();
        assert_eq!(h2, h0);

        assert!(pool.release(h1));
        assert!(pool.release(h2));
    }

    #[test]
    fn ttl_decay_releases_transient_lights() {
        let mut pool = LightPool::new();
        let persistent = pool.spawn(light(), None).unwrap();
        let transient = pool.spawn(light(), Some(1.0)).unwrap();

        pool.decay(0.6);
        assert_eq!(pool.active_count(), 2);

        pool.decay(0.5);
        assert_eq!(pool.active_count(), 1);

        // The freed slot is reusable.
        let reused = pool.spawn(light(), None).unwrap();
        assert_eq!(reused, transient);
        assert!(pool.release(persistent));
        assert!(pool.release(reused));
    }

    #[test]
    fn gather_matches_slot_order() {
        let mut pool = LightPool::new();
        let mut a = light();
        let mut b = light();
        a.position.x = 1.0;
        b.position.x = 2.0;
        pool.spawn(a, None);
        pool.spawn(b, None);

        let gathered = pool.gather();
        assert_eq!(gathered.len(), 2);
        assert_eq!(gathered[0].position.x, 1.0);
        assert_eq!(gathered[1].position.x, 2.0);
    }

    #[test]
    fn capacity_is_bounded() {
        let mut pool = LightPool::new();
        let mut handles = Vec::new();
        for _ in 0..classic_gfx::MAX_LIGHTS {
            handles.push(pool.spawn(light(), None).unwrap());
        }
        assert_eq!(pool.spawn(light(), None), None);
        // Releasing one frees capacity for another.
        pool.release(handles[0]);
        assert!(pool.spawn(light(), None).is_some());
    }
}
