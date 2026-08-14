---
name: classic-text
description: >
    SDF text rendering system for classic-wgl's Rust port.
    Covers SdfTextRender component, glyph buffer builder, font atlas
    loading, draw_sdf, is_ui justify behavior, scissor clipping,
    and the build-time atlas generator.
    Trigger phrases: "SDF text", "SdfTextRender", "build_sdf_glyph_buffer",
    "draw_sdf", "dejavusans", "font atlas", "glyph buffer",
    "text_height", "is_ui", "justify", "clip_rect", "SCISSOR_TEST",
    "measure_all_ui_labels", "make-font-atlas".
---

# Classic Text — SDF Font Rendering

## 1. Architecture

SDF text rendering is a single-pass GPU system. Text entities carry an
`SdfTextRender` component with style/colour/text data. Each frame, the render
loop rebuilds the glyph buffer if the text or scale changed (justification
changes are **not** part of the dirty check), caches the GPU buffer in
`SdfTextGpu`, and dispatches a single `draw_sdf` call.

The render list sorts `DrawKind::SdfText` items by `Transform.position.z`, so
text shares the same z-sorted draw order as other UI elements. There is no
separate text post-pass.

The `SdfTextRender` component:
```rust
pub struct SdfTextRender {
    pub atlas_name: String,        // "dejavusans" (without -sdf suffix)
    pub color: [f32; 4],
    pub bgcolor: [f32; 4],         // STORED, NOT RENDERED (§10)
    pub outline_color: [f32; 4],
    pub outline_width: f32,
    pub shadow_offset: [f32; 2],   // STORED, NOT RENDERED (§10)
    pub shadow_color: [f32; 4],    // STORED, NOT RENDERED (§10)
    pub shadow_blur: f32,          // STORED, NOT RENDERED (§10)
    pub ignore_cam: bool,          // always true for UI text
    pub text: String,
    pub justify: TextJustify,
    pub weight: f32,               // SDF weight (anti-aliasing control)
    pub gamma: f32,                // SDF gamma correction
}
```

The GPU cache (`SdfTextGpu`) stores a `GlBuffer` per entity, keyed by entity
ID. When the text or scale changes (or the cache is absent), the buffer is
rebuilt and uploaded. The cache is in the `Engine` struct.

## 2. Font Atlas Loading

Fonts are loaded via `Engine::load_sdf_font(atlas_name, metrics_json,
atlas_png)`:

1. Deserialises `metrics_json` into `SdfFontMetrics` (name, family, atlas_size,
   glyph_size, spread, baseline, line_height, glyphs map)
2. Stores metrics in `Engine::sdf_fonts: HashMap<String, SdfFontMetrics>`
3. Decodes the PNG atlas texture, uploads it to GL via `Gfx::add_texture_rgba8`
4. Sets the texture filter to `LINEAR` (non-SDF fonts would use NEAREST)

The atlas name convention: the metrics JSON key is the bare name (e.g.
`"dejavusans"`), but the GL texture name appends `-sdf` (e.g.
`"dejavusans-sdf"`). The render loop constructs the full atlas texture name
before binding.

`SdfFontMetrics` stores per-glyph data in a `HashMap<String, GlyphMetrics>`
keyed by character. Each `GlyphMetrics` contains:
- `x, y, w, h` — pixel region in the atlas texture
- `x_offset, y_offset` — offset from the glyph origin
- `x_advance` — horizontal advance to the next glyph

## 3. Glyph Buffer Builder

`build_sdf_glyph_buffer(metrics, text, scale, justify, layout_width)` builds
an interleaved vertex buffer in four phases:

### Phase 1 — Per-line layout
Splits text by `\n`, iterates characters. For each glyph with metrics, records
`(char, x, line_index, advance)`. Missing glyphs (no metrics entry) are silently
skipped. Space width comes from the space glyph or `glyph_size * 0.5`. Tab
width is 4× space.

### Phase 2 — Justification
For `Center` or `Right`: computes per-line width, then shifts each glyph's `x`
by `(column_width - line_width) / 2` or `(column_width - line_width)`. Column
width is `layout_width` if > 0, else the maximum line width.

### Phase 3 — Glyph-extent height
Computes `text_height` starting from `max_h * line_count`; then, **only when**
the glyph extent is non-trivial (`glyph_extent_min < glyph_extent_max`), it
overrides to `(glyph_extent_min + glyph_extent_max).max(1.0)` (the two
candidates are never `max()`-combined).  The glyph extent is the visual range
from the top of the highest glyph to the bottom of the lowest glyph, centred so
that the glyph row's visual centre coincides with the element's geometric
centre at `h/2`. This padding corrects SDF text centering so
`UiAnchor::MidCenter` aligns the visual text, not the line-height box.

### Phase 4 — Vertex buffer
For each glyph, builds 6 vertices (2 triangles) in `{local_x, local_y, tex_u,
tex_v}` interleaved format. `local_x`/`local_y` are in [0..1] space normalized
by `text_width` and `text_height`. `tex_u`/`tex_v` are normalized atlas
coordinates.

The returned `SdfGlyphBuffer` contains:
- `vertices: Vec<SdfGlyphVertex>` — length = `glyph_count * 6`
- `text_width: f32` — total bounding-box width
- `text_height: f32` — total bounding-box height
- `vertex_count: usize` — `vertices.len()`

The vertex stride is 16 bytes (4× f32), interleaving position (2 floats) and
texcoord (2 floats).

## 4. draw_sdf

The `Gfx::draw_sdf()` function binds the `"sdf"` shader and draws with
`gl.draw_arrays(TRIANGLES, 0, vertex_count)`. No index buffer — the glyph
buffer already contains triangle vertices.

```
draw_sdf(model, camera, atlas_name, color, outline_color, outline_width,
         spread, atlas_size, weight, gamma, vertex_count, vertex_buffer, ignore_cam)
```

Uniforms set:
- `texSampler` — texture unit 0
- `projectionMatrix` — ortho projection (viewport sized)
- `cameraMatrix` — identity if `ignore_cam`, else camera
- `modelMatrix` — supplied model matrix
- `color` — SDF foreground colour
- `outlineColor`, `outlineWidth` — outline rendering
- `softEdge` — hardcoded to 0.08
- `spread` — from font metrics (SDF spread in cell pixels)
- `atlasSize` — atlas texture dimensions (vec2)
- `weight`, `gamma` — SDF sharpness controls

Vertex attributes are bound from the provided `GlBuffer` with stride 16:
- `vertexPos` (float32×2, offset 0)
- `texCoord` (float32×2, offset 8)

## 5. is_ui Justify

The render loop checks whether the text entity has a `UiNode` parent:
```rust
let is_ui = self.world.get::<&UiNode>(*entity).map(|n| n.parent.is_some()).unwrap_or(false);
```

The render loop **always** passes `layout_width = 0` to
`build_sdf_glyph_buffer` — the `is_ui` flag only controls a post-buffer
justify **x-offset** on the model matrix (see §10), not the `layout_width`
argument.  When `is_ui` is true the SDF text element's `Transform` is set by
the UI layout and the buffer uses natural glyph widths; the anchor system
handles overall positioning.  When `is_ui` is false (free-standing text, e.g.
text demo), the position comes from `Transform.position` directly.

## 6. Scissor Clipping

When a parent container has `clip_children = true` and the child text entity
has `clip_rect != Vec4::ZERO`, the render arm:

1. Reads `clip_rect` from `UiNode`
2. Computes a Y-flipped scissor rectangle: `(clip_rect.x, viewport_h -
   clip_rect.y - clip_rect.w, clip_rect.z, clip_rect.w)`
3. Enables `SCISSOR_TEST`, calls `gl.scissor(...)`, draws, then disables
   `SCISSOR_TEST`

The Y-flip is required because GL scissor uses bottom-left origin while the
ortho projection uses top-left. Only `DrawKind::SdfText` applies scissoring —
`UiRect` and `UiSprite` render arms do not check `clip_rect`.

## 7. measure_all_ui_labels

`Engine::measure_all_ui_labels()` is called once after all UI init completes
(before the first frame). It pre-measures all UI-managed SDF text entities to
set correct `UiNode.size` before the first render.

Without this step, text entities created by `spawn_sdf_text` have
`UiNode.size = (max_width, 0)`. The layout pass uses these dimensions for
anchor math. If a text entity is a child of another element (e.g. a button),
`position_children_of` positions it based on the stale `(max_width, 0)` size.
On the first frame, the text appears at the wrong position until the render
pass updates `UiNode.size` with the measured dimensions.

`measure_all_ui_labels`:
1. Queries all entities with `Transform + SdfTextRender + UiNode` where
   `parent.is_some()`
2. Calls `build_sdf_glyph_buffer()` for each to get true `text_width` and
   `text_height`
3. Updates `UiNode.size` to match measured dimensions
4. If any size changed, calls `ui.refresh_layout()` and
   `ui.sync_colliders()`

## 8. SDF Text on Button Children

When a button is created with `sdf_text: true` in `ButtonOptions`, the button
spawns a separate SDF text child entity and anchors it `MidCenter → MidCenter`
within the button container. The text entity is a sibling of the container in
the ECS world but a child in the UI tree.

The button's base color is the container's `RectRender.color`. The text
entity has its own `SdfTextRender.color` (opts.text_color). Hover highlighting
only affects the container's `RectRender` — the text colour is independent.

To change button label text at runtime, query the child entity's
`SdfTextRender` and modify `text`. The change is picked up in the next render
pass via the dirty-text check.

## 9. Atlas Generator

`scripts/make-font-atlas.mjs` is a Node.js script that produces:
- `public/res/{name}-sdf.png` — grayscale SDF atlas texture (power-of-two)
- `public/res/{name}-sdf.json` — glyph metrics JSON

Key parameters:
- `GLYPH_SIZE = 64` (cell pixels in the source raster)
- `PAD = 2` (padding between glyphs in the atlas)
- `FONT_CELL_SIZE = GLYPH_SIZE * 0.4 = 25.6` (size used for baseline/line_height)
- Supersampling factor default: 12×
- Spread default: 4 cell pixels

Process per glyph:
1. Renders the character at `fontSize * supersampling` resolution
2. Runs separable squared-distance transform (Felzenszwalb algorithm) on
   inside and outside masks
3. Normalizes distances to [-1, 1] and encodes as byte values (128 = on-edge)
4. Packs glyphs into a power-of-two atlas texture

Metrics JSON fields:
- `atlasSize: [w, h]` — atlas pixel dimensions
- `glyphSize: 64`
- `spread: 4`
- `baseline` — `fontSize * 0.78` (in cell pixels)
- `lineHeight` — `fontSize * 1.3`
- `glyphs: { char: { x, y, w, h, xOffset, yOffset, xAdvance } }`

The generator also has a content-hash cache (`*-sdf.sig`) to skip regeneration
when inputs are unchanged.

## 10. Known-divergent / non-functional

- **Shadow rendering** — `shadow_offset`, `shadow_color`, and `shadow_blur`
  fields are stored in `SdfTextRender` and deserialized from JSON. They are
  NEVER rendered. The render loop makes exactly one `draw_sdf` call per text
  entity with the main `color` and `outline` uniforms. There is no
  shadow-pass draw call.

- **Background colour** — `bgcolor` is stored but never used. The SDF shader
  renders only the glyph interior (via the distance field); the background
  colour would require drawing a full quad behind the text.

- **Weight and gamma** — These SDF sharpness parameters are stored in the
  component but `spawn_sdf_text()` initializes them to `weight=0.0, gamma=1.0`
  (no effect). The `init_ui` FPS label sets weight to 0.15 directly. The
  `draw_sdf` call always passes the stored values.

- **Single-pass only** — The original TypeScript `SdfText` rendered three
  passes (shadow, glow/background, main). The Rust implementation renders only
  one pass (main + outline). This means text shadow and glow effects do not
  appear even when configured in the component.

- **`UiKind::Text`** — Similar to the UI skill note, `UiKind::Text` exists but
  no factory creates it. `spawn_sdf_text` creates `UiKind::SdfText`. There is
  no non-SDF text rendering path.

- **`layout_width` in render loop** — The render loop always passes
  `layout_width = 0` to `build_sdf_glyph_buffer()` (for both UI and non-UI
  text); `is_ui` only toggles the justify x-offset applied to the model
  matrix.  `build_sdf_glyph_buffer` only uses `layout_width` as a column width
  for justification — it does NOT perform word-wrapping. Multi-line text relies
  on explicit `\n` characters.
