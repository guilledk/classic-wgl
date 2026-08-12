---
name: classic-physics
description: >
    Collision detection, interaction dispatch, and pathfinding for
    classic-wgl's Rust port.  Covers PhysicsProvider lifecycle,
    click/hover/enter/exit dispatch with priority sorting,
    consumes_click pre-scan, selection rectangle, collider enabled
    state, UI collider integration, GJK algorithm, Quadtree spatial
    partition, and A* pathfinding.
    Trigger phrases: "perform_calls", "gjk_test", "begin_frame",
    "begin_selection", "consumes_click", "mouse_clicked",
    "click_priority", "enter handler", "exit handler",
    "collider enabled", "collider PID", "find_path", "A*",
    "pathfinder", "pathfinding".
---

# classic-physics

Collision detection, interaction dispatch, GJK, Quadtree, and A\* pathfinding
for `classic-wgl`.

---

## 1. PhysicsProvider Lifecycle

`PhysicsProvider` owns all registered colliders in a `HashMap<u32, ColliderEntry>`
(keyed by PID — provider-assigned integer ID).  Two `VirtualCollider` singletons
live at fixed PIDs:

| PID | Name        | Shape   | Purpose |
| --- | ----------- | ------- | ------- |
| 0   | `mouse`     | Circle  | Mouse cursor for click / enter-exit testing |
| 1   | `selection` | Polygon | Drag-selection rectangle, rebuilt each frame |

The per-frame sequence in `Engine::frame` is:

1. **`resize_screen(w, h)`** — rebuilds the quadtree `screen_collider` bounds
   and clears all quadtree nodes.  Called only when the viewport changes.
2. **`begin_frame()`** — clears the quadtree, then re-inserts every *enabled*
   collider whose bounding `Rect` intersects the screen.  Disabled colliders
   are skipped entirely.  The mouse virtual collider is always inserted.
3. **Mouse position update** — `physics.mouse.position` and `update_rect()` are
   set from `InputState::mouse_pos` before dispatch.
4. **`consumed_click = false`** and **`mouse_clicked = true/false`** are set
   from `InputState::was_mouse_pressed(0)`.
5. **`perform_calls()`** — runs collision detection, click dispatch, and
   enter/exit dispatch (see sections 2–4).
6. **`update_hover()`** — (called on `UIManager` after `perform_calls`) reads
   the per-frame `colliding` table to highlight hovered UI elements.

`perform_calls` MUST be called exactly once per frame, after `begin_frame` and
mouse-position updates.

---

## 2. Click Dispatch

Click dispatch only fires when `mouse_clicked` is `true` (i.e. on the exact
frame the mouse button goes down).  If the mouse is outside the screen rect
(viewport bounds), dispatch is skipped entirely.

### consumes\_click Pre-scan

Before sorting and dispatching click handlers, `perform_calls` does a pre-scan
of all colliders in the mouse's quadtree cells.  If any collider with
`consumes_click == true` intersects the mouse (via GJK), the scan breaks
early.  The caller is responsible for checking this state: in `Engine::frame`,
if `physics.consumed_click` is true after `perform_calls`,
`self.ui_consumed_click` is set to `true`, which gates tilemap drag-selection
on the next frame.

### Priority Sort

Click targets are collected from the quadtree: only colliders with PID ≥ 2
that have at least one `Click` handler and collide with the mouse via GJK.
Targets are sorted by **`click_priority` descending**, then **PID ascending**
as tie-break.  Higher priority numbers fire first.

### Dispatch Flow

For each target in sorted order:

1. Fire each `Click` handler closure.  Handlers return `bool` — `true` means
   "stop propagation".
2. If any handler returns `true`, and the collider has `consumes_click` set,
   `self.consumed_click` is set to `true`.
3. A `stop` flag breaks the priority-ordered loop, so lower-priority targets
   do NOT fire after a handler requests stop.

A click handler that returns `false` does NOT set `consumes_click` and does
NOT stop propagation to the next target.

---

## 3. Enter / Exit Dispatch

Each frame, `perform_calls` builds a `colliding` table: for every PID (including
0 and 1), it queries the quadtree for candidates, runs GJK pairwise tests, and
records `(a_pid → Set of intersecting PIDs)`.

The previous frame's `colliding` table is snapshotted into `collided` before
`colliding` is rebuilt.  This allows transition detection:

### Enter

For every *new* pair `(a_pid, b_pid)` in this frame's `colliding` that was NOT
in last frame's `collided`, all `Enter` handlers on the collider with PID
`a_pid` fire.  Virtual PIDs (0,1) are skipped — only registered colliders may
have enter/exit handlers.

### Exit

For every pair that was in last frame's `collided` but is NOT in this frame's
`colliding`, all `Exit` handlers on the collider with PID `a_pid` fire.

Both enter and exit dispatch iterate per `a_pid`; each collider tracks which
other PIDs it is colliding with.  The handlers are closures stored in
`Collider::handlers: HashMap<HandlerKind, Vec<Box<dyn FnMut() -> bool>>>`.

---

## 4. Selection Rectangle

Drag-selection uses the `selection` virtual collider (PID 1).  State tracking
is shared between `PhysicsProvider` and `Engine`:

- `Engine::selection_mode` = -1 (idle), 1 (actively dragging).
- `Engine::selection_begin_screen` stores the screen-space origin of the drag.

### Per-frame lifecycle

1. **Mouse down** (not UI-consumed): `selection_mode = 1`, record
   `selection_begin_screen`, call `physics.begin_selection(mouse_pos)`.
2. **While dragging** (`selection_mode == 1`): call
   `physics.update_selection(begin, current)` which computes the bounding rect
   from min/max and sets the selection's position + scale.
3. **Mouse release**: `selection_mode = -1`,
   `physics.end_selection()` fires `Selection` handlers on every registered
   collider whose bounding rect intersects the selection rect AND passes GJK
   test against the selection virtual collider.
4. **`apply_editor_selection()`** runs after `end_selection`, applying the
   editor target (tile paint, height paint, or nav paint) to the final
   selection region.

The selection virtual collider positions itself at `(-1, -1)` when idle
(offscreen).

---

## 5. Collider Enabled State

Each `ColliderEntry` has an `enabled: bool` flag.  The engine sets this via
`PhysicsProvider::set_collider_enabled(pid, enabled)`.

### Behaviour

- **`begin_frame` skip**: disabled colliders are not inserted into the
  quadtree, so they are invisible to click dispatch, enter/exit, selection,
  and hover.
- **Quadtree rebuild**: `begin_frame` rebuilds the quadtree from scratch every
  frame, so toggling `enabled` takes effect on the very next frame — no
  explicit removal needed.
- **`set_enabled` on Engine**: the engine-side `set_enabled(entity, bool)`
  adds/removes the `Disabled` component on the entity (and all UI children),
  then calls `physics.set_collider_enabled` for each collider PID associated
  with the entity.  Collider PIDs are collected via
  `UIManager::collect_collider_pids`.
- **`is_disabled` check**: walks the parent chain of `UiNode::parent` — if any
  ancestor has `Disabled`, the entity is considered disabled.  This gates
  render-list inclusion (skipped entirely) and pathfinding (disabled agents
  are skipped by the `IsoAgent` query).

---

## 6. UI Collider Integration

UI elements (buttons, containers) register colliders with the physics system
through `UIManager`.

### `add_collider_to_elem`

Called by `spawn_button` and manual collider attachments.  Registers a new
collider with `PhysicsProvider`, sets it as a `Polygon` matching the UI
element's size, and returns the PID.  The PID is stored in the
`UiNode::collider_pid` field.

### `sync_colliders`

Called from `refresh_layout()` after the UI tree is laid out.  For every UI
element with an associated collider PID, `sync_collider_rect(pid, x, y, w, h)`
rebuilds the collider polygon to match the element's current screen
position and size.  This ensures button hit-testing stays aligned with
layout.

### `sync_collider_rect`

Receives `(x, y, w, h)` in screen-space (pixel coordinates).  Rebuilds the
collider as a 4-vertex polygon from `(0,0)` to `(w,h)` at position
`(x, y, 0)`.  Called after every layout refresh and every manual element
reposition.

---

## 7. GJK Algorithm

The Gilbert-Johnson-Keerthi algorithm lives in `classic-core/src/gjk.rs` and
implements 2D simplex evolution.  It operates on any type implementing the
`GjkShape` trait:

```rust
pub trait GjkShape {
    fn center(&self) -> Vec3;
    fn support(&self, dir: Vec3) -> Option<Vec3>;
}
```

### ShapeRef Adapter

`ShapeRef` in `collision.rs` adapts a `&Shape` + `position` + `scale` into a
`GjkShape`.  It delegates `center()` and `support()` to the corresponding
methods on `Shape`, passing through the position and scale.

### Circle Support

Returns `position + normalize(dir) * radius * scale`, with `z = 0`.  The
radius is `diameter / 2.0`.

### Polygon Support

Transforms each vertex by the model matrix `T(position) * S(scale)`, then
returns the vertex with the largest dot product against `dir`.

### Simplex Evolution

`GjkContext::evolve_simplex` handles 0, 1, 2, and 3-vertex cases:

- **0 verts**: seed direction = `center(B) - center(A)`.
- **1 vert**: flip direction.
- **2 verts**: use triple product to find perpendicular direction towards
  origin.
- **3 verts**: check which Voronoi region the origin lies in, drop a vertex if
  needed, or return `Intersection`.
- **`panics` on > 3 verts** (only 2D simplex is supported).

The outer loop in `perform_test` runs up to 1000 iterations; a panic fires if
this is exceeded.

### Known Edge Case

`evolve_simplex` panics for simplex sizes > 3.  In practice this never occurs
because the 3-vertex case always returns `Intersection` or reduces to 2
vertices, but degenerate inputs (exactly co-linear points) could theoretically
cause a 4-vertex accumulation via the `add_support` → `evolve_simplex` cycle.
No practical repro exists.

---

## 8. Quadtree

The quadtree in `classic-core/src/quadtree.rs` is a generic spatial partition
with a `RectBounds` trait:

```rust
pub trait RectBounds {
    fn rect(&self) -> Rect;
}
```

### ColliderHandle

`ColliderHandle` implements `RectBounds` with `{ pid, rect }`.  The `rect`
field is the cached bounding rect of the collider's shape (in screen-space),
computed at insertion time in `begin_frame`.

### Spatial Partition Behaviour

- **max\_objects = 10, max\_levels = 4** — the quadtree splits a node into 4
  children when it exceeds 10 objects and hasn't reached depth 4.
- **Split quadrants**: 0 = top-right, 1 = top-left, 2 = bottom-left, 3 =
  bottom-right.
- **Straddling objects**: objects whose bounding rects overlap multiple
  quadrants stay in the parent node rather than being pushed into children.
- **Retrieval**: `retrieve(r)` returns all objects in overlapping quadrants
  plus all objects in the current node (straddlers and pre-split objects).
  No deduplication is needed — each object lives in exactly one node.
- **Rebuild every frame**: `begin_frame` calls `clear()` + re-inserts all
  enabled colliders.  This is cheap; collider counts are small (< 100 in
  typical scenes).

---

## 9. A\* Pathfinding

The pathfinder in `classic-core/src/pathfinder.rs` runs in-thread (no web
worker — the TS `pathfinder.ts` worker pattern was dropped in the Rust port).

### Signature

```rust
pub fn find_path(
    nav_data: &[i32],   // 1=walkable, 0=blocked, row-major
    size_x: i32,
    size_y: i32,
    from: GridCell,      // (x, y) integer cell coordinates
    to: GridCell,
) -> Option<Vec<GridCell>>
```

### Octile Heuristic

The heuristic in `find_path` is a Chebyshev-approximation:

```
dx + dy + (√2 - 2) * min(dx, dy)
```

This is admissible and consistent for 8-directional grids where cardinal
moves cost 1.0 and diagonal moves cost √2 ≈ 1.414.

### BinaryHeap with Key Wrapper

A custom `Key { cost: f32, cell: (i32, i32) }` struct implements `Ord` with
**reversed** cost comparison (lower cost = higher priority).  The tie-break
uses `cell` tuple ordering, which prevents non-deterministic pop ordering for
equal-cost nodes.

### Path Reconstruction

`reconstruct_path` walks the `came_from` array backwards from `to` to `from`,
reverses the resulting vector, and returns it.  The returned path includes
both endpoints.

### Integration in Engine::init\_navigation

- Click-to-move converts the agent's iso position and the tilemap's
  `mouse_iso_pos` to integer grid cells.
- On click (when the agent is selected), `find_path` is called with the
  current nav data as `&[i32]`.
- **Impassable destinations are rejected before the search runs**: if
  `nav_data[cy * size_x + cx] == 0` the click is ignored and `find_path` is
  never called.  Without this, a click on a cliff makes A* exhaust every
  reachable cell before it can return `None` — 3 ms on a 200x200 map, 21 ms on
  a 400x400 map, a dropped frame.  Rejecting early is also the correct
  behaviour (clicking a wall should do nothing, not walk to somewhere
  adjacent).
- Waypoints are offset by +0.5 to centre them within tiles (matching TS
  behaviour).
- The first waypoint is replaced with the agent's exact floating-point
  position (matches TS `this._path[0] = [this.position[0],
  this.position[1]]`).
- The agent's state is set to `AgentState::FollowPath` with `target_index = 1`
  and `delta = 0.0`.

---

## 10. Nav Mesh Walkability

The nav mesh is a flat `Vec<u32>` where `1` = walkable, `0` = blocked.  It is
stored on the `NavMesh` component and also used as the GPU data texture for
the nav mesh overlay.

### Height-based Passability

`sync_nav_heights()` runs after every height editing operation.  For each nav
cell, it checks the corresponding height value in the parent tilemap's
`height_data`.  A cell is marked blocked if any adjacent cell has a height
difference > 2.0 (cliff condition).  This check is done in 4 directions
(left, right, up, down).

Walkability is recomputed in two contexts:

1. **`init_navigation()`**: syncs nav mesh walkable flags from parent tilemap
   heights, then overwrites nav data with the decoded `map001.nav.txt`
   contents (nav file is authoritative).
2. **`sync_nav_heights()`**: called after height paint operations
   (`apply_editor_selection`), checks all nav cells, marks changed cells, and
   rebuilds the nav GPU mesh if any cell changed.

### GPU Rebuild

`rebuild_nav_gpu()` reconstructs the nav mesh vertex buffer and tile data
texture from the current `NavMesh::data`.  This uses the same `build_mesh` /
`build_tile_texture` path as the terrain tilemap, with flat heights (1.0).

---

## 11. Known-divergent / Non-functional

- **GJK max-iteration panic**: `perform_test` panics with `max iterations
  (1000) reached` if the simplex evolution fails to converge.  This is a
  direct port of the TS guard; it has never fired in practice but the
  fallback behaviour (return `false` or use a separating-axis fallback) was
  not implemented.

- **Single-frame enter/exit**: because enter/exit dispatch compares
  frame(N) vs frame(N+1) `colliding`/`collided` tables, an enter + exit
  that both occur within a single frame (collider passing through another at
  very high velocity) will be missed entirely.  The TS original has the same
  limitation.

- **No multi-agent pathfinding**: A\* runs on a single static nav grid.
  There is no cooperative avoidance, no dynamic obstacle updating, and no
  local-repair.  Multiple `IsoAgent` entities cannot share the nav grid
  safely — only one agent (`navAgent`) is provisioned.

- **Nav mesh is NOT automatically updated during gameplay**: walkability is
  only recomputed when height edits occur through the tool panel.  Runtime
  terrain changes from other systems would not trigger `sync_nav_heights`.

- **`click_priority` tie-sorting by PID**: when two colliders have the same
  `click_priority`, the one with the lower PID fires first.  This is
  deterministic but unintuitive — PID is assigned by registration order,
  which in turn depends on UI widget construction order.  If you need
  deterministic priority between equal-priority colliders, use explicit
  priority values rather than relying on PID ordering.

- **No timeout/abort for long paths**: `find_path` has no iteration limit or
  timeout, and runs synchronously on the render thread.  On very large grids
  with complex obstacle mazes it will explore the entire reachable space
  before returning `None` — measured 3 ms (exhaustive) at 200x200 vs 21 ms at
  400x400.  The impassable-destination pre-check removes the only routine
  trigger, but beyond roughly 600x600 a real solution (iteration budget, or
  moving A* off the render thread as the TS Web Worker did) is required.
