//! # Skill: `classic-ecs`
//!
//! **Read `.agents/skills/classic-ecs/SKILL.md` before working on this module.**
//!
use std::sync::OnceLock;

/// Spawner function signature: takes the entity builder, raw JSON value,
/// and adds one or more components to the builder.
pub type Spawner = fn(&mut hecs::EntityBuilder, serde_json::Value) -> anyhow::Result<()>;

/// Dumper function: given a world and entity, produce a JSON value for this
/// component type (including the `type` key), or `None` if the entity doesn't
/// have this component.
pub type Dumper = fn(&hecs::World, hecs::Entity) -> Option<serde_json::Value>;

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
