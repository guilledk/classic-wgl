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

## 2. Cartesian↔Iso Transforms

Defined in `classic-core::math`.

### `cartesian_to_iso_4()`

```rust
Mat4::from_rotation_z(std::f32::consts::FRAC_PI_4) * Mat4::from_scale(Vec3::new(1.0, 2.0, 1.0))
```

Rotates the coordinate frame by 45° (π/4) around Z, then scales Y by 2×.
This is the canonical 2:1 isometric projection matrix.  Applying it to a
cartesian position yields iso-space coordinates.

Companion `cartesian_to_iso_3()` produces a `Mat3` variant (used for normal
matrix computation).

### `iso_to_cartesian_4()`

The inverse of `cartesian_to_iso_4()`.  Converts from iso space back to
cartesian world space.  This is used throughout the engine to compute world
positions from tile coordinates:

- `compute_iso_sprite_model` converts an IsoSprite's `(tx, ty)` position to
  cartesian world space via `iso_to_cartesian_4() * Mat4::from_scale(tilemap_tf.scale)`.
- Footprint collider construction transforms iso-space footprint vertices to
  world space for the quadtree.
- Debug footprint rendering does the reverse transform.

### Tilemap-scale adjustment

The iso transform is always composed with the tilemap entity's `Transform.scale`:

```rust
let iso_to_cart_world = iso_to_cartesian_4() * Mat4::from_scale(tilemap_tf.scale);
```

This ensures that tile coordinates scale correctly with the tilemap's visual
size.  The inverse is composed similarly for the mouse-to-iso pipeline:

```rust
let cart_to_iso = cartesian_to_iso_4() * Mat4::from_scale(inv_scale);
// where inv_scale = Vec3::new(1.0 / scale.x, 1.0 / scale.y, 1.0)
```

---

## 3. Depth Formula

Depth in the classic-wgl iso renderer is **not** the standard GL depth-buffer
value.  Instead, `gl_Position.z` is overridden in both the tilemap vertex
shader and the IsoSprite image-sheet vertex shader with a synthetic iso-depth
value clamped to `[0, 1]`.

### Tilemap vertex shader (`iso_tilemap.vert`)

```glsl
vec4 worldPos = modelMatrix * isoMatrix * vec4(vertexPos, 1.0);
worldPos.y -= vertexPos.z;

float isoDepth = clamp(
    (vertexPos.x - vertexPos.y) / 400.0 + 0.5 - vertexPos.z / 14500.0,
    0.0,
    1.0
);
clipPos.z = isoDepth;
```

Key aspects:

1. **Height offset on Y:** `worldPos.y -= vertexPos.z` — height (Z) is subtracted
   from the screen-space Y coordinate.  This produces the correct vertical
   displacement for elevated tiles.
2. **Depth axis is `tx - ty`:**  The map's depth direction runs along the
   `(1, -1)` diagonal in iso space.  Tiles with larger `tx - ty` are farther
   from the camera.
3. **Constants:**  `400.0` controls the depth granularity per iso unit;
   `14500.0` controls the depth compression per height unit.  `+0.5` centres
   the range.
4. **Clamp to `[0, 1]`:**  The result is written to `clipPos.z`, overriding
   the standard orthographic projection Z.

### CPU-side equivalent (`compute_iso_depth_corners`)

The IsoSprite renderer needs depth values for each of the 4 footprint corners
(see Section 8).  The CPU-side formula mirrors the shader:

```rust
let d = (pos.x + pt.x - pos.y - pt.y) / 400.0 + 0.5 - pos.z / 14500.0 - 0.005;
```

The `-0.005` bias ensures sprites sit slightly behind the terrain at their
feet, avoiding z-fighting.

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
The index is `ty * (size_x + 1) + tx`.  This is a deliberate divergence from the
TypeScript original (which uses a tile grid of `size_x × size_y`).  The vertex
grid avoids off-by-one interpolation edge cases at the right/bottom map boundary.

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

### Uses

- **Mouse-to-iso solve:**  3-pass iterative height-parallax.
- **IsoSprite model matrix:**  position in iso → world cartesian, then
  `cart_pos.y -= height * height_scale` to lift/drop the sprite.
- **Agent terrain sampling:**  during `FollowPath` lerp, the agent samples
  terrain height at its current `(px, py)` and smooth-z-interpolates.
- **Footprint collider construction:**  each footprint vertex is
  height-adjusted in world space.
- **Debug footprint rendering:**  same vertex → world-space transform.

---

## 5. Mouse-to-Iso (3-Pass Height Parallax Solve)

Registered in `commit_terrain` as an `on_update` closure on `Engine`.  Runs every
frame to convert screen-space mouse position into iso tile coordinates,
accounting for height parallax.

### Algorithm

1. **Screen → camera → iso:**
   ```
   iso_pos = mouse_pos + camera.fix()
   iso_pos /= camera.scale
   iso_pos = cart_to_iso.transform_point3(iso_pos)
   ```
   Where `cart_to_iso = cartesian_to_iso_4() * Mat4::from_scale(inv_scale)`.

2. **Compute `tile_step`:**
   ```
   tile_step = scale.x * FRAC_1_SQRT_2   // ≈ scale.x * 0.7071
   ```

3. **3-pass iteration:**
   ```
   for _ in 0..3:
       h = bilinear_height(iso_pos.x, iso_pos.y)
       if h <= 0: break
       z_offset = (h * height_scale) / tile_step
       iso_pos.x = orig.x - z_offset
       iso_pos.y = orig.y + z_offset
   ```
   Each pass refines the iso position by shifting it along the depth axis
   `(+1, -1)` proportional to the terrain height at the current estimate.
   The `z_offset` moves the point closer to the camera (screen-left/down in
   iso terms) to compensate for parallax.

4. **Result written to `Tilemap.mouse_iso_pos`.**

This drives the tile selection cursor and editor paint rectangle.

---

## 6. Tilemap Mesh Generation

Defined in `classic-core::tilemap::build_mesh`.

### Vertex layout

Each vertex is **9 floats = 36 bytes**, interleaved:

| Offset | Size | Attribute | Description |
|--------|------|-----------|-------------|
| 0 | 3×f32 | `vertexPos` | Position in tile-grid space `(x, y, z)` |
| 12 | 2×f32 | `mapCoord` | Normalized map UV `[0..1, 0..1]` |
| 20 | 1×f32 | `tileId` | Tile index (≤0 = steep face, >0 = wall) |
| 24 | 3×f32 | `normal` | Smooth per-vertex normal |

Drawn as non-indexed `TRIANGLES`.

### Top face

Each non-empty tile generates two triangles (NW→NE→SW, NE→SE→SW) forming a
single quad.  Heights at the four corners (`z_nw`, `z_ne`, `z_sw`, `z_se`)
are computed from the height data grid multiplied by `height_scale`.

The `tileId` for the top face is `-steepness` where:

```rust
let steepness = ((z_max - z_min) / height_scale.max(0.001)).min(1.0);
```

This negative value routes the fragment shader to tile-data texture lookup
(`vTileId > 0.5` check passes only for positive tile IDs — walls).

### Wall faces

Generated only at map borders, one wall per exterior tile edge:

- **East wall** (`tx+1 >= size_x`): normal `[1, 0, 0]`
- **South wall** (`ty+1 >= size_y`): normal `[0, 1, 0]`
- **West wall** (`tx == 0`): normal `[-1, 0, 0]`
- **North wall** (`ty == 0`): normal `[0, -1, 0]`

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

The normals are passed through a `normalMatrix` uniform (transpose of the
inverse of the iso matrix) in the vertex shader for correct lighting.

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
    let iso_to_cart_world = iso_to_cartesian_4() * Mat4::from_scale(tilemap_tf.scale);
    let mut cart_pos = iso_to_cart_world.transform_point3(sprite_tf.position);
    cart_pos += tilemap_tf.position;

    let h = bilinear_height(...);
    cart_pos.y -= h * tilemap.height_scale;

    let anchor_delta = Vec3::new(
        -tex_dim.0 * iso_sprite.anchor.x,
        -tex_dim.1 * iso_sprite.anchor.y,
        0.0,
    );

    Mat4::from_translation(cart_pos)
        * Mat4::from_scale(sprite_tf.scale)
        * Mat4::from_translation(anchor_delta)
        * Mat4::from_scale(Vec3::new(tex_dim.0, tex_dim.1, 1.0))
}
```

Key pieces:
- Iso position → cartesian world via `iso_to_cartesian_4() * tilemap_scale`.
- Height offset: `cart_pos.y -= h * height_scale`.
- `anchor_delta` shifts the sprite origin from top-left (default GL quad) to the
  sprite's anchor point (e.g. `[0.5, 0.98]` anchors at bottom-centre).
- `tex_dim` is the texture size divided by tile-set dimensions, giving the
  aspect-ratio scale needed to make each tile square in the sprite sheet.

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
gl_Position.z = clamp(cornerDepth, 0.0, 1.0);
```

### Ghost pass (`draw_iso_sprite`)

IsoSprites are drawn with two draw calls:

1. **Ghost pass:** `ghostAlpha = 0.4`, `depthFunc(ALWAYS)`, `depthMask(false)`.
   Renders a translucent silhouette that is visible through occluding terrain.
   This lets the player see units behind walls.

2. **Normal pass:** `ghostAlpha = 0.0`, `depthFunc(LEQUAL)`, `depthMask(false)`.
   Renders the full-colour sprite on top of terrain, respecting depth occlusion.

Both passes leave `depthMask(false)` to avoid writing to the depth buffer and
interfering with subsequent draw calls.  `DEPTH_TEST` is enabled only for the
scope of these two passes, then disabled before returning.

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
- `Engine::update_vehicles` (`classic-engine/src/vehicle.rs`) computes the body
  pitch/roll as angles and quantizes them against `pitch_max`/`roll_max` to pick
  the frame; `frame_offset.y` still carries the suspension/jump delta.

---

## 9. IsoAgent

`IsoAgent` is a pathfinding-capable `IsoSprite` subtype (its registry spawner
creates both an `IsoAgent` and an `IsoSprite`).  It adds `speed`, `anim_speed`,
and `anim_prefix` over `IsoSprite`.

The agent's *behaviour* — click-to-move, path-following, idle/walk animation,
terrain-z following — lives in the **ROM guest** (`guest/demo-guest`), not in
Rust.  The retired `init_agent_system` (and the engine's click-to-move wiring)
were replaced by the guest driving the entity through the `classic-guest` SDK:
`find_path` (binary waypoints), `set_pos`/`get_pos` (3D), `set_anim`,
`height_at`, `mouse_iso`, `agent_selected`, `ui_consumed_click`.  The
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
from the tilemap's **actual** `height_data` + `height_scale` (the flat
`height_scale = 64.0` fallback is only used when there is no tilemap).  The
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

For each IsoSprite:

1. For each footprint vertex `pt`:
   - Compute iso position: `px = sprite_iso_pos.x + pt.x`, `py = sprite_iso_pos.y + pt.y`.
   - Bilinear height at `(px, py)`.
   - Convert to world space:
     ```
     v = iso_to_cart_world.transform_point3(Vec3::new(px, py, 0.0))
     v += tilemap_pos
     v.y -= h * height_scale
     ```

2. Build a `Shape::Polygon` via `polygon_from_verts(world_verts)`, which
   auto-computes `center`, `min`, and `max` AABB.

3. Register with `PhysicsProvider` and set the sprite's Z-offset to the
   terrain height: `tf.position.z = bilinear_height(px, py) * height_scale`.

Agents are explicitly skipped — they receive their own collider handling.

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

The tilemap shader receives a `normalMatrix` uniform computed as:

```rust
let iso3 = Mat3::from_mat4(iso_matrix);
let normal_matrix = iso3.inverse().transpose();
```

This corrects normals for the non-uniform 2:1 scale of the isometric transform.

---

## 14. Known-Divergent / Non-Functional

### Height data format divergence

- **TS:**  height data is `sizeX * sizeY` (per-tile grid).
- **Rust:**  height data is `(size_x + 1) * (size_y + 1)` (per-vertex grid).
- **Impact:**  The vertex grid avoids an off-by-one interpolation boundary
  condition.  For flat height data (all 1.0) both produce identical results.
  The vertex grid naturally supports per-vertex heights from `build_mesh`.

### Camera matrix order

- **TS:** `S(scale) * T(-fix)` (scale first).
- **Rust:** `T(-fix) * S(scale)` (translate first).
- **Impact:**  Both produce visually plausible results because the fix formula
  compensates.  At very high zoom levels the difference becomes more apparent
  (see `classic-rust-camera` skill for details).

### IsoSprite ghost alpha not configurable

The ghost pass uses a hardcoded `ghostAlpha = 0.4`.  The TS engine exposed
this as a per-sprite parameter.  In Rust it is fixed.  Adjust `draw_iso_sprite`
in `classic-gfx` if needed.

### SDF shadow/glow not rendered

The `SdfTextRender` component stores `outline_color` and `outline_width`
fields, but secondary draw passes for shadow and glow (as in TS) are not
implemented.  Only the main text pass is drawn.

### TS bitmap text not ported

The TS engine's `UIText` (traditional glyph-map bitmap text used for dev UI)
was not ported.  All text in the Rust port uses SDF rendering via `SdfTextRender`.

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

### Collider quadtree divergence

- **TS:**  All colliders (including disabled ones) are inserted into the quadtree.
- **Rust:**  Disabled colliders are skipped in `begin_frame`.  This avoids clicks
  landing on invisible/disabled elements but differs from TS behaviour.

### Selection rectangle rendering

The selection mode paint path uses `selectionMode` and `selectionColor` uniforms
in the tilemap fragment shader.  Modes are:
- `-1` = no selection.
- `0` = invert colour (placeholder for fill).
- `1` = desaturate-to-selection-colour (place-paint preview).

### `order()` formula divergence

- **TS:** `position.x - position.y - tilemap.sizeX - tilemap.sizeY`.
- **Rust:** `position.x - position.y` (no size subtraction).
- **Impact:**  The subtraction term in TS acts as a global bias to lower all
  sprite orders relative to the tilemap.  In Rust, the tilemap is pinned at
  `20000.0` z-order, making the bias unnecessary.
