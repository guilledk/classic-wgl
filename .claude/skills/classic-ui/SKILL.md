---
name: classic-ui
description: >
    Retained-mode UI/layout system for classic-wgl's Rust port.
    Covers UIManager, factory methods, anchor system, layout pipeline,
    collider sync, spawn_button, set_button_base_color, rendering
    (DrawKind::UiRect/UiSprite), and the UI entity lifecycle.
    Trigger phrases: "UIManager", "anchor", "UiAnchor", "UiNode",
    "spawn_button", "spawn_container", "spawn_sdf_text", "spawn_array",
    "spawn_sprite", "refresh_layout", "mark_dirty", "sync_colliders",
    "add_collider_to_elem", "position_children_of", "set_button_base_color",
    "HUD", "container_add_child".
---

# Classic UI — Retained-Mode Layout System

## 1. Architecture

The UI system is a retained-mode layout manager built on top of the ECS. Every
UI element is a regular ECS entity carrying a `Transform` (position + scale), a
render component (`RectRender` or `SpriteRender`), and a `UiNode` layout
component. The `UIManager` owns the root container, tracks named elements, and
orchestrates the layout pipeline.

`UIManager` fields:
- `root` — the top-level container entity, sized to the viewport
- `elements` — `HashMap<String, Entity>` mapping generated names to entities
- `dirty` — boolean flag indicating layout needs refresh
- `viewport_w` / `viewport_h` — current viewport dimensions
- `index_counter` — monotonic counter for generating unique element names
- `zlayer` — default z-ordering for new elements (starts at -1000)
- `element_colliders` — list of `UiColliderEntry` for hover/click integration

The `UiColliderEntry` (private struct) tracks each registered UI element:
```rust
struct UiColliderEntry {
    elem: hecs::Entity,
    collider_pid: u32,       // PhysicsProvider handle
    base_color: [f32; 4],    // original RectRender color
    click_frames: u32,       // click-feedback countdown
}
```

The `Engine` calls `ui.refresh_layout()` and `ui.sync_colliders()` inside its
`frame()` method, after the render list is built but before draw calls. The
layout pipeline is not a continuous system — it runs only when `dirty` is true
and is explicitly triggered by factory methods and `resize()`.

## 2. UiNode Component

Every UI entity carries a `UiNode` with these fields:

| Field | Type | Description |
|---|---|---|
| `parent` | `Option<Entity>` | Parent container (None for root) |
| `children` | `Vec<UiChild>` | Child references with anchor pairs |
| `size` | `Vec2` | Logical layout size (may differ from Transform.scale) |
| `anchor` | `UiAnchor` | Own anchor point (default TopLeft) |
| `fixed` | `bool` | Unused / reserved |
| `clip_children` | `bool` | When true, clip_rect is propagated to children |
| `scroll_y` | `f32` | Vertical scroll offset (applied in position_children_of) |
| `clip_rect` | `Vec4` | `(x, y, w, h)` scissor rectangle (set when parent clips) |
| `kind` | `UiKind` | Discriminant controlling layout behaviour |

`UiChild` links a child entity with anchor pair:
```rust
struct UiChild {
    entity: hecs::Entity,
    self_anchor: UiAnchor,   // anchor ON the parent
    child_anchor: UiAnchor,  // anchor ON the child
}
```

## 3. Anchor System

Nine anchor variants determine how child position is computed relative to a parent:

```rust
pub enum UiAnchor {
    TopLeft, TopCenter, TopRight,
    MidLeft, MidCenter, MidRight,
    BotLeft, BotCenter, BotRight,
}
```

The `offset(w, h)` method returns a `Vec2` from the top-left of a box to the
anchor point. Y grows downward (matches the ortho projection, Y=0 at top):

| Anchor | offset(w, h) |
|---|---|
| TopLeft | (0, 0) |
| TopCenter | (w/2, 0) |
| TopRight | (w, 0) |
| MidLeft | (0, h/2) |
| MidCenter | (w/2, h/2) |
| MidRight | (w, h/2) |
| BotLeft | (0, h) |
| BotCenter | (w/2, h) |
| BotRight | (w, h) |

**Child positioning formula** (in `set_child_position`):
```
child.position.x = parent.position.x + parent_anchor_offset.x - child_anchor_offset.x
child.position.y = parent.position.y + parent_anchor_offset.y - child_anchor_offset.y
```

The anchors describe points on the parent and child boxes. A `MidCenter` →
`MidCenter` pairing centers the child within the parent. A `MidLeft` → `MidLeft`
pairing left-aligns the child vertically centered on the parent's left edge.

When `position_children_of` runs (post-manual-position), it also applies the
parent's `scroll_y` offset to the Y position and propagates `clip_rect` when
`clip_children` is true.

## 4. Factory Methods

All factory methods use `self.zlayer` as the Z component and prefix entity names
with `ui-{counter}-{kind}`. They call `mark_dirty()` after creation.

### spawn_container(world, w, h, color) → Entity

Creates a `RectRender` + `UiNode::Container` entity. The `Transform.scale` is
INITIALIZED to `Vec3::ONE` (not to `(w, h)`) — the rendering arm overrides the
model matrix using `UiNode.size` instead. Default anchor is `MidCenter`.

### spawn_sdf_text(world, text, scale, max_width, color, justify) → Entity

Creates an `SdfTextRender` + `UiNode::SdfText` entity. The `UiNode.size` starts
as `(max_width, 0)`. `measure_all_ui_labels()` must be called afterwards to
compute the true text dimensions and update `size`. Text scale is applied via
the SDF tform scale.

### spawn_array(world, vertical, align, spacing, color) → Entity

Flex-like container. `UiKind::Array { vertical, align, spacing }`. `UiKind`
variant `UiAlign` is `Left`/`Center`/`Right` for cross-axis alignment.
Initial size is `(10, 10)` — layout pass resizes to fit children.

### spawn_padding(world, top, right, bottom, left, color) → Entity

Single-child wrapper. `UiKind::Padding { top, right, bottom, left }`. Padding
adds space around the child; container is resized to (child + padding).

### spawn_sprite(world, texture, w, h, frame, tile_set_size) → Entity

`SpriteRender` + `UiNode::Sprite`. The sprite anchor is `Vec2::ZERO` (top-left).
Uses `DrawKind::UiSprite` and always ignores the camera.

### spawn_button(world, physics, w, h, color, opts) → Entity

Composite factory: creates a container, optionally adds a child (sprite via
`spawn_sprite` or SDF text via `spawn_sdf_text`), registers a collider via
`add_collider_to_elem`, sets `consumes_click = true`, and optionally wires up
a `HandlerKind::Click` callback. The collider PID is not exposed — full
management is internal.

## 5. ButtonOptions

```rust
pub struct ButtonOptions {
    pub text: Option<String>,
    pub text_scale: f32,           // default 0.5
    pub text_color: [f32; 4],
    pub sdf_text: bool,            // default false
    pub sprite: Option<String>,
    pub sprite_frame: f32,         // default 0.0
    pub sprite_tile_set: [f32; 2], // default [1.0, 1.0]
    pub click_priority: i32,       // default 0
    pub hover: bool,               // NOT WIRED (see §12)
    pub click_feedback: Option<u32>, // PARTIALLY WIRED (see §12)
    pub click_action: Option<Box<dyn FnMut() -> bool>>,
}
```

When `sdf_text` is true, the text scale is multiplied by 2.5 internally and the
child is created as `SdfTextRender` instead of the legacy `Text` variant. The
child is anchored `MidCenter → MidCenter` within the container.

## 6. Layout Pipeline

### mark_dirty()
Sets `dirty = true`.

### refresh_layout(world)
Calls `measure_and_position(root, world)` if dirty, then sets `dirty = false`.
Only the root tree is refreshed — standalone containers must use
`layout_standalone()`.

### layout_standalone(entity, world)
Runs `measure_and_position()` on an entity NOT in the root tree. Used after
manually setting a container's position.

### measure_and_position(entity, world)
Dispatches by `UiKind`:
- `Array` → `layout_array()`: measures all enabled children, resizes self to fit stacked children (spacing included), positions each child along the main axis with cross-axis alignment. Skips `Disabled` children entirely.
- `Padding` → `layout_padding()`: measures the first non-disabled child, resizes self to (child + padding), positions the child at (left, top) offset.
- Others (Container, SdfText, Sprite) → recursively measures children, then calls `set_child_position()` for each.

### layout_array internals
- Measures all children first
- Filters out disabled children
- Computes `total_main` = sum of main-axis sizes + spacing between
- Computes `max_cross` = max cross-axis size
- Resizes self to `(max_cross, total_main)` or `(total_main, max_cross)`
- Updates `Transform.scale` to match computed size
- Positions each child along main axis with cross-axis alignment

### layout_padding internals
- Measures all children, finds first non-disabled
- Computes self size = child + padding offsets
- Positions child at (parent.x + left, parent.y + top)

### set_child_position
Static method. Computes parent position + parent anchor offset - child anchor
offset, then writes the child's `Transform.position`.

### position_children_of (static)
After a container's position has been manually set, repositions all children
according to their stored anchor pairs. Also applies the parent's `scroll_y`
offset (subtracted from Y) and propagates `clip_rect` when `clip_children` is
true. Called by `Engine::frame()` for the root container after resize.

## 7. Collider Integration

### add_collider_to_elem(world, elem, physics) → u32

Reads the element's `UiNode.size` and `Transform` (x, y), builds a 4-vertex
polygon at `(0,0) → (size.x,0) → (size.x,size.y) → (0,size.y)`, registers it
with `PhysicsProvider`, and stores the PID + base color in `element_colliders`.

The collider is registered at world position `(pos.x, pos.y, 0)` with scale
ONE. The polygon is in local coordinates; `PhysicsProvider` applies the
collider's position and scale during spatial queries.

### sync_colliders(world, physics)

Iterates all `element_colliders` and calls `physics.sync_collider_rect(pid, x,
y, w, h)` using the current `Transform.position` and `UiNode.size`. This is
called at the start of each frame, before `physics.perform_calls()`, ensuring
colliders match the layout.

### position_children_of and clip_rect

When a parent has `clip_children = true`, `position_children_of` computes a
`clip_rect = (parent.position.x, parent.position.y, parent.size.x,
parent.size.y)` and writes it into each child's `UiNode.clip_rect`. This value
is later read by the SDF text render arm to set `SCISSOR_TEST`. However, the
GL scissor rectangle uses Y-flipped coordinates (viewport height - clip_rect.y
- clip_rect.h).

### collect_collider_pids(world, entity) → Vec<u32>

Recursively walks children to collect all collider PIDs for an entity
subtree. Used by `set_enabled` to enable/disable colliders.

## 8. UI Rendering

UI elements are drawn in two `DrawKind` variants, sorted by `Transform.position.z`
in the render list:

### DrawKind::UiRect
For entities with `RectRender`. The model matrix uses `UiNode.size` (not
`Transform.scale`):
```
model = translate(position) * scale(size.x, size.y, 1.0)
```
Always rendered with `ignore_cam` treatment — if `RectRender.ignore_cam` is
true (which it always is for UIManager-spawned rects), the camera matrix is
identity.

### DrawKind::UiSprite
For entities with `SpriteRender` that also carry a `UiNode` with
`UiKind::Sprite`. Uses `UiNode.size` for the model matrix scale, always
renders with `ignore_cam = true`.

Common traits:
- Both read `UiNode.size` rather than `Transform.scale` for rendering dimensions
- Both are sorted by `Transform.position.z` for draw order
- Depth test is OFF during UI rendering (the frame draw loop never enables it for these arms)

## 9. Z-Layering

`UIManager.zlayer` defaults to `-1000`. All factory-created elements get this
Z value. The engine can override it at spawn time for overlay elements:

- Root container and HUD children: z = -1000
- Overlay / tool panel containers: typically z = -1050 or -1100 (set manually
  after spawn)

Z-sort is purely draw-order; there is no depth-testing in the UI phase. Higher
Z renders on top. The zlayer field is NOT automatically decremented — callers
must manage Z when layering panels.

## 10. Button Hover and Click

### update_hover(world, physics)

Called per frame after `physics.begin_frame() + perform_calls()`. For each
entry:

1. If `click_frames > 0`: decrements, sets color to white (keeping base alpha),
   skips hover test. This is the click-feedback flash.
2. Otherwise: runs `physics.gjk_test(entry.collider_pid, mouse_pid)` where
   `mouse_pid = 0` (the hardware cursor's virtual collider). On hover, the
   `RectRender.color` is lerped toward white by 25%. On exit, restored to
   `base_color`.

### click_action

Set via `ButtonOptions.click_action`. Internally, this creates a
`HandlerKind::Click` closure on the collider. The engine's `perform_calls()`
dispatches clicks through the quadtree, sorted by `click_priority` (highest
first). Buttons set `consumes_click = true` so a click on a button stops
propagation.

### set_button_base_color(elem, color)

Updates `base_color` in the `element_colliders` entry so hover lerp is
recalibrated. Essential after programmatic color changes (e.g. editor mode
toggles).

## 11. set_enabled and Visibility

The `Disabled` component marker controls visibility. Entities marked `Disabled`
are:
- Skipped during render list building (not added to items)
- Skipped during layout (array/padding layout ignores Disabled children)

Colliders on disabled elements are NOT automatically disabled by the UI
manager. The engine's `set_enabled()` function calls
`ui.collect_collider_pids()` to gather PIDs, then sets
`physics.set_collider_enabled(pid, enabled)` for each. Parent-chain checks are
handled at the engine level via `is_disabled()` which walks `parent` links.

## 12. Known-divergent / non-functional

- **`ButtonOptions.hover`** — This boolean is stored but never read. Hover
  highlighting is always active (via `update_hover` color lerp) regardless of
  this flag.

- **`ButtonOptions.click_feedback`** — The `click_frames` countdown mechanism
  exists and sets color to white during feedback frames, but there is no
  external trigger that sets `click_frames > 0`. The click handler closure
  doesn't capture the `UiColliderEntry` reference. So this path is dead code.

- **`UiKind::Text`** — The variant exists in the enum and in `kind_str()`, but
  there is no factory method that spawns a `Text` element, no `TextRender`
  component, and no render arm for it. It is a legacy stub.

- **`UiNode.fixed`** — The field exists and is deserialized from JSON, but is
  never read by any layout or rendering code.

- **`clip_children` / scissor** — `clip_rect` is propagated to child `UiNode`
  entries, and the SDF text render arm does set `SCISSOR_TEST` using it. But
  `UiRect` and `UiSprite` render arms do NOT apply scissoring. Only SDF text
  children of clipping containers are visually clipped.

- **`NativePlatform::window()` and `gl_context()`** — These `Platform` trait
  methods return `unimplemented!()`. The platform is only used through
  `run_loop` which captures everything before these accessors can be called.
