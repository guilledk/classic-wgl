---
name: classic-rust-ecs
description: >
    Entity Component System patterns for classic-wgl's Rust port.
    Covers `hecs` entity spawning, `Transform` insertion patterns,
    `Disabled` component + parent-chain visibility checks,
    `set_enabled` with collider sync, `Rc<Cell<T>>`/`Rc<RefCell<T>>`
    for cross-closure state sharing between collider handlers and
    on_update closures, and component field initialization patterns.
    Use when debugging state loading failures, missing component
    fields, entity visibility, or borrow-checker conflicts with hecs.
    Trigger phrases: "Transform", "Disabled", "set_enabled",
    "is_disabled", "Rc<Cell>", "Rc<RefCell>", "hecs borrow",
    "insert_one", "remove_one", "component field", "entity enabled".
compatibility: hecs 0.10
metadata:
    author: classic-wgl
    version: '0.2'
allowed-tools: Read, Grep, Glob, Bash(git *), Edit
---

# Skill: classic-rust-ecs

## Scope

Covers ECS patterns in the Rust port: `hecs` entity/component management,
visibility (`Disabled` + `is_disabled` + `set_enabled`), shared state
(`Rc<Cell<T>>` / `Rc<RefCell<T>>`), borrow-checker workarounds, and
component field initialization.

---

## 1. hecs Borrow Patterns

### Extract data before mutable insert

When inserting a component on entity A while needing data from entity B:
extract the data into locals, drop the read borrow, then insert:

```rust
// Pattern: extract → drop → insert
let (pos, scl) = self.world.get::<&Transform>(other_entity)
    .map(|tf| (tf.position, tf.scale))
    .unwrap_or((Vec3::ZERO, Vec3::ONE));
// Borrow from world.get is dropped here (map + unwrap_or releases the Ref)
let _ = self.world.insert_one(target_entity, Transform::new(pos, scl));
```

**Avoid**: holding a `Ref<T>` from `world.get::<&T>()` while calling
`world.insert_one()` — mutable borrow conflicts with immutable borrow.

### Drop RefMut before re-accessing world

```rust
let Ok(mut tm) = self.world.get::<&mut Tilemap>(e) else { return };
// ... modify tm.height_data ...
drop(tm);  // MUST drop before calling methods that access world
self.rebuild_tilemap_mesh("tilemap");
```

`RefMut<Tilemap>` holds a mutable borrow on `world`. Subsequent calls
that access `world` will fail unless the `RefMut` is explicitly dropped.

---

## 2. set_enabled + Collider Sync

```rust
fn set_enabled(&mut self, entity: hecs::Entity, enabled: bool) {
    // 1. Collect collider PIDs BEFORE ECS mutations (avoids borrow conflict)
    let pids = if let Some(ref ui) = self.ui {
        ui.collect_collider_pids(&self.world, entity)
    } else { Vec::new() };

    // 2. Toggle Disabled component on entity
    if enabled && has_disabled {
        self.world.remove_one::<Disabled>(entity);
    } else if !enabled && !has_disabled {
        self.world.insert_one(entity, Disabled);
    }

    // 3. Recurse into UiNode.children
    for child in children {
        self.set_enabled(child, enabled);
    }

    // 4. Sync collider state in physics system
    for pid in &pids {
        self.physics.set_collider_enabled(*pid, enabled);
    }
}
```

**Requirement**: `set_enabled` MUST sync collider state via
`physics.set_collider_enabled`. Without this, hidden UI elements
still fire click handlers because their colliders remain in the quadtree.

**Borrow ordering**: Collect collider PIDs FIRST (reads `self.ui` + `self.world`
immutably), THEN do ECS mutations (writes `self.world`), THEN do physics
mutations (writes `self.physics`). Each phase accesses different subsets
of `self` fields.

---

## 3. is_disabled — Parent Chain Check

```rust
fn is_disabled(&self, entity: hecs::Entity) -> bool {
    if self.world.get::<&Disabled>(entity).is_ok() { return true; }
    let mut parent = self.world.get::<&UiNode>(entity)
        .ok().and_then(|n| n.parent);
    while let Some(p) = parent {
        if self.world.get::<&Disabled>(p).is_ok() { return true; }
        parent = self.world.get::<&UiNode>(p)
            .ok().and_then(|n| n.parent);
    }
    false
}
```

Walks `UiNode.parent` chain. Needed because `set_enabled` recurses
to children — a child entity whose parent is disabled should also be
skipped in render queries.

Used in all render list queries:
```rust
if self.is_disabled(e) { continue; }
```

---

## 4. Rc<Cell<T>> / Rc<RefCell<T>> Shared State

Collider `click_action` closures fire in `perform_calls`. `on_update`
closures fire later in the same frame. To share mutable state between
them, use `Rc`:

```rust
// Simple values (Copy types):
let h_val = Rc::new(Cell::new(0i32));
// In click_action:
h_val.set(h_val.get() + 1);
// In on_update:
engine.editor_height = h_val.get();

// String values:
let editor_tgt = Rc::new(RefCell::new(String::from("none")));
// In click_action:
*editor_tgt.borrow_mut() = "tilemap".into();
// In on_update:
engine.editor_target = editor_tgt.borrow().clone();
```

**Key insight**: `on_update` closures run AFTER `perform_calls`. So
click_action writes are visible to on_update reads in the SAME frame.

**Pitfall**: Other `on_update` closures may reset `engine` fields before
your closure runs (e.g., tool_buttons syncs `editor_target` from its
own Rc every frame). During synthetic drags, re-apply editor state
in the drag processing loop every frame.

---

## 5. Disabled Component

```rust
#[derive(Clone, Copy, Debug, Default)]
pub struct Disabled;
```

Marker component. When present, the entity should be skipped in render
queries. `set_enabled` toggles this component. `is_disabled` checks the
entity and its parent chain.

---

## 6. Nav Mesh Entity Transform

The `tilemapNavigation` entity loaded from `state.json` gets a
`Transform` from the `IsometricNavMesh` spawner
(`crates/classic-core/src/lib.rs:112`), but the Transform has zero scale
(`Vec3::ZERO`) because `NavMesh` uses `#[serde(default)]` for
position/scale. The render query is `(&Transform, &NavMesh)`. With zero
scale the nav mesh is invisible; `init_nav_mesh_render` overwrites this
Transform by copying position+scale from the parent tilemap entity.

Fix (already in place in the spawner):
```rust
let (pos, scl) = self.names.get("tilemap")
    .and_then(|&e| self.world.get::<&Transform>(e).ok())
    .map(|tf| (tf.position, tf.scale))
    .unwrap_or((Vec3::ZERO, Vec3::ONE));
self.world.insert_one(nav_entity, Transform::new(pos, scl));
```

Matches TS behavior where `IsometricNavMesh` constructor borrows
position + scale from the parent `Tilemap`.
