---
name: classic-rust-text
description: >
    SDF text rendering system for classic-wgl's Rust port.
    Covers `build_sdf_glyph_buffer()` in `classic-core/src/sdf_builder.rs`,
    font loading via `load_sdf_font()`, the SDF text render loop with
    `SdfTextGpu` cache, `text_height` centering formula, justify behavior,
    and SDF text on button children (container vs child entity pattern).
    Use when debugging text not appearing, wrong positioning, text clipping,
    glyph buffer sizes, justify behavior, font atlas loading failures,
    or button label updates not working.
    Trigger phrases: "SDF text", "sdf_builder", "glyph buffer", "text_height",
    "build_sdf_glyph_buffer", "dejavusans", "font atlas", "SdfTextRender",
    "SdfTextGpu", "glyph extent", "text not showing", "button text",
    "UiNode.children", "justify", "text on button".
compatibility: hecs 0.10, glam 0.29
metadata:
    author: classic-wgl
    version: '0.2'
allowed-tools: Read, Grep, Glob, Bash(git *), Edit
---

# Skill: classic-rust-text

## Scope

Covers SDF font rendering in the Rust port: glyph buffer building
(`classic-core/src/sdf_builder.rs`), font loading, the render loop in
`classic-engine/src/lib.rs`, and the `SdfTextGpu` cache.

---

## 1. SDF Text Render Loop

In `frame()`, SDF text renders inline as an arm (`DrawKind::SdfText`) inside
the single z-sorted render loop. The render-list builder wraps each
`(&Transform, &SdfTextRender)` entity, and the draw loop dispatches
`gfx.draw_sdf()` directly — there is no separate post-pass.

```rust
for (e, (tf, sdf)) in self.world.query::<(&Transform, &SdfTextRender)>().iter() {
    if self.is_disabled(e) { continue; }
    // Dirty check: rebuild glyph buffer only when text or scale changes
    let dirty = self.sdf_text_gpu.get(e)
        .map(|st| st.last_text != sdf.text || (st.last_scale - tf.scale.x).abs() > 0.001)
        .unwrap_or(true);
    if dirty {
        let buf = build_sdf_glyph_buffer(font, &sdf.text, scale, sdf.justify, 0.0);
        // ... create new GlBuffer, store in sdf_text_gpu ...
        // Sync UiNode.size from glyph buffer (for UI elements):
        if let Ok(mut node) = self.world.get::<&mut UiNode>(e) {
            node.size.x = buf.text_width;
            node.size.y = buf.text_height;
        }
    }
    // Determine justify offset
    let is_ui = self.world.get::<&UiNode>(e)
        .map(|n| n.parent.is_some()).unwrap_or(false);
    let x_off = if is_ui {
        0.0  // anchor system handles positioning for UI children
    } else {
        match sdf.justify {
            TextJustify::Left => 0.0,
            TextJustify::Center => -st.text_width / 2.0,
            TextJustify::Right => -st.text_width,
        }
    };
    // Draw
    gfx.draw_sdf(&model, &cam, &atlas_name, &sdf.color, ...);
}
```

**Key**: `UiNode.size` is synced from the glyph buffer for UI elements.
`text_height = glyphExtentMin + glyphExtentMax` places the visual
center of glyphs at the geometric center `ch/2`.

**Justify offset**: 0 for UI-managed text (anchor system handles
positioning); active for standalone SDF text.

**WARNING**: Removing the `is_ui` x_off=0 override causes double-justification
for anchor-positioned text (button labels, HUD). The anchor system already
centers the element, and the renderer's justify offset would double-shift.

---

## 2. SdfTextGpu Cache

```rust
struct SdfTextGpu {
    glyph_buf: GlBuffer,
    vertex_count: usize,
    text_width: f32,
    text_height: f32,
    last_text: String,
    last_scale: f32,
}
```

Stored in `HashMap<hecs::Entity, SdfTextGpu>`. Rebuilt only when
`last_text != sdf.text` or `last_scale != tf.scale.x`.

---

## 3. Font Loading

```rust
pub fn load_sdf_font(&mut self, atlas_name: &str, metrics_json: &str, atlas_png: &[u8]) {
    let metrics: SdfFontMetrics = serde_json::from_str(metrics_json).expect("...");
    self.sdf_fonts.insert(metrics.name.clone(), metrics);
    let img = image::load_from_memory(atlas_png).expect("...");
    gfx.add_texture_rgba8(atlas_name, &rgba, w, h);
    // Must set LINEAR filtering on the atlas texture
    tex.set_linear(&gfx.gl);
}
```

---

## 4. build_sdf_glyph_buffer

```rust
pub fn build_sdf_glyph_buffer(
    font: &SdfFontMetrics,
    text: &str,
    scale: f32,
    justify: TextJustify,
    layout_width: f32,
) -> SdfGlyphBuffer { ... }
```

Returns interleaved vertex data (16 bytes per vertex: position[2], uv[2])
and metadata (text_width, text_height, vertex_count). Uses 6 vertices per
glyph (two triangles forming a quad).

---

## 5. SDF Text on Button Children

When a `spawn_button` creates a text label, the SDF text entity is a
CHILD of the container entity. To update the label dynamically:

```rust
// CORRECT — find child entity, update its SdfTextRender:
if let Ok(node) = engine.world.get::<&UiNode>(button_container) {
    if let Some(child) = node.children.first() {
        if let Ok(mut sdf) = engine.world.get::<&mut SdfTextRender>(child.entity) {
            sdf.text = "new label".into();
        }
    }
}

// WRONG — container has RectRender, not SdfTextRender:
if let Ok(mut sdf) = engine.world.get::<&mut SdfTextRender>(button_container) {
    sdf.text = "...";  // never executes — no SdfTextRender on container
}
```

The `UiNode.children` list contains `UiChild` entries with `entity` and
anchor info. `container_add_child` stores these in order.

---

## 6. TextJustify

```rust
pub enum TextJustify { Left, Center, Right }
```

Passed to `build_sdf_glyph_buffer`. For UI-managed text, justify x-offset
is skipped (= 0) because the anchor system positions the element box.
For standalone text, justify computes the offset to align text within
the specified `layout_width`.

---

## 7. Pre-Measurement — `measure_all_ui_labels()`

Called once after all UI init, before the first frame renders:

```rust
pub fn measure_all_ui_labels(&mut self) {
    let font = self.sdf_fonts.get("dejavusans").cloned();
    // For each SdfTextRender with UiNode.parent.is_some():
    //   1. Build glyph buffer (CPU-only, no GPU buffer)
    //   2. Set UiNode.size from text_width / text_height
    // Then refresh_layout() to reposition children with correct sizes.
}
```

**Why needed**: `spawn_sdf_text` creates entities with `UiNode.size = (max_width, 0)`.
`position_children_of` uses this stale size for anchor math, causing 1-frame
mispositioning of SDF text children inside button containers. Pre-measuring
ensures correct sizes from frame 0.

Call site: in `apps/desktop/src/main.rs` and `apps/web/src/lib.rs`, after
`e.init_editor_mode_control()`.

---

## 8. Scissor Clipping for SDF Text

For containers with `clip_children = true` (see classic-rust-ui §11),
child entities get their `UiNode.clip_rect` populated by `position_children_of`.
The SDF render loop enables `SCISSOR_TEST` per-entity:

```rust
let clip = world.get::<&UiNode>(*e).ok()
    .map(|n| n.clip_rect)
    .filter(|r| *r != Vec4::ZERO);
if let Some(r) = clip {
    gl.enable(SCISSOR_TEST);
    gl.scissor(r.x as i32, (vh - r.y - r.w) as i32, r.z as i32, r.w as i32);
    // Y is flipped: GL scissor origin is bottom-left, UI is top-left
}
gfx.draw_sdf(...);
if clip.is_some() {
    gl.disable(SCISSOR_TEST);
}
```

`begin_frame` disables `SCISSOR_TEST` globally; each clipped entity
enables/disables it locally.  Non-clipped entities are unaffected.

---

## 9. Multi-Line Spacing

When iterating text entities with `\n` line breaks, compute vertical
spacing proportional to line count:

```rust
let line_count = text.matches('\n').count() as f32 + 1.0;
cy += line_count * font_scale.max(0.5) * line_h + line_gap * line_count;
```

Without this, a 4-line text block gets the same vertical space as a
single line, causing overlap with subsequent labels.  Both the line height
AND the gap scale with `line_count`.

---

## 10. Post-Spawn Effect Properties

Mutate `SdfTextRender` directly after `spawn_sdf_text` to set visual
effects that aren't exposed through the factory method:

```rust
let e = ui.spawn_sdf_text(&mut self.world, text, scale, max_width, color, justify);
if let Ok(mut sdf) = self.world.get::<&mut SdfTextRender>(e) {
    // Match on text content to apply per-label effects:
    match text.as_str() {
        "Weight 0.0 — thinner" => sdf.weight = 0.0,
        "Weight 0.3 — bolder" => sdf.weight = 0.3,
        "Gamma 0.5 — sharper" => sdf.gamma = 0.5,
        "Thick outline" => {
            sdf.outline_width = 0.2;
            sdf.outline_color = [0.1, 0.08, 0.0, 1.0];
        }
        "Drop shadow" => {
            sdf.shadow_offset = [3.0, 3.0];
            sdf.shadow_color = [0.0, 0.0, 0.0, 0.6];
            sdf.shadow_blur = 0.05;
        }
        _ => {}
    }
}
```

Available properties on `SdfTextRender`: `weight` (f32), `gamma` (f32),
`outline_width` (f32), `outline_color` ([f32; 4]), `shadow_offset` ([f32; 2]),
`shadow_color` ([f32; 4]), `shadow_blur` (f32).

**Note**: `shadow_offset`, `shadow_color`, and `shadow_blur` are stored fields
but are **not rendered** — the draw_sdf shader does not consume or apply them
(they exist in the struct for future use).
