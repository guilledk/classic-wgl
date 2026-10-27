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
early (it only short-circuits — it does not set `consumed_click`; that is done
in the dispatch loop below).  The caller reacts to the result in
`Engine::frame`: if `physics.consumed_click` is true after `perform_calls`, the
`"ui_consumed_click"` guest flag is set, which gates tilemap drag-selection.

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
other PIDs it is colliding with.  The handlers are closures stored in the
private `ColliderEntry::handlers: HashMap<HandlerKind, Vec<Box<dyn FnMut() ->
bool>>>` map in `collision.rs` (not on the serializable `ColliderData`
component).

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
4. **Selection end** runs the `on_selection_end` hook surface.  The demo
   registers `editor::apply_editor_selection` into it (a demo-crate function,
   not an engine method), which applies the editor target (tile paint, height
   paint, or nav paint) to the final selection region.

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
element's size, and returns the PID.  The PID is tracked in the private
`UIManager` `UiColliderEntry { elem, collider_pid, … }` table (not on
`UiNode`), accessed via `collider_pid_for` / `collect_collider_pids`.

### `sync_colliders`

Called by `Engine::frame` (after `ui.refresh_layout`) once the UI tree is laid
out.  For every UI
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

Returns `position + normalize_or_zero(dir) * radius * scale`, with `z = 0`.
The radius is `diameter / 2.0`.  `normalize_or_zero` is load-bearing: a plain
`normalize()` of a zero-length `dir` is NaN, and a NaN support point makes
`add_support`'s `dir.dot(diff) >= 0.0` false, which GJK reads as
`NoIntersection`.

### Polygon Support

Transforms each vertex by the model matrix `T(position) * S(scale)`, then
returns the vertex with the largest dot product against `dir`.

### Simplex Evolution

`GjkContext::evolve_simplex` handles 0, 1, 2, and 3-vertex cases:

- **0 verts**: seed direction = `center(B) - center(A)`, falling back to
  `Vec3::X` when that delta is degenerate (concentric shapes).
- **1 vert**: flip direction.
- **2 verts**: use triple product to find perpendicular direction towards
  origin; a degenerate perpendicular returns `Intersection` (see below).
- **3 verts**: check which Voronoi region the origin lies in, drop a vertex if
  needed, or return `Intersection`.
- **`panics` on > 3 verts** (only 2D simplex is supported).

The outer loop in `perform_test` runs up to 1000 iterations; a panic fires if
this is exceeded.

### Degenerate Directions

**A zero search direction is a wrong answer, not a crash.**  GJK used to hand
`Vec3::ZERO` to `support` for concentric shapes and silently report
`NoIntersection` for two fully overlapping colliders.  Two guards keep that
from recurring:

- **Coincident centres** (0-vert case).  `center(B) - center(A)` is zero, so the
  seed direction falls back to `FALLBACK_DIR` (`Vec3::X`).  Any fixed axis does;
  only non-zero matters.
- **Collinear 2-simplex.**  `c` and `b` were found along opposite directions, so
  the origin already lies between them.  When `triple_product(cb, c0, cb)`
  degenerates, the origin sits *on* segment CB and is inside the Minkowski
  difference: return `Intersection` rather than searching along nothing.

Both use `MIN_DIR_LEN_SQ` (`1e-12`) as the squared-length floor.  Regression
coverage is the `detects_concentric_*` / `circle_support_of_zero_dir_is_finite`
tests in `crates/classic-core/tests/gjk.rs`.

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

**The quadtree is screen-space, always.**  A `ColliderSpace::World` collider is
inserted by the projection `begin_frame` computes and stores on
`ColliderEntry.projected`.  Every reader of that entry must go through the same
projection, so both `shape_of` and `handle_for` derive from it (`handle_for`
calls `shape_of` rather than re-deriving the rect).  Handing a *world* rect to
`Quadtree::retrieve` looks harmless in a small scene, because `retrieve`
returns everything held at each visited level, but it silently drops candidates
the moment the tree subdivides — which is how Enter/Exit stopped firing for
world colliders under a non-identity camera.  A test for this needs >10
colliders to force the split.

The projection is a `Projected { shape, position }`, not a bare `Shape`.  A
`Shape::Circle` carries only a diameter, so its screen position has to travel
beside it — the `Circle` arm once scaled the diameter and dropped the position,
putting every world-space circle at screen origin.  A projected polygon bakes
its placement into its verts and carries `position: ZERO`.  Both read back at
unit scale.  `world_circle_is_hit_at_its_projected_position` and
`world_circle_is_not_hit_at_screen_origin` guard it.

**Handler tests share state through `Rc<Cell<_>>`, never `Cell::clone`.**  A
cloned `Cell` (or a `Copy` flag moved into the closure) is a detached copy: the
handler mutates it and the test's assertion can never fail.  Pair every
"did not fire" assertion with a same-geometry case that *does* fire
(`click_does_not_fire_without_mouse_clicked` / `click_fires_with_mouse_clicked`),
or the negative passes just as well when nothing could have fired.

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

The standalone `classic-pathfinder` crate (re-exported as `classic_core::pathfinder`,
`#![no_std]`) implements A* over an immutable `NavSnapshot`
(`size_x`/`size_y`/`data`, `Arc`-shared).  It is the single source of truth for
native + web: compiled to `pathfinder.wasm` for the web `Worker`.  The engine
offloads searches to a host `PathfinderWorker` (`classic-worker` — native
`std::thread` on desktop, a dedicated `Worker` on web) so the render thread
never blocks mid-frame.  The engine owns the request id and the snapshot;
`request_path` submits, `poll_path` collects, `join_workers` is the determinism
barrier.  `Engine::find_path` (the inline search over the live `NavMesh`) is
retained only for tests.

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

### Vehicle search (footprint / slope / jump / turn-cost)

Wheeled vehicles use a richer search than the 1×1 humanoid A\* — still a
grid search (no state lattice), but with four layered gates:

- **Footprint** (`VehicleFootprint`): an authored `path_footprint` is
  `Cells` — `erode_for_footprint` inflates the walkability grid by it once,
  then A\* runs as a point agent.  Otherwise it is `Oriented { body, pivot }`
  (the `oriented` module): `body` is a `BodyRect` in tiles, chassis frame
  (`+x` forward, `+y` left) — the def's `path_body_m`, else the wheel
  contacts' rectangle (`IsoVehicle.path_body`).  The search state is a cell
  plus its incoming heading (8 directions, or none at the start; collapsed
  to one per cell when `turn_cost` is 0 and there is no pivot).  A step
  `a -> b` along `d` is legal when the body swept from `a` to `b` at `d`
  (front pushed out by the step length) is clear: that is the body at `a`
  turned to `d`, so a turn needs both headings clear; a `pivot_turn`
  vehicle also clears every 22.5° heading it turns through at `a`.  The
  first step checks only the body at `b` (the start is relaxed).  Each
  footprint is rasterized once per heading into row spans (a cell counts
  when the body's interior overlaps it), blocked cells into per-row prefix
  counts, and each `(cell, d)` answer is memoized, so only cells the A\*
  reaches are checked.  `IsoVehicle.path_footprint` stays the East-facing
  axis box for selection and `vehicle_footprint_radius`.
- **Slope**: `derive_vehicle_slope_nav` derives a per-tile feasibility grid by
  sampling terrain at the wheel positions (`±wheelbase/2`, `±track/2`) in all 8
  headings; a tile is walkable if *some* heading keeps `|pitch| ≤ pitch_max` and
  `|roll| ≤ roll_max`.
- **Jump**: `find_path_for_footprint_with_jumps` allows a *downward* step whose
  drop (metres) is within `safe_fall_m` even when the slope grid marks it blocked,
  scaled by `jump_cost`.
- **Turn cost**: the cell-state `a_star` step-cost closure takes the *previous* cell —
  `FnMut(prev: Option<GridCell>, current, neighbour)` — and `turn_penalty` adds
  `turn_cost` per 45° of heading change (straight free, 90° = `2·turn_cost`,
  180° = `4·turn_cost`).  The penalty only *raises* costs, so the octile
  heuristic stays admissible.

`turn_cost` is threaded end-to-end as a plain `f32` (a `VehicleDef` field →
`IsoVehicle` → `Engine::vehicle_goto` → `request_vehicle_path` →
`Command::FindVehicle` → `PathfinderState::find_vehicle` →
`find_vehicle_path_with_slope` → `find_path_for_footprint_with_jumps`), and
mirrored through the web path (`worker.js` `findVehicle` message →
`find_vehicle` wasm ABI).  The slope grid cache keys off `(pitch, roll,
wheelbase, track)` only — `turn_cost` does not affect it.  The path stays a
`Vec<GridCell>`; reverse is a *follower* manoeuvre (see `classic-ecs` /
`classic-iso`), not a planner edge.

### Pose goals (`pathfinder::pose`, `vehicle_goto_pose`)

`vehicle_goto_pose(name, x, y, heading_deg)` drives to a point *facing* a
heading.  The search (`find_vehicle_pose_*`, `PoseGoal`) is a state
lattice, on the pathfinder worker like any vehicle search (the query's
`pose`; on the web the `findVehicle` message's `pose` and the wasm
`set_pose_goal` before `find_vehicle`):

- **State:** cell × 16 headings (22.5°) × the gear it arrived in.
- **Primitives** from each heading: straight, or a 22.5° arc of the
  turning radius (the follower's own, `speed / turn_rate`, or the def's
  `turn_radius_m` when wider: `control::pose_turn_radius`) left or right
  then a short straight, forward and (when
  `reverse_speed > 0`) backward; a `pivot_turn` vehicle also turns 22.5° in
  place.  Lengths are picked so the end rounds onto a cell centre.  Each
  primitive's swept body (the body rectangle along the motion, and along
  the chords the follower will drive between its waypoints) is rasterized
  once into row spans; the first move ignores the cells under the body
  where it stands.
- **Cost** (in time, as tiles of forward driving): length, a backward
  tile at `1.5 · speed / reverse_speed` (`POSE_REVERSE_PENALTY`: a mule
  that reverses as fast as it drives backs up where that saves a third of
  the drive; one reversing at 0.9 of 2.4 pays 4× and turns round
  instead), `turn_cost` per 45°, and per gear change 1.5 tiles
  (`POSE_GEAR_COST`, the easing in and out) plus the def's
  `gear_change_s` at speed (`PoseGoal::gear_cost`).  Heuristic: the larger of the straight line and the goal's
  8-connected distance field over the window's open cells (scaled by
  `cos 22.5°`, so it never exceeds the straight line between two cells),
  weighted 1.1 (`HEURISTIC_WEIGHT`: a route within 10% of the cheapest,
  without expanding the band of near-equal states a long off-lattice drive
  leaves).  The field steers round walls, prunes cells that cannot reach
  the goal, and refuses a walled-off goal before any lattice search.
  Searched in a window round start and goal (8 + 3·radius + body reach
  tiles of margin), doubled while the field finds no way across (a detour
  round a long wall), up to the map.  Goal: the goal cell at the nearest
  of the 16 headings.
- **Cost of a search** (400² open map, mule body, warm primitives): 5 ms
  for a 40 × 25 tile manoeuvre, 12 ms for a 140 × 120 drive, 14 ms to
  refuse a goal walled off across the map.  The primitives (≈6 ms to
  sweep) stay in a `PrimCache` on the worker's `PathfinderState` between
  searches; the free functions (`find_vehicle_pose_*`) build them per
  call.
- **Route:** waypoints with the gear packed into `y`
  (`pack_waypoint` / `unpack_waypoint`, `REVERSE_FLAG`); the engine unpacks
  them into `IsoVehicle.path` + `path_reverse` and keeps `goal_heading`.
- **Follower:** drives one gear run at a time (pursuit never looks across a
  cusp), stops at each cusp and changes gear; backing up, the tail leads
  (the heading turns so heading + π bends toward the target, the wheels
  steering as in reverse); on the last run it pursues a virtual waypoint 3
  tiles past the goal along its heading, so it arrives lined up (tests hold
  it within half a 16-direction step).  It pursues one planned radius
  ahead (`POSE_LOOKAHEAD`; the plain follower 1.5), so it holds arcs drawn
  at its own radius, eases over the last 0.75 tile into a cusp or the
  goal, to half speed at most (`POSE_EASE_*`), and stands the def's
  `gear_change_s` at each cusp (`IsoVehicle::gear_wait`; `0`: none).
  A `pivot_turn` vehicle then turns in place onto the exact heading (the
  mule, front-steered, arcs and backs up instead: it has no pivot).
- **Blocks:** a vehicle block cutting a pose route re-plans it as a pose
  route (`route_clear_pose` checks the chords in gear).

### Follower body guard (`vehicle/guard.rs`)

The planners keep the oriented body clear along a route's lines; the
pure-pursuit followers (plain and pose) aim a look-ahead point down the
route and so cut inside its corners, a few cm onto cells it went round.
While following a route (not `vehicle_face` / `vehicle_reverse_to`) each
step is guarded against the vehicle grid (the shared snapshot plus the
blocks other vehicles own), rasterized as the planner does
(`pathfinder::body_blocked_cells`: cells the body's interior overlaps):

- **Look-ahead:** when a blocked cell lies within reach (a square scan of
  body reach + look-ahead + 1 tiles), the follower plays its pursuit
  forward (0.1 tile steps, its turn rate per tile, look-ahead + 1 tiles)
  and takes the longest of its look-ahead, ⅔, ⅖ of it or 0.5 tile whose
  played path stays off blocked cells (`GuardGrid::safe_lookahead`).
- **Step:** the step's pose may not add a blocked cell to those the body
  is on (a relaxed start keeps its own).  Else the first clear of: the
  step aimed at the next waypoint, the step straight on, then each at ½,
  ¼ and 1/10 of the step (slower turns tighter); none clear, the one
  adding the fewest cells.  It never holds a vehicle still: the grid does
  not change under it, and a block change re-plans cut routes anyway.
- The plain planner turns its body at a cell; a corner tighter than the
  vehicle's speed / turn rate can still be grazed (a 3.8-tile-wide test
  body round a 2-tile pillar: 3 frames); the mule-like test kit and the
  padtest runs stay clear.
- `Engine::vehicle_follow_guard` (default on) switches it off for A/B.
  Cost: padtest, three mules, `vehicles` phase mean 0.073 → 0.093 ms.

### Vehicle blocking grid (`vehicle::block`, `set_vehicle_block`)

The vehicle search never reads the nav grid: `NavMesh.data` is *human*
walkability (slope-limited), and a vehicle derives its own slope limits.
Its `structural` grid (`build_vehicle_nav_snapshot`) is the `blocks_nav`
colliders' footprints (`compute_nav_obstacles`, unchanged) with every
**vehicle block** cell zeroed.  A vehicle block is a named set of inclusive
tile rects (`Engine::vehicle_blocks: BTreeMap<id, Vec<[x0, y0, x1, y1]>>`):

- **Why ids, in bulk:** one call per structure (a pad's ~90 rail rects),
  and clearing one structure never reopens cells another still covers (a
  per-cell flag would).  Set replaces, an empty list or
  `clear_vehicle_block` removes.
- **Invalidation:** a change runs `refresh_nav_snapshot`, which re-shares
  the vehicle snapshot with the pathfinder worker (native thread and the web
  `pathfinder.wasm`: the stamped `structural` rides the existing
  `vehicleSnapshot` message) and resets its slope cache.  The oriented
  search keeps no state between requests.
- **Re-plan:** then every vehicle whose remaining route (from the waypoint
  it left) fails `pathfinder::route_clear` (the search's own step rule) is
  stopped and re-submitted to its goal; `update_vehicles` polls those
  (`poll_vehicle_replans`), installing the new route or leaving it stopped.
  Collider `blocks_nav` changes still only refresh the snapshots.
- Human / `IsoAgent` pathing (`NavSnapshot`) never sees vehicle blocks.
- **Owners** (`set_vehicle_block_owner(id, owner)`,
  `Engine::vehicle_block_owners`): an owned block is open to its owner
  only.  The shared snapshot (`BlockView::Shared`, the background worker's
  current one) leaves owned blocks out; every vehicle query carries
  `closed`, the rects of the blocks *other* vehicles own
  (`Engine::closed_vehicle_blocks`), which the worker stamps on the
  walkable grid it builds per search anyway (`pathfinder::walkable_grid`;
  web: the `findVehicle` message's `closed`, the wasm `set_closed`).  So no
  request builds, copies or re-sends a grid on the main thread, and the
  worker's derived slope grid survives (`PathfinderState::set_vehicle`
  keeps it while the heights are unchanged).  Only the synchronous
  (deterministic) path builds a fresh snapshot per request, with
  `BlockView::Vehicle(name)`.  An ownership change re-shares the snapshot
  and re-plans with each vehicle's own view, so the old owner stops (and
  re-plans) if its route crosses the block.  Why a
  separate call rather than an owner argument on `set_vehicle_block`:
  ownership changes on its own (the lane goes to whichever vehicle is
  fetching) without re-sending the rects, and `set_vehicle_block` keeps
  its signature.

### Integration

Pathfinding is exposed to ROM guests as the async `request_path`/`poll_path`
SDK imports (see the `classic-guest` skill).  Click-to-move is implemented in
`classic-roms/guest/demo-guest`: on a left click with the agent tool selected (and not
consumed by UI), it submits a request and polls each frame until the waypoints
land, then follows them via `set_pos`/`get_pos`.  The engine no longer wires
click-to-move — it only holds the nav grid (installed by `load_state` inline
data or the guest's `set_nav` + `commit_terrain`).

The retired engine click-to-move pre-rejected impassable destinations (so a
click on a wall never ran A*).  The guest does not: `find_path` exhausts the
grid and returns an empty path, which the guest treats as no-op.  Functionally
equivalent (clicking a wall does nothing) at the cost of a full search
(~3 ms at 200x200, ~21 ms at 400x400) — now paid on the worker, not the render
thread.

---

## 10. Nav Mesh Walkability

The nav mesh is a flat `Vec<u32>` where `1` = walkable, `0` = blocked.  It is
stored on the `NavMesh` component and also used as the GPU data texture for
the nav mesh overlay.

### Height-based Passability

`sync_nav_heights()` runs after every height editing operation.  For each nav
cell, it checks the corresponding height value in the parent tilemap's
`height_data`.  A cell is marked blocked if any adjacent cell has a height
difference above `Engine::nav_slope_threshold` (a configurable field,
defaulting to `2.0` for the flat demo map; generated terrain needs a finer
value).  This check is done in 4 directions (left, right, up, down).

Walkability is recomputed in two contexts:

1. **`commit_terrain()`**: the ROM guest installs the authoritative nav grid
   (its `set_nav` upload, or the inline `NavMesh.data` for a hand-authored map).
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
  (1000) reached` if the simplex evolution fails to converge.  It has never
  fired in practice, but the fallback behaviour (return `false` or use a
  separating-axis fallback) is not implemented.

- **Single-frame enter/exit**: because enter/exit dispatch compares
  frame(N) vs frame(N+1) `colliding`/`collided` tables, an enter + exit
  that both occur within a single frame (collider passing through another at
  very high velocity) will be missed entirely.

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
  timeout.  On very large grids with complex obstacle mazes it will explore the
  entire reachable space before returning `None` — measured 3 ms (exhaustive) at
  200x200 vs 21 ms at 400x400.  The search now runs on the host `PathfinderWorker`
  (off the render thread), so the frame cost is hidden; the impassable-destination
  pre-check remains removed, but beyond roughly 600x600 an iteration budget is
  still worthwhile.
