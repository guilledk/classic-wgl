---
name: classic-rust-collision
description: >
    A* pathfinding, nav mesh loading, click-to-move navigation, and
    collision/interaction dispatch for classic-wgl's Rust port.
    Covers `classic-core/src/pathfinder.rs`, `init_navigation()`,
    `PhysicsProvider` lifecycle (begin_frame, perform_calls,
    begin_selection/update_selection/end_selection), `HandlerKind`
    dispatch (Click, Selection, Enter, Exit), drag-selection pipeline
    (selection_mode, selection_iso_begin/end), collider enabled state,
    and click dispatch guard (mouse_clicked, consumed_click,
    ui_consumed_click). Use when debugging pathfinding failures, click
    not firing, hover triggering handlers, selection not working, or
    drag painting issues.
    Trigger phrases: "gjk_test", "perform_calls", "consumes_click",
    "mouse_clicked", "click dispatch", "selection dispatch", "drag",
    "selection_mode", "begin_selection", "end_selection",
    "apply_editor_selection", "HandlerKind", "collider enabled".
compatibility: hecs 0.10, glam 0.29
metadata:
    author: classic-wgl
    version: '0.2'
allowed-tools: Read, Grep, Glob, Bash(git *), Edit
---

# Skill: classic-rust-collision

## Scope

Covers `crates/classic-core/src/collision.rs` (~600 LOC),
`crates/classic-engine/src/lib.rs` (drag pipeline + editor painting),
and click/selection dispatch across the PhysicsProvider.

---

## 1. Click Dispatch Guard

```rust
pub struct PhysicsProvider {
    pub consumed_click: bool,     // set when handler fires with consumes_click
    pub mouse_clicked: bool,      // set from was_mouse_pressed(0) before perform_calls
}
```

In `frame()`:
```rust
self.physics.mouse_clicked = self.input.was_mouse_pressed(0);
self.physics.perform_calls();

if self.physics.consumed_click {
    self.ui_consumed_click = true;
}
```

**`mouse_clicked` MUST be set before `perform_calls()`.** Without this,
collider click handlers fire every frame on hover (not just on press).

In `perform_calls`:
```rust
} else if self.mouse_clicked {
    // ... dispatch click handlers sorted by priority desc, pid asc ...
}
```

---

## 2. Persistent Handlers (NOT std::mem::take)

Click, enter, exit, and selection handlers must survive across frames.

```rust
// CORRECT — handlers survive:
if let Some(handlers) = entry.collider.handlers.get_mut(&HandlerKind::Click) {
    for h in handlers.iter_mut() {
        if h.as_mut()() {   // call the FnMut
            stop = true;
            if consumes { self.consumed_click = true; }
            break;
        }
    }
}

// WRONG — handlers consumed on first fire:
let taken = std::mem::take(entry.collider.handlers.entry(...).or_default());
for mut h in taken { h(); }
```

Applied to: Click dispatch in `perform_calls`, Selection dispatch in
`end_selection`, Enter/Exit dispatch in `perform_calls`.

---

## 3. Click Dispatch: Priority + consumes_click Flow

1. Pre-scan: checks if any collider under mouse has `consumes_click = true`
2. If `mouse_clicked` is true: dispatch handlers sorted by
   `click_priority` desc, then `pid` asc
3. First handler returning `true` stops propagation
4. If handler returned `true` AND collider had `consumes_click = true`:
   `self.consumed_click = true`
5. Engine reads `physics.consumed_click` → `self.ui_consumed_click = true`

**Palette pattern**: collider with `consumes_click = true` and a dummy
handler `|| true`. This sets `consumed_click` and blocks map editing
without doing any state change itself. The actual click logic lives in
`on_update` which fires later.

---

## 4. Drag-Selection Pipeline

### State fields (Engine)
```rust
pub selection_mode: i32;           // -1 = idle, 1 = dragging
pub selection_begin_screen: Vec3;  // screen pixel where drag started
```

### Mouse press handler (in frame())
```rust
if self.input.was_mouse_pressed(0) && !self.ui_consumed_click {
    self.selection_mode = 1;
    self.selection_begin_screen = Vec3::new(mp.x, mp.y, 0.0);
    // Snapshot iso begin from tilemap
    tm.selection_iso_begin = tm.mouse_iso_pos;
    self.physics.begin_selection(Vec3::new(mp.x, mp.y, 0.0));
}
```

### Per-frame update (in frame())
```rust
if self.selection_mode == 1 {
    self.physics.update_selection(self.selection_begin_screen, Vec3::new(mp.x, mp.y, 0.0));
}
```

### Mouse release handler (in frame())
```rust
if self.input.was_mouse_released(0) && !self.ui_consumed_click {
    let just_finished = self.selection_mode == 1;
    if self.selection_mode == 1 {
        self.selection_mode = -1;
        tm.selection_iso_end = tm.mouse_iso_pos;
    }
    self.physics.end_selection();
    if just_finished { self.apply_editor_selection(); }
}
```

### Shader receives dynamic selection_mode
```rust
gfx.draw_tilemap(..., self.selection_mode, ...);
// -1 = no selection visual, 1 = cyan drag rectangle
```

---

## 5. apply_editor_selection — Bounding Box + Paint

Reads `selection_iso_begin` and `selection_iso_end`, computes bounding box
using min/max of both coords (handles any drag direction):

```rust
let from_x = b.x.min(e.x).floor().max(0.0) as i32;
let from_y = b.y.min(e.y).floor().max(0.0) as i32;
let to_x   = b.x.max(e.x).ceil().min(tm.size_x as f32) as i32;
let to_y   = b.y.max(e.y).ceil().min(tm.size_y as f32) as i32;
```

**DO NOT assume begin < end.** TS `getSelection()` uses `vec2.min/max`.
Without this, right-to-left or bottom-to-top drags produce `tile_count=0`.

Handles three targets: `"height"`, `"tilemap"`, `"navMesh"`.
After painting, calls appropriate GPU rebuild (`rebuild_tilemap_mesh`
or `rebuild_nav_gpu`).

---

## 6. Height Data Stride

`height_data` uses `(size_x + 1)` stride (extra sample per row for
edge vertices). Array size = `(size_x + 1) * (size_y + 1)`.

Paint loops MUST use `y * (size_x + 1) + x`:
```rust
let idx = (y * (tm.size_x + 1) + x) as usize;  // CORRECT
let idx = (y * tm.size_x + x) as usize;         // WRONG — offset bug
```

---

## 7. Nav Mesh Editing

When `editor_target == "navMesh"`, `apply_editor_selection` fills
`NavMesh.data` with `editor_nav_tile` and calls `rebuild_nav_gpu()`.

Nav mesh GPU rebuild reads nav data, tilemap height_scale, and builds
mesh + tile texture via `build_mesh` + `build_tile_texture`.

After height paint, `sync_nav_heights` recalculates walkability from
height differences (>2 = unpassable) and calls `rebuild_nav_gpu`.

---

## 8. ui_consumed_click Guard Pattern

Any closure that responds to mouse clicks must check `ui_consumed_click`:

```rust
// Navigation:
if !engine.input.was_mouse_pressed(0) || engine.ui_consumed_click { return; }

// Map painting (in frame()):
if self.input.was_mouse_released(0) && !self.ui_consumed_click { ... }

// Palette click detection (on_update):
if engine.input.was_mouse_pressed(0) {
    // Don't check ui_consumed_click here — the palette collider set it,
    // but we still need to process the click on the palette itself.
}
```
