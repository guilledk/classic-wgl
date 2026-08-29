//! # Skill: `classic-ecs`
//!
//! **Read `.claude/skills/classic-ecs/SKILL.md` before working on this module.**
//!
//! Item catalog + inventory component types for the container-logistics layer.
//!
//! The item catalog is *ROM-namespaced data*: ROM manifests declare `items[]`
//! and `inventory_types[]`, the host interns the item names into a per-ROM
//! [`ItemRegistry`] (numeric [`ItemId`]s so hot paths never compare strings),
//! and [`Inventory`] is a serializable ECS component that round-trips through
//! the component registry.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// An interned item identifier.  The string name is kept only for
/// serialization and logging; hot paths compare these integers.
pub type ItemId = u32;

/// The physical/material class of an item.  [`InventoryType`] scales a
/// container's capacity per class, so a gas tank stacks [`ItemClass::Gas`]
/// better than a bulk hopper does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemClass {
    Ore,
    Metal,
    Gas,
    Fluid,
    Munition,
    Electronics,
    Fuel,
    Container,
}

/// How items of a class stack inside an inventory.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(tag = "rule", rename_all = "snake_case")]
pub enum StackRule {
    /// Loose solids: capped count per stack.
    Bulk { max_per_stack: u32 },
    /// Compressible gases: the effective capacity is the raw capacity scaled
    /// by `pressure_factor`; still bounded by `max_per_stack` per stack.
    Gaseous { pressure_factor: f32, max_per_stack: u32 },
    /// Discrete, non-stacking units (vehicles, munitions boxes).
    Unit { max_per_stack: u32 },
}

impl StackRule {
    /// The maximum number of items allowed in a single stack.
    pub fn max_per_stack(&self) -> u32 {
        match *self {
            StackRule::Bulk { max_per_stack }
            | StackRule::Gaseous { max_per_stack, .. }
            | StackRule::Unit { max_per_stack } => max_per_stack,
        }
    }
}

fn default_stack_rule() -> StackRule {
    StackRule::Unit { max_per_stack: 1 }
}

/// A catalogued item definition (ROM manifest `items[]`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ItemDef {
    /// The ROM-namespaced item name (also the interning key).
    pub name: String,
    pub class: ItemClass,
    #[serde(default = "default_stack_rule")]
    pub stack_rule: StackRule,
    /// Mass per unit item.  Informational for v1.
    #[serde(default)]
    pub mass: f32,
    /// Volume per unit item.  Informational for v1.
    #[serde(default)]
    pub volume: f32,
    /// Icon frame name (resolved through the `icons` packed-atlas frame table).
    /// `None` means the icon frame == the item `name` (they match by
    /// convention).  A ROM may set an explicit override for future divergence.
    #[serde(default)]
    pub icon: Option<String>,
}

impl ItemDef {
    /// The packed-atlas frame name to use for this item's icon, defaulting to
    /// the item name when no explicit `icon` override is set.
    pub fn icon_frame_name(&self) -> &str {
        self.icon.as_deref().unwrap_or(&self.name)
    }
}

/// A named inventory type (ROM manifest `inventory_types[]`).  `capacity_mult`
/// scales an inventory's raw capacity per [`ItemClass`] — "gas containers stack
/// gas better" — expressed as data, not code.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InventoryType {
    pub name: String,
    /// Per-class capacity multipliers.  Absent classes default to `1.0`.
    #[serde(default)]
    pub capacity_mult: Vec<(ItemClass, f32)>,
}

impl InventoryType {
    /// The capacity multiplier for a class (defaults to `1.0`).
    pub fn multiplier(&self, class: ItemClass) -> f32 {
        self.capacity_mult.iter().find(|(c, _)| *c == class).map(|(_, m)| *m).unwrap_or(1.0)
    }
}

/// An inventory attached to an entity: a capacity, the item classes it accepts
/// and provides, and its current stacks (sorted by [`ItemId`] for binary
/// search).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Inventory {
    /// Raw capacity, in "units".  [`InventoryType::multiplier`] scales it per
    /// class at insertion time (see `classic-engine` `inventory.rs`).
    #[serde(default)]
    pub capacity: u32,
    /// The name of the [`InventoryType`] this inventory realizes.
    #[serde(default)]
    pub kind: String,
    /// Item classes this inventory accepts on input.
    #[serde(default)]
    pub accepts: Vec<ItemClass>,
    /// Item classes this inventory provides on output.
    #[serde(default)]
    pub provides: Vec<ItemClass>,
    /// Current contents: `(item id, count)`, sorted ascending by id.
    #[serde(default)]
    pub stacks: Vec<(ItemId, u32)>,
}

impl Inventory {
    /// Total units currently stored (sum over stacks).
    pub fn used(&self) -> u32 {
        self.stacks.iter().map(|(_, n)| *n).sum()
    }

    /// Free capacity (raw capacity minus used).  Saturating at zero.
    pub fn free(&self) -> u32 {
        self.capacity.saturating_sub(self.used())
    }
}

/// The host-side item catalog, interned once per ROM at `load_rom`.
///
/// Read-only after construction: hot paths look items up by [`ItemId`].
#[derive(Clone, Debug, Default)]
pub struct ItemRegistry {
    /// Item name → interned id.
    pub by_name: HashMap<String, ItemId>,
    /// Interned definitions, indexed by [`ItemId`].
    pub defs: Vec<ItemDef>,
    /// Inventory types by name.
    pub inventory_types: HashMap<String, InventoryType>,
}

impl ItemRegistry {
    /// Build a registry from manifest data, interning item names in order.
    pub fn build(items: &[ItemDef], inventory_types: &[InventoryType]) -> Self {
        let mut reg = Self::default();
        for def in items {
            reg.intern(def.clone());
        }
        for it in inventory_types {
            reg.inventory_types.insert(it.name.clone(), it.clone());
        }
        reg
    }

    /// Intern an item definition, returning its id (reusing an existing id if
    /// the name is already present).
    pub fn intern(&mut self, def: ItemDef) -> ItemId {
        if let Some(&id) = self.by_name.get(&def.name) {
            return id;
        }
        let id = self.defs.len() as ItemId;
        self.defs.push(def.clone());
        self.by_name.insert(def.name.clone(), id);
        id
    }

    /// Look up an item's interned id by name.
    pub fn id(&self, name: &str) -> Option<ItemId> {
        self.by_name.get(name).copied()
    }

    /// The definition for an interned item id.
    pub fn def(&self, id: ItemId) -> Option<&ItemDef> {
        self.defs.get(id as usize)
    }

    /// The inventory type by name.
    pub fn inventory_type(&self, name: &str) -> Option<&InventoryType> {
        self.inventory_types.get(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ore() -> ItemDef {
        ItemDef {
            name: "regolith_ore".into(),
            class: ItemClass::Ore,
            stack_rule: StackRule::Bulk { max_per_stack: 100 },
            mass: 1.0,
            volume: 0.5,
            icon: None,
        }
    }

    #[test]
    fn interning_reuses_ids_and_looks_up_defs() {
        let mut reg = ItemRegistry::default();
        let a = reg.intern(ore());
        let b = reg.intern(ore());
        assert_eq!(a, b, "same name interns to the same id");
        assert_eq!(a, 0);

        let fuel = reg.intern(ItemDef {
            name: "lox".into(),
            class: ItemClass::Fuel,
            stack_rule: StackRule::Gaseous { pressure_factor: 2.0, max_per_stack: 500 },
            mass: 1.1,
            volume: 1.0,
            icon: None,
        });
        assert_eq!(fuel, 1);

        assert_eq!(reg.id("regolith_ore"), Some(0));
        assert_eq!(reg.id("lox"), Some(1));
        assert_eq!(reg.id("nope"), None);
        assert_eq!(reg.def(0).map(|d| d.class), Some(ItemClass::Ore));
        assert!(reg.def(2).is_none());
    }

    #[test]
    fn inventory_type_multipliers_default_to_one() {
        let ty =
            InventoryType { name: "gas_tank".into(), capacity_mult: vec![(ItemClass::Gas, 4.0)] };
        assert_eq!(ty.multiplier(ItemClass::Gas), 4.0);
        assert_eq!(ty.multiplier(ItemClass::Ore), 1.0);
    }

    #[test]
    fn stack_rule_max_per_stack() {
        assert_eq!(StackRule::Bulk { max_per_stack: 100 }.max_per_stack(), 100);
        assert_eq!(
            StackRule::Gaseous { pressure_factor: 2.0, max_per_stack: 500 }.max_per_stack(),
            500
        );
        assert_eq!(StackRule::Unit { max_per_stack: 1 }.max_per_stack(), 1);
    }

    #[test]
    fn inventory_used_and_free() {
        let inv = Inventory { capacity: 100, stacks: vec![(0, 30), (1, 5)], ..Default::default() };
        assert_eq!(inv.used(), 35);
        assert_eq!(inv.free(), 65);

        let full = Inventory { capacity: 10, stacks: vec![(0, 99)], ..Default::default() };
        assert_eq!(full.used(), 99);
        assert_eq!(full.free(), 0, "free saturates at zero");
    }

    #[test]
    fn item_def_defaults_stack_rule_to_unit() {
        let def: ItemDef = serde_json::from_value(serde_json::json!({
            "name": "container_box",
            "class": "container"
        }))
        .unwrap();
        assert_eq!(def.stack_rule.max_per_stack(), 1);
    }

    #[test]
    fn item_def_icon_defaults_to_name() {
        let def: ItemDef = serde_json::from_value(serde_json::json!({
            "name": "regolith",
            "class": "ore",
            "stack_rule": {"rule": "bulk", "max_per_stack": 50}
        }))
        .unwrap();
        assert_eq!(def.icon, None);
        assert_eq!(def.icon_frame_name(), "regolith");

        let overridden: ItemDef = serde_json::from_value(serde_json::json!({
            "name": "regolith",
            "class": "ore",
            "stack_rule": {"rule": "bulk", "max_per_stack": 50},
            "icon": "regolith_alt"
        }))
        .unwrap();
        assert_eq!(overridden.icon.as_deref(), Some("regolith_alt"));
        assert_eq!(overridden.icon_frame_name(), "regolith_alt");
    }

    #[test]
    fn item_class_serializes_snake_case() {
        let s = serde_json::to_value(ItemClass::Electronics).unwrap();
        assert_eq!(s, "electronics");
        let back: ItemClass = serde_json::from_value(serde_json::json!("electronics")).unwrap();
        assert_eq!(back, ItemClass::Electronics);
    }
}
