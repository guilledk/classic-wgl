---
name: classic-ecs
description: >
    Entity Component System patterns and camera math for classic-wgl's
    Rust port.  Covers hecs entity spawning, Transform, component
    definitions, the bidirectional registry, update_fns closure system,
    set_enabled with collider sync, is_disabled parent-chain walk,
    Rc<Cell>/Rc<RefCell> shared state pattern, and 2D camera math
    (fix formula, matrix order, zoom).
    Trigger phrases: "Transform", "Disabled", "set_enabled",
    "is_disabled", "Rc<Cell>", "Rc<RefCell>", "hecs", "update_fns",
    "on_update", "registry", "ComponentReg", "subsumes", "camera fix",
    "camera matrix", "zoom", "orthographic".
compatibility: hecs 0.10, glam 0.29
metadata:
    author: classic-wgl
    version: '1.0'
allowed-tools: Read, Grep, Glob, Bash(git *), Edit
---

# Skill: classic-ecs

## Scope

Covers the ECS layer (`hecs` entity/component management, visibility, shared
state, registry, update closures) and the 2D orthographic camera.  Together
these form the data model and coordinate space the entire engine runs on.

---

## 1. hecs Usage Patterns

The engine uses `hecs` directly — no scheduler abstraction.  The `Engine` struct
owns the `hecs::World` and all access goes through `self.world`.

### Spawning

```rust
let entity = self.world.spawn((Transform::new(pos, scale), SpriteRender { ... }));
```

### Querying

```rust
// Immutable query (borrows the component refs)
for (e, (tf, sprite)) in self.world.query::<(&Transform, &SpriteRender)>().iter() {
    // ...
}

// Mutable access to a single entity
let Ok(mut tm) = self.world.get::<&mut Tilemap>(e) else { return; };
```

### Insert / remove on live entities

```rust
self.world.insert_one(entity, DebugName("tilemap".to_string()));
self.world.remove_one::<Disabled>(entity);
```

### Borrow-order rule

You cannot hold a `Ref<T>` or `RefMut<T>` from `world.get*()` while calling any
other method on `world`.  Extract data into locals, drop the borrow, then
proceed:

```rust
// Pattern: extract → drop → insert
let (pos, scl) = self.world.get::<&Transform>(other_entity)
    .map(|tf| (tf.position, tf.scale))
    .unwrap_or((Vec3::ZERO, Vec3::ONE));
// Borrow released (unwrap_or consumes the Ref)
self.world.insert_one(target_entity, Transform::new(pos, scl));
```

When mutating a component through a `RefMut`, explicit `drop()` may be needed
before any subsequent `world` access — the mutable borrow must end first.

---

## 2. Component Definitions

All components live in `classic-core/src/components/mod.rs`.  Every serializable
component derives `Serialize + Deserialize`.  Components fall into three
categories:

### Core spatial

- **`Transform`** — `position: Vec3`, `scale: Vec3`.  Required by every
  renderable entity.  Provides `model_matrix()`:
  `T(position) * S(scale)`.

- **`Disabled`** — empty marker struct.  When present, the entity is skipped
  in render-list queries (see §6).

- **`DebugName(pub String)`** — stable human-readable label.  Used by logging,
  golden traces, UI debug output.  Every entity loaded from `state.json` gets
  a `DebugName` matching its JSON key.

### Render components

- **`SpriteRender`** — single-frame sprite.  Fields: `position`, `scale`,
  `texture`, `ignore_cam`, `frame`, `tile_set_size`, `anchor`.  Drawn via
  `draw_sprite`.

- **`SdfTextRender`** — SDF text.  Fields: `atlas_name`, `color`, `bgcolor`,
  `outline_color`, `outline_width`, `shadow_offset`, `shadow_color`,
  `shadow_blur`, `ignore_cam`, `text`, `justify` (Left/Center/Right),
  `weight`, `gamma`.  Drawn via `draw_sdf`.

- **`RectRender`** — solid-colour rectangle.  Fields: `color: [f32; 4]`,
  `ignore_cam`.  Drawn via `draw_rect`.

- **`Tilemap`** — the iso terrain grid.  Fields include `position`, `scale`,
  `size_x`, `size_y`, `tile_set`, `tile_pixel_size`, `height_scale`,
  `data: Vec<u32>` (tile indices, row-major), `height_data: Vec<f32>`
  (per-vertex heights), `mouse_iso_pos`, `selection_iso_begin`,
  `selection_iso_end`.  Drawn via `draw_tilemap`.

- **`NavMesh`** — navigation mesh overlay.  Fields: `position`, `scale`,
  `map_entity` (name of source tilemap), `tile_set`, `data`, `data_url`,
  `size_x`, `size_y`.  Rendered on top of the tilemap at z-order 19999.

- **`IsoSprite`** — billboard in iso space.  Fields: `position`, `scale`,
  `texture`, `tilemap` (entity name), `frame`, `tile_set_size`, `anchor`,
  `footprint: Vec<Vec2>` (four corner vertices in iso tile coords).
  Drawn via `draw_iso_sprite`.

- **`IsoAgent`** — pathfinding sprite.  Subsumes `IsoSprite` (i.e. the
  spawner creates both an `IsoAgent` and an `IsoSprite`).  Adds `speed`,
  `anim_speed`, `anim_prefix`, plus internal path-tracking state: `path`,
  `target_index`, `delta`, `init_dist`, `direction`, `anim_index`, `state`
  (`Idle` / `FollowPath`).

- **`Animator`** — frame-animator tied to a sprite by `target` field
  (`"entityName.ComponentName"` format).  Internal state: `animation`,
  `counter`, `frame`, `repeat`, `playing`.

### Collision / UI

- **`Collider`** — physics shape.  Contains `shape: Shape` (`Circle` or
  `Polygon`), `position`, `scale`, `rotation`, `pid` (assigned by
  `PhysicsProvider`), `consumes_click`, `click_priority`, and a `handlers`
  map keyed by `HandlerKind`.  Handlers are `Box<dyn FnMut() -> bool>`.

- **`UiNode`** — retained-mode UI visual + layout element.  Fields:
  `parent: Option<Entity>`, `children: Vec<UiChild>`, `size: Vec2`,
  `anchor: UiAnchor` (9-point), `fixed`, `clip_children`, `scroll_y`,
  `clip_rect: Vec4`, `kind: UiKind` (Container, Array, Padding, Text,
  SdfText, Sprite).

- **`Shape`** — `Circle { diameter }` or `Polygon { verts, center, min, max }`.

- **`UiAnchor`** — 9-point anchor (`TopLeft` through `BotRight`).  Provides
  `offset(w, h) -> Vec2` for layout math.

---

## 3. Transform and Model Matrix

`Transform` is the universal spatial component.  Every entity that appears in
the render list must have one.  The model matrix is:

```rust
Mat4::from_translation(self.position) * Mat4::from_scale(self.scale)
```

This is `T(position) * S(scale)` — translate first, THEN scale around the
origin.  The opposite order (`S * T`) would multiply the translation by the
scale factor, sending the entity far from its intended position at large
scale values.

### Layering (z-order)

Render-list items are sorted by z-depth and drawn back-to-front:

| DrawKind      | Sort key                              |
|---------------|---------------------------------------|
| Sprite        | `tf.position.z` (or -20000 if `ignore_cam`) |
| Tilemap       | `20000.0`                             |
| NavMesh       | `19999.0`                             |
| IsoSprite     | `tf.position.x - tf.position.y`       |
| UiRect        | `tf.position.z`                       |
| SdfText       | `tf.position.z`                       |

UI sprites (`SpriteRender` on an entity with `UiNode.kind == Sprite`) use the
`UiSprite` draw kind and position.z sorting.  Isometric sprites use the
`tx - ty` depth axis from the iso coordinate system.

---

## 4. Component Registry

The registry in `classic-core/src/registry.rs` is a bidirectional system that
maps component type names to spawners (JSON → ECS) and dumpers (ECS → JSON).
It is a global `LazyLock<RwLock<HashMap>>`.

### ComponentReg fields

```rust
pub struct ComponentReg {
    pub name: &'static str,          // "type" field in state.json
    pub spawn: Spawner,              // JSON → EntityBuilder
    pub dump: Option<Dumper>,        // World + Entity → Option<Value>
    pub order: i32,                  // dump priority (lower = earlier)
    pub subsumes: &'static [&'static str],  // fan-out de-duplication
}
```

### Subsumes rules

When a component says `subsumes: &["Transform"]`, its spawner also creates
a `Transform` component.  During `dump_state`, the dumper deduplicates:
if the primary component dumper fires, the subsumed names are skipped
so `Transform` is not emitted separately.  The current subsumes graph:

| Component         | Subsumes              |
|-------------------|-----------------------|
| Sprite            | Transform             |
| Tilemap           | Transform             |
| IsoSprite         | Transform             |
| IsoAgent          | IsoSprite, Transform  |
| IsometricNavMesh  | Transform             |
| Animator          | (none)                |

### Order priority

During `dump_state`, `ordered_regs()` sorts by `order` (ascending).  The
current priorities: Tilemap(10), IsometricNavMesh(15), Sprite(20),
IsoSprite(30), Animator(35), IsoAgent(40).  This controls the field order in
the serialized JSON.

### Registering a component

```rust
registry::register(ComponentReg {
    name: "Sprite",
    spawn: |b, v| { ... b.add(sprite); Ok(()) },
    dump: Some(dumper_sprite),
    order: 20,
    subsumes: &["Transform"],
});
```

Spawn-only (no dumper) can use `registry::register_spawner(name, spawner)`.

### Thread safety

The global registry requires `--test-threads=1` for test binaries that share
the registry across tests (clearing via `registry::clear()`).

---

## 5. update_fns Closure System

The engine has no formal system scheduler.  Gameplay logic is registered as
closures via `on_update`:

```rust
pub fn on_update(&mut self, f: impl FnMut(&mut Engine) + 'static) {
    self.update_fns.push(Box::new(f));
}
```

### Execution model

Every frame, the engine takes the vector, iterates, then restores it:

```rust
let mut fns = std::mem::take(&mut self.update_fns);
for f in fns.iter_mut() {
    f(self);
}
self.update_fns = fns;
```

This take/restore dance avoids holding a borrow on `self.update_fns` while
`f(self)` runs, which allows closures to call `on_update` to register new
closures during execution (commonly done in `init_*` functions during
startup).

### Ordering matters

Closures execute in registration order.  The camera WASD/zoom closure is
registered by `init_camera_wasd()` early in the boot sequence.  Tilemap
mouse-position code is registered inside `init_tilemap()`.  Since each runs
in order, later closures see camera state already updated for this frame.

### capture patterns

Closures capture `hecs::Entity` handles by copy and `Rc<Cell>`/`Rc<RefCell>`
by clone.  The closure signature is `FnMut(&mut Engine)` — `&mut Engine`
gives access to `self.world`, `self.physics`, `self.input`, `self.camera`,
etc.

---

## 6. set_enabled and Disabled

`Disabled` is a zero-size marker component.  `Engine::set_enabled` toggles it:

```rust
fn set_enabled(&mut self, entity: hecs::Entity, enabled: bool) {
    // 1. Collect collider PIDs before ECS mutations
    let pids: Vec<u32> = if let Some(ref ui) = self.ui {
        ui.collect_collider_pids(&self.world, entity)
    } else { Vec::new() };

    // 2. Toggle Disabled
    let has_disabled = self.world.get::<&Disabled>(entity).is_ok();
    if enabled && has_disabled {
        let _ = self.world.remove_one::<Disabled>(entity);
    } else if !enabled && !has_disabled {
        let _ = self.world.insert_one(entity, Disabled);
    }

    // 3. Recurse into UiNode.children
    let children: Vec<hecs::Entity> = self.world.get::<&UiNode>(entity)
        .map(|n| n.children.iter().map(|c| c.entity).collect())
        .unwrap_or_default();
    for child in children {
        self.set_enabled(child, enabled);
    }

    // 4. Sync collider state in physics
    for pid in &pids {
        self.physics.set_collider_enabled(*pid, enabled);
    }
}
```

**Critical rule**: collider PIDs must be collected BEFORE ECS mutations.  The
borrow checker prevents reads from `self.ui` (which reads `self.world`) while
`self.world` is mutably borrowed.  Collect first, mutate second, physics third.

Without the collider sync, hidden UI elements remain in the quadtree and still
fire click/hover handlers despite being invisible.

---

## 7. is_disabled Parent-Chain Check

`is_disabled` checks the entity AND all ancestors through `UiNode.parent`:

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

Used in every render-list query as a guard:

```rust
if self.is_disabled(e) { continue; }
```

The parent walk is required because `set_enabled` recurses to children — a
child whose parent was disabled by a non-recursive path should still be
hidden.

---

## 8. Rc<Cell<T>> / Rc<RefCell<T>> Shared State

Collider `click_action` closures and `on_update` closures are separate
FnMut boxes with different lifetimes.  To share mutable state between them,
use `Rc` with interior mutability:

```rust
// Simple values (Copy types):
let h_val = Rc::new(Cell::new(0i32));
// In click_action:
h_val.set(h_val.get() + 1);
// In on_update:
engine.editor_height = h_val.get();

// String / heap values:
let editor_tgt = Rc::new(RefCell::new(String::from("none")));
// In click_action:
*editor_tgt.borrow_mut() = "tilemap".into();
// In on_update:
engine.editor_target = editor_tgt.borrow().clone();
```

### Frame ordering guarantee

`perform_calls` (which fires click/hover/enter/exit handlers) runs BEFORE
the `on_update` closure loop in `Engine::frame()`.  So click_action writes are
visible to on_update reads in the **same frame** — no one-frame lag.

### caution: other on_update closures may overwrite

Some on_update closures reset engine fields from their own Rc values each
frame.  For example, the tool_buttons closure syncs `editor_target` from
its internal state.  During synthetic test drags, the test framework must
re-apply editor state in the drag processing loop every frame to avoid
being overwritten by the tool_buttons sync.

---

## 9. Camera Math

The `Camera` struct holds `position: Vec3`, `scale: Vec3`, and `size: Vec3`
(viewport dimensions set by `resize`).

### fix() formula

The camera's "fix point" ensures `position * scale` maps to the viewport
centre:

```rust
pub fn fix(&self) -> Vec3 {
    (self.position * self.scale - self.size) / Vec3::new(2.0, 2.0, 1.0)
}
```

The division applies to the entire `(position*scale - size)` expression.
A previous bug applied the division only to `size`, producing
`pos*scale - size/2`, which at `scale=50` was off by `position*25` pixels.

### Matrix order: `T(-fix) * S(scale)`

```rust
pub fn matrix(&self) -> Mat4 {
    let fix = self.fix();
    Mat4::from_translation(-fix) * Mat4::from_scale(self.scale)
}
```

This is `T(-fix) * S` — translate to center the view first, THEN scale.
The opposite order (`S * T`) would multiply the translation by `scale`,
sending the visible area far from the intended camera position at high zoom.
`Transform::model_matrix()` uses the same time-tested `T(pos) * S(scale)`.

### Zoom: additive, not proportional

The TS zoom is additive:

```rust
let dz = engine.input.mouse_wheel * engine.time.delta;
engine.camera.scale.x += dz;
engine.camera.scale.y += dz;
let min = Vec3::new(0.1, 0.1, 1.0);
engine.camera.scale = engine.camera.scale.max(min);
```

Proportional zoom (`scale *= 1 + wheel * dt`) would produce the same
world-space change per scroll notch regardless of current zoom level —
additive zoom tracks the TS behaviour.  The `scroll_speed` constant (600)
is for WASD panning only, not zoom.

### WASD panning

Panning speed is `scroll_speed * delta` pixels per frame:

```rust
let speed = engine.scroll_speed * engine.time.delta;
if inp.is_key_down("KeyW") { engine.camera.position.y -= speed; }
if inp.is_key_down("KeyS") { engine.camera.position.y += speed; }
if inp.is_key_down("KeyA") { engine.camera.position.x -= speed; }
if inp.is_key_down("KeyD") { engine.camera.position.x += speed; }
```

### Orthographic projection

Set per-frame in `Engine::frame()`:

```rust
Mat4::orthographic_rh(0.0, viewport_w, viewport_h, 0.0, -10000.0, 10000.0)
```

- `left=0, right=vw` — screen x maps to [-1, +1]
- `bottom=vh, top=0` — screen y maps to [-1, +1] (Y grows downward)
- `near=-10000, far=10000` — deep range for isometric depth layering

### Real delta impact

The camera zoom/pan closures use `engine.time.delta` from the platform's
real frame timing.  A hardcoded `0.016` at 144 Hz causes zoom to decay
too fast and introduces wobble.  Always use real delta.

---

## 10. State Loading

`Engine::load_state(json)` parses a `state.json` document and spawns all
named entities.  The format is:

```json
{
  "entities": {
    "tilemap": { "components": [
      { "type": "Tilemap", "position": [...], ... }
    ]},
    ...
  }
}
```

### ordered entity spawn

Entities are iterated in JSON key order (serde_json with `preserve_order`
preserves insertion order).  Each entity gets:

1. Components deserialized via `registry::lookup(name)` → spawner.
   Spawners add both their own component and any subsumed components
   (e.g. `IsoAgent` spawner adds `IsoAgent`, `IsoSprite`, and `Transform`).
2. If `ed.components` is empty, a bare `()` component is added so the
   entity exists in the world.
3. `DebugName(name)` is inserted on every entity.
4. The entity handle is stored in `self.names: HashMap<String, Entity>`.
5. The name is appended to `self.name_order: Vec<String>`.

### name_order preservation

`self.name_order` is critical for `dump_state()` serialization — it
reproduces the entity iteration order when writing `state.json`.

### Sidecar decoding

`Engine::decode_map_data(base64_str)` decodes base64-encoded JSON arrays
for tilemap and nav mesh data (`map001.txt`, `map001.nav.txt`).

---

## 11. Known-divergent / non-functional

### No system scheduler

Unlike most ECS frameworks, there is no system graph, no stage ordering,
and no parallel dispatch.  `on_update` closures execute sequentially in
registration order.  This was an intentional parity choice with the TS
original's imperative update model.

### No ECS serialization for non-spatial components

`Collider`, `UiNode`, and `SdfTextRender` have no dumper
implementations and are not part of the registry.  They are created
programmatically at runtime by `init_*` functions and are not persisted
to `state.json`.  Serializing them would require dumper functions and
coordinating with the TS `state.json` schema.

### No mock hecs backend for testing

Tests of render-list queries and physics dispatch are end-to-end only
(CLASSIC_TEST).  There is no mock `hecs::World` or component spy layer.

### Global registry is not async-safe

The `LazyLock<RwLock<HashMap>>` global registry is safe for single-threaded
use and parallel tests with `--test-threads=1`.  It would need an
`Arc<RwLock>` pattern for multi-threaded workloads.

### Camera does not support rotation or 3D orbit

The Camera is strictly 2D orthographic with pan and uniform zoom.
There is no rotation, no perspective projection, and no look-at target.
This matches the TS camera, which was designed for isometric map editors.

### Transform::scale.z is unused in rendering

Most shaders use 2D scaling (x, y) from the transform model matrix.
The z component of scale exists for parity with TS but is not actively
used by GPU draw calls except in the iso sprite depth computation.
