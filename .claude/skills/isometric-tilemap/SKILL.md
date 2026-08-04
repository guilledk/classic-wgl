---
name: isometric-tilemap
description: >
    Expertise for the isometric tilemap rendering system in classic-wgl.
    Use when modifying or debugging tilemap rendering, shaders, mesh generation,
    height/slope/wall geometry, tile editing tools, or the isometric coordinate
    transform chain. Covers the GPU-driven sprite-sheet lookup, per-tile 3D
    mesh generation, wall face generation, and the Tilemap/IsoSprite/IsoAgent
    class hierarchy. Trigger phrases: "tilemap", "isometric", "slope", "wall",
    "height tile", "buildMesh", "iso_tilemap", "depth formula", "isoDepth",
    "Tilemap component", "IsometricNavMesh", "isometric rendering",
    "isometric shader", "sprite occlusion", "terrain rendering".
compatibility: Requires WebGL context with depth buffer. Relies on gl-matrix,
    custom Vite shader plugin, and the ECS call-registry pattern.
metadata:
    author: classic-wgl
    version: '0.5'
allowed-tools: Read, Grep, Glob, Bash(git *), Edit, Write
---

## Scope: isometric tilemap rendering only

This skill covers the isometric tilemap subsystem: rendering pipeline, mesh generation,
shader programs, coordinate transforms, height/wall geometry, depth-sorting,
editor tools, and the component class hierarchy. It does NOT cover the generic ECS,
physics, UI layout system, camera, or animation subsystems except where they directly
intersect the tilemap (e.g. collider for tile selection, UIManager for palettes).

---

## 1. MAP ORIENTATION — MUST READ FIRST

The most common source of bugs in this system is getting the isometric depth
direction backwards. **Memorise this:**

### Camera and cardinal directions

The isometric projection places the camera looking from the **southwest**.
On-screen:

| Screen position | Iso-grid direction | Depth        | Example tile |
| --------------- | ------------------ | ------------ | ------------ |
| Bottom-left     | **SW**             | **Closest**  | `(0, 199)`   |
| Bottom-right    | SE                 | Close-mid    | `(199, 199)` |
| Top-left        | NW                 | Mid-far      | `(0, 0)`     |
| Top-right       | **NE**             | **Farthest** | `(199, 0)`   |

### Iso ↔ cardinal / screen mapping

The iso-grid directions map to cardinal compass points. Use the debug overlay
("Footprints" toggle in DEV menu) to verify:

| Iso direction | Cardinal | Screen | `(dtx, dty)` |
|:---|:---|:---|:---|
| NE | **N** | straight UP | `(+1, −1)` |
| SE | **E** | straight RIGHT | `(+1, +1)` |
| SW | **S** | straight DOWN | `(−1, +1)` |
| NW | **W** | straight LEFT | `(−1, −1)` |

Intercardinals map to 2:1 diagonal screen vectors:

| Compass | Iso | Screen vector |
|:---|:---|:---|
| NE | `(+1, 0)` | right-up 2:1 |
| SE | `(0, +1)` | right-down 2:1 |
| SW | `(−1, 0)` | left-down 2:1 |
| NW | `(0, −1)` | left-up 2:1 |

### The depth axis is `tx − ty`, NOT `tx + ty`

- `tx − ty` is negative at SW (closest) and positive at NE (farthest) — correctly
  distinguishes the two corners.
- `tx + ty` is **identical** for SW `(0,199)` and NE `(199,0)` — it cannot tell
  them apart. Never use `x+y` for depth ordering.

### World-space mapping

The iso-to-cartesian transform (`isoToCartesian4 * scale(45)`):

```
worldX = 31.82 × (tx + ty)
worldY = 15.91 × (ty − tx)
```

- `worldX` goes right on screen; grows with both `tx` and `ty`
- `worldY` goes down on screen; grows with `ty`, shrinks with `tx`

### Vertex shader Y-offset

```glsl
worldPos.y -= vertexPos.z;
```

This pushes elevated vertices **up** on screen (since `ortho(top=0, bottom=vh)`
flips Y — subtracting from `worldY` means the vertex moves toward screen-top).

---

## 2. ARCHITECTURE OVERVIEW

### Data flow

```
data[] + heightData[]
  └─ uploadToGPU() → mapDataTexture (GL_TEXTURE_2D, RGBA, NEAREST)
                        │
buildMesh() → Float32Array verts → GL_ARRAY_BUFFER (DYNAMIC_DRAW)
                        │
rawDraw() each frame:
  bind mesh VBO → set vertexAttribPointer(pos, mapCoord, tileId)
  bind mapDataTexture → TEXTURE0
  bind tileSet → TEXTURE1
  set uniforms (including isoDepth for sprites) → drawArrays / drawElements
```

### Key files

| File                           | Role                                                                           |
| ------------------------------ | ------------------------------------------------------------------------------ |
| `src/classic/isometric.ts`     | Tilemap, IsometricNavMesh, IsoSprite, IsoAgent classes; buildMesh(), rawDraw() |
| `src/shaders/iso_tilemap.vert` | Isometric transform + iso-coordinate depth formula                             |
| `src/shaders/iso_tilemap.frag` | Sprite-sheet lookup, wall colour, slope darkening                              |
| `src/shaders/direct_tex.vert`  | Sprite vertex shader — isoDepth uniform pass-through                           |
| `src/shaders/sheet.frag`       | Sprite fragment shader — alpha discard                                         |
| `src/classic/utils.ts`         | isoToCartesian4 / cartesianToIso4 matrices, Buffer class                       |
| `src/demo/prefabs.ts`          | Editor logic handlers (tile/nav/height fill-on-selection)                      |
| `src/demo/uiPrefabs.ts`        | Dev tool buttons, palettes, height widget                                      |
| `src/classic/state.ts`         | WebGL context (depth: true), draw loop, camera/projection                      |

---

## 3. TRANSFORM CHAIN

The iso_tilemap vertex shader:

```glsl
vec4 worldPos = modelMatrix * isoMatrix * vec4(vertexPos, 1.0);
worldPos.y -= vertexPos.z;
vec4 clipPos = projectionMatrix * cameraMatrix * worldPos;
// iso-coordinate depth formula (see §5)
float isoDepth = clamp((vertexPos.x - vertexPos.y) / 400.0 + 0.5 - vertexPos.z / 14500.0, 0.0, 1.0);
clipPos.z = isoDepth;
gl_Position = clipPos;
```

### What each matrix does

| Matrix             | Construction                                     | Purpose                                                                     |
| ------------------ | ------------------------------------------------ | --------------------------------------------------------------------------- |
| `projectionMatrix` | `mat4.ortho(0, vw, vh, 0, -10000, 10000)`        | Maps to screen pixels, top-left origin, Y flipped                           |
| `cameraMatrix`     | `translate(-getFix()) * scale(cameraScale)`      | 2D camera pan + zoom                                                        |
| `modelMatrix`      | `translate(tilemap.position)`                    | Places tilemap. **No mapSize scaling** — vertices are already at map scale. |
| `isoMatrix`        | `invert(cartesianToIso4) * scale(tilemap.scale)` | Map-space → isometric screen-space                                          |

`cartesianToIso4` = `rotateZ(π/4) * scale(1, 2, 1)` — rotates 45° and stretches Y by 2×.

### Vertex space

Vertices from `buildMesh()` are in **map space**: x ∈ [0, sizeX], y ∈ [0, sizeY], z = height × heightScale.

---

## 4. DEPTH SYSTEM (ISO-COORDINATE DEPTH)

This is the **cornerstone** of the rendering system. All isometric objects
(tilemap vertices and sprites) use the same depth formula. Depth is derived
purely from grid coordinates and elevation — no camera-dependent terms.

### Depth formula

```
isoDepth = clamp((tx − ty) / 400 + 0.5 − Z / 14500, 0, 1)
```

Where:

- `400 = sizeX + sizeY` — normalises the grid range [-199, 199] to [-0.5, 0.5]
- `14500` — calibrates elevation contribution so one height unit (Z=32) is
  equivalent to ~0.88 grid-tile steps of isometric distance
- `Z = height × heightScale` — world-space elevation
- `clamp(…, 0, 1)` — safety guard for extreme height values

### Per-tile step

| Term          | Depth change per unit           | What it represents                  |
| ------------- | ------------------------------- | ----------------------------------- |
| `(tx−ty)/400` | ±0.0025 per tile step           | Isometric distance along SW→NE axis |
| `Z/14500`     | +0.00221 per height unit (Z=32) | Elevation pushing object "closer"   |

One height unit ≈ 0.88 tile steps — matches the visual ratio: Z=32 displaces
32 pixels vertically, while one tile-step displaces ~35.6 pixels diagonally.

### Why this depth metric

- **Monotonic**: no two objects at genuinely different iso-grid positions
  produce identical depth (unlike Y-to-depth mapping).
- **Screen-Y-free**: doesn't depend on camera position or projection.
- **Correct SW→NE ordering**: `tx−ty` is negative at SW (closest) and positive
  at NE (farthest).
- **Consistent across tilemap and sprites**: same formula in vertex shader
  (tilemap) and on CPU (sprites via `isoDepth` uniform).

### Depth test setup

- `glDepthFunc(gl.LEQUAL)` — closer objects (smaller depth) pass.
- Depth buffer cleared each frame (`gl.clear(DEPTH_BUFFER_BIT)`).
- Tilemap draws with depth test → writes depth.
- Each IsoSprite draws with depth test → reads depth from tilemap, writes its own.
- Non-isometric UI/sprites draw **without** depth test (disabled by default in
  draw loop; enabled/disabled per-drawable in `rawDraw()`).

### Sprite depth (IsoSprite)

Per-vertex depth via `isoDepthCorners` (vec4 uniform), computed each frame in
`rawDraw()`. The SW-footprint and SE-footprint vertices both get the minimum
depth across all footprint corners (flat base edge). NE/NW vertices are capped
at the anchor depth (`baseDepth`) to prevent the top of the sprite from
exceeding a safe bound:

```typescript
const baseDepth = (pos[0]-pos[1])/400 + 0.5 - pos[2]/14500 - 0.005;
for (let i = 0; i < 4; i++) {
    const d = (pos[0] + fp[i][0] - pos[1] - fp[i][1])/400 + 0.5 - pos[2]/14500 - 0.005;
    raw[i] = Math.min(d, baseDepth);
}
const minFp = Math.min(raw[0], raw[1], raw[2], raw[3]);
cornerDepths[0] = minFp; // bottom-left  = min across all footprint verts
cornerDepths[1] = minFp; // bottom-right = same (flat base edge)
cornerDepths[2] = raw[3]; // top-left     = NW, capped at baseDepth
cornerDepths[3] = raw[0]; // top-right    = NE, capped at baseDepth
```

Footprint indices: `[NE, SE, SW, NW]`. Shader indices: `x=SW, y=SE, z=NW, w=NE`.
The `-0.005` bias (not `-0.001`) overcomes GPU-interpolated terrain vertex
depths that can dip below the exact-iso-position terrain depth by ~0.0026.

The `isoDepthCorners` uniform in `direct_tex.vert` uses branchless GLSL100
corner selection via `mix(vertexPos.x, vertexPos.y)`:

```glsl
float bottomDepth = mix(isoDepthCorners.x, isoDepthCorners.y, vertexPos.x);
float topDepth    = mix(isoDepthCorners.z, isoDepthCorners.w, vertexPos.x);
float cornerDepth = mix(topDepth, bottomDepth, vertexPos.y);
gl_Position.z     = clamp(cornerDepth, 0.0, 1.0);
```

### Sprite alpha discard

`sheet.frag` discards fragments with near-zero alpha **before** the depth test:

```glsl
vec4 color = getTilePixel(tileIdFlat, texCoord);
if (color.a < 0.01) discard;
gl_FragColor = color;
```

This is critical: without `discard`, transparent sprite fragments would pass
the depth test and write depth values, blocking terrain behind them. With
`discard`, transparent fragments are dropped — depth buffer is untouched,
terrain shows through.

### RenderList sort

The `order()` for IsometricDrawables is:

```typescript
order(): number {
    return this.position[0] - this.position[1]
         - this.tilemap.sizeX - this.tilemap.sizeY;
}
```

The `− sizeX − sizeY` offset guarantees ALL isometric sprites have
order < 0 and draw **after** the tilemap (order = 0), regardless of map
position. Without this, NE sprites (`tx > ty`, order > 0) would draw
before the tilemap and be overpainted by opaque terrain — breaking
ghost rendering and depth‑based occlusion.

Relative sprite-to-sprite ordering is handled by the renderList sort, NOT by
the depth buffer. The normal pass sets `depthMask(false)` — sprites do not
write depth, so they never occlude each other via the Z-buffer. The renderList
draw order (closer = smaller `tx−ty` = drawn later = on top) fully controls
sprite-sprite visibility.

Sprite-vs-terrain occlusion still uses the depth buffer: the terrain writes
depth first (order 0, drawn before all sprites), then sprites compare against
it via LEQUAL but don't write depth themselves.

---

## 5. SHADER PIPELINE

### iso_tilemap.vert — tilemap vertex shader

**Attributes:** `vertexPos` (vec3), `mapCoord` (vec2), `tileId` (float)

**Key logic:**

1. Apply isoMatrix + modelMatrix to get world-space position
2. `worldPos.y -= vertexPos.z` — visual Y-offset for elevation
3. Compute iso-coordinate depth formula (§4)
4. Override `clipPos.z = isoDepth`

**Varyings:** `vMapCoord`, `vTileId`

### iso_tilemap.frag — tilemap fragment shader

**Three rendering paths, gated by `vTileId`:**

| Condition         | Path            | Behaviour                                                      |
| ----------------- | --------------- | -------------------------------------------------------------- |
| `vTileId > 0.5`   | Wall face       | Flat `wallColor` ([0.3, 0.2, 0.15, 1.0])                       |
| `vTileId < -0.01` | Sloped top face | Sprite-sheet lookup, then `rgb *= 1.0 + vTileId * slopeDarken` |
| Otherwise         | Flat top face   | Normal sprite-sheet lookup + selection overlay                 |

`slopeDarken` uniform defaults to `0.4` — a 1-height-unit cliff darkens by
40%, a gentle 0.2-unit ramp by ~8%.

**Lighting uniforms:** `ambientColor` (vec3), `lightDirection` (vec3, world‑space
surface→light), `lightColor` (vec3). After colour sampling, the shader applies
Lambert diffuse lighting:

```glsl
float diff = max(dot(normalize(vNormal), lightDirection), 0.0);
color.rgb *= ambientColor + diff * lightColor;
```

This runs after wall/tile colour, slope darkening, and selection overlay — so
all three rendering paths are lit. The `vNormal` varying comes from the vertex
shader via the `normal` attribute and `normalMatrix` transform (see §16).

### direct_tex.vert — sprite vertex shader

**Uniforms:** `useIsoDepth`, `isoDepthCorners` (vec4)

When `useIsoDepth > 0.5`: sets per-vertex depth by selecting the corner depth
using branchless `mix` operations on `vertexPos.xy` (**GLSL 100 compatible**;
do NOT use `gl_VertexID` which requires GLSL 300 es). All four components carry
the same value (single uniform depth), logically equivalent to `float isoDepth`:

```glsl
float bottomDepth = mix(isoDepthCorners.x, isoDepthCorners.y, vertexPos.x);
float topDepth    = mix(isoDepthCorners.z, isoDepthCorners.w, vertexPos.x);
float cornerDepth = mix(topDepth, bottomDepth, vertexPos.y);
gl_Position.z     = clamp(cornerDepth, 0.0, 1.0);
```

Used by IsoSprites. Non-isometric Sprites set `useIsoDepth = 0`.

### sheet.frag — sprite fragment shader

Shared by IsoSprite and Sprite. Does sprite-sheet frame lookup + alpha discard.

---

## 6. MESH GENERATION — buildMesh()

### Vertex format (9 floats, interleaved)

```
[x, y, z, mx, my, tileId, nx, ny, nz]
```

| Offset | Count | Content                              | Attribute   |
| ------ | ----- | ------------------------------------ | ----------- |
| 0      | 3     | Map-space position (tx, ty, z)       | `vertexPos` |
| 12     | 2     | Normalised map coords (tx/sX, ty/sY) | `mapCoord`  |
| 20     | 1     | -steepness (top faces), ≥1 (walls)   | `tileId`    |
| 24     | 3     | Face normal in map space             | `normal`    |

### Top face (floor/slope)

For each non-empty tile, 4 corners form 2 triangles (6 vertices):

```
hNW = heightData[tx + ty * sX]            (this tile)
hNE = heightData[tx+1 + ty * sX]          (east neighbour)
hSW = heightData[tx + (ty+1) * sX]        (south neighbour)
hSE = heightData[tx+1 + (ty+1) * sX]      (SE neighbour)
```

This is a **continuous surface**: corners come from the tile's own height AND
its neighbours'. Setting `heightData[5+3*sX] = 3` affects FOUR tiles'
corners (NW of this, NE of west, SW of north, SE of NW).

**Slope detection:** `steepness = clamp((zMax - zMin) / hs, 0, 1)` →
`faceTileId = -steepness`. Flat tiles get `faceTileId = 0`.

### Wall faces

**Only at map boundaries.** Interior walls were removed because they
conflicted with the continuous surface — walls used per-tile stored heights
while top faces used neighbour-shared corners, creating visible gaps.

```typescript
if (tx + 1 >= sX && hThis > 0) {
    /* east boundary wall */
}
if (ty + 1 >= sY && hThis > 0) {
    /* south boundary wall */
}
if (tx === 0 && hThis > 0) {
    /* west boundary wall */
}
if (ty === 0 && hThis > 0) {
    /* north boundary wall */
}
```

Boundary walls extend from Z=0 to Z = hThis × hs.

### Performance optimizations

1. **Pre-sized Float32Array** — `new Float32Array(sizeX * sizeY * 180)` at
   worst-case, direct index writes via `vi++`, no `Array.push()` intermediate
2. **Buffer reuse** — `DYNAMIC_DRAW` + `bufferSubData` instead of
   `deleteBuffer`/`createBuffer` per rebuild
3. **Pre-computed normalised coords** — `mx[i]` / `my[i]` lookup tables
   computed once
4. **Dirty flag** — `_meshDirty` prevents rebuilds every frame; only set by
   `fillRegion()`, `generateNoiseMap()`, `uploadToGPU()`, `setHeight()`

---

## 7. DATA MODEL

### Core arrays

| Field                   | Type       | Size            | Description                                                                            |
| ----------------------- | ---------- | --------------- | -------------------------------------------------------------------------------------- |
| `data`                  | `number[]` | `sizeX * sizeY` | Tile sprite index (0 = empty)                                                          |
| `heightData`            | `number[]` | `sizeX * sizeY` | Height of each tile's NW corner. Defaults to `1`.                                      |
| `heightScale`           | `number`   | —               | World-space Z per height unit. Default = `tilePixelSize[0]`.                           |
| `heightScaleMultiplier` | `number`   | game state      | User-adjustable multiplier (default 1). `heightScale = tilePixelSize[0] * multiplier`. |

### Continuous surface model

HeightData stores one value per tile, but each tile's top-face corners read
**neighbour** entries. Setting a single height entry affects the corners of
four tiles. This creates natural slopes between tiles at different heights
— the shared corner interpolates smoothly.

### Slope darkening

`faceTileId = -steepness` where `steepness = min((zMax−zMin) / hs, 1)`.
Fragment shader multiplies `rgb *= 1.0 + vTileId * slopeDarken` for
proportional darkening (vTileId is negative, so this darkens).
`slopeDarken = 0.4` by default.

---

## 8. AGENT HEIGHT AND MOVEMENT

### IsoAgent.update() — smooth height tracking

After XY lerp, bilinearly interpolates terrain height at the agent's exact
float position:

```typescript
const px = this.position[0];
const py = this.position[1];
const tx = Math.floor(px);
const ty = Math.floor(py);
const fx = px - tx;
const fy = py - ty;
const hNW = heightData[tx + ty * sX];
const hNE = heightData[tx + 1 + ty * sX];
const hSW = heightData[tx + ty + 1 * sX];
const hSE = heightData[tx + 1 + ty + 1 * sX];
const hTop = hNW + (hNE - hNW) * fx;
const hBot = hSW + (hSE - hSW) * fx;
const hi = hTop + (hBot - hTop) * fy;
const targetZ = hi * heightScale;
position[2] += (targetZ - position[2]) * min(1, deltaTime * 4);
```

This matches the GPU's vertex interpolation exactly — the agent's depth Z
matches the terrain surface. Lerp rate of 4/s gives smooth uphill walking.

### Pathfinding tile centers

The A* pathfinder (`pathfinder.ts`) floors both `from` and `to` to integer
tile coordinates, returning paths of integer waypoints (tile corners rather
than centers). After receiving a path, offset every waypoint by `[0.5, 0.5]`
(except `path[0]` which `followPath` overwrites with the agent's current
position):

```typescript
pathfinder.findPath(start, end).then((p) => {
    if (p != null) {
        const path = p as [number, number][];
        for (let i = 1; i < path.length; i++) {
            path[i][0] += 0.5;
            path[i][1] += 0.5;
        }
        agent.followPath(path);
    }
});
```

Without this, the agent snaps to tile vertices (e.g. `[100, 50]`) instead of
tile centers (`[100.5, 50.5]`).

---

## 9. CLASS HIERARCHY

```
Drawable (transforms.ts)
  ├─ Tilemap (isometric.ts)              GPU-driven isometric tilemap
  │    └─ IsometricNavMesh               Pathfinding nav mesh (extends Tilemap)
  └─ IsometricDrawable (isometric.ts)    Base for iso-placed sprites
       └─ IsoSprite                      Sprite on isometric map
            └─ IsoAgent                  Animated pathfinding agent
```

### Key methods on Tilemap

| Method                                    | Purpose                                                              |
| ----------------------------------------- | -------------------------------------------------------------------- |
| `uploadToGPU()`                           | Uploads `data[]` to `mapDataTexture`, dirties mesh                   |
| `buildMesh()`                             | Generates 3D geometry from `data[]` + `heightData[]` into VBO        |
| `rawDraw()`                               | Binds mesh VBO + textures + uniforms, `drawArrays()` with depth test |
| `setHeight(x,y,v)`                        | Sets one height entry, dirties mesh                                  |
| `fillRegion(from,to,v)`                   | Fills tile data in rectangle, dirties mesh                           |
| `modelMatrix()`                           | `translate(position)` — no mapSize scaling                           |
| `isoToCartesian(v)` / `cartesianToIso(v)` | Coordinate transforms                                                |

### Key methods on IsometricDrawable

| Method          | Purpose                                                              |
| --------------- | -------------------------------------------------------------------- |
| `order()`       | `position[0] - position[1]` — matches depth axis for renderList sort |
| `modelMatrix()` | iso→cartesian, adds terrain Y-offset, no Z manipulation              |

### IsometricNavMesh caveats

- Extends Tilemap with `tilePixelSize: [8,8]`, `maxTile: 2`, `'navTileset'`
- Spawns a Web Worker (`pathfinder.ts`) for A* pathfinding
- `dump()` has its own serialization — does not inherit `tileSet`/`tilePixelSize`

---

## 10. EDITING TOOLS

### editorTarget routing

Four modes: `'tilemap'`, `'navMesh'`, `'height'`, plus `'none'` when panel closed.

The tilemap's single `Collider` has multiple `'selection'` handlers registered.
Each checks `game.editorTarget` and only executes if it matches:

```typescript
collider.addHandler('selection', function () {
    if (game.editorTarget !== 'tilemap') return;
    compTilemap.fillRegion(begin, end, game.editorTile ?? 0);
    compTilemap.uploadToGPU();
});
// ... same pattern for 'navMesh' and 'height'
```

### Height widget

Bottom-right of canvas. Two rows with manual pixel positioning:

- Row 1: `[−] [height value] [+]` — sets `game.editorHeight`
- Row 2: `[s−] [×N] [s+]` — sets `game.heightScaleMultiplier`, calls
  `tilemap.heightScale = tilePixelSize[0] * multiplier` and dirties mesh

### Agent selection

- Green `[A]` button at bottom-left toggles `game.agentSelected`
- Clicking within ~1.5 tiles of the agent on the map toggles selection
- P key toggles `game.agentEnabled`
- Map clicks only pathfind when agent is selected AND enabled

### Adding a new editor mode

1. Add state to `IGameState` via module augmentation in `uiPrefabs.ts` and `prefabs.ts`
2. Add a dev button in `initToolButtons()` (use next frame of `'editorIcons'`)
3. Create a UI widget patterned after `initHeightWidget()` or `initTilePalette()`
4. Add a selection handler in `prefabs.ts` guarding on `editorTarget`
5. Call the init function from `initContext()` in `init.ts`
6. Register enable/disable in `initEditorModeControl()`

---

## 11. PERFORMANCE NOTES

### Pre-sized Float32Array

`buildMesh()` writes directly into a pre-allocated `Float32Array` at worst-case
capacity. Manual index `vi` tracks write position — no `Array.push()`, no
intermediate JS `Number[]`, no GC pressure.

### Buffer reuse

First build allocates VBO at max capacity with `DYNAMIC_DRAW`. Subsequent
rebuilds use `bufferSubData` to update in-place. The `_needsBufferResize`
flag is only true at construction (map size never changes).

### Dirty flag

Mesh is only rebuilt when data/height changes. During normal rendering
(camera pan, agent movement), `rawDraw()` just rebinds the existing VBO.

### Pre-computed normalized coords

`mx[i]` and `my[i]` arrays computed once in `buildMesh()`, eliminating
repeated division in the inner loop (~200K divisions saved per 200×200 map).

### order() computation

Simplified from `isoDistanceToCam(pos)` (which required camera-world-to-iso
conversion + Euclidean distance) to `position[0] - position[1]` — a single
subtraction.

---

## 12. COMMON PITFALLS

### Never use x+y for depth ordering

`x+y` is identical for SW `(0,199)` and NE `(199,0)` — it cannot distinguish
the closest and farthest corners, producing backwards occlusion on one diagonal.
Always use `tx − ty`.

### modelMatrix must NOT scale by mapSize

Vertices from `buildMesh()` are already in map space (x ∈ [0, sizeX]).
Adding `scale([mapSize, 1])` would scale them again.

### vTileId gate values

- `> 0.5` → wall colour path (wall vertex tileId = `data[idx] || 1`)
- `< -0.01` → sloped face darkening path (faceTileId = `-steepness`)
- Between -0.01 and 0.5 → normal sprite-sheet lookup (flat top faces, tileId=0)

### Sprite depth must match tilemap depth

The `isoDepth` formula on CPU must use the SAME denominators as the vertex
shader (400 and 14500). Any mismatch causes sprites to render at the wrong
depth relative to terrain.

### Transparent sprite fragments need discard

Without `discard` in `sheet.frag`, transparent sprite pixels pass the depth
test and write depth — blocking terrain behind them. Always include:

```glsl
if (color.a < 0.01) discard;
```

### Continuous surface vs walls

The continuous surface model (corners from neighbours) is incompatible with
interior wall generation. Walls used per-tile stored heights while top faces
used shared corner heights — causing gaps. Interior walls were removed; only
map-boundary walls remain.

### Depth test scope

Depth test is enabled per-drawable, not globally. Tilemap and IsoSprite
each enable/disable depth test in their `rawDraw()` methods. Non-isometric
drawables (UI, cursor) never enable it. The global draw loop does NOT
enable depth test.

### `getComponent` uses strict constructor equality

`entity.getComponent(IsoSprite)` checks `component.constructor === IsoSprite`.
This will **not** find `IsoAgent` instances (which extend `IsoSprite`).
When querying for a base class, iterate `entity.components` with `instanceof`:

```typescript
let iso: IsoSprite | null = null;
for (const c of entity.components) {
    if (c instanceof IsoSprite) { iso = c as IsoSprite; break; }
}
```

### Static sprites need position[2] initialized

Static IsoSprites (tree, house, semaphores) loaded from `state.json` default to
`position[2] = -1`. On height>0 terrain this causes the sprite to render
behind the terrain surface (clipping at the base). Initialize `position[2]`
to the bilinear terrain height in `initFootprintColliders()`:

```typescript
const h = /* bilinear at (sprite.position[0], sprite.position[1]) */;
sprite.position[2] = h * tilemap.heightScale;
```

### GLSL100: no gl_VertexID

`gl_VertexID` requires GLSL 300 es / WebGL2-only features. The existing shaders
use GLSL 100 (`attribute`, `varying`). For per-vertex data in GLSL 100, use
`mix` with `vertexPos.xy` to select corners branchlessly (see §5
`direct_tex.vert`).

### Debug footprint state must be restored

When debug-footprint rendering changes `depthFunc(ALWAYS)` or `depthMask(false)`,
restore `depthFunc(LEQUAL)` and `depthMask(true)` after the debug loop.
Unrestored state leaks into the next frame's terrain rendering, causing
"dark trails" on elevated terrain.

### GPU pipeline flush required after Text init

Creating `UIText` entities (which allocate internal framebuffers and call
`setText` → `appendText` at construction time) can leave the GPU pipeline in
a state that produces terrain flickering. The fix is a minimal draw-call flush
at frame end:

```typescript
gl.drawArrays(gl.LINE_STRIP, 0, 0); // 0-vertex draw = pipeline flush
gl.depthFunc(gl.LEQUAL);
gl.depthMask(true);
```

### GL state isolation at frame start

The draw loop must reset depth state EVERY frame at frame start — never assume
the previous frame's last drawable left clean state:

```typescript
gl.depthFunc(gl.LEQUAL);
gl.depthMask(true);
```

Sprites set `depthMask(false)` on their normal pass, and if the terrain draws
before any sprite restores it, terrain won't write depth.

### Text.setText must restore FBO + viewport

`Text.setText` binds an internal framebuffer for render-to-texture, changes
the viewport to the texture size, and renders glyphs. It MUST restore the
default framebuffer and viewport after completion:

```typescript
gl.bindFramebuffer(gl.FRAMEBUFFER, null);
gl.viewport(0, 0, gl.drawingBufferWidth, gl.drawingBufferHeight);
```

Without this, subsequent `gl.clear` and draw calls operate on the text's
internal FBO (a tiny 32×32px texture) instead of the main canvas.

---

## 13. FAILED APPROACHES (DO NOT REPEAT)

| Approach                                | Why it failed                                                                                             |
| --------------------------------------- | --------------------------------------------------------------------------------------------------------- |
| `x+y` for depth axis                    | Identical values for SW and NE — can't distinguish closest from farthest                                  |
| Y-to-depth mapping                      | Same-screen-Y objects get identical depth regardless of iso position — sprite always wins against terrain |
| Polygon offset for wall-ground z-fight  | Shifted ALL terrain toward camera, breaking sprite-terrain occlusion                                      |
| Z tiebreaker (`-z/10000`) in Y-to-depth | Made terrain always closer than sprites at same Y — backwards occlusion                                   |
| `isoZBias` from isoDistanceToCam        | CPU distance metric didn't match GPU depth formula — sprite always appeared closer                        |
| `worldPos.x` term in depth formula      | Absolute world-X overpowered the iso-coordinate distance — sprite always closer                           |
| Interior wall generation                | Created gaps between surfaces: walls used per-tile heights, faces used shared corners                     |
| Per-vertex isoDepth without anchor cap  | Top vertices of tall sprites get large (= far-away) depth values. Terrain at sprite's base clips the sprite. All iso sprites appear in ghost mode. Cap top vertices at anchor depth via `Math.min()`                               |
| SW-corner single uniform depth          | Large sprites get depth shifted far toward camera (Δ0.03 for 12×12 house footprint). Makes them render in front of smaller sprites that are genuinely closer. Use per-vertex capped depth instead                                                                         |
| Texture-pixel→iso coordinate footprint  | Hundreds of texture pixels convert to massive iso offsets (12+ tiles), producing enormous footprints that span the full sprite silhouette. Use explicit tile-based footprints in `state.json`                                                          |
| Depth buffer for sprite-sprite ordering | Sprites writing depth block each other. A large sprite's per-vertex depth gradient can extend far toward the camera, blocking smaller sprites that should render in front. Use `depthMask(false)` + renderList sort instead                                                     |

---

## 14. ADDING FEATURES

### Wall texturing (future)

`tileId` ≥ 1 already identifies wall vertices. Replace `wallColor` fragment
shader path with a sprite-sheet lookup. May need a dedicated wall-tile UV
attribute.

### New geometry types (ramps, overhangs)

Add data arrays to Tilemap, generate custom vertex positions in `buildMesh()`,
update fragment shader if different texturing is needed.

### Dynamic map size

Replace hardcoded `400` denominator with `sizeX + sizeY` uniform. Replace
`14500` with a derived uniform. Currently safe for 200×200 maps.

---

## 15. GHOST RENDERING (DUAL-PASS)

Every `IsoSprite.rawDraw()` renders in two passes within a single draw call
sequence. This makes all isometric sprites visible through occluding terrain
without modifying the terrain shader.

### How it works

**Pass 1 — Ghost (silhouette through terrain):**

```
depthFunc → ALWAYS         // render regardless of depth
depthMask → false           // don't write depth
ghostAlpha → 0.4            // 40 % alpha silhouette
drawElements(...)
```

The ghost always renders at 40 % opacity on top of whatever is already in
the framebuffer. Since all sprites draw after the tilemap (§4 RenderList
sort), the ghost blends over terrain, walls, and other terrain geometry.

**Pass 2 — Normal (full sprite where visible):**

```
depthFunc → LEQUAL         // only render where sprite is in front of terrain
depthMask → true            // write depth
ghostAlpha → 0.0            // full opacity
drawElements(...)
```

The normal pass overdraws the ghost with the full sprite where the sprite
is isometrically in front of the terrain (depth test passes). Where the
terrain occludes the sprite, the normal pass is blocked — leaving the
ghost silhouette visible.

### Shader support

`sheet.frag` has a `ghostAlpha` uniform. When > 0, the output alpha is
overridden:

```glsl
uniform float ghostAlpha;
// ...
if (ghostAlpha > 0.0) {
    color.a = ghostAlpha;
}
```

Non‑isometric sprites (`Sprite.rawDraw()`) set `ghostAlpha = 0` — no ghost.

### Key requirement: draw order

The ghost only works if sprites draw **after** the tilemap. See §4
RenderList sort: `order()` subtracts `sizeX + sizeY` to guarantee all
isometric sprites have negative order.

### Why ALWAYS not GEQUAL

`GEQUAL` was tried first — it only renders the ghost where the sprite
depth ≥ terrain depth. This works for the SW quadrant (terrain closer to
camera) but fails for the NE quadrant (terrain farther). `ALWAYS` renders
the ghost through all terrain; the normal `LEQUAL` pass cleans up the
visible areas.

### Why this is performant

- One extra `drawElements` call per isometric sprite per frame
- Same VAO, same texture, same uniforms — only `ghostAlpha` and
  `depthFunc`/`depthMask` change
- No FBOs, no additional texture lookups, no terrain shader changes

---

## 16. LIGHTING SYSTEM

### Overview

The tilemap uses ambient + directional diffuse (Lambert) lighting.
Per‑face flat‑shaded normals are computed during mesh construction and
transformed through a `normalMatrix` uniform to account for the non‑uniform
isometric scale. Light parameters are driven by selectable presets with
adjustable azimuth/elevation.

### Key files

| File                           | Role                                                                                                          |
| ------------------------------ | ------------------------------------------------------------------------------------------------------------- |
| `src/classic/lighting.ts`      | Presets dict, `applyLightPreset()`, `updateLightDirection()`, `initLighting()`                                |
| `src/shaders/iso_tilemap.vert` | `normal` attribute, `normalMatrix` uniform, `vNormal` varying                                                 |
| `src/shaders/iso_tilemap.frag` | `ambientColor` / `lightDirection` / `lightColor` uniforms, diffuse calc                                       |
| `src/classic/isometric.ts`     | Per‑face normal computation in `buildMesh()`, `normalMatrix` in `setScale()`, uniform plumbing in `rawDraw()` |
| `public/manifest.json`         | `normal` attr + `normalMatrix`/`ambientColor`/`lightDirection`/`lightColor` unifs                             |
| `src/demo/uiPrefabs.ts`        | Light config widget (preset cycle + azimuth/elevation ± buttons)                                              |

### Normal computation in `buildMesh()`

Normals are computed **per triangle** and stored in map space (same coordinate
frame as vertex positions, pre‑isoMatrix). All three vertices of a triangle
share the same normal (flat shading).

**Top face, triangle 1 (NW → NE → SW):**

```
edge1 = (1, 0, zNE − zNW)
edge2 = (0, 1, zSW − zNW)
n = normalize(cross(edge1, edge2)) = normalize((zNW−zNE, zNW−zSW, 1))
```

**Top face, triangle 2 (NE → SE → SW):**

```
edge1 = (0, 1, zSE − zNE)
edge2 = (−1, 1, zSW − zNE)
n = normalize(cross(edge1, edge2)) = normalize((zSW−zSE, zNE−zSE, 1))
```

**Wall normals** are axis‑aligned unit vectors in map space:

| Wall boundary | Condition   | Map‑space normal |
| ------------- | ----------- | ---------------- |
| East          | `tx+1 ≥ sX` | `(−1, 0, 0)`     |
| South         | `ty+1 ≥ sY` | `(0, 1, 0)`      |
| West          | `tx = 0`    | `(1, 0, 0)`      |
| North         | `ty = 0`    | `(0, −1, 0)`     |

The `pushVert()` helper writes the same `(nx, ny, nz)` for all three
vertices of each triangle. `writeTriNormal()` computes the normal from
two edge vectors using the standard cross‑product formula.

### `normalMatrix` uniform

The `isoMatrix` applies a non‑uniform scale (from `isoToCartesian4`:
`scale(1, 0.5, 1)` in XY). Normals must be transformed by the
**inverse‑transpose** of the model‑view matrix's upper‑left 3×3:

```typescript
// In setScale():
const iso3 = mat3.create();
mat3.fromMat4(iso3, this._isoToCartesian);
const invIso3 = mat3.create();
mat3.invert(invIso3, iso3);
mat3.transpose(this._normalMatrix, invIso3);
```

In the vertex shader:

```glsl
vNormal = normalize(normalMatrix * normal);
```

The normal matrix is static — it only changes when the tilemap scale changes
(via `setScale()`), which dirties the mesh and triggers a rebuild.

### `rawDraw()` uniform plumbing

```typescript
// New attribute pointer (normal at byte offset 24, 3 floats):
this.gl.vertexAttribPointer(shader.attr.normal, 3, this.gl.FLOAT, false, stride, 24);
this.gl.enableVertexAttribArray(shader.attr.normal);

// New uniforms:
this.gl.uniformMatrix3fv(shader.unif.normalMatrix, false, this._normalMatrix);
this.gl.uniform3fv(shader.unif.ambientColor, game.lightAmbient ?? [0.2, 0.2, 0.25]);
this.gl.uniform3fv(shader.unif.lightDirection, game.lightDir ?? [0.45, -0.35, 0.82]);
this.gl.uniform3fv(shader.unif.lightColor, game.lightColor ?? [0.8, 0.75, 0.65]);
```

### Light presets

Defined in `src/classic/lighting.ts`:

```
LIGHT_PRESETS = {
    sunny:  ambient:[0.15,0.15,0.20] direction:norm([0.453,0.211,0.866]) color:[1.0,0.95,0.85]
    cloudy: ambient:[0.35,0.35,0.40] direction:norm([0.0,-0.2,1.0]) color:[0.70,0.72,0.78]
    dawn:   ambient:[0.20,0.15,0.25] direction:norm([0.5, 0.2,0.3]) color:[1.0,0.40,0.20]
    night:  ambient:[0.10,0.12,0.25] direction:norm([-0.2,-0.5,0.8]) color:[0.30,0.40,0.70]
}
```

`applyLightPreset(key)` copies a preset into `game.lightAmbient`,
`game.lightDir`, and `game.lightColor`, and derives `game.lightAzimuth`
and `game.lightElevation` from the direction vector.

### Direction ↔ azimuth/elevation

The light direction is stored as a world‑space unit vector (`surface → light`).
It's convertible to/from azimuth/elevation for UI display:

```
azimuth   = atan2(d[0], −d[1]) × 180/π   (0° = South of iso map)
elevation = asin(d[2]) × 180/π           (0° = horizon, 90° = zenith)

lightDir.x = cos(el) × sin(az)
lightDir.y = −cos(el) × cos(az)   (negated because world Y goes down)
lightDir.z = sin(el)
```

When the user tweaks azimuth or elevation via the light config widget,
`updateLightDirection()` recomputes the direction vector and stores it
in `game.lightDir`. The preset changes to `'custom'` to indicate deviation
from a named preset.

### Slope darkening removed

The manual slope‑darkening pass (`color.rgb *= 1.0 + vTileId * slopeDarken`)
was removed from the fragment shader. The lighting system now provides the
sole slope contrast: steeper faces have angled normals that reduce the
`dot(N, L)` term, naturally darkening them relative to flat terrain. No
separate darkening uniform or math is needed.

### Shader apply order

```
1. Sample tile colour (getTilePixel)
2. Apply selection overlay (if active)
3. Alpha discard (if near‑zero)
4. Apply lighting: color.rgb *= ambientColor + dot(N, L) * lightColor
```

Lighting runs after selection so the selection overlay (an editor tool)
retains its visual intensity regardless of light settings.

---

## 17. SPRITE FOOTPRINTS

### Overview

Each `IsoSprite` carries a `footprint: Vec2Like[]` — a list of iso-space
`[tx, ty]` offsets from `this.position` (the anchor). The footprint serves
three purposes: depth computation (`getDepth()`), collision (`attachCollider()`),
and debug visualization.

The footprint is specified in `state.json` as a `footprint` array on the
component config. If omitted, `IsoSprite.defaultFootprint()` provides a
0.5-tile diamond.

### Diamond format

A proper isometric diamond on screen uses diagonal iso offsets:

```json
"footprint": [
    [r, -r],   // NE — top-center on screen
    [r,  r],   // SE — right-center on screen
    [-r, r],   // SW — bottom-center on screen
    [-r,-r]    // NW — left-center on screen
]
```

This produces a 2:1 diamond on screen (width = 2× height).

Axis-aligned footprints like `[[0,-r],[r,0],[0,r],[-r,0]]` produce
**rectangles** on screen — avoid this.

### Default footprint

```typescript
static defaultFootprint(): Vec2Like[] {
    return [
        [0.5, -0.5],
        [0.5,  0.5],
        [-0.5, 0.5],
        [-0.5,-0.5],
    ];
}
```

### getDepth()

Returns the minimum `(tx−ty)/400` across all footprint vertices (the
SW‑most / closest-to-camera point), plus the standard Z-bias of `-0.005`.
Used by `rawDraw()` as one input. The final depth actually passed to the
shader is computed inline in `rawDraw()` using per-vertex footptint depths
capped at `baseDepth` (anchor depth). See §4 for the full computation.

```typescript
getDepth(): number {
    let minV = Infinity;
    for (const pt of this.footprint) {
        const d = (this.position[0] + pt[0] - this.position[1] - pt[1]) / 400.0;
        if (d < minV) minV = d;
    }
    return minV + 0.5 - this.position[2] / 14500.0 - 0.005;
}
```

The `-0.005` bias overcomes GPU-interpolated terrain vertex depths that can dip
~0.0026 below the exact-iso-position terrain depth at adjacent mesh vertices.

### attachCollider()

Creates a `Collider` with a world-space `Polygon` from the footprint. Each
footprint vertex is converted to world-space via `isoToCartesian`, and
bilinear terrain height compensation is applied per-vertex:

```typescript
const h = /* bilinear at (position[0] + pt[0], position[1] + pt[1]) */;
v[1] -= h * tilemap.heightScale;
```

This ensures colliders sit at the terrain surface, matching the visual
sprite's `modelMatrix()` Y-offset. Static sprites (tree, house, semaphores)
compute this once; the agent's footprint is drawn on-the-fly each frame in
the debug loop.

### Debug visualization

`state.ts` draws each sprite's footprint as a `LINE_LOOP` (cyan-green) and
an anchor X marker (magenta) when `game.debugFootprints` is true. The
footprint is computed on-the-fly each frame from `iso.position` + `iso.footprint`,
so it follows the agent's movement. Both the footprint and anchor X apply
bilinear terrain height Y-offset. Toggled via the "Footprints" item in the
DEV panel menu.

When debug is OFF, a minimal GPU pipeline flush runs at frame end to prevent
state corruption from the 10+ UIText elements that allocate internal
framebuffers at init time:

```typescript
gl.drawArrays(gl.LINE_STRIP, 0, 0);
gl.depthFunc(gl.LEQUAL);
gl.depthMask(true);
```

### Anchor placement

The anchor should be at the sprite's visual ground-contact point. For
sprites with transparent pixels below the visible content (tree, traffic
lights), the anchor row must be detected from the texture's non-alpha
pixels and the position compensated:

```
anchorY = groundY / textureHeight
delta_worldY = (oldAnchorRow - groundY) * scaleY
dty = delta_worldY / 31.82, dtx = -dty
```

Without this, the anchor X marker (and the footprint diamond center) appear
offset from the visible sprite base by 1–3 tiles.

### Footprint in state.json

Example for a 12×12 house diamond:

```json
{
    "type": "IsoSprite",
    "position": [32.90, 2.86, -1],
    "scale": [1.7, 1.7, 1],
    "anchor": [0.5, 0.768],
    "footprint": [
        [6, -6],
        [6,  6],
        [-6, 6],
        [-6,-6]
    ]
}
```

The `position[2]` should be initialized to the bilinear terrain height at
the anchor point to prevent base clipping on height>0 terrain (handled in
`initFootprintColliders()`).


---

## 18. MOUSE-HOVER GRID OVERLAY

### Overview

The fragment shader draws black grid lines at tile boundaries within a
configurable Chebyshev radius of the mouse cursor.  Toggled via G key;
suppressed during editor selection and never drawn on wall faces.
Enabled by default (`game.showGrid = true`).

### Uniforms

| Uniform      | Type   | Purpose                                   |
| ------------ | ------ | ----------------------------------------- |
| `showGrid`   | int    | Toggle (0 = off, 1 = on)                  |
| `gridRadius` | float  | Tile-radius around mouse (default 3.0)   |
| `gridColor`  | vec3   | Line colour (default black [0,0,0])      |

Declared in `iso_tilemap.frag`, added to `manifest.json` under
`isoTilemap.unif`, set each frame in `Tilemap.rawDraw()`.

### Fragment shader logic

1. Gate: `showGrid > 0 && selectionMode == -1 && vTileId <= 0.5`
   (off during selection overlay; walls excluded)
2. Recover tile-space coords from the varying:
   `tileCoord = vMapCoord * mapSize`
3. Identify the current tile: `ct = floor(tileCoord.x)`,
   `rt = floor(tileCoord.y)`
4. Identify the mouse tile: `mt = floor(selectedTile.x)`,
   `nt = floor(selectedTile.y)`
5. Chebyshev distance: `dist = max(abs(ct - mt), abs(nt - rt))`
6. If `dist <= gridRadius`, compute edge-proximity:
   ```glsl
   float dx = min(localUV.x, 1.0 - localUV.x);
   float dy = min(localUV.y, 1.0 - localUV.y);
   float edgeDist = min(dx, dy);
   float border = 1.0 - smoothstep(0.0, edge, edgeDist);
   ```
7. Distance fade: `fade = 1.0 - dist / max(gridRadius, 0.01)`
8. Blend: `color.rgb = mix(color.rgb, gridColor, border * fade * 0.85)`

Runs after lighting so grid lines are always visible regardless of
light settings.

### Toggle state

`game.showGrid` (boolean) added via module augmentation in
`prefabs.ts` / `uiPrefabs.ts`.  The G-key handler lives in
`initTilemap()`'s `'update'` callback via
`game.wasKeyPressed('KeyG')`.  Defaults to `true` set in
`init.ts`.

### Pitfall: max vs min for edge distance

Using `max()` to combine edge-distances (e.g. `max(smoothstep(0, e,
localUV.x), smoothstep(0, e, 1.0 - localUV.x))`) always produces ~1
because any point is far from at *least* one edge.  The correct
operation is `min(localUV, 1.0 - localUV)` — distance to the
**nearest** edge — so `border = 1` only at boundaries.
