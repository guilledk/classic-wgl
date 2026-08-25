---
name: classic-gfx
description: >
    OpenGL/WebGL2 graphics layer for classic-wgl's Rust port.
    Covers draw_* functions, GL state contract, GlBuffer, GlFrameBuffer,
    shader compilation, texture management, and orthographic z-clipping.
    Use when debugging black screens, GL errors, missing textures,
    tilemap rendering, depth test issues, or visual artifacts.
    Trigger phrases: "draw_tilemap", "draw_iso_sprite", "draw_sprite",
    "draw_rect", "draw_sdf", "draw_line_loop", "GlBuffer", "GlFrameBuffer",
    "begin_frame", "z-clipping", "DEPTH_TEST", "projection", "shader".
compatibility: glow 0.15, GLSL 300 es
metadata:
    author: classic-wgl
    version: '1.0'
allowed-tools: Read, Grep, Glob, Bash(git *), Edit
---

# Skill: classic-gfx

## Scope

Covers the `classic-gfx` crate: the `Gfx` struct, all `draw_*` functions,
`GlBuffer`, `GlFrameBuffer`, `Shader`, `GlTexture`, and the embedded GLSL 300 es
shader sources.  Render-loop glue lives in `classic-engine` (not covered here).

---

## 1. Gfx Struct Overview

Holds all GPU state for the frame:

```rust
pub struct Gfx {
    pub gl: Rc<glow::Context>,
    pub shaders: HashMap<String, Shader>,
    pub textures: HashMap<String, GlTexture>,
    pub quad: QuadBuffers,              // shared unit-quad VBO/IBO
    pub viewport_w: f32,
    pub viewport_h: f32,
    pub render_target: Option<GlFrameBuffer>,
    vao: glow::VertexArray,             // single persistent VAO
}
```

A single VAO is bound in `begin_frame` and stays bound.  Individual draw calls
set up attribute pointers via `vertex_attrib_ptr_f32` within that VAO.

Textures and shaders are keyed by name string (from the manifest).
`quad` is a shared 1×1 quad used by `draw_rect`, `draw_sprite`, `draw_iso_sprite`.

### Shared draw types

- `SpriteRegion<'a>` — how a sprite's texture region is addressed:
  `Grid { frame, tile_set_size }` (uniform-grid index, non-packed fallback) or
  `Uv { uv_rect, trim_offset, source_size, content_size }` (packed-atlas rect).
  The packed form is canonical.
- `IsoSpritePass` — `Normal` | `Ghost`, selecting which phase of the two-phase
  isometric draw to run.
- `RenderSettings` — `ambient`/`light_dir`/`light_color`/`depth_scale`/
  `normal_matrix`, the shared lighting/projection bundle passed to
  `draw_tilemap` and (its ambient/dir/color fields) to `draw_sprite` /
  `draw_iso_sprite` for the runtime Lambertian term.

The projection/camera/model preamble shared by every draw is factored into the
private `Gfx::bind_view(s, camera, model, ignore_cam)` helper.

---

## 2. begin_frame GL State Contract

```rust
fn begin_frame(&self) {
    flush GL error queue (while gl.get_error() != 0 {})
    bind FBO (render_target or default framebuffer)
    viewport(0, 0, w, h)
    bind VAO
    clear_color(0, 0, 0, 1)
    clear(COLOR | DEPTH)
    enable(BLEND)
    blend_func(SRC_ALPHA, ONE_MINUS_SRC_ALPHA)
    depth_func(LEQUAL)
    depth_mask(true)
    disable(SCISSOR_TEST)
}
```

What is set: BLEND enabled (SRC_ALPHA/ONE_MINUS_SRC_ALPHA), depth func LEQUAL,
depth mask true.  What is **NOT** enabled: DEPTH_TEST (left disabled — see §3),
active texture unit, active shader program, SCISSOR_TEST.

---

## 3. DEPTH_TEST Contract

The depth test is scoped per draw call. Only `draw_tilemap` and the two
isometric sprite passes enable it. The entire UI/SDF phase runs without depth
test — layering is purely draw-order (z-sort on the render list).

```
draw_tilemap()          → ENABLE; draw; DISABLE
draw_iso_sprite(Normal) → ENABLE; LEQUAL; depthMask(depth_map.is_some());
                           stencil ALWAYS/REPLACE group; draw; DISABLE
draw_iso_sprite(Ghost)  → ENABLE; GREATER; depthMask(false);
                           stencil NOTEQUAL group / ALWAYS; draw; DISABLE
draw_line_loop()        → depthFunc(ALWAYS); depthMask(false); draw;
                           depthMask(true); depthFunc(LEQUAL)
draw_line_strip()       → same as line_loop
```

The engine drives isometric sprites in **two phases** (all normals, then all
ghosts, before the UI overlay) so sprite-vs-sprite occlusion is resolved by
the depth buffer, not draw order.  The stencil buffer records a per-instance
`ghost_group` id during the normal pass (`REPLACE`) so a ghost pass can skip
pixels its own group already occludes (`NOTEQUAL`).

**Gotcha**: If DEPTH_TEST is accidentally left enabled (e.g. a draw function
doesn't disable it), the next 2D draw call renders nothing. Symptom: black
screen in UI phase, missing SDF text. The `draw_line_*` functions use
`depth_func(ALWAYS)` + `depth_mask(false)` as a wireframe-over-terrain pattern
— lines always pass depth but don't write to the buffer. They must restore
`depth_mask(true)` and `depth_func(LEQUAL)` after drawing.

---

## 4. draw_tilemap

```rust
fn draw_tilemap(
    &self, model: &Mat4, camera: &Mat4, iso_matrix: &Mat4,
    tile_data_tex: &glow::Texture,      // raw GL handle (not named)
    tileset_name: &str,                 // key into Gfx.textures
    tile_set_size: &[f32; 2],           // tiles per row/col in tileset PNG
    tile_pixel_size: &[f32; 2],         // pixel size of one tile in tileset
    map_size: &[f32; 2],
    selected_tile: &[f32; 2], selection_begin: &[f32; 2],
    selection_mode: i32, selection_color: &[f32; 4],
    settings: &RenderSettings,          // ambient/light_dir/light_color/depth_scale/normal_matrix
    show_grid: bool,
    vertex_count: i32, vertex_buffer: &GlBuffer,
);
```

Uses the `isoTilemap` shader. Two texture units: unit 0 = tile data texture
(raw `glow::Texture` passed as handle, NOT a named `GlTexture`), unit 1 = tileset
sprite sheet (named, from `Gfx.textures`).

Interleaved vertex attribs (36 bytes/vertex): `vertexPos`(3f, offset 0),
`mapCoord`(2f, offset 12), `tileId`(1f, offset 20, >0.5 = wall), `normal`(3f, offset 24).

Vertex shader: `projectionMatrix * cameraMatrix * modelMatrix * isoMatrix * vertexPos`,
then `worldPos.y -= vertexPos.z` for height correction, then the canonical
iso-depth formula sets `gl_Position.z` in window space:

```glsl
highp float isoDepth = (vertex_pos.x - vertex_pos.y) / depth_scale.x + 0.5 + (vertex_pos.z / ppm) / depth_scale.y;
clipPos.z = isoDepth * 2.0 - 1.0;
```

`depth_scale` and `ppm` are set from the `classic-core::tilemap` constants
(`horizontal_depth_scale`, `HEIGHT_DEPTH_SCALE_M`, `PPM_TARGET`) so there are no
GLSL literals.

`selection_mode` (0=invert, 1=colorize, -1=none) highlights tiles between
`selectionBegin` and `selectedTile`.  Lighting is diffuse:
`ambientColor + max(dot(normal, lightDirection), 0) * lightColor`.  Grid overlay
uses edge-detection blended with `gridColor` when `show_grid` is true and not selecting.

### Nav mesh variant

The same `draw_tilemap` function draws the nav mesh overlay. Only the parameters
differ: the `NavMesh.tile_set` texture (default `"navTileset"`), `show_grid:
false`, `selection_mode: -1`, and a nav-specific tile data texture built from
`NavMesh.data`.  The nav entity must have a `Transform` component matching the
parent tilemap for correct depth ordering.

---

## 5. draw_iso_sprite

```rust
fn draw_iso_sprite(
    &self, model: &Mat4, camera: &Mat4, texture_name: &str,
    region: SpriteRegion<'_>,              // Grid { frame, tile_set_size } | Uv { … }
    iso_depth_corners: &[f32; 4],          // [sw, se, nw, ne]
    depth_map: Option<(&str, f32)>,        // (depth texture name, depth_range)
    depth_base: f32,                       // anchor-plane iso depth (0.5 gray)
    normal_map: Option<&str>,              // normal-map texture name (runtime Lambertian)
    settings: &RenderSettings,             // ambient/light_dir/light_color (sprite uses these three)
    ghost_group: u32,
    pass: IsoSpritePass,                   // Normal | Ghost
);
```

The two-phase isometric sprite draw is a single entry point selected by
`pass` (`IsoSpritePass::Normal` / `IsoSpritePass::Ghost`).  The `region`
selects the texture region (`SpriteRegion::Grid` for the uniform-grid fallback,
`SpriteRegion::Uv` for a packed-atlas rect).

Uses `imageSheet` shader with `useIsoDepth = 1.0`.  The vertex shader
(`direct_tex.vert`) interpolates `iso_depth_corners` across the quad to set
per-fragment `gl_Position.z`; when the sprite has a depth map the fragment
shader (`sheet.frag`) instead writes `gl_FragDepth` from the grayscale map:

```glsl
gl_FragDepth = depth_base + (0.5 - gray) * depth_range;
```

`depth_base` and `depth_range` are window-space iso depths (like the tilemap's
`gl_Position.z`), so no clip→window remap is needed — the depth map and terrain
share one consistent depth space.

When `normal_map` is `Some(name)`, the sprite is shaded with a runtime
Lambertian term in `sheet.frag` (matching `iso_tilemap.frag`): the normal map
is sampled at the same `sheetUv` as colour/depth, decoded `n = rgb * 2 - 1`,
and applied as `color.rgb *= ambient_color + max(dot(n, light_direction), 0)
* light_color`.  `use_normal_map = 0` (no normal map) is byte-identical to the
baked-lit path.

**Unlit sentinel:** a `(0.5, 0.5, 0.5)` normal texel decodes to
`(0, 0, 0)`, and `sheet.frag` skips the Lambertian term when
`dot(n, n) < 0.001`.  classic-assets emits this sentinel for emissive sprite
regions (e.g. the rocket's flame cones) so they stay flat albedo instead of
being shaded.

**Per-sheet normal/depth:** `SpriteSheetEntry` now carries optional
`normal`/`depth`/`depth_range` per sheet (shared-atlas parallel companions).
The engine's `resolve_frame` derives the GL texture names
`"{sheet_name}-normal"` / `"{sheet_name}-depth"` (bundled as plain textures by
classic-roms) and binds them per-frame, falling back to the per-texture
`entry.normal`/`entry.depth` manifest fields for non-shared assets.  The
`depth_map`/`normal_map` args remain `Option<(&str, f32)>` / `Option<&str>`
resolved GL texture names.

Two **separate** passes (driven by two engine loops, all normals then all
ghosts):

- **normal** — `LEQUAL`, `depth_mask(depth_map.is_some())` (depth-mapped
  sprites write depth), stencil `ALWAYS`/`REPLACE ghost_group`,
  `stencil_mask(0xFF)`, `ghost_alpha=0`.
- **ghost** — `GREATER`, `depth_mask(false)`, `ghost_alpha=0.4`, stencil
  `NOTEQUAL ghost_group` (`ALWAYS` when group 0), `stencil_mask(0x00)`.

Both restore `depth_mask(true)`, `depth_func(LEQUAL)`, `disable(DEPTH_TEST)`,
`disable(STENCIL_TEST)`.

The depth map binds on texture unit 1 (`depth_sampler`) and the normal map on
unit 2 (`normal_sampler`); colour is unit 0 (`tex_sampler`).

---

## 6. draw_sprite / draw_rect

### draw_sprite

```rust
fn draw_sprite(
    &self, model: &Mat4, camera: &Mat4, texture_name: &str,
    region: SpriteRegion<'_>,          // Grid { frame, tile_set_size } | Uv { … }
    ignore_cam: bool, ghost_alpha: f32,
    settings: &RenderSettings,         // light preset; unused unless a normal map is bound
);
```

Uses `imageSheet` shader with `useIsoDepth = 0.0`.  The `region` selects the
texture region: `SpriteRegion::Grid { frame, tile_set_size }` for the
uniform-grid frame index (non-packed fallback), or `SpriteRegion::Uv { uv_rect,
trim_offset, source_size, content_size }` for a packed-atlas rect.  For UI:
`ignore_cam: true` (identity camera), `ghost_alpha: 1.0`.  For world sprites,
camera transform applies.  `draw_sprite` never binds a normal map
(`use_normal_map = 0`), so `settings` only supplies the (unused) light
uniforms — plain sprites stay baked-lit.

### draw_rect

```rust
fn draw_rect(&self, model: &Mat4, camera: &Mat4, color: &[f32; 4], ignore_cam: bool);
```

Uses `solid` shader.  Model matrix must incorporate size as scale
(e.g. `Mat4::from_translation(pos) * Mat4::from_scale(Vec3::new(w, h, 1.0))`).

Both use shared `quad` buffers: `quad.verts` → `vertexPos` (vec3),
`quad.uv` → `texCoord` (vec2, sprite only), `quad.indices` → element array.

---

## 7. draw_sdf

```rust
fn draw_sdf(
    &self,
    model: &Mat4,
    camera: &Mat4,
    atlas_name: &str,                 // SDF font atlas texture name
    color: &[f32; 4],
    outline_color: &[f32; 4],
    outline_width: f32,               // 0.0 = no outline
    spread: f32,                      // from font metrics
    atlas_size: &[f32; 2],            // pixel dimensions of atlas texture
    weight: f32,                      // font weight (0.0–1.0, 0.5 = normal)
    gamma: f32,                       // gamma correction (> 0.001 enables)
    vertex_count: i32,
    vertex_buffer: &GlBuffer,         // interleaved [pos, uv] x N glyph quads
    ignore_cam: bool,
);
```

Uses the `sdf` shader.  Texture unit 0 = SDF font atlas.  Vertex buffer is
interleaved [pos(vec2), uv(vec2)] — 16 bytes/vertex, drawn as TRIANGLES.

Fragment shader: smoothstep around edge (0.5 - weight) with `fwidth`-based pixel
range calculation.  Outline rendered when `|outlineWidth| > 0.001` by offsetting
the edge threshold by `outlineWidth / (2 * spread)`.

**Single-pass only** — one draw per `DrawKind::SdfText`.  Shadow/glow effects
would require separate draw-list entries with distinct `SdfTextRender`
components; `softEdge` is hardcoded to 0.08.

---

## 8. draw_line_loop / draw_line_strip

```rust
fn draw_line_loop(
    &self, vertex_buffer: &GlBuffer, vertex_count: i32,
    model: &Mat4, camera: &Mat4, color: &[f32; 4],
);

fn draw_line_strip(
    &self, vertex_buffer: &GlBuffer, first: i32, count: i32,
    model: &Mat4, camera: &Mat4, color: &[f32; 4],
);
```

Both use `solid` shader with `depth_func(ALWAYS)` + `depth_mask(false)` —
wireframe-over-terrain pattern.  `line_loop` draws a closed polygon,
`line_strip` draws one or more line segments.  Both restore `depth_mask(true)`
and `depth_func(LEQUAL)` after drawing.

---

## 9. GlBuffer

```rust
pub struct GlBuffer {
    buffer: glow::Buffer,
    target: u32,           // ARRAY_BUFFER or ELEMENT_ARRAY_BUFFER
    count: usize,          // element count
}
```

`GlBuffer::from_slice(gl, target, data, usage)` takes `bytemuck::Pod` data,
casts to bytes, uploads.  `bind(gl)` binds to target.  `sub_data(gl, data)`
re-uploads at offset 0.  Drop intentionally leaks — no GL context access.

---

## 10. GlFrameBuffer

```rust
pub struct GlFrameBuffer {
    fbo: glow::Framebuffer,
    depth_rb: Option<glow::Renderbuffer>,
    pub texture: glow::Texture,
    pub width: u32,
    pub height: u32,
}
```

`GlFrameBuffer::new(gl, width, height, with_depth)` creates an RGBA color
texture (`NEAREST` filtering) attached to `COLOR_ATTACHMENT0`.  With depth: a
`DEPTH_COMPONENT16` renderbuffer is also attached.

Methods: `bind(gl)` / `unbind(gl)` toggle the FBO.  `clear(gl, rgba)` clears
color+depth.  `resize(gl, w, h)` resizes both attachments (no-op if unchanged).
`read_pixels_rgba(gl)` binds, reads RGBA, unbinds.

Set/clear via `Gfx::set_render_target()` / `Gfx::clear_render_target()`.
`begin_frame` checks `render_target` and binds accordingly.  Drop is a no-op
(buffers live for process lifetime).

---

## 11. Shader Compilation

```rust
pub struct Shader {
    program: glow::Program,
    attr: HashMap<String, u32>,
    unif: HashMap<String, glow::UniformLocation>,
}
```

`Shader::compile(gl, vs_src, fs_src, attr_names, unif_names)` compiles VS + FS,
creates program, binds attribute locations by `attr_names` index (index =
location — critical for matching attribute array layout), links, retrieves
attribute/uniform locations into HashMaps.

Uniform setters (`uniform_mat4`, `uniform_vec4`, `uniform_1f`, `uniform_1i`,
`uniform_bool`) silently skip if the uniform is not found.  Attribute lookups
via `s.attr(name)` panic on missing attributes.

All shader sources are embedded at compile time (GLSL 300 es).  A standalone
`ShaderSourceRegistry`
(`resolve_vertex` / `resolve_fragment`) maps manifest URL strings to embedded
sources by the filename's last `/`-segment (via `shader_filename`), not a
substring match.

**Shader ownership:** the ROM manifest's `shaders[]` is now `[]`.  The
engine compiles its built-in declaration catalog via
`classic_gfx::builtin_shaders()` (a `Vec<BuiltinShader>` = name + vertex/
fragment filenames + attr/unif layout), resolving each source through
`ShaderSourceRegistry::builtin()`.  A non-empty manifest `shaders[]` entry
overrides a builtin **by name** (swapping its vertex/fragment filenames +
layout).  `classic_core::types::Manifest.shaders` is `#[serde(default)]`, so
the field may be absent.

### Shader → draw-function mapping

| Shader name | Used by | Vertex | Fragment |
|---|---|---|---|
| `solid` | draw_rect, draw_line_* | `direct.vert` | `solid.frag` |
| `imageSheet` | draw_sprite, draw_iso_sprite | `direct_tex.vert` | `sheet.frag` |
| `sdf` | draw_sdf | `sdf.vert` | `sdf.frag` |
| `isoTilemap` | draw_tilemap | `iso_tilemap.vert` | `iso_tilemap.frag` |

`image` and `imageColorize` are compiled but have no public `draw_*` functions.

---

## 12. Orthographic Z-Clipping Trap

The projection matrix is:

```rust
Mat4::orthographic_rh(0.0, viewport_w, viewport_h, 0.0, -10000.0, 10000.0)
```

z ∈ [-10000, +10000] is visible; anything outside is silently clipped with no
GL error.  `ignore_cam: true` elements bypass the camera matrix, so their model
z passes directly to projection.  The render-list z-order sort key is a separate
value:

| Element     | Sort key | Model z   | Clipped? |
|-------------|----------|-----------|----------|
| Tilemap     | 20000    | 0         | No       |
| HUD/UI      | -1000    | -1000     | No       |
| Debug lines | -1500    | -1500     | No       |
| Cursor      | -20000   | -10000    | No (boundary) |

The cursor uses sort key -20000 for draw order but model z = -10000 for the
near-plane limit.  Symptom of z-clipping: draw appears in golden trace but
renders nothing — usually caused by using the sort key directly as model z
when it exceeds the ±10000 range.

---

## 13. Texture Management

### GlTexture

```rust
pub struct GlTexture {
    texture: glow::Texture,
    pub size: (u32, u32),
}
```

`from_rgba8(gl, rgba, w, h)` uploads RGBA8 with `NEAREST`/`CLAMP_TO_EDGE`.
`set_linear(gl)` switches to `LINEAR` (SDF atlas).  `bind(gl, unit)` binds to
`TEXTURE0 + unit`.  Stored in `Gfx.textures` by name; `gfx.texture(name)` panics
if missing.

Tile ID data is uploaded as a raw `glow::Texture` (not `GlTexture`) — each pixel
encodes the raw tile-id byte (clamped to 255).  In the fragment shader the
GPU-normalised `R` channel (`v/255`) is decoded as `floor(R * 256.0)`.

**The data texture is one pixel per tile** (`size_x × size_y`), with no
power-of-two padding.  `build_tile_texture` and `upload_data_texture` never
query `GL_MAX_TEXTURE_SIZE`, so a map dimension beyond the GPU limit (WebGL 2
guarantees only 2048; desktop GL usually 8192–16384) silently produces a
garbage texture and corrupts every tile lookup — no GL error is raised.  See
`classic-procmaps` §5 for the full scaling envelope.

---

## 14. draw_iso_sprite Ghost Pass Numeric Constraint

The ghost-pass `ghostAlpha` is hardcoded to 0.4 inside `draw_iso_sprite`
(`IsoSpritePass::Ghost`).  This value is NOT a parameter — callers cannot
override it.  A ghost pass only runs for `draw_iso_sprite(…, IsoSpritePass::Ghost)`;
`draw_sprite` with `ghost_alpha: 0.4` is not an equivalent substitute (it lacks
the depth-corner interpolation and the `GREATER`/stencil ghost-group test).

---

## 15. Known-divergent / non-functional

- **GLSL 300 es**: shaders are written in GLSL 300 es (`in`, `out`, `texture`),
  which is incompatible with the older GLSL 100 (`attribute`, `varying`,
  `texture2D`) syntax.

- **Single-pass SDF text**: one draw per `DrawKind::SdfText`.  Shadow/glow
  effects require separate draw-list entries with distinct `SdfTextRender`
  components.  The `softEdge` uniform is hardcoded to 0.08 in `draw_sdf`.

- **Missing `draw_image_colorized`**: The `imageColorize` program (from the
  `image_colorized.frag` file) is compiled but there is no public `draw_*`
  function for it.  The grayscale+colorize fragment shader path has no current
  caller in the render loop.

- **Missing `draw_image`**: The `image` shader is compiled but has no public
  `draw_*` function.  Full-texture (non-spritesheet) draws are not used in the
  current render loop.

- **No mipmap generation**: `GlTexture::from_rgba8` never calls `gl.generate_mipmap`.
  The SDF atlas uses `LINEAR` filtering at level 0 — mip-free but may alias at
  extreme minification.

- **`GlBuffer` / `GlFrameBuffer` Drop**: Both intentionally leak GL resources.
  The buffers and framebuffer objects live for the process lifetime and `Drop`
  cannot access the GL context to call `gl.delete_*`.  This is a known design
  choice, not a bug.

- **No GL debug output / KHR_debug**: No `gl.debug_message_callback` or error
  introspection beyond the per-frame `gl.get_error()` flush in `begin_frame`.
  GL errors that occur mid-frame are silently accumulated until the next frame
  start, where they are discarded.
