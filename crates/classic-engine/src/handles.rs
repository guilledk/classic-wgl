//! Stable guest handles over ECS entities: a fixed-capacity, generational
//! slot table with a free-list allocator.
//!
//! Guest ABIs that hand out integer handles (`light_spawn`, `emitter_spawn`)
//! map each handle to a spawned entity here, so the handle stays valid while
//! the entity is a first-class world object.  A released slot is recycled by
//! the next allocation, but under a bumped **generation**: a handle is
//! `(generation << INDEX_BITS) | index`, and [`get`](EntityHandles::get) /
//! [`take`](EntityHandles::take) reject a handle whose generation no longer
//! matches its slot.  A guest that keeps a handle past its release therefore
//! gets "unknown handle" instead of silently driving whatever reused the slot
//! (the ABA problem).
//!
//! The encoding fits a non-negative `i32` (bit 31 is never set), so `-1`
//! stays the ABI's failure value and a generation-0 handle equals its slot
//! index — the first handles a table mints are `0, 1, 2, …` as before.
//! Handles are otherwise opaque: guests must not do arithmetic on them or
//! use them as array indices.

/// A handle: a slot index plus that slot's generation (see the module docs).
pub type Handle = u32;

/// Low bits holding the slot index (65 536 slots, far above any pool cap).
const INDEX_BITS: u32 = 16;
const INDEX_MASK: u32 = (1 << INDEX_BITS) - 1;
/// The generation's bits: everything above the index up to, not including,
/// the `i32` sign bit.  It wraps, so a stale handle aliases again only after
/// its slot has been recycled `GEN_MASK + 1` (32 768) times.
const GEN_MASK: u32 = (1 << (31 - INDEX_BITS)) - 1;

fn encode(index: usize, generation: u32) -> Handle {
    (generation << INDEX_BITS) | index as u32
}

fn decode(handle: Handle) -> (usize, u32) {
    ((handle & INDEX_MASK) as usize, handle >> INDEX_BITS)
}

#[derive(Clone, Copy)]
struct Slot {
    entity: Option<hecs::Entity>,
    generation: u32,
}

/// A handle→entity slot table with a free-list allocator, bounded by `cap`.
pub struct EntityHandles {
    slots: Vec<Slot>,
    /// Free slot indices (not handles: the generation lives in the slot).
    free: Vec<usize>,
    cap: usize,
}

impl EntityHandles {
    /// An empty table that mints at most `cap` live handles.
    pub fn new(cap: usize) -> Self {
        assert!(cap <= INDEX_MASK as usize + 1, "handle table cap exceeds the index bits");
        Self { slots: Vec::new(), free: Vec::new(), cap }
    }

    /// Reserve a handle for an entity about to be spawned, or `None` when the
    /// table is full.  Pair with [`bind`](Self::bind).
    pub fn reserve(&mut self) -> Option<Handle> {
        let index = match self.free.pop() {
            Some(i) => i,
            None if self.slots.len() < self.cap => {
                self.slots.push(Slot { entity: None, generation: 0 });
                self.slots.len() - 1
            }
            None => return None,
        };
        Some(encode(index, self.slots[index].generation))
    }

    /// Bind a reserved handle to its entity.
    pub fn bind(&mut self, handle: Handle, entity: hecs::Entity) {
        let (index, generation) = decode(handle);
        let slot = &mut self.slots[index];
        debug_assert_eq!(slot.generation, generation, "binding a stale handle");
        slot.entity = Some(entity);
    }

    /// The slot behind a handle whose generation is current.
    fn slot_mut(&mut self, handle: Handle) -> Option<&mut Slot> {
        let (index, generation) = decode(handle);
        self.slots.get_mut(index).filter(|s| s.generation == generation)
    }

    /// The entity behind an active handle (`None` once released, even if the
    /// slot has since been reused).
    pub fn get(&self, handle: Handle) -> Option<hecs::Entity> {
        let (index, generation) = decode(handle);
        self.slots.get(index).filter(|s| s.generation == generation)?.entity
    }

    /// Unbind an active handle, returning its entity and recycling the slot
    /// under the next generation, which invalidates `handle` for good.
    pub fn take(&mut self, handle: Handle) -> Option<hecs::Entity> {
        let slot = self.slot_mut(handle)?;
        let entity = slot.entity.take()?;
        slot.generation = (slot.generation + 1) & GEN_MASK;
        self.free.push(decode(handle).0);
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
        assert_eq!((a, b), (0, 1), "generation-0 handles are the slot indices");
        assert_eq!(t.reserve(), None, "full");
        assert!(t.take(a).is_some());
        assert_eq!(t.take(a), None, "double release");
        assert_eq!(t.get(a), None);
        let c = t.reserve().unwrap();
        assert_eq!(decode(c), (0, 1), "a released slot is recycled under a new generation");
        assert_eq!(t.get(7), None, "out-of-range handle");
    }

    #[test]
    fn stale_handles_never_reach_the_slot_reuser() {
        let mut world = hecs::World::new();
        let mut t = EntityHandles::new(1);
        let old = t.reserve().unwrap();
        t.bind(old, world.spawn(()));
        assert!(t.take(old).is_some());
        let new = t.reserve().unwrap();
        let reuser = world.spawn(());
        t.bind(new, reuser);
        assert_ne!(old, new);
        assert_eq!(t.get(old), None, "a stale handle resolves to nothing");
        assert_eq!(t.take(old), None, "a stale handle cannot release the reuser");
        assert_eq!(t.get(new), Some(reuser));
    }

    #[test]
    fn handles_stay_non_negative_and_generations_wrap() {
        let mut world = hecs::World::new();
        let mut t = EntityHandles::new(256);
        for _ in 0..256 {
            let h = t.reserve().unwrap();
            t.bind(h, world.spawn(()));
        }
        // Cycle the last slot through every generation and past the wrap.
        let mut h = encode(255, 0);
        let mut seen_max = 0;
        for _ in 0..=GEN_MASK + 1 {
            assert!(t.take(h).is_some());
            h = t.reserve().unwrap();
            t.bind(h, world.spawn(()));
            assert!(i32::try_from(h).is_ok_and(|v| v >= 0), "handle {h:#x} fits a positive i32");
            assert_eq!(decode(h).0, 255);
            seen_max = seen_max.max(h);
        }
        assert_eq!(seen_max, encode(255, GEN_MASK), "the top generation is reachable");
        assert_eq!(decode(h).1, 1, "the generation wrapped back round");
        assert!(t.get(h).is_some());
        assert_eq!(t.get(h | 1 << 31), None, "a handle with the sign bit set is never valid");
    }
}
