//! Stable guest handles over ECS entities: a fixed-capacity slot table with a
//! free-list allocator.
//!
//! Guest ABIs that hand out integer handles (`light_spawn`, `emitter_spawn`)
//! map each handle to a spawned entity here, so the handle stays valid while
//! the entity is a first-class world object.  A released handle is recycled
//! by the next allocation; a handle past the capacity is never minted.

/// A handle: a slot index into an [`EntityHandles`] table.
pub type Handle = u32;

/// A handle→entity slot table with a free-list allocator, bounded by `cap`.
pub struct EntityHandles {
    slots: Vec<Option<hecs::Entity>>,
    free: Vec<Handle>,
    cap: usize,
}

impl EntityHandles {
    /// An empty table that mints at most `cap` live handles.
    pub fn new(cap: usize) -> Self {
        Self { slots: Vec::new(), free: Vec::new(), cap }
    }

    /// Reserve a handle for an entity about to be spawned, or `None` when the
    /// table is full.  Pair with [`bind`](Self::bind).
    pub fn reserve(&mut self) -> Option<Handle> {
        if let Some(h) = self.free.pop() {
            return Some(h);
        }
        if self.slots.len() >= self.cap {
            return None;
        }
        self.slots.push(None);
        Some((self.slots.len() - 1) as Handle)
    }

    /// Bind a reserved handle to its entity.
    pub fn bind(&mut self, handle: Handle, entity: hecs::Entity) {
        self.slots[handle as usize] = Some(entity);
    }

    /// The entity behind an active handle.
    pub fn get(&self, handle: Handle) -> Option<hecs::Entity> {
        self.slots.get(handle as usize).copied().flatten()
    }

    /// Unbind an active handle, returning its entity and recycling the slot.
    pub fn take(&mut self, handle: Handle) -> Option<hecs::Entity> {
        let entity = self.slots.get_mut(handle as usize)?.take()?;
        self.free.push(handle);
        Some(entity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_recycle_and_respect_capacity() {
        let mut world = hecs::World::new();
        let mut t = EntityHandles::new(2);
        let a = t.reserve().unwrap();
        t.bind(a, world.spawn(()));
        let b = t.reserve().unwrap();
        t.bind(b, world.spawn(()));
        assert_eq!((a, b), (0, 1));
        assert_eq!(t.reserve(), None, "full");
        assert!(t.take(a).is_some());
        assert_eq!(t.take(a), None, "double release");
        assert_eq!(t.get(a), None);
        assert_eq!(t.reserve(), Some(a), "a released slot is recycled");
        assert_eq!(t.get(7), None, "out-of-range handle");
    }
}
