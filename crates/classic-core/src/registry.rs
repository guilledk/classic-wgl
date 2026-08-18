//! # Skill: `classic-ecs`
//!
//! **Read `.claude/skills/classic-ecs/SKILL.md` before working on this module.**
//!
use std::collections::HashMap;
use std::sync::RwLock;

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

static REGISTRY: std::sync::LazyLock<RwLock<HashMap<&'static str, ComponentReg>>> =
    std::sync::LazyLock::new(|| RwLock::new(HashMap::new()));

/// Register a component with its spawner, dumper, priority, and subsumes rules.
pub fn register(reg: ComponentReg) {
    if REGISTRY.write().unwrap().insert(reg.name, reg).is_some() {
        crate::cl_warn!(
            crate::instrument::Chan::Ecs,
            "Component \"{}\" is already registered. Overwriting.",
            reg.name
        );
    }
}

/// Register a spawner-only component (backward-compatible convenience).
pub fn register_spawner(name: &'static str, spawner: Spawner) {
    register(ComponentReg { name, spawn: spawner, dump: None, order: 0, subsumes: &[] });
}

/// Look up a component spawner by name.
pub fn lookup(name: &str) -> Option<Spawner> {
    REGISTRY.read().unwrap().get(name).map(|r| r.spawn)
}

/// Get all registrations ordered by dump priority (lowest first).
pub fn ordered_regs() -> Vec<ComponentReg> {
    let mut regs: Vec<_> = REGISTRY.read().unwrap().values().cloned().collect();
    regs.sort_by_key(|r| r.order);
    regs
}

/// Check whether a component name is registered.
pub fn has(name: &str) -> bool {
    REGISTRY.read().unwrap().contains_key(name)
}

/// Clear all registered components (for tests).
pub fn clear() {
    REGISTRY.write().unwrap().clear();
}

/// Return all registered component names.
pub fn names() -> Vec<&'static str> {
    REGISTRY.read().unwrap().keys().copied().collect()
}
