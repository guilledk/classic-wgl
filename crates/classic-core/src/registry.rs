//! # Skill: `classic-ecs`
//!
//! **Read `.agents/skills/classic-ecs/SKILL.md` before working on this module.**
//!
use std::sync::OnceLock;

/// Spawner function signature: takes the entity builder, raw JSON value,
/// and adds one or more components to the builder.
pub type Spawner = fn(&mut hecs::EntityBuilder, serde_json::Value) -> anyhow::Result<()>;

/// Dumper function: given a world and entity, produce the JSON body of this
/// component (without the `type` key — see [`ComponentReg::dump_value`]), or
/// `None` if the entity doesn't have this component.  [`dump_as`] is the
/// dumper for any serializable component.
pub type Dumper = fn(&hecs::World, hecs::Entity) -> Option<serde_json::Value>;

/// The [`Dumper`] for a serializable component type `T`: its serde body.
pub fn dump_as<T: hecs::Component + serde::Serialize>(
    world: &hecs::World,
    entity: hecs::Entity,
) -> Option<serde_json::Value> {
    let component = world.get::<&T>(entity).ok()?;
    serde_json::to_value(&*component).ok()
}

/// A registered component entry with bidirectional support.
#[derive(Clone, Copy)]
pub struct ComponentReg {
    /// String name used in state.json ("type" field).
    pub name: &'static str,
    /// Spawner for deserialization.
    pub spawn: Spawner,
    /// Optional dumper for serialization.
    pub dump: Option<Dumper>,
    /// Dump priority (lower = emitted earlier in the component list).
    pub order: i32,
    /// Names of other component types that this component subsumes (fan-out de-duplication).
    pub subsumes: &'static [&'static str],
}

impl ComponentReg {
    /// Dump this component of `entity` as a `state.json` component value: the
    /// dumper's body with the `type` key first.  `None` when the component has no
    /// dumper or the entity lacks it.
    pub fn dump_value(
        &self,
        world: &hecs::World,
        entity: hecs::Entity,
    ) -> Option<serde_json::Value> {
        let body = (self.dump?)(world, entity)?;
        let mut value = serde_json::Map::new();
        value.insert("type".into(), serde_json::Value::String(self.name.into()));
        if let serde_json::Value::Object(fields) = body {
            value.extend(fields);
        }
        Some(serde_json::Value::Object(value))
    }
}

/// The immutable component registry.  Populated once via [`init`]; lookups are
/// read-only, so tests sharing the process no longer need `--test-threads=1`.
static REGISTRY: OnceLock<Vec<ComponentReg>> = OnceLock::new();

/// Install the component registry.  Idempotent: the first call wins and later
/// calls are no-ops.
pub fn init(regs: Vec<ComponentReg>) {
    let _ = REGISTRY.set(regs);
}

/// Look up a component spawner by name.
pub fn lookup(name: &str) -> Option<Spawner> {
    REGISTRY.get().and_then(|r| r.iter().find(|c| c.name == name)).map(|c| c.spawn)
}

/// Get all registrations ordered by dump priority (lowest first).
pub fn ordered_regs() -> Vec<ComponentReg> {
    let mut regs = REGISTRY.get().cloned().unwrap_or_default();
    regs.sort_by_key(|r| r.order);
    regs
}

/// Check whether a component name is registered.
pub fn has(name: &str) -> bool {
    REGISTRY.get().is_some_and(|r| r.iter().any(|c| c.name == name))
}

/// Return all registered component names.
pub fn names() -> Vec<&'static str> {
    REGISTRY.get().map(|r| r.iter().map(|c| c.name).collect()).unwrap_or_default()
}
