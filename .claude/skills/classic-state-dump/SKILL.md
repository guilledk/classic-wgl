---
name: classic-state-dump
description: >
    State serialization (dump/load) system for classic-wgl's Rust port.
    Covers the bidirectional component registry (`ComponentReg`, `Dumper`,
    `subsumes`), `Engine::dump_state()`, sidecar files (`dump_map_data`,
    `dump_nav_data`, `dump_height_data`), cross-platform `save_file`,
    TS JSON format parity, and F9/Shift+F9 keyboard triggers.
    Use when implementing state persistence, debugging round-trip
    deserialization, adding new dumpable component types, or diagnosing
    key-order issues in the TS positional loader.
    Trigger phrases: "dump state", "state dump", "dump_state", "save_file",
    "state.json", "ComponentReg", "Dumper", "subsumes", "map001.txt",
    "serialize", "deserialize", "F9", "sidecar", "base64 map data".
compatibility: serde_json 1.0 (preserve_order), base64 0.22
metadata:
    author: classic-wgl
    version: '1.0'
allowed-tools: Read, Grep, Glob, Bash(git *), Edit, Write
---

# Skill: classic-state-dump

## Scope

Covers `crates/classic-core/src/registry.rs` (bidirectional registry),
`crates/classic-core/src/lib.rs` (dumper functions + `register_all_components`),
`crates/classic-engine/src/lib.rs` (`dump_state`, `dump_*_data`, `save_file`),
and `apps/{desktop,web}` (F9/Shift+F9 triggers, web download).

---

## 1. Architecture

```
                    ┌──────────────────────────────┐
                    │  registry.rs: ComponentReg   │
                    │  { name, spawn, dump, order, │
                    │    subsumes }                │
                    └──────┬──────────────┬────────┘
                           │              │
                    ┌──────▼──────┐ ┌────▼──────────┐
                    │ spawn()     │ │ dump()        │
                    │ deserialize │ │ serialize     │
                    │ JSON→hecs  │ │ hecs→JSON     │
                    └─────────────┘ └───────────────┘
                           │              │
                    ┌──────▼──────┐ ┌────▼──────────┐
                    │ load_state  │ │ dump_state    │
                    │ (TS→Rust)   │ │ (Rust→TS)     │
                    └─────────────┘ └───────────────┘
```

## 2. Bidirectional Registry

### Types

```rust
pub type Dumper = fn(&hecs::World, hecs::Entity) -> Option<serde_json::Value>;

#[derive(Clone, Copy)]
pub struct ComponentReg {
    pub name: &'static str,       // JSON "type" string
    pub spawn: Spawner,           // deserializer
    pub dump: Option<Dumper>,     // serializer (None = spawner-only)
    pub order: i32,               // dump priority (low = emitted first)
    pub subsumes: &'static [&'static str],  // subsumed component names
}
```

### Registration API

```rust
// Full registration (spawner + dumper)
registry::register(ComponentReg {
    name: "IsoAgent", spawn: …, dump: Some(dumper_isoagent),
    order: 40, subsumes: &["IsoSprite", "Transform"],
});

// Spawner-only (Transform is subsumed by all others, never dumped standalone)
registry::register_spawner("Transform", |b, v| { … });
```

### Registered components (7 total)

| Name | order | subsumes | dumper? |
|---|---|---|---|
| `Transform` | — | — | No (spawner-only) |
| `Tilemap` | 10 | `Transform` | Yes |
| `IsometricNavMesh` | 15 | `Transform` | Yes |
| `Sprite` | 20 | `Transform` | Yes |
| `IsoSprite` | 30 | `Transform` | Yes |
| `Animator` | 35 | — | Yes |
| `IsoAgent` | 40 | `IsoSprite, Transform` | Yes |

### Subsumes Logic

When dumping an entity, components are emitted in `order` priority (lowest first).
After each successful dump, the component's name AND all names in `subsumes` are
marked as "already dumped." If a later dumper's name (or a name it subsumes) is
already marked, that dumper is skipped.

Example: entity `navAgent` has `IsoAgent`, `IsoSprite`, and `Transform`.
1. Tilemap (order 10): no match, skip.
2. IsometricNavMesh (order 15): no match, skip.
3. Sprite (order 20): no match, skip.
4. IsoSprite (order 30): matches → dump, mark `IsoSprite` + `Transform` as done.
5. Animator (order 35): matches → dump.
6. IsoAgent (order 40): matches → dump, mark `IsoAgent` + `IsoSprite` + `Transform`.
   But `IsoSprite` was already dumped at step 4 — still, step 4 was correct
   because `IsoSprite` appeared before `IsoAgent`'s subsumption took effect.

**Important**: In practice, `IsoAgent` subsumes `IsoSprite` because an
`IsoAgent` entity always has an `IsoSprite` (the spawner creates both —
`classic-core/src/lib.rs:72-85`). The subsumes rule prevents emitting both
`"IsoSprite"` and `"IsoAgent"` component objects in the same entity's dump.

---

## 3. Engine::dump_state()

### Format

```json
{
  "entities": {
    "<name>": {
      "components": [
        { "type": "Tilemap", "position": [0,0,0], "scale": [45,45,1], … },
        { "type": "IsoSprite", "position": [5,10,0], "texture": "house", … }
      ]
    }
  }
}
```

Iterates `name_order: Vec<String>` (deterministic, matches JSON key order from
`state.json`). For each entity, runs dumpers in `order` priority, skipping
previously subsumed. Only entities in `self.names` are dumped (UI/widget entities
are skipped — they're rebuilt from code on every launch).

### Key rules (TS positional loader compatibility)

1. **`"type"` must be the first key** in every component object. The TS loader
   uses `getObjectValues()` which iterates JSON keys in insertion order, finds
   `component.type` by value identity, and removes it. Key order IS load-bearing.
2. **Remaining keys must be in TS constructor parameter order.**
3. **Vector fields are JSON arrays** `[x, y, z]` (glam serde default).
4. **`data` field is a filename string** (e.g. `"map001.txt"`), not the inline
   array. Actual tile/height/nav arrays go to sidecar files.
5. **`NavMesh` dumper deliberately omits `position`/`scale`** — TS
   `IsometricNavMesh.dump()` doesn't emit them (inherited from the tilemap).

### Tilemap dumper specifics

- `"data"`: maps to `data_url` (filename string: `"map001.txt"`)
- `"heightScale"`: only emitted if non-zero (TS parity — optional field)
- `"heightData"`: NOT emitted (goes to sidecar `map001.height.txt`)

### IsoAgent dumper specifics

- Includes all `IsoSprite` fields (position, scale, texture, tilemap, frame,
  tileSetSize, anchor, footprint) plus agent-specific fields (speed, animSpeed).
- This matches TS `IsoAgent.dump()` which calls `super.dump()` = `IsoSprite.dump()`.
- `animSpeed` maps to the `Animator.speed` field — TS `IsoAgent.anim` holds
  a reference to the Animator component and dumps its `.speed`.

---

## 4. Sidecar Files

Three methods on `Engine`:

```rust
pub fn dump_map_data(&self) -> Option<String>   // base64(JSON Vec<u32>), 200×200
pub fn dump_nav_data(&self) -> Option<String>   // base64(JSON Vec<u32>), 200×200
pub fn dump_height_data(&self) -> Option<String>  // base64(JSON Vec<f32>), (200+1)×(200+1)
```

Format matches `public/map001.txt`: single-line base64 of a JSON integer array
(`[0,0,0,19,9,…]`). The `decode_map_data()` function in `classic-engine` can
decode these back.

---

## 5. Engine::save_file()

```rust
pub fn save_file(&self, name: &str, data: &str)
```

- **Native**: writes to `{CLASSIC_DUMP_DIR}/{name}` (default `./dump/`).
  Creates the directory if it doesn't exist. Logs path + byte count on success,
  warns on failure.
- **Web**: creates a `Blob` from the data, generates an object URL, creates a
  synthetic `<a download="name">` element, clicks it, revokes the URL.
  Uses `wasm-bindgen`, `web-sys`, `js-sys` (gated behind `#[cfg(target_arch)]`).

No platform-trait changes needed — `#[cfg]` gates handle both targets in
`classic-engine` directly.

---

## 6. Triggers

| Trigger | What it does |
|---|---|
| **F9** | Dumps `state.json` only |
| **Shift+F9** | Dumps `state.json` + 3 sidecars (`map001.txt`, `map001.nav.txt`, `map001.height.txt`) |
| `CLASSIC_DUMP_ON_EXIT=1` | (not yet wired — deferred) |

Implemented in `init_debug_toggles()` (`classic-engine/src/lib.rs:~1635`). The
`is_key_down("ShiftLeft")` / `is_key_down("ShiftRight")` check gates the sidecar
dump.

---

## 7. Adding a New Dumpable Component

1. Write a dumper function with signature `fn(&hecs::World, hecs::Entity) -> Option<serde_json::Value>`.
   - Build a `serde_json::Map` with `"type"` as the first key.
   - Use `serde_json::json!([x, y, z])` for glam vectors.
   - Return `Some(Value::Object(map))` on success, `None` if entity doesn't have the component.
2. Add a `ComponentReg` entry in `register_all_components()`:
   ```rust
   registry::register(ComponentReg {
       name: "MyComponent",
       spawn: |b, v| { … },
       dump: Some(my_dumper_fn),
       order: 25,  // choose an unused priority slot
       subsumes: &[],
   });
   ```
3. Choose `order` to control emission position among the other components on the entity.
4. If your component synthesizes a `Transform` in its spawner (like `Tilemap` does),
   add `"Transform"` to `subsumes`.
5. Add a round-trip test in `tests/dumpers.rs`.

---

## 8. Common Pitfalls

| Symptom | Cause | Fix |
|---|---|---|
| `"type"` not first key | `serde_json::Map` insertion order not controlled | Insert `"type"` first, then all other fields |
| Dump missing `"type"` field | Forgot to add type key to the map | Always `m.insert("type", Value::String(...))` first |
| `IsoAgent` dump includes extra `"IsoSprite"` entry | `subsumes` rule not set | Check `subsumes: &["IsoSprite", "Transform"]` |
| State.json has snake_case keys | Missing `camelCase` key names in dumper | Use explicit camelCase string keys in inserts |
| `dump_map_data()` returns `None` | Tilemap entity not named `"tilemap"` in `self.names` | The function hardcodes the name lookup |
| Tests: `registry::register` takes 2 args | Using old API | Use `registry::register_spawner(name, fn)` for spawner-only |
| NavMesh dump includes position/scale | Dumper emitted fields that TS doesn't expect | NavMesh dumper deliberately omits them (TS parity) |
| Float precision in tests | `speed: 2.6` not exactly representable | Use values that round-trip cleanly (e.g. `2.5`) |
