---
name: classic-rust-ui
description: >
    Retained-mode UI/layout system for classic-wgl's Rust port.
    Covers `UIManager` in `classic-engine/src/ui.rs` (~750 LOC),
    all factory methods (spawn_container, spawn_sdf_text, spawn_array,
    spawn_padding, spawn_sprite, spawn_button), the anchor system,
    collider integration (addColliderToElem, update_hover, sync_colliders,
    position_children_of), button state (set_button_base_color,
    collect_collider_pids), and the DrawKind::UiRect/UiSprite render arms.
    Use when debugging element positioning, anchor math, hover/click
    interaction, layout trees, button state, or collider sync.
    Trigger phrases: "UIManager", "anchor", "UiAnchor", "UiNode",
    "spawn_button", "ButtonOptions", "click_action", "position_children_of",
    "add_collider_to_elem", "sync_colliders", "update_hover",
    "set_button_base_color", "collect_collider_pids",
    "spawn_array", "spawn_padding", "spawn_sprite",
    "UiRect", "UiSprite", "HUD", "init_ui", "set_child_position",
    "refresh_layout", "mark_dirty", "container_add_child".
compatibility: hecs 0.10, glam 0.29
metadata:
    author: classic-wgl
    version: '0.2'
allowed-tools: Read, Grep, Glob, Bash(git *), Edit
---

# Skill: classic-rust-ui

## Scope

Covers `crates/classic-engine/src/ui.rs` (UIManager, ~750 LOC),
the `DrawKind::UiRect` / `DrawKind::UiSprite` render arms in
`classic-engine/src/lib.rs`, collider integration, button factories,
and the `init_ui()` HUD migration method.  UI elements are regular
ECS entities with `Transform` + a render component (`RectRender` /
`SdfTextRender` / `SpriteRender`) + a `UiNode` layout component.

---

## 1. UIManager — Creation + Fields

```rust
pub struct UIManager {
    pub root: hecs::Entity,
    pub dirty: bool,
    pub viewport_w: f32,
    pub viewport_h: f32,
    elements: HashMap<String, hecs::Entity>,
    index_counter: u32,
    zlayer: i32,
    element_colliders: Vec<UiColliderEntry>,  // for hover + click sync
}
```

`UIManager::new(vp_w, vp_h, &mut world)` creates root container
sized to viewport, zlayer -1000, transparent `[0,0,0,0]`.

---

## 2. Factory Methods

### spawn_container(w, h, color) → Entity
RectRender(ignore_cam:true) + UiNode(Container).

### spawn_sdf_text(text, scale, max_width, color, justify) → Entity
SdfTextRender + UiNode(SdfText). Initial `UiNode.size = (max_width, 0)` —
synced from glyph buffer in render pass.

### spawn_array(vertical, align, spacing, color) → Entity
RectRender + UiNode(Array {vertical, align, spacing}). Layout handled by
`layout_array()` in `measure_and_position` (see §3).

### spawn_padding(top, right, bottom, left, color) → Entity
RectRender + UiNode(Padding {...}). Layout handled by `layout_padding()`
in `measure_and_position` (see §3). Single child only.

### spawn_sprite(texture, width, height, frame, tile_set_size) → Entity
SpriteRender(ignore_cam:true) + UiNode(Sprite). Rendered via
`DrawKind::UiSprite` using `gfx.draw_sprite()` with `ignore_cam: true`.

### spawn_button(width, height, color, ButtonOptions) → Entity

Returns the container entity. `ButtonOptions` struct:

```rust
pub struct ButtonOptions {
    pub text: Option<String>,
    pub text_scale: f32,
    pub text_color: [f32; 4],
    pub sdf_text: bool,
    pub sprite: Option<String>,
    pub sprite_frame: f32,
    pub sprite_tile_set: [f32; 2],
    pub click_priority: i32,
    pub hover: bool,
    pub click_feedback: Option<u32>,
    pub click_action: Option<Box<dyn FnMut() -> bool>>,
}
```

Creates: container + optional text/sprite child (MidCenter anchor) +
collider with `consumes_click = true`.

**Important**: The text child is a SEPARATE entity. To update a button's
label dynamically (e.g., blend/set toggle), read `UiNode.children[0].entity`
and update THAT entity's `SdfTextRender.text`. The container entity only
has `RectRender`.

---

## 3. Layout System

### Anchor math

```
child.x = parent.x + parent_anchor.offset(pw, ph).x - child_anchor.offset(cw, ch).x
child.y = parent.y + parent_anchor.offset(pw, ph).y - child_anchor.offset(cw, ch).y
```

### measure_and_position — dispatches by UiKind

- `UiKind::Container | SdfText | Text | Sprite` → anchor-based recursion + `set_child_position`
- `UiKind::Array {..}` → `layout_array()` — flex stacking with align
- `UiKind::Padding {..}` → `layout_padding()` — child + padding

### position_children_of(container, world)

Public static method. After manually setting a container's Transform.position,
call this to reposition all its children according to their anchors.
ALWAYS call this after manual position updates in `on_update` closures.

```rust
UIManager::position_children_of(container, &mut engine.world);
```

### Mark dirty

Call `ui.mark_dirty()` in `on_update` closures when UI state changes.
Triggers `refresh_layout()` on next frame.

---

## 4. Collider Integration

### add_collider_to_elem(world, elem, physics) → u32 (pid)

Creates a `Shape::Polygon` from element's `(0,0)→(w,h)` rect, creates a
`Collider`, registers with PhysicsProvider, stores in `element_colliders`.
Returns the collider PID.

### sync_colliders(world, physics)

Called after `refresh_layout()` each frame. Updates collider position/shape
from current UiNode position/size.

### update_hover(world, physics)

Called after `perform_calls()` each frame. Iterates `element_colliders`,
runs `physics.gjk_test(pid, 0)` (mouse pid=0) to detect hover. On hover,
blends element color towards white (base_color + 25% white). On exit,
restores base_color. Click feedback: flashes white for N frames.

### collect_collider_pids(world, entity) → Vec<u32>

Walks the UiNode tree from entity, collecting all collider PIDs from
`element_colliders`. Used by `set_enabled` to sync collider state.

### set_button_base_color(elem, color)

Updates the `base_color` stored in `element_colliders` for hover blending.
Used for active-tool highlighting on menu item rows.

---

## 5. DrawKind Render Arms

### UiRect

```rust
DrawKind::UiRect => {
    let (w, h) = world.get::<&UiNode>(*entity)
        .map(|n| (n.size.x, n.size.y))
        .unwrap_or((tf.scale.x, tf.scale.y));
    let model = Mat4::from_translation(tf.position) * Mat4::from_scale(Vec3::new(w, h, 1.0));
    gfx.draw_rect(&model, &cam_mat, &rect.color, rect.ignore_cam);
}
```

**Must read `UiNode.size` — NOT `Transform.scale`.** Transform.scale is
`Vec3::ONE` at spawn. Rendering at 1×1 would be invisible.

### UiSprite

```rust
DrawKind::UiSprite => {
    let (w, h) = world.get::<&UiNode>(*entity)
        .map(|n| (n.size.x, n.size.y))
        .unwrap_or((tf.scale.x, tf.scale.y));
    let model = Mat4::from_translation(tf.position) * Mat4::from_scale(Vec3::new(w, h, 1.0));
    gfx.draw_sprite(&model, &IDENTITY, &sprite.texture, sprite.frame, &ts, true, 1.0);
}
```

Uses `draw_sprite` with `ignore_cam: true`, ghost_alpha 1.0.

---

## 6. set_enabled + Collider Sync

```rust
fn set_enabled(&mut self, entity: hecs::Entity, enabled: bool) {
    // 1. Collect collider PIDs BEFORE ECS mutations
    let pids = self.ui.collect_collider_pids(&self.world, entity);

    // 2. Toggle Disabled mark on entity
    // 3. Recurse into UiNode.children

    // 4. Sync collider enabled state
    for pid in &pids {
        self.physics.set_collider_enabled(*pid, enabled);
    }
}
```

**MUST sync collider state.** Without this, hidden UI elements still fire
click handlers because their colliders remain active in the physics quadtree.

### is_disabled(entity) → bool

Walks `UiNode.parent` chain checking for `Disabled` marker. Used in render
queries to skip entities whose ancestors are disabled.

---

## 7. SDF Text on Button Children

When a button has an SDF text label that needs to update dynamically:

```rust
// WRONG — container has RectRender, not SdfTextRender:
if let Ok(mut sdf) = engine.world.get::<&mut SdfTextRender>(button_container) {
    sdf.text = "...";
}

// CORRECT — find the child entity:
if let Ok(node) = engine.world.get::<&UiNode>(button_container) {
    if let Some(child) = node.children.first() {
        if let Ok(mut sdf) = engine.world.get::<&mut SdfTextRender>(child.entity) {
            sdf.text = "...";
        }
    }
}
```

---

## 8. Palette Click Handling

Tile/nav palettes use `on_update` for AABB click detection. They also need
a collider with `consumes_click = true` and a dummy handler to block map
editing when clicking the palette:

```rust
let pid = ui.add_collider_to_elem(&mut self.world, container, &mut self.physics);
self.physics.set_collider_consumes_click(pid, true);
self.physics.add_collider_handler(pid, HandlerKind::Click, || true);
```

The dummy handler (`|| true`) sets `physics.consumed_click` → propagates to
`engine.ui_consumed_click`. Palette `on_update` should NOT check
`ui_consumed_click` — it needs to process its own clicks.

---

## 9. SDF Text with TextJustify

```rust
pub enum TextJustify { Left, Center, Right }
```

In `spawn_sdf_text`, the `justify` field is stored in `SdfTextRender`.
UI-managed text (those with `UiNode.parent.is_some()`) skip justify
x-offset — the anchor system handles positioning.

**Critical**: The `is_ui` x_off=0 override MUST be kept. Removing it causes
double-justification for anchor-positioned text (button labels, HUD): the
anchor system centers the element box, and the renderer's justify offset
would double-shift.  Standalone SDF text (no parent) still gets full
justify offsets.

---

## 10. `layout_standalone` — Non-Root-Tree Entities

Public method on UIManager for arrays/containers NOT in the root tree:

```rust
pub fn layout_standalone(&self, entity: hecs::Entity, world: &mut World) {
    self.measure_and_position(entity, world);
}
```

Call after manually setting a container's `Transform.position`. Dispatches
via `measure_and_position` → UiKind-aware layout:
- `UiKind::Array` → `layout_array()` (flex stacking)
- `UiKind::Padding` → `layout_padding()` (child + padding)
- `UiKind::Container` → anchor-based children via `set_child_position`

Use case: button array (agent + DEV) positioned outside root tree.

---

## 11. `clip_rect` + Scissor Clipping

`UiNode.clip_rect: Vec4` (x, y, w, h; `Vec4::ZERO` = no clip).
When a container has `clip_children = true`, `position_children_of`
computes the container's screen-space rect and writes it into each
child's `UiNode.clip_rect`:

```rust
let clip = if node.clip_children {
    Vec4::new(tf.position.x, tf.position.y, node.size.x, node.size.y)
} else {
    Vec4::ZERO
};
// ... for each child:
if clip != Vec4::ZERO {
    cn.clip_rect = clip;
}
```

The SDF text render loop then checks `clip_rect` and enables `SCISSOR_TEST`
per-entity (see classic-rust-text skill §8).

---

## 12. `scroll_y` Support

`position_children_of` applies `-node.scroll_y` to child Y positions:

```rust
let y = tf.position.y + po.y - co.y - node.scroll_y;
```

This allows scrollable containers outside the root tree.  The container's
`UiNode.scroll_y` is updated by external logic (e.g., mouse wheel routing
in `frame()`), and `position_children_of` offsets all children accordingly.

---

## 13. Standalone Entity Initial Positioning

Entities NOT in the root tree (menu panel, text showcase, widget containers)
spawn at `(0, 0)`.  They must have their `tf.position` set immediately
after creation — NOT just in `on_update`:

```rust
let panel = ui.spawn_container(&mut self.world, w, h, color);
if let Ok(mut tf) = self.world.get::<&mut Transform>(panel) {
    tf.position = glam::Vec3::new(px, py, tf.position.z);
}
self.set_enabled(panel, false); // disable after positioning
```

Without this, the entity's first visible frame renders at `(0, 0)` before
`on_update` repositions it.  `on_update` runs AFTER `refresh_layout` and
BEFORE the render list, so positions set there are correct for rendering —
but the entity may still flash at origin on the first frame of visibility
if its initial position was never set.
