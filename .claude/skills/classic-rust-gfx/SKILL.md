---
name: classic-rust-gfx
description: >
    OpenGL/WebGL2 graphics layer for classic-wgl's Rust port.
    Covers shader porting, mandatory VAO binding, uniform setters,
    texture loading from PNG, tilemap vertex layout and draw call,
    nav mesh overlay rendering (Transform requirement, z-order,
    tile_set_size from texture), quad buffers, and the `Gfx` state
    struct.  Use when debugging black screens, GL errors, missing
    textures, tilemap rendering, or nav mesh visibility issues.
    Trigger phrases: "black screen", "GL error", "VAO", "shader",
    "draw_tilemap", "draw_sprite", "draw_rect", "draw_sdf",
    "tilemap", "navTileset", "tile_set_size", "build_mesh",
    "nav mesh", "overlay", "z-order", "texture dimensions".
compatibility: glow 0.15
metadata:
    author: classic-wgl
    version: '0.2'
allowed-tools: Read, Grep, Glob, Bash(git *), Edit
---

# Skill: classic-rust-gfx

## Scope

Covers `crates/classic-gfx/src/lib.rs`, `crates/classic-engine/src/lib.rs`
(render loop, tilemap + nav mesh draw calls), and the `draw_*` methods
on the `Gfx` struct.

---

## 1. draw_tilemap — Full Signature

```rust
pub fn draw_tilemap(
    &self,
    model: &Mat4,
    camera: &Mat4,
    iso_matrix: &Mat4,
    tile_data_tex: &glow::Texture,
    tileset_name: &str,
    tile_set_size: &[f32; 2],     // tiles per row/column in tileset PNG
    tile_pixel_size: &[f32; 2],    // pixel dimensions of one tile
    map_size: &[f32; 2],           // size_x, size_y of the map grid
    selected_tile: &[f32; 2],
    selection_begin: &[f32; 2],
    selection_mode: i32,           // -1=none, 1=drag highlight
    selection_color: &[f32; 4],
    normal_matrix: &Mat3,
    ambient: &[f32; 3],
    light_dir: &[f32; 3],
    light_color: &[f32; 3],
    show_grid: bool,
    vertex_count: i32,
    vertex_buffer: &GlBuffer,
);
```

Used for BOTH terrain tilemap and nav mesh overlay — same shader,
different textures. The tileset texture determines which tiles are
rendered (road_tileset for terrain, nav_tileset for nav mesh).

---

## 2. Nav Mesh Overlay Rendering

### Required: Transform on nav entity

The `tilemapNavigation` entity has `NavMesh` component but NOT
`Transform`. The render query requires both. Add Transform during init:

```rust
let (pos, scl) = self.names.get("tilemap")
    .and_then(|&e| self.world.get::<&Transform>(e).ok())
    .map(|tf| (tf.position, tf.scale))
    .unwrap_or((Vec3::ZERO, Vec3::ONE));
self.world.insert_one(nav_entity, Transform::new(pos, scl));
```

### Render order (z-value)

Render list sorts DESCENDING. Terrain at z=20000, nav at z=19999.
Terrain renders first, nav renders second and overwrites via LEQUAL
depth test (same position/scale/height → same depth values → passes LEQUAL).

```rust
// Tilemap:
items.push((20000.0, e, DrawKind::Tilemap));
// Nav mesh:
items.push((19999.0, e, DrawKind::Tilemap));
```

**Don't use `sort_unstable_by` with equal z-values** — it's not stable.

### tile_set_size from texture dimensions

Don't hardcode. Read from loaded texture:
```rust
let nav_ts = gfx.textures.get("navTileset")
    .map(|t| [t.size.0 as f32 / 8.0, t.size.1 as f32 / 8.0])
    .unwrap_or([2.0, 1.0]);
```

The tile pixel size (8.0, 8.0) is known — nav tiles are 8×8 pixels.
The tile_set_size varies based on the actual PNG dimensions (e.g.,
16×16 → [2, 2]; 16×8 → [2, 1]).

### Nav mesh GPU building

```rust
let (mesh_data, vcount) = build_mesh(sx, sy, &nav_data, &heights, height_scale);
let (tile_pixels, tw, th) = build_tile_texture(sx, sy, &nav_data);
```

Uses parent tilemap's height_data and height_scale so nav tiles render
at exactly the terrain surface.

---

## 3. draw_sprite

```rust
pub fn draw_sprite(
    &self, model: &Mat4, camera: &Mat4, texture_name: &str,
    frame: f32, tile_set_size: &[f32; 2],
    ignore_cam: bool, ghost_alpha: f32,
);
```

For UI sprites: `ignore_cam: true`, `ghost_alpha: 1.0`. Uses `imageSheet`
shader.

---

## 4. draw_rect

```rust
pub fn draw_rect(&self, model: &Mat4, camera: &Mat4, color: &[f32; 4], ignore_cam: bool);
```

For UI rects: `ignore_cam: true`. Model matrix uses `UiNode.size`
for scale (NOT `Transform.scale`, which is `Vec3::ONE` at spawn).

---

## 5. draw_sdf

```rust
pub fn draw_sdf(
    &self, model: &Mat4, camera: &Mat4, atlas_name: &str,
    color: &[f32; 4], outline_color: &[f32; 4], outline_width: f32,
    spread: f32, atlas_size: &[f32; 2], weight: f32, gamma: f32,
    vertex_count: i32, vertex_buffer: &GlBuffer, ignore_cam: bool,
);
```

SDF text uses a glyph buffer built by `build_sdf_glyph_buffer` from
`classic-core`. The glyph buffer is rebuilt only when text or scale
changes (dirty check via `SdfTextGpu` cache).

**`ignore_cam` is handled internally**: the shader uniform `cameraMatrix` is
set to `Mat4::IDENTITY` when `ignore_cam` is true (line 533 of `lib.rs`).
There is no need to pre-select the camera matrix in the engine — the function
handles it. Passing `Mat4::IDENTITY` from the engine side just makes both
the `camera` parameter and the internal check match; it's harmless but redundant.

---

## 6. draw_iso_sprite

```rust
pub fn draw_iso_sprite(
    &self, model: &Mat4, camera: &Mat4, texture_name: &str,
    frame: f32, tile_set_size: &[f32; 2],
    iso_depth_corners: &[f32; 4],
);
```

Two-pass: ghost (ALWAYS depth function) then normal (LEQUAL).
**Must restore** `depth_mask(true)` and `depth_func(LEQUAL)` after the
ghost pass, or terrain depth writes fail on subsequent frames.

---

## 7. begin_frame GL State

```rust
// Flush stale GL errors
while gl.get_error() != 0 {}
// Depth state (DEPTH_TEST remains DISABLED — see §10)
gl.depth_func(glow::LEQUAL);
gl.depth_mask(true);
// Set blending for UI
gl.enable(glow::BLEND);
gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
```

---

## 8. Tilemap Mesh Building

`build_mesh(size_x, size_y, tiles, heights, height_scale) → (Vec<f32>, usize)`

Returns interleaved vertex data (36 bytes per vertex: pos[3], mapCoord[2],
tileId[1], normal[3]) and vertex count.

Skips empty tiles where tile_id == 0 AND all corner heights == 0.

`build_tile_texture(size_x, size_y, tiles) → (Vec<u8>, u32, u32)`

Encodes tile data into RGBA pixels. Each tile ID → `[id, id, id, 255]`.
Uploaded as a GL texture for sampling in the fragment shader.

---

## 9. GlBuffer

```rust
pub struct GlBuffer {
    pub vbo: glow::Buffer,
    pub target: u32,
}
```

Factory: `GlBuffer::from_slice(gl, target, data, usage)`.
Custom `Drop` impl calls `gl.delete_buffer()`.

---

## 10. GL State Flow Per Frame

The GL state machine across a frame is:

```
begin_frame()
  → glEnable(BLEND), glBlendFunc(SRC_ALPHA, ONE_MINUS_SRC_ALPHA)
  → glDepthFunc(LEQUAL), glDepthMask(true)
  → DEPTH_TEST remains DISABLED (no glEnable call)
  → glClear(COLOR | DEPTH)

draw_tilemap()   → glEnable(DEPTH_TEST) … glDisable(DEPTH_TEST)
draw_iso_sprite() → glEnable(DEPTH_TEST) … glDisable(DEPTH_TEST)

UiRect / UiSprite / SdfText  → DEPTH_TEST = disabled throughout
debug footprints             → glEnable(DEPTH_TEST) per draw call
```

**Key gotcha**: `begin_frame` sets depth state but does NOT enable the test.
Each 3D draw function (`draw_tilemap`, `draw_iso_sprite`) temporarily enables
and then disables it. The entire 2D UI phase runs without depth test — layering
is purely draw-order based on z-sort. Do NOT globally enable DEPTH_TEST in
begin_frame; it will cause UI elements to be depth-rejected against the tilemap
(massively different z values under the ortho projection).

---

## 11. Orthographic Z-Clipping Trap

The projection is constructed via `Mat4::orthographic_rh(0, vw, vh, 0, -10000, 10000)`.
This maps z ∈ [-10000, 10000] to NDC z ∈ [-1, 1]. Any model-matrix z value
outside this range is silently clipped by the GPU — **fragments are discarded**,
the draw call happens but nothing renders.

### Screen-space overlay z requirements

All `ignore_cam` elements (UI, cursor, debug overlays) pass the model z
directly through the orthographic projection with identity camera. Their
model z must stay within [-10000, 10000]:

| Element      | Sort key (z-order) | Model z (actual GL) | In clip range? |
|---|---|---|---|
| Tilemap      | 20000              | 0 (from Mat4)          | ✓ |
| HUD / UI     | -1000             | -1000                  | ✓ |
| Iso debug    | -1500             | -1500                  | ✓ |
| Cursor       | -20000            | -10000                 | ✓ (sort ≠ model) |

### The cursor pattern: separate sort key from model z

The render-list sort key controls draw order. When you need extreme z-ordering
(e.g. cursor must be on top of everything), set a large sort key but keep the
model z within clip range:

```rust
// In the render-list builder:
let sort_z = if sprite.ignore_cam { -20000.0 } else { tf.position.z };
items.push((sort_z, e, DrawKind::Sprite));

// The sprite model uses tf.position.z = -10000 (within [-10000, 10000])
```

### Symptom of z-clipping

Draw calls appear in the golden trace (the render list includes them) but
nothing is visible on screen. The `draw_*` functions execute and the golden
`TraceCollector` records the model matrix — but the GPU clips all fragments
because z is outside the near/far planes. In headless Mesa, this is silent;
on desktop GL, there is no GL error either.

### Common failure values

- `z = -30000` — clipped (far outside near plane)
- `z = -20000` — clipped (outside near plane)
- `z = -1500` — OK (within range)
- `z = -10000` — OK (at near plane boundary, cursor uses this)
