use std::collections::HashMap;

use hecs::Entity;

use crate::types::Call;

/// A user-defined script closure.
///
/// Scripts live *outside* the ECS world so they can capture `&mut Ctx`
/// without borrow-checker conflicts.
pub type ScriptFn = Box<dyn FnMut(Entity)>;

/// Registry of per-frame script closures.
///
/// When a script stabilises, promote it to a typed `fn(&mut Ctx)` system.
#[derive(Default)]
pub struct Scripts {
    slots: HashMap<Call, Vec<(Entity, ScriptFn)>>,
    pending: Vec<(Call, Entity, ScriptFn)>,
}

impl Scripts {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a script closure for a given entity and call phase.
    /// Equivalent to `entity.registerCall('update', fn)` in TS.
    pub fn on(&mut self, entity: Entity, call: Call, f: impl FnMut(Entity) + 'static) {
        self.pending.push((call, entity, Box::new(f)));
    }

    /// Move all pending registrations into their slots.
    fn flush_pending(&mut self) {
        for (call, entity, f) in std::mem::take(&mut self.pending) {
            self.slots.entry(call).or_default().push((entity, f));
        }
    }

    /// Run all scripts for a given call phase.
    pub fn run(&mut self, call: Call) {
        self.flush_pending();

        let mut list = self.slots.remove(&call).unwrap_or_default();

        for (_e, f) in list.iter_mut() {
            f(*_e);
        }

        // Put back (scripts registered during execution go to pending).
        self.slots.insert(call, list);
        self.flush_pending();
    }

    /// Remove all scripts for an entity (called on entity destruction).
    pub fn remove_entity(&mut self, entity: Entity) {
        for list in self.slots.values_mut() {
            list.retain(|(e, _)| *e != entity);
        }
        self.pending.retain(|(_, e, _)| *e != entity);
    }
}
