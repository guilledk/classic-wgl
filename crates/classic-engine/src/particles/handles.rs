//! Guest emitter handles: `emitter_spawn`/`emitter_set`/`emitter_release`
//! mirror the light pool.  Each handle maps to a spawned [`Emitter`] entity,
//! so a guest emitter is an ordinary world emitter — the particle system
//! drives it exactly like one declared in `state.json`.

use classic_core::components::{DebugName, Emitter};

use crate::handles::{EntityHandles, Handle};
use crate::Engine;

/// Live guest emitters at once (each owns one GPU ring per particle layer).
pub const MAX_EMITTER_HANDLES: usize = 256;

/// The emitter handle→entity table.
pub struct EmitterHandles {
    slots: EntityHandles,
    /// Monotonic spawn counter naming each emitter entity.  An instance's PRNG
    /// stream and identity are keyed by the entity's debug name, and an
    /// unnamed entity's fallback name is its (recycled) index, so a released
    /// emitter still finishing its particles would collide with the next one
    /// spawned into the same index.  A never-reused serial keeps them apart
    /// and the spawn order deterministic.
    serial: u64,
}

impl EmitterHandles {
    pub fn new() -> Self {
        Self { slots: EntityHandles::new(MAX_EMITTER_HANDLES), serial: 0 }
    }
}

impl Default for EmitterHandles {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    /// Spawn a guest emitter entity, returning its handle — `None` when the
    /// effect is unknown or the pool is full.
    pub fn spawn_emitter(&mut self, emitter: Emitter) -> Option<Handle> {
        self.particles.resolve(&emitter.effect)?;
        let handle = self.emitter_handles.slots.reserve()?;
        let name = DebugName(format!("guest_emitter#{}", self.emitter_handles.serial));
        self.emitter_handles.serial += 1;
        let entity = self.world.spawn((emitter, name));
        self.emitter_handles.slots.bind(handle, entity);
        Some(handle)
    }

    /// Overwrite an active guest emitter by handle.
    pub fn update_emitter(&mut self, handle: Handle, emitter: Emitter) -> bool {
        match self.emitter_handles.slots.get(handle) {
            Some(e) => self.world.insert_one(e, emitter).is_ok(),
            None => false,
        }
    }

    /// Read an active guest emitter by handle.
    pub fn emitter_by_handle(&self, handle: Handle) -> Option<Emitter> {
        let e = self.emitter_handles.slots.get(handle)?;
        self.world.get::<&Emitter>(e).ok().map(|em| (*em).clone())
    }

    /// Despawn a guest emitter and recycle its handle.  Particles it already
    /// spawned live out their lifetimes.
    pub fn release_emitter(&mut self, handle: Handle) -> bool {
        match self.emitter_handles.slots.take(handle) {
            Some(e) => {
                let _ = self.world.despawn(e);
                true
            }
            None => false,
        }
    }
}
