//! # Skill: `classic-ecs`
//!
//! **Read `.claude/skills/classic-ecs/SKILL.md` before working on this module.**
//!
//! Generic host-side inventory mechanics over the `classic-core` [`Inventory`]
//! component and the per-ROM [`ItemRegistry`].  These functions enforce the
//! I/O rules (accepts/provides, capacity, stack rule); *which* unit moves
//! *what* *when* is ROM-guest behaviour.
//!
//! All rules are pure functions of `(&ItemRegistry, &mut Inventory)` — no GL,
//! no engine state — so they are natively unit-tested.

use classic_core::inventory::{Inventory, InventoryType, ItemClass, ItemId, ItemRegistry};

use crate::Engine;

/// The effective capacity of an inventory for a given item class: the raw
/// capacity scaled by the inventory type's per-class multiplier (default 1.0).
pub fn capacity_for(reg: &ItemRegistry, inv: &Inventory, class: ItemClass) -> u32 {
    let mult = reg.inventory_type(&inv.kind).map(|t| t.multiplier(class)).unwrap_or(1.0);
    (inv.capacity as f32 * mult).max(0.0) as u32
}

/// Whether an inventory accepts the given class on input.
///
/// An empty `accepts` list is *closed* (accepts nothing); a non-empty list must
/// contain the class.
pub fn accepts(reg: &ItemRegistry, inv: &Inventory, item: ItemId) -> bool {
    let Some(def) = reg.def(item) else { return false };
    !inv.accepts.is_empty() && inv.accepts.contains(&def.class)
}

/// Whether an inventory provides the given class on output.
///
/// An empty `provides` list is *closed* (provides nothing).
pub fn provides(reg: &ItemRegistry, inv: &Inventory, item: ItemId) -> bool {
    let Some(def) = reg.def(item) else { return false };
    !inv.provides.is_empty() && inv.provides.contains(&def.class)
}

/// The stack count of an item already held by an inventory.
pub fn count(inv: &Inventory, item: ItemId) -> u32 {
    inv.stacks.binary_search_by_key(&item, |(id, _)| *id).map(|i| inv.stacks[i].1).unwrap_or(0)
}

/// Insert `n` units of `item`, returning the number actually added (0 = fully
/// rejected).  Enforces `accepts`, per-class capacity, and the stack rule's
/// `max_per_stack` (one stack per item id, so `max_per_stack` also caps the
/// total for that item).
pub fn add(reg: &ItemRegistry, inv: &mut Inventory, item: ItemId, n: u32) -> u32 {
    if n == 0 || !accepts(reg, inv, item) {
        return 0;
    }
    let def = match reg.def(item) {
        Some(d) => d,
        None => return 0,
    };
    let cap = capacity_for(reg, inv, def.class);
    let used = inv.used();
    let room = cap.saturating_sub(used);
    let existing = count(inv, item);
    let max_stack = def.stack_rule.max_per_stack();
    let allowed = n.min(room).min(max_stack.saturating_sub(existing));

    if allowed == 0 {
        return 0;
    }
    if let Ok(i) = inv.stacks.binary_search_by_key(&item, |(id, _)| *id) {
        inv.stacks[i].1 += allowed;
    } else {
        // `binary_search_by_key` returns the insertion index on miss.
        let i = inv.stacks.binary_search_by_key(&item, |(id, _)| *id).unwrap_err();
        inv.stacks.insert(i, (item, allowed));
    }
    allowed
}

/// Remove `n` units of `item`, returning the number actually removed.  Does not
/// enforce `provides` (removal is an internal operation); use [`transfer`] for
/// cross-inventory moves that must honour the output side's rules.
pub fn remove(inv: &mut Inventory, item: ItemId, n: u32) -> u32 {
    if n == 0 {
        return 0;
    }
    let Ok(i) = inv.stacks.binary_search_by_key(&item, |(id, _)| *id) else {
        return 0;
    };
    let taken = inv.stacks[i].1.min(n);
    inv.stacks[i].1 -= taken;
    if inv.stacks[i].1 == 0 {
        inv.stacks.remove(i);
    }
    taken
}

/// Transfer up to `n` units of `item` from `from` to `to`, enforcing both
/// sides' rules (`from.provides` must contain the class, `to.accepts` must
/// contain the class, and `to`'s capacity/stack rule bound the amount).
/// Returns the number actually transferred (0 = rejected).
pub fn transfer(
    reg: &ItemRegistry,
    from: &mut Inventory,
    to: &mut Inventory,
    item: ItemId,
    n: u32,
) -> u32 {
    if n == 0 || !provides(reg, from, item) || !accepts(reg, to, item) {
        return 0;
    }
    let available = count(from, item);
    let take = n.min(available);
    if take == 0 {
        return 0;
    }
    // Reserve the target-side allowance first (so a partial transfer is never
    // double-counted), then commit both sides.
    let def = match reg.def(item) {
        Some(d) => d,
        None => return 0,
    };
    let cap = capacity_for(reg, to, def.class);
    let to_used = to.used();
    let room = cap.saturating_sub(to_used);
    let existing = count(to, item);
    let max_stack = def.stack_rule.max_per_stack();
    let amount = take.min(room).min(max_stack.saturating_sub(existing));
    if amount == 0 {
        return 0;
    }

    remove(from, item, amount);
    add(reg, to, item, amount);
    amount
}

/// Resolve an inventory's type by name, if declared.
pub fn inventory_type<'a>(reg: &'a ItemRegistry, inv: &Inventory) -> Option<&'a InventoryType> {
    if inv.kind.is_empty() {
        None
    } else {
        reg.inventory_type(&inv.kind)
    }
}

impl Engine {
    /// Serialize a named entity's [`Inventory`] to JSON (empty if it has none).
    pub fn inventory_dump(&self, name: &str) -> Option<String> {
        let entity = *self.names.get(name)?;
        let inv = self.world.get::<&Inventory>(entity).ok()?;
        serde_json::to_string(&*inv).ok()
    }

    /// Read a named entity's raw [`Inventory`] capacity.
    pub fn inventory_capacity(&self, name: &str) -> Option<u32> {
        let entity = *self.names.get(name)?;
        self.world.get::<&Inventory>(entity).ok().map(|inv| inv.capacity)
    }

    /// Serialize an item definition to JSON.
    pub fn item_def(&self, name: &str) -> Option<String> {
        let id = self.items.id(name)?;
        let def = self.items.def(id)?;
        serde_json::to_string(def).ok()
    }

    /// Add `n` units of `item` (by name) to a named entity's inventory,
    /// returning the amount actually added.
    pub fn inventory_add(&mut self, name: &str, item: &str, n: u32) -> u32 {
        let Some(item_id) = self.items.id(item) else { return 0 };
        let Some(&entity) = self.names.get(name) else { return 0 };
        let Ok(mut inv) = self.world.get::<&mut Inventory>(entity) else { return 0 };
        add(&self.items, &mut inv, item_id, n)
    }

    /// Remove `n` units of `item` (by name) from a named entity's inventory,
    /// returning the amount actually removed.
    pub fn inventory_remove(&mut self, name: &str, item: &str, n: u32) -> u32 {
        let Some(item_id) = self.items.id(item) else { return 0 };
        let Some(&entity) = self.names.get(name) else { return 0 };
        let Ok(mut inv) = self.world.get::<&mut Inventory>(entity) else { return 0 };
        remove(&mut inv, item_id, n)
    }

    /// Transfer up to `n` units of `item` (by name) between two named
    /// entities' inventories, enforcing both sides' I/O rules.  Returns the
    /// amount actually transferred.
    pub fn inventory_transfer(&mut self, from: &str, to: &str, item: &str, n: u32) -> u32 {
        let Some(item_id) = self.items.id(item) else { return 0 };
        let (Some(&from_e), Some(&to_e)) = (self.names.get(from), self.names.get(to)) else {
            return 0;
        };
        if from_e == to_e {
            return 0;
        }
        // Take both inventories out (hecs forbids two simultaneous `&mut`
        // borrows), transfer, then write them back.
        let Ok((mut from_inv,)) = self.world.remove::<(Inventory,)>(from_e) else { return 0 };
        let Ok((mut to_inv,)) = self.world.remove::<(Inventory,)>(to_e) else {
            let _ = self.world.insert(from_e, (from_inv,));
            return 0;
        };
        let moved = transfer(&self.items, &mut from_inv, &mut to_inv, item_id, n);
        let _ = self.world.insert(from_e, (from_inv,));
        let _ = self.world.insert(to_e, (to_inv,));
        moved
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use classic_core::inventory::{Inventory, InventoryType, ItemClass, ItemDef, StackRule};

    fn reg() -> ItemRegistry {
        let items = vec![
            ItemDef {
                name: "regolith_ore".into(),
                class: ItemClass::Ore,
                stack_rule: StackRule::Bulk { max_per_stack: 100 },
                mass: 1.0,
                volume: 0.5,
            },
            ItemDef {
                name: "lox".into(),
                class: ItemClass::Fuel,
                stack_rule: StackRule::Gaseous { pressure_factor: 4.0, max_per_stack: 500 },
                mass: 1.1,
                volume: 1.0,
            },
            ItemDef {
                name: "shipping_container".into(),
                class: ItemClass::Container,
                stack_rule: StackRule::Unit { max_per_stack: 1 },
                mass: 2200.0,
                volume: 33.0,
            },
        ];
        let types = vec![
            InventoryType { name: "gas_tank".into(), capacity_mult: vec![(ItemClass::Fuel, 4.0)] },
            InventoryType { name: "cargo_bay".into(), capacity_mult: vec![] },
        ];
        ItemRegistry::build(&items, &types)
    }

    fn cargo_bay() -> Inventory {
        Inventory {
            capacity: 100,
            kind: "cargo_bay".into(),
            accepts: vec![ItemClass::Ore, ItemClass::Container],
            provides: vec![ItemClass::Ore, ItemClass::Container],
            ..Default::default()
        }
    }

    #[test]
    fn add_enforces_accepts_and_capacity() {
        let reg = reg();
        let ore = reg.id("regolith_ore").unwrap();
        let lox = reg.id("lox").unwrap();

        let mut inv = cargo_bay();
        // Fuel is not in `accepts` → rejected.
        assert_eq!(add(&reg, &mut inv, lox, 10), 0);
        // Ore accepted, capped by capacity (100) and max stack (100).
        assert_eq!(add(&reg, &mut inv, ore, 150), 100);
        // Now full → further adds rejected.
        assert_eq!(add(&reg, &mut inv, ore, 1), 0);
        assert_eq!(inv.used(), 100);
    }

    #[test]
    fn add_caps_by_max_per_stack() {
        let reg = reg();
        let container = reg.id("shipping_container").unwrap();
        let mut inv = Inventory {
            capacity: 10,
            kind: "cargo_bay".into(),
            accepts: vec![ItemClass::Container],
            provides: vec![ItemClass::Container],
            ..Default::default()
        };
        // Unit stack caps at 1 even though capacity allows 10.
        assert_eq!(add(&reg, &mut inv, container, 3), 1);
        assert_eq!(count(&inv, container), 1);
    }

    #[test]
    fn gaseous_capacity_multiplier_scales_storage() {
        let reg = reg();
        let lox = reg.id("lox").unwrap();
        let mut tank = Inventory {
            capacity: 100,
            kind: "gas_tank".into(),
            accepts: vec![ItemClass::Fuel],
            provides: vec![ItemClass::Fuel],
            ..Default::default()
        };
        // Raw 100 × 4.0 multiplier = 400 effective, capped by max stack 500.
        assert_eq!(capacity_for(&reg, &tank, ItemClass::Fuel), 400);
        assert_eq!(add(&reg, &mut tank, lox, 1000), 400);
    }

    #[test]
    fn remove_and_count_round_trip() {
        let reg = reg();
        let ore = reg.id("regolith_ore").unwrap();
        let mut inv = cargo_bay();
        add(&reg, &mut inv, ore, 50);
        assert_eq!(count(&inv, ore), 50);
        assert_eq!(remove(&mut inv, ore, 20), 20);
        assert_eq!(count(&inv, ore), 30);
        // Removing an absent item is a no-op.
        assert_eq!(remove(&mut inv, ore, 999), 30);
        assert_eq!(count(&inv, ore), 0);
    }

    #[test]
    fn transfer_enforces_both_sides() {
        let reg = reg();
        let ore = reg.id("regolith_ore").unwrap();

        let mut mine = Inventory {
            capacity: 100,
            accepts: vec![ItemClass::Ore],
            provides: vec![ItemClass::Ore],
            stacks: vec![(ore, 40)],
            ..Default::default()
        };
        let mut hopper = cargo_bay();

        // `mine` provides ore, `hopper` accepts ore → ok.
        assert_eq!(transfer(&reg, &mut mine, &mut hopper, ore, 25), 25);
        assert_eq!(count(&mine, ore), 15);
        assert_eq!(count(&hopper, ore), 25);

        // A hopper that does not *provide* ore cannot send it back.
        let mut closed = Inventory {
            capacity: 100,
            accepts: vec![ItemClass::Ore],
            provides: vec![], // closed output
            stacks: vec![(ore, 5)],
            ..Default::default()
        };
        assert_eq!(transfer(&reg, &mut closed, &mut hopper, ore, 5), 0);
    }

    #[test]
    fn transfer_clamps_to_target_capacity() {
        let reg = reg();
        let ore = reg.id("regolith_ore").unwrap();

        let mut mine = Inventory {
            capacity: 1000,
            accepts: vec![ItemClass::Ore],
            provides: vec![ItemClass::Ore],
            stacks: vec![(ore, 1000)],
            ..Default::default()
        };
        let mut tiny = Inventory {
            capacity: 10,
            accepts: vec![ItemClass::Ore],
            provides: vec![],
            ..Default::default()
        };
        assert_eq!(transfer(&reg, &mut mine, &mut tiny, ore, 500), 10);
        assert_eq!(count(&tiny, ore), 10);
        assert_eq!(count(&mine, ore), 990);
    }
}
