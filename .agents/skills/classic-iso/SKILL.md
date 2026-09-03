---
name: classic-iso
description: >
    Isometric coordinate system, tilemap rendering, depth formulas,
    and sprite occlusion for classic-wgl's Rust port.  Covers
    cartesian↔iso transforms, the isoDepth formula, bilinear height
    interpolation, mouse-to-iso parallax solving, tilemap mesh
    generation, wall faces, nav mesh overlay, IsoSprite ghost rendering,
    IsoAgent state machine, footprint colliders, and the render sort
    order.  Use when debugging selection accuracy, depth occlusion,
    sprite-terrain alignment, tile editing, or any coordinate-space
    mismatch between CPU and GPU.
    Trigger phrases: "iso coords", "mouse to iso", "depth formula",
    "isoDepth", "cartesianToIso", "camera position", "bilinear height",
    "tilemap mesh", "build_mesh", "wall faces", "IsoSprite", "IsoAgent",
    "ghost render", "nav mesh overlay", "sort order", "footprint",
    "selection offset".
---

# Isometric Rendering in classic-wgl

This is the authoritative reference for the isometric render pipeline in the
Rust port.  Everything from coordinate math through tilemap mesh generation,
depth occlusion, sprite rendering, agent animation, nav mesh overlay, and the
render sort order is covered here.

> **⚠️ Coordinate-system unification (post-2026-09).**  The renderer now lives
> in one Blender-canonical **world-metre** space; the whole isometric projection
> is the single orthographic camera [`iso_camera_matrix`] and depth is the camera
> view depth [`iso_view_depth`] normalised over `DEPTH_NEAR`/`DEPTH_FAR`.  The
> canonical math is **`crates/classic-core/src/math.rs`** (doc comments) plus the
> shaders `iso_tilemap.vert` / `direct_tex.vert` / `sheet.frag`.  This document
> has been migrated to that model; if you spot any of the old tile-space terms
> (`cartesian_to_iso_4`, `iso_to_cartesian_4`, `iso_matrix`, `height_scale`,
> `depth_scale`, `depth_range`, or `cart_pos.y -= …`), treat them as stale and
> prefer `math.rs`.  See `plans/opencode/coordinate-system.md` for the full
> cheat-sheet.

All code lives in three crates:

| Crate | Role |
|---|---|
| `classic-core` | Math, tilemap mesh builder, components, collision, bilinear height |
| `classic-gfx` | GL draw calls, shaders, GPU buffers |
| `classic-engine` | Engine god-object, init prefabs, model-matrix helpers, agent system |

---

## 1. Map Orientation and Coordinate System

The map is a 2:1 isometric diamond grid.  The origin is in the top-left
(north-west) corner of the tile grid in cartesian space.

**Compass directions in iso-space:**

- **SW** (screen bottom-left) — closest to the viewer
- **NE** (screen top-right) — farthest from the viewer
- `+x` iso axis = screen down-right (along the `(1, 1)` diagonal in screen space)
- `+y` iso axis = screen down-left (along the `(-1, 1)` diagonal in screen space)

Every iso coordinate `(tx, ty)` maps to a screen position after rotation by
π/4 and Y-scale ×2.  The engine converts tile-grid positions to world-space
cartesian, then the camera matrix projects to screen.

---

## 2. World-metre transforms (`classic-core::math`)

The renderer no longer uses separate `cartesian_to_iso` / `iso_to_cartesian`
matrices.  Everything lives in one Blender-canonical world space and the whole
isometric projection is the single orthographic camera.  The primitives (all in
`classic-core::math`):

| Helper | Does |
|---|---|
| `iso_world_pos(tx, ty, h)` | tile `(tx, ty)` + world height `h` (metres) → world `(tx·TILE_M, −ty·TILE_M, h)` |
| `iso_basis()` | camera basis `(right, up, back)` — unit, 45° yaw / 30° elevation / `ty→−Y` flip |
| `iso_camera_matrix()` | world → camera view; `view·w = (dot(right,w), dot(up,w), dot(back,w))` |
| `iso_view_depth(world)` | `dot(back, world)` — metres, *decreases* with distance |
| `iso_camera_px(world)` | `(view.x·ppm, −view.y·ppm)` — camera-view screen px (before pan/zoom) |
| `iso_camera_px_inverse(px)` | camera-view screen px → ground-plane (`z=0`) world point |
| `iso_world_light_matrix(scale)` | world metres → metric light space (px, +Z up) — independent of the screen camera |
| `iso_world_normal_matrix(scale)` | metric world normal → light space |

`TILE_M = 45/64 = 0.703125` (metres/tile) and `PPM_TARGET = 64.0` (px/m) are the
two fixed constants (both in `classic-core::tilemap`).  `iso_world_pos` is the
single tile→world conversion; the `+tx→+X`, `+ty→−Y` flip is Blender's clockwise
top-down convention.  `iso_camera_matrix` folds the 45° yaw + 30° elevation +
`ty→−Y` flip into one matrix — there is no separate squash or shear, and the
screen `y` is negated because `up` projects to the negative old-cartesian y.

---

## 3. Depth Formula

Depth is the single orthographic camera's **view depth** normalised to window
`[0, 1]` — no synthetic tile-space `tx − ty`, no separate height divisor, no
per-map horizontal scale.  The camera basis `back = right × up =
(−√(3/8), −√(3/8), +0.5)` points **toward** the camera, so view depth
`dot(back, world)` *decreases* with distance: the nearest map corner (SW) is the
most positive, the farthest (NE) the most negative.

The canonical formula (one definition, `classic-core::math`):

```text
iso_view_depth(world) = dot(back, world)                         // metres
depth (window [0,1])  = (DEPTH_NEAR − iso_view_depth) / (DEPTH_NEAR − DEPTH_FAR)
DEPTH_NEAR = 220.0, DEPTH_FAR = −220.0                            // fixed global bounds
```

`DEPTH_NEAR` (closest) is positive and `DEPTH_FAR` (farthest) negative —
numerically `near > far` — so the normalised depth is
`(DEPTH_NEAR − dot) / (DEPTH_NEAR − DEPTH_FAR)`, **not** `(dot − near) /
(far − near)`.  `0` = nearest, `1` = farthest (standard window depth).  The
bounds are fixed so every scene shares one range; a 400×400 map spans
`±√(3/8)·400·TILE_M ≈ ±172` view-depth metres, and the tallest sprite (the ~47 m
rocket) adds `0.5·47 ≈ 24` toward the near side — `220` covers both with margin.
Mirrored by classic-assets `render/presets.py::DEPTH_NEAR`/`DEPTH_FAR`.

Height is carried in **world metres** on the mesh/sprite `z` (never a pixel
offset): the tilemap builds vertices as `(tx·TILE_M, −ty·TILE_M, h_m)`, and
`height_data` is metres.  The depth bounds are passed to the shaders as the
`depth_span` uniform (`vec2(DEPTH_NEAR, DEPTH_FAR)`) — no GLSL depth literals.

### Tilemap vertex shader (`iso_tilemap.vert`)

```glsl
// World metres -> camera view space (metres): `iso_camera_matrix`.
vec4 view = world_matrix * vec4(world, 1.0);
// Screen pixels before pan/zoom (the camera `up` axis projects to -y).
vec4 screenPos = vec4(view.x * ppm, -view.y * ppm, 0.0, 1.0);
// Camera view depth in window space [0, 1] (0 = nearest, 1 = farthest).
highp float isoDepth = (depth_span.x - view.z) / (depth_span.x - depth_span.y);
clipPos.z = isoDepth * 2.0 - 1.0;
vLightPos = (light_matrix * vec4(world, 1.0)).xyz;   // metric light space, +Z up
```

Key aspects:

1. **No shear.**  There is no `y -= vertex.z`; the screen position comes straight
   from the camera `right`/`up` view components (`view.x`, `view.y`), and depth
   from the `back` component (`view.z`).  Height reads as height because
   `up.z = cos30°` projects world `z` into the screen `y`.
2. **Depth axis is `back`.**  `view.z = dot(back, world)` is camera view depth;
   a larger `dot` is nearer.  Normalised to window `[0, 1]` over the fixed
   `depth_span = [DEPTH_NEAR, DEPTH_FAR]`.
3. **Window space, no clamp:**  the value is `depth_span`-derived window depth;
   geometry beyond `[0, 1]` is clipped by the fixed function (`z_clip ∉ [-1, 1]`).
4. **Light space is separate.**  `light_matrix` (`iso_world_light_matrix`) maps
   world metres to metric light space (+Z up, px) for `vLightPos` — independent
   of the screen camera (see §12).
5. `isoDepth` is computed in `highp` to match the highp sprite/depth-map path and
   the 24-bit depth buffer.

### CPU-side equivalent (`compute_iso_depth_corners`)

The IsoSprite renderer needs depth values for each of the 4 footprint corners
(see Section 8).  The CPU side mirrors the shader via
`Self::world_depth(world) = (DEPTH_NEAR − iso_view_depth(world)) /
(DEPTH_NEAR − DEPTH_FAR)` — the same window-space depth, no bias term.

### `DrawKind::IsoSprite` sort order uses `tx - ty`

Items are sorted descending by their `order` value (larger = farther, drawn
first).  For IsoSprites this is `tf.position.x - tf.position.y`.  For the
tilemap it is fixed at `20000.0` (drawn first, behind everything).  For the
nav mesh it's `19999.0`.

---

## 4. Bilinear Height Interpolation

Defined in `classic-core::tilemap::bilinear_height`.

### Data layout

Height data is stored as a **vertex grid** of size `(size_x + 1) × (size_y + 1)`.
The index is `ty * (size_x + 1) + tx`.  The vertex grid (rather than a per-tile
grid of `size_x × size_y`) avoids off-by-one interpolation edge cases at the
right/bottom map boundary.

### Formula

```rust
pub fn bilinear_height(heights: &[f32], size_x: i32, size_y: i32, px: f32, py: f32) -> f32 {
    let ftx = px.floor() as i32;
    let fty = py.floor() as i32;
    let fx = px - ftx as f32;
    let fy = py - fty as f32;

    // at(tx, ty) clamps to [0, size_x] and [0, size_y]
    let h_nw = at(ftx, fty);
    let h_ne = at(ftx + 1, fty);
    let h_sw = at(ftx, fty + 1);
    let h_se = at(ftx + 1, fty + 1);

    h_nw + (h_ne - h_nw) * fx + (h_sw - h_nw) * fy + (h_nw - h_ne - h_sw + h_se) * fx * fy
}
```

The `at` closure clamps coordinates to the valid range `[0, size_x]` and
`[0, size_y]`, so out-of-bounds accesses return the nearest edge vertex.

### Mesh-matched sampler (`sample_height_mesh`)

`classic-core::tilemap::sample_height_mesh` samples the **same** triangle-linear
interpolation the terrain mesh (`build_mesh`) uses — the top face is split into
`NW→NE→SW` and `NE→SE→SW`, so it is linear within each triangle, not bilinear:

```rust
if fx + fy <= 1.0 {
    h_nw * (1.0 - fx - fy) + h_ne * fx + h_sw * fy
} else {
    h_ne * (1.0 - fy) + h_se * (fx + fy - 1.0) + h_sw * (1.0 - fx)
}
```

`bilinear_height` bows on a non-planar quad (e.g. a saddle across the shared
diagonal), so depth-relevant paths — `Engine::height_at`, the vehicle
`TerrainSnapshot::height`, and `compute_iso_sprite_model` — use
`sample_height_mesh` to match the rendered mesh exactly and avoid ghosting at
slope corners.  (The mouse-to-iso solve and pathfinder still use `bilinear_height`.)

### Uses

- **Mouse-to-iso solve:**  ground-plane ray + height-field intersection (see §5).
- **IsoSprite model matrix:**  `iso_world_pos(tx, ty, h + altitude)` — height is
  world metres in `z`, never a `cart_pos.y` pixel offset (see §8).
- **Agent terrain sampling:**  during `FollowPath` lerp, the agent samples
  terrain height at its current `(px, py)` and smooth-z-interpolates.
- **Footprint collider construction:**  each footprint vertex is
  height-adjusted in world space.
- **Debug footprint rendering:**  same vertex → world-space transform.

---

## 5. Mouse-to-Iso (ground-plane ray + height-field intersect)

Registered in `commit_terrain` as an `on_update` closure.  Runs every frame and
stores the result in `Tilemap.mouse_iso_pos` as `(x, y, z)` — `x/y` in tiles,
`z` the terrain height (metres) under the cursor.

### Algorithm

1. **Un-project to the ground plane.**  Undo pan/zoom
   (`screen += camera.fix(); screen /= camera.scale`), then intersect the camera
   view ray with the world-metre ground plane `z = 0`:

   ```text
   view_x = screen.x / PPM_TARGET
   view_y = -screen.y / PPM_TARGET
   view_z = -up.z · view_y / back.z       // world.z = up.z·view_y + back.z·view_z == 0
   ground  = right·view_x + up·view_y + back·view_z
   ```

2. **Ray/height-field intersect (parallax).**  The ground point's tile coords
   shift along the depth axis by the terrain height; iterate the fixed point
   (8 passes) until the sampled height stabilises:

   ```text
   tx0 = ground.x / TILE_M
   ty0 = -ground.y / TILE_M
   parallax = 2 · √(3/8) / TILE_M
   for _ in 0..8:
       h = sample_height_mesh(tx, ty); if h <= 0: break
       z_off = h * parallax
       tx = tx0 - z_off
       ty = ty0 + z_off
   ```

   Each pass shifts the tile point toward the camera along the depth axis in
   proportion to the terrain height — the parallax compensation.

3. **Ground the height.**  `z = sample_height_mesh(tx, ty)` (the same
   triangle-linear interpolation the rendered mesh uses), then write
   `Tilemap.mouse_iso_pos = (tx, ty, z)`.

This drives the tile selection cursor and editor paint rectangle.  The inverse
(iso → screen) is `iso_to_screen_px` (`iso_world_pos(x, y, h) + tilemap.position`
→ `iso_camera_px` → the `world_to_screen_matrix` pan/zoom).

---

## 6. Tilemap Mesh Generation

Defined in `classic-core::tilemap::build_mesh`.

### Vertex layout

Each vertex is **9 floats = 36 bytes**, interleaved:

| Offset | Size | Attribute | Description |
|--------|------|-----------|-------------|
| 0 | 3×f32 | `vertexPos` | Position in **world metres** `(tx·TILE_M, −ty·TILE_M, h)` |
| 12 | 2×f32 | `mapCoord` | Normalized map UV `[0..1, 0..1]` |
| 20 | 1×f32 | `tileId` | Tile index (≤0 = steep face, >0 = wall) |
| 24 | 3×f32 | `normal` | Smooth per-vertex normal (metric world space) |

Drawn as non-indexed `TRIANGLES`.

### Top face

Each non-empty tile generates two triangles (NW→NE→SW, NE→SE→SW) forming a
single quad.  Heights at the four corners (`z_nw`, `z_ne`, `z_sw`, `z_se`) are
the height grid values verbatim — **already world metres**, no `* height_scale`.
The vertex position is `iso_world_pos(tx, ty, h)` = `(tx·TILE_M, −ty·TILE_M, h)`.

The `tileId` for the top face is `-steepness` where:

```rust
let steepness = ((z_max - z_min) / TILE_M).min(1.0);
```

This negative value routes the fragment shader to tile-data texture lookup
(`vTileId > 0.5` check passes only for positive tile IDs — walls).

### Wall faces

Generated only at map borders, one wall per exterior tile edge:

- **East wall** (`tx+1 >= size_x`, outward `+tx → +X`): normal `[1, 0, 0]`
- **South wall** (`ty+1 >= size_y`, outward `+ty → −Y`): normal `[0, -1, 0]`
- **West wall** (`tx == 0`, outward `−tx → −X`): normal `[-1, 0, 0]`
- **North wall** (`ty == 0`, outward `−ty → +Y`): normal `[0, 1, 0]`

Each wall is a twisted quad (two triangles: lo→mid→hi, lo→hi→mid) where
`mid` is the centre-point of the edge extruded to the midpoint height.  The
`tileId` is `tile_id.max(1)` (positive routing to `wallColor` in the fragment
shader).  Walls only generate when the average height > 0.

### Normals

Top-face normals are **smooth per-vertex normals**, built by
`build_vertex_normals`: the two triangle normals of every adjacent tile are
accumulated onto its four corner vertices and then normalised.  This hides the
triangulation — per-face normals made the herringbone of facets visible on any
non-level terrain.  On a flat map every face normal is already `+Z`, so the
averaged result is bit-identical to the old per-face value and flat scenes
(including the demo golden baseline) are unaffected.

The normals are passed through a `normalMatrix` uniform (`iso_world_normal_matrix`,
the `inverse_transpose(S(scale)·Rz(−45°)) · D` world-normal → light-space matrix)
in the vertex shader for correct lighting.  World normals are
`normalize(D⁻¹ · tile_normal)` with `D⁻¹ = diag(1/TILE_M, −1/TILE_M, PPM_TARGET)`
— see §13.

### Allocation

The mesh pre-allocates the **exact** vertex count, not a worst-case guess:
`6` vertices per non-empty tile (two top-face triangles) plus `6` per wall,
and walls only exist on the map perimeter — `(6 * size_x * size_y + 6 * 2 *
(size_x + size_y)) * 9` floats.  The old formula (`size_x * size_y * 30 * 9`)
assumed all four walls could appear on *every* tile, over-reserving by 5x
(173 MB instead of 35 MB at 400x400).  A test asserts `capacity == bound`.
Empty tiles (tile ID = 0 and all four corners at height 0) are skipped.

---

## 7. Tile Data Texture

Defined in `classic-core::tilemap::build_tile_texture`.

### Purpose

Instead of passing tile indices as vertex attributes or a uniform array, the
tilemap fragment shader sampes a **tile data texture** (texture unit 0) to
look up the tile ID at each fragment.

### Encoding

Each tile's value `v` (clamped to 0..255) is written as an RGBA pixel:

```rust
pixels[p] = v;       // R
pixels[p + 1] = v;   // G
pixels[p + 2] = v;   // B
pixels[p + 3] = 255; // A
```

Dimensions are `size_x × size_y` pixels, one pixel per tile.  NEAREST filtering
(with `CLAMP_TO_EDGE` wrapping) is set in `Engine::upload_data_texture` when the
tile-data texture is uploaded.

### Fragment shader decoding

In `iso_tilemap.frag`:

```glsl
float getMapData(vec2 pos) {
    vec4 rawData = texture(mapData, pos);
    return floor(rawData.r * 256.0);
}
```

The tile index is then used to look up the sprite-sheet tile:

```glsl
vec2 tileId = vec2(floor(mod(tileIdFlat, tileSetSize.x)), floor(tileIdFlat / tileSetSize.x));
```

### Upload

Created during `commit_terrain` and stored as a raw GL texture in `TilemapGpu.tile_tex`.

---

## 8. IsoSprite Rendering (Two-Pass Ghost)

Isometric sprites are billboards placed on the 3D tilemap.  They use the
`imageSheet` shader but with `useIsoDepth = 1.0`, enabling the depth-corner
interpolation path.

### Model matrix (`compute_iso_sprite_model`)

```rust
fn compute_iso_sprite_model(...) -> Mat4 {
    let h = sample_height_mesh(...);                    // terrain height, metres
    let altitude = iso_sprite.frame_offset.z;           // frame altitude, metres
    let drift = Vec3::new(frame_offset.x, frame_offset.y, 0.0);  // horizontal drift

    // Ground anchor in world metres (terrain + altitude) + tilemap offset.
    let world_pos = iso_world_pos(x, y, h + altitude) + drift + tilemap_tf.position;

    // Quad size in metres (source-cell px → metres at PPM_TARGET).
    let w  = tex_dim.0 * scale.x / PPM_TARGET;
    let hh = tex_dim.1 * scale.y / PPM_TARGET;

    // Anchor in normalized [0,1] quad space.
    let ua = anchor_px.x / tex_dim.0;
    let wa = anchor_px.y / tex_dim.1;

    // Billboard: width along (1/√2, −1/√2, 0), height down world −Z.
    let billboard = Mat4::from_cols(
        Vec4::new(FRAC_1_SQRT_2, -FRAC_1_SQRT_2, 0.0, 0.0),
        Vec4::new(0.0, 0.0, -1.0, 0.0),
        Vec4::new(FRAC_1_SQRT_2,  FRAC_1_SQRT_2, 0.0, 0.0),
        Vec4::W,
    );

    Mat4::from_translation(world_pos)
        * billboard
        * Mat4::from_scale(Vec3::new(w, hh, 1.0))
        * Mat4::from_translation(Vec3::new(-ua, -wa, 0.0))
}
```

Key pieces:
- Position → world metres via `iso_world_pos(x, y, h + altitude)` (height in `z`,
  never a `cart_pos.y` pixel offset); `frame_offset.z` is the altitude, `x/y` the
  horizontal drift.
- The billboard basis stands the quad up in world space: width along the screen
  `right` axis `(1/√2, −1/√2, 0)`, height down world `−Z` (so the sheet's `v=0`
  top is at higher `z` and the feet at the anchor).  The screen camera then
  projects this standing quad.
- `tex_dim` is the source-cell pixel size; `w/hh` scale it to metres via
  `PPM_TARGET`.  The `-ua/-wa` translation shifts the quad so the anchor point
  (e.g. `[0.5, 0.98]` = bottom-centre) sits at the world origin.

### Depth corners (`compute_iso_depth_corners`)

Returns `[f32; 4]` where the element ordering matches the `imageSheet` vertex
shader layout:

| Index | Logical name | Vertex map |
|-------|-------------|------------|
| 0 | bottom-left | `min_fp` (global min depth across all four corners) |
| 1 | bottom-right | `min_fp` |
| 2 | top-left | NW depth |
| 3 | top-right | NE depth |

Per-corner depths are first computed as
`min(per-corner-depth, base_depth)` where `base_depth` is the depth at the
sprite's exact position; then the two bottom slots are collapsed to the global
`min_fp` (the minimum of all four raw depths) before returning
`[min_fp, min_fp, raw[3], raw[0]]` (footprint order `[NE, SE, SW, NW]`, so
`raw[3]`=NW and `raw[0]`=NE).  This keeps the quad's bottom edge at the
shallowest point, preventing occlusion artifacts on tall sprites.

The shader interpolates depth across the quad:

```glsl
float bottomDepth = mix(isoDepthCorners.x, isoDepthCorners.y, vertexPos.x);
float topDepth    = mix(isoDepthCorners.z, isoDepthCorners.w, vertexPos.x);
float cornerDepth = mix(topDepth, bottomDepth, vertexPos.y);
gl_Position.z = cornerDepth * 2.0 - 1.0;
```

### Two-pass draw (`draw_iso_sprite`, engine order: all normals, then all ghosts)

The engine drives isometric sprites in **two phases** (all `Normal` passes first,
then all `Ghost` passes) so sprite-vs-sprite occlusion is resolved by the depth
buffer, not draw order:

1. **Normal pass:** `ghostAlpha = 0.0`, `depthFunc(LEQUAL)`,
   `depthMask(depth_map.is_some())` (depth-mapped sprites **write** depth),
   stencil `ALWAYS` / `REPLACE ghost_group`, `stencilMask(0xFF)`.
   Renders the full-colour sprite on top of terrain, respecting depth occlusion.

2. **Ghost pass:** `ghostAlpha = 0.4`, `depthFunc(GREATER)`, `depthMask(false)`,
   stencil `NOTEQUAL ghost_group` (`ALWAYS` when `ghost_group == 0`),
   `stencilMask(0x00)`.  Draws the 40%-alpha silhouette **only where the sprite
   is behind the depth buffer** (occluded by terrain or another sprite), letting
   the player see units through walls.  The stencil skips pixels the sprite's
   own group already occluded (a vehicle's body + wheels share a `ghost_group`
   id so they never ghost through each other).

Both passes restore `depthMask(true)` + `depthFunc(LEQUAL)` and disable
`DEPTH_TEST`/`STENCIL_TEST` on exit.  The stencil buffer records the per-instance
`ghost_group` id during the normal pass (`REPLACE`); the ghost pass reads it
(`NOTEQUAL`).  Sprites with no depth map (`depth_map == None`) do **not** write
depth in the normal pass (`depthMask(false)`) — occlusion is then draw-order
only, and the ghost pass uses the same `GREATER` test against whatever the
terrain/other sprites wrote.

### Sheet fragment shader (`sheet.frag`)

When `ghostAlpha > 0.0`, the output alpha is overridden to `ghostAlpha`:
```glsl
if (ghostAlpha > 0.0) {
    color.a = ghostAlpha;
}
```

### Footprint

Each `IsoSprite` has a `footprint` field — a `Vec<Vec2>` of vertices in iso
tile-space relative to the sprite's position.  The default footprint (4 corners
of a unit diamond) is:

```
NE: (0.5, -0.5), SE: (0.5, 0.5), SW: (-0.5, 0.5), NW: (-0.5, -0.5)
```

Custom footprints (e.g. for rectangular buildings) can have more vertices.

### Vehicle body pitch/roll frames

The `IsoVehicle` body is a single `IsoSprite` whose sheet carries **more than
8 directions**: it stacks `pitch_levels` × `roll_levels` direction blocks
vertically (the exporter renders the body tilted about its ground origin on a
`yaw → pitch → roll` rig; wheels stay flat at 8 directions).

- Body `tile_set_size = [columns, rows · pitch_levels · roll_levels]`; wheel
  sheets stay `[columns, rows]`.
- Body `frame = (pitch_index · roll_levels + roll_index) · directions +
  direction`, where `pitch_index 0..pitch_levels` (0 = nose-down, centre =
  level, top = nose-up) and `roll_index 0..roll_levels` (0 = right-up, centre =
  level, top = left-up).
- The ground-origin `anchor` is pose-invariant (the tilt pivots on the fixed
  ground origin), so the body keeps one anchor per direction, not per pose.
- `VehicleDef` (the `vehicles` manifest sidecar, `classic-core/src/types.rs`)
  carries `directions`/`columns`/`rows`/`cell`, `pitch_levels`/`pitch_max_deg`
  and `roll_levels`/`roll_max_deg`, plus per-part ground-origin `anchors`; the
  exporter emits it and `Engine::spawn_vehicle` derives the wheel tile offsets
  from it.
- `Engine::update_vehicles` (`classic-engine/src/vehicle.rs`) drives the body as
  a single **chassis plane** `(altitude, pitch, roll)` fit to the four wheel
  contacts and spring-smoothed, then quantizes pitch/roll against
  `pitch_max`/`roll_max` to pick the frame; `frame_offset.z` carries the
  suspension/jump altitude (x/y are horizontal drift — world metres).  Each wheel is a per-wheel suspension spring clamped to
  a travel envelope (`wheel_travel_up`/`wheel_travel_down`, derived from the def
  geometry at spawn) around the body plane, so wheels never ride over the body —
  the plane lifts/tilts to absorb terrain instead.  A soft dead-zone
  (`tilt_dead_zone`) suppresses body tilt on sub-frame slopes.

### Front-wheel steering tires

With front-wheel steering (the `front_wheel_steering` work), each front wheel is
split into a static **suspension arm** (the wheel `IsoSprite`) plus a rotating
**steering tire** `IsoSprite` (`tire_entities`, matched by index).  The tire
sheet stacks `steer_levels` direction blocks vertically, so its
`tile_set_size = [columns, rows · steer_levels]` and the frame is
`steer_index · 8 + direction` (`steer_index 0..steer_levels`: 0 = full-right,
centre = straight, top = full-left — the exporter's steer-major order).

`steer_index` is quantized from the *integrated* steering **state**
(`IsoVehicle.steer`, rate-limited at `steer_rate`) rather than the raw heading
error, so the tires sweep through their steer frames instead of snapping.  When
the vehicle reverses (target ~100° behind, `should_reverse` hysteresis), it
drives backward along `heading` while the tires steer into the same turn; the
tire anchor is steer-invariant (the yaw is about the axle's vertical axis), so
the tire reuses the wheel's ground-origin anchor.

---

## 9. IsoAgent

`IsoAgent` is a pathfinding-capable `IsoSprite` subtype (its registry spawner
creates both an `IsoAgent` and an `IsoSprite`).  It adds `speed`, `anim_speed`,
and `anim_prefix` over `IsoSprite`.

The agent's *behaviour* — click-to-move, path-following, idle/walk animation,
terrain-z following — lives in the **ROM guest** (`classic-roms/guest/demo-guest`), not in
Rust.  The retired `init_agent_system` (and the engine's click-to-move wiring)
were replaced by the guest driving the entity through the `classic-guest` SDK:
`request_path`/`poll_path` (async binary waypoints), `set_pos`/`get_pos` (3D),
`set_anim`, `height_at`, `mouse_iso`, `agent_selected`, `ui_consumed_click`.  The
`IsoAgent` component no longer carries runtime state — `path`, `target_index`,
`delta`, `init_dist`, `direction`, `anim_index`, `state` (and the `AgentState`
enum) were removed; the guest keeps its own static path buffer.

The 8 direction names remain `[East, SouthEast, South, SouthWest, West,
NorthWest, North, NorthEast]`; the guest maps a step delta `(dx, dy)` to an
index directly (no `atan2` — unavailable in `core`).

### Animator integration

`init_animator_system` (still Rust) advances all `Animator` counters each frame
and pushes the current frame value to the target's `IsoAgent.frame` and
`IsoSprite.frame` (both kept in sync).  The guest selects which animation plays
via `set_anim`; the `Animator.target` string format is `"entityName.IsoAgent"`,
parsed to resolve the target entity.

---

## 10. Nav Mesh Overlay

The navigation mesh is a secondary tilemap rendered as a translucent overlay
on top of the main terrain.

### Entity

- Named `"tilemapNavigation"` in the world.
- Has a `NavMesh` component with `size_x`, `size_y`, `data` (walkability values),
  and `map_entity` pointing to `"tilemap"`.
- Must have a `Transform` component — the draw code uses `tf.position` for the
  model matrix.

### Mesh generation

The nav mesh uses the same `build_mesh` function as the main tilemap, built
from the parent tilemap's **actual** `height_data` (world metres); when the grid
is absent or the wrong size it falls back to a flat all-`1.0` height grid.  The
data array contains walkability flags where `1` = walkable, `0` = blocked.

### Rendering

Drawn via `draw_tilemap` with:
- **`tile_data_tex`:**  the nav data texture (walkability values).
- **`tileset_name`:** the `NavMesh.tile_set` name (default `"navTileset"`) — a
  small tileset with transparent solid tiles for non-walkable areas.
- **`tile_set_size`:** `[nav_tileset_width / 8, nav_tileset_height / 8]`.
- **`model`:** `Mat4::from_translation(tf.position)`.
- **Sort order:**  `19999.0` — between the main tilemap (`20000.0`) and sprites.

The nav mesh entity (`tilemapNavigation`, `Role == NavMesh`) is iterated in the
render loop whenever `self.nav_gpu.is_some()`; it is drawn regardless of the
`debug_footprints` toggle.

### Depth test

Uses `LEQUAL` depth function — the same as the main tilemap — so it
depth-writes and occludes behind terrain.  The overlay renders below the
IsoSprite z-order and above the main tilemap.

---

## 11. Footprint Colliders

IsoSprite entities (excluding agents) get footprint colliders registered with
`PhysicsProvider` during `init_footprint_colliders`.

### Construction

`init_footprint_colliders` (now in `classic-demo/src/prefabs.rs`) iterates every
`IsoSprite` (skipping `IsoAgent`s, which get their own collider handling):

1. For each footprint vertex `pt`:
   - Iso position `px = sprite_iso_pos.x + pt.x`, `py = sprite_iso_pos.y + pt.y`.
   - Terrain height `h = bilinear_height(...)` (metres).
   - World-metre vertex `iso_world_pos(px, py, h) + tilemap_pos`, then projected
     to screen px via `iso_camera_px`.

2. Build a `Shape::Polygon` via `polygon_from_verts(world_verts)` (auto
   `center`/`min`/`max` AABB).

3. `register_named_collider` with the entity name, and set the sprite's z-offset
   to the terrain height (world metres — no `* height_scale`):
   `tf.position.z = terrain_z`.

---

## 12. Sort Order

The render loop in `Engine::frame` builds a flat list of `(order: f32, entity, DrawKind)`
items and sorts descending (larger order = farther = drawn first).

| DrawKind | Order value | Notes |
|----------|------------|-------|
| `Tilemap` | `20000.0` | Always behind everything |
| `Tilemap` (nav) | `19999.0` | Behind sprites, on top of terrain |
| `IsoSprite` | `tf.position.x - tf.position.y` | Depth-major sort |
| `Sprite` (non-UI) | `tf.position.z` (or `-20000.0` if `ignore_cam`) | Z-order |
| `UiSprite` | `tf.position.z` | UI z-slice |
| `UiRect` | `tf.position.z` | |
| `SdfText` | `tf.position.z` | |

The IsoSprite sort formula `tx - ty` mirrors the depth axis in the vertex
shader (`vertexPos.x - vertexPos.y`).  A sprite at iso `(10, 2)` has order
`8.0`; one at `(2, 10)` has order `-8.0`.  Larger values are farther from the
camera and drawn first.

Non-UI sprites with `ignore_cam = true` get a fixed order of `-20000.0`.  Since
the list is sorted **descending**, `-20000.0` is the smallest key and is drawn
**last** (on top), like UI — this is how the HUD cursor sprite works.

`UiRect` and `UiSprite` use `tf.position.z` for layering within the UI
plane.  Items are drawn **without** `DEPTH_TEST` (enabled only during tilemap
and IsoSprite draw calls).

---

## 13. Lighting

Lighting uniforms are passed to the tilemap shader each frame from `Engine`
fields:

| Field | Default | Description |
|-------|---------|-------------|
| `light_ambient` | `[0.15, 0.15, 0.2]` | Ambient light (base illumination) |
| `light_dir` | `[0.45, -0.35, 0.82]` | Normalized light direction |
| `light_color` | `[1.0, 0.95, 0.85]` | Diffuse light colour |

### Fragment shader lighting

In `iso_tilemap.frag`:
```glsl
float diff = max(dot(normalize(vNormal), lightDirection), 0.0);
color.rgb *= ambientColor + diff * lightColor;
```

A standard Lambertian diffuse model: the normal (already transformed by
`normalMatrix` in the vertex shader) is dotted with the light direction.
The result modulates the colour additively over ambient.

### LIGHT_PRESETS

Four named presets in `apply_light_preset` (defined as unnormalized direction
vectors, normalized at application time):

| Preset | Ambient | Direction (unnorm) | Colour |
|--------|---------|-------------------|--------|
| `sunny` | `[0.15, 0.15, 0.2]` | `[0.453, 0.211, 0.866]` | `[1.0, 0.95, 0.85]` |
| `cloudy` | `[0.35, 0.35, 0.4]` | `[0.0, -0.2, 1.0]` | `[0.7, 0.72, 0.78]` |
| `dawn` | `[0.2, 0.15, 0.25]` | `[0.5, 0.2, 0.3]` | `[1.0, 0.4, 0.2]` |
| `night` | `[0.1, 0.12, 0.25]` | `[-0.2, -0.5, 0.8]` | `[0.3, 0.4, 0.7]` |

The light widget translates between azimuth/elevation angles and the direction
vector in `update_light_direction`:

```rust
let d = Vec3::new(
    el.cos() * az.sin(),
    -el.cos() * az.cos(),
    el.sin(),
).normalize();
```

### Normal matrix

The tilemap shader receives a `normalMatrix` uniform computed from the world→light
normal matrix:

```rust
let normal_matrix = classic_core::math::iso_world_normal_matrix(tilemap_tf.scale);
```

`iso_world_normal_matrix` is `inverse_transpose(S(scale)·Rz(−45°)) · D` (where
`D = diag(TILE_M, −TILE_M, 1/PPM_TARGET)`), **not**
`inverse_transpose(mat3(iso_world_light_matrix))` — `D` does not commute with the
rotation, and the wrong order subtly re-axes slope lighting.  World normals are
`normalize(D⁻¹ · tile_normal)`.

### Dynamic point lights (UBO)

In addition to the sun (above), `Engine` maintains a pooled set of **dynamic
point lights** uploaded to a `std140` UBO once per frame.  See the `classic-gfx`
skill (§16 "Dynamic lights") for the buffer layout and the `classic-ecs` skill
for the `Light` component.  Only two points matter for iso placement:

- **`Engine::iso_to_world(x, y, elevation)`** is the single conversion from an
  iso tile to a **light-space** position:
  `iso_world_pos(x, y, height_at(x, y) + elevation) + tilemap.position`.
  `elevation` is metres above the terrain (same units as `height_data`).
  **Height goes in `z` alone.**  Light space is +Z up — the same space
  `light_dir` and `vNormal` live in.  This function used to also apply the
  renderer's isometric shear (`p.y -= z_px`), which put lights in screen space
  while the normals they are dotted against stayed in light space; `dot(n, L)`
  then mixed spaces.  See `classic-gfx` §17.

Point lights are **unoccluded** (no point-light shadows).  A bare point light on
terrain reads as a symmetric "sphere"; that's expected until point-light shadows
(M5).  The sun *does* cast a directional shadow map — see `classic-gfx` §17.

---

## 14. Known-Divergent / Non-Functional

### Height data layout

- Height data is `(size_x + 1) * (size_y + 1)` (per-vertex grid, not per-tile).
- The vertex grid avoids an off-by-one interpolation boundary condition and
  naturally supports per-vertex heights from `build_mesh`.

### Camera matrix order

`T(-fix) * S(scale)` (translate first, then scale).  The fix-point formula
compensates for the translation so the visible area stays centred.

### IsoSprite ghost alpha not configurable

The ghost pass uses a hardcoded `ghostAlpha = 0.4`.  Adjust
`draw_iso_sprite_ghost` in `classic-gfx` if needed.

### Per-pixel depth maps (per-sheet)

A texture may declare a grayscale depth map (`depth` in the manifest, keyed per
**sheet**).  When present, the sprite writes `gl_FragDepth` from the map in
`sheet.frag`, so overlapping sprites occlude each other per-pixel rather than by
draw order, and the ghost pass becomes `GREATER` against the depth buffer.
Sprites sharing a non-zero `ghost_group` (a vehicle's body + wheels) never ghost
through each other (stencil `NOTEQUAL`).  Depth maps are emitted by the Blender
exporter (`classic-assets` `render/materials.py`), baked as camera view depth
`gray = (DEPTH_NEAR − dot(back, world)) / (DEPTH_NEAR − DEPTH_FAR)`.

The sheet bakes `gray` **relative to the Blender origin** (`gray ≈ 0.5` at the
asset's ground anchor), so `sheet.frag` re-anchors per sprite:
`gl_FragDepth = depth_base + (gray − 0.5)`, where `depth_base` is the sprite's
own window-space anchor depth (`world_depth(model.w_axis.truncate())`, the
*render* height) passed as a `sheet.frag` uniform.  **TODO** (noted in
`sheet.frag` + `render/materials.py`): bake the sheet *relative to the ground
anchor* so `gray = 0.5` is exactly the anchor and the `− 0.5` stops carrying an
implicit Blender-origin assumption.

### Per-pixel normal maps (per-sheet)

A texture may declare a normal map (`normal` in the manifest, keyed per sheet
like `depth`).  When present, `sheet.frag` samples the RGB normal
(same `sheetUv` as colour/depth), decodes `n = rgb * 2 - 1`, and shades with
the tilemap's Lambertian term:

```glsl
float diff = max(dot(n, light_direction), 0.0);
color.rgb *= ambient_color + diff * light_color;
```

The normal is baked **world-space** (Blender `Geometry.Normal` — or the
material's wired tangent normal map when present — remapped `[-1,1] → [0,1]`),
so no `normal_matrix` is applied in the sprite shader — the light dir must be
in the same world space as the tilemap's `vNormal`.  `use_normal_map = 0` (no
normal map) is byte-identical to the baked-lit path.  Normal-mapped sprites
therefore react to the `light_*` presets at runtime, which is why those
sprites are baked **unlit** (`--lighting unlit`): the normal map + light preset
supply all shading.

An **unlit sentinel** marks emissive sprite regions (e.g. the rocket's
flame cones): a `(0.5,0.5,0.5)` texel decodes to `(0,0,0)` and `sheet.frag`
skips the Lambertian term when `dot(n,n) < 0.001`, so those pixels stay flat
albedo instead of being shaded.

### SDF shadow/glow not rendered

The `SdfTextRender` component stores `outline_color` and `outline_width`
fields, but secondary draw passes for shadow and glow are not implemented.
Only the main text pass is drawn.

### No bitmap text

All text uses SDF rendering via `SdfTextRender`; there is no bitmap/glyph-map
text renderer.

### IsoAgent does not write depth independently

Unlike `IsoSprite` draw calls, `IsoAgent` instances are rendered as `DrawKind::IsoSprite`
entries (because `IsoAgent` subsumes `IsoSprite` in the component registry).
They go through the same `draw_iso_sprite` path with the same two-pass ghost
and depth-corner interpolation.  There is no agent-specific depth fallback.

### Wall faces only at map borders

Wall geometry is only generated for tiles on the map perimeter (east/south/west/north
edges).  Interior cliffs (height changes between adjacent tiles) do **not**
generate wall faces.  The walls that exist use a twisted-quad geometry (6 vertices,
two triangles) with a shared midpoint hedge.

### Collider quadtree

Disabled colliders are skipped in `begin_frame`, so they are not inserted into
the quadtree.  This avoids clicks landing on invisible/disabled elements.

### Selection rectangle rendering

The selection mode paint path uses `selectionMode` and `selectionColor` uniforms
in the tilemap fragment shader.  Modes are:
- `-1` = no selection.
- `0` = invert colour (placeholder for fill).
- `1` = desaturate-to-selection-colour (place-paint preview).

### `order()` formula

`position.x - position.y` (no size subtraction).  The tilemap is pinned at
`20000.0` z-order, so no global bias is needed.
