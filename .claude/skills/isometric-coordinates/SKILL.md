---
name: isometric-coordinates
description: >
    Coordinate transforms and depth math for the classic-wgl isometric tilemap.
    Covers map orientation (SW=closest, NE=farthest), the tx−ty depth axis,
    the isoDepth formula, vertex shader Y-offset compensation, mouse-to-iso
    coordinate conversion, bilinear height interpolation, and camera positioning.
    Use when debugging selection accuracy, depth occlusion, sprite-terrain
    alignment, or any coordinate-space mismatch between CPU and GPU.
    Trigger phrases: "iso coords", "mouse to iso", "depth formula", "isoDepth",
    "tx minus ty", "worldPos.y -= vertexPos.z", "mouseIsoPos", "getFix",
    "cartesianToIso", "isoToCartesian", "camera position", "selection offset",
    "bilinear height", "slope click".
compatibility: Relies on gl-matrix (vec3, mat4). Tilemap is 200×200 with
    scale 45. Projection is mat4.ortho(0, vw, vh, 0, -10000, 10000).
metadata:
    author: classic-wgl
    version: '0.1'
allowed-tools: Read, Grep, Bash
---

## Scope: coordinate transforms and depth math only

This skill covers the coordinate systems, transform chains, depth formulas,
mouse-to-tile conversion, and height compensation math. It does NOT cover
shader internals (beyond the transforms they apply), mesh generation, UI
layout, or agent movement — those are in `isometric-tilemap`.

---

## 1. MAP ORIENTATION (memorise this)

The camera looks from the **southwest**. The isometric diamond on screen:

```
          NE (199,0)  ←── farthest
         /\
        /  \
       /    \
  NW (0,0)   SE (199,199)
       \    /
        \  /
         \/
     SW (0,199)  ←── closest
```

| Corner        | Grid coords | x+y | **tx−ty** | Distance  |
| ------------- | ----------- | --- | --------- | --------- |
| SW (closest)  | (0, 199)    | 199 | **−199**  | Closest   |
| SE            | (199, 199)  | 398 | 0         | Mid-close |
| NW            | (0, 0)      | 0   | 0         | Mid-far   |
| NE (farthest) | (199, 0)    | 199 | **+199**  | Farthest  |

### ℹ️ ALWAYS use `tx − ty` for depth; NEVER use `tx + ty`

`tx + ty` is **identical** for SW `(0,199)` and NE `(199,0)` — it cannot tell
the closest and farthest corners apart. `tx − ty` gives `−199` vs `+199` —
correctly separating them.

### Iso ↔ cardinal / screen mapping

In this engine, the iso-grid directions map to cardinal compass points and
screen directions as follows (verify with the debug compass rose overlay):

| Iso direction | Cardinal | Screen direction | Depth (`tx−ty`) |
|:---|:---|:---|:---|
| NE `(+tx, −ty)` | **N** | straight **UP** | Farthest |
| SE `(+tx, +ty)` | **E** | straight **RIGHT** | Mid |
| SW `(−tx, +ty)` | **S** | straight **DOWN** | Closest |
| NW `(−tx, −ty)` | **W** | straight **LEFT** | Mid |

Intercardinals (NE/SE/SW/NW in compass terms) map to the 2:1 diagonal screen
vectors between these cardinals:

| Compass | Iso `(dtx, dty)` | Screen vector |
|:---|:---|:---|
| **NE** | `(+1, 0)` | right-up 2:1 |
| **SE** | `(0, +1)` | right-down 2:1 |
| **SW** | `(−1, 0)` | left-down 2:1 |
| **NW** | `(0, −1)` | left-up 2:1 |

---

## 2. ISO ↔ CARTESIAN TRANSFORM MATRICES

### Cartesian to ISO

```typescript
cartesianToIso4 = rotateZ(π / 4) * scale(1, 2, 1);
```

`scale(1,2)` first, then `rotateZ(π/4)`:

```
rot * S = [[cos45, -2sin45, 0, 0], [sin45, 2cos45, 0, 0], [0,0,1,0], [0,0,0,1]]
        = [[0.707, -1.414, 0, 0], [0.707, 1.414, 0, 0], [0,0,1,0], [0,0,0,1]]
```

### ISO to Cartesian (inverse)

```
isoToCartesian4 = inverse of cartesianToIso4
```

```
Matrix:
[[ 0.707,  0.707, 0, 0 ],
 [−0.354,  0.354, 0, 0 ],
 [ 0,      0,     1, 0 ],
 [ 0,      0,     0, 1 ]]
```

Scaled by tilemap scale `[45, 45, 1]`:

```
_isoToCartesian:
[[ 31.82,  31.82, 0, 0 ],
 [−15.91,  15.91, 0, 0 ],
 [ 0,      0,     1, 0 ],
 [ 0,      0,     0, 1 ]]
```

### Tile-step sizes in world space

| Axis                                                 | Per tile step (world units)      | Value         |
| ---------------------------------------------------- | -------------------------------- | ------------- |
| `worldX = isoToCart.mat[0]*tx + isoToCart.mat[4]*ty` | `tx+1` → +31.82, `ty+1` → +31.82 | 31.82 px/tile |
| `worldY = isoToCart.mat[1]*tx + isoToCart.mat[5]*ty` | `tx+1` → −15.91, `ty+1` → +15.91 | 15.91 px/tile |
| `tx = (worldX/31.82 + worldY/15.91) / 2`             | —                                | —             |
| `ty = (worldX/31.82 - worldY/15.91) / 2`             | —                                | —             |

Key constants:

- `tx+ty` → worldX step = `scale[0] * √2/2` ≈ **31.82**
- `ty-tx` → worldY step = `scale[0] * √2/4` ≈ **15.91**

These are used to convert between screen-pixel offsets and iso-tile offsets
(e.g., height compensation below).

---

## 3. DEPTH FORMULA (isoDepth)

Used in both the tilemap vertex shader and the CPU-side IsoSprite depth.

```
isoDepth = clamp(
    (tx − ty) / 400.0 + 0.5 − Z / 14500.0,
    0.0, 1.0
)
```

| Term          | Meaning                                        | Per-unit change   |
| ------------- | ---------------------------------------------- | ----------------- |
| `(tx−ty)/400` | Isometric distance along SW→NE axis            | ±0.0025 per tile  |
| `+0.5`        | Centres range [-0.5, +0.5] into [0, 1]         | —                 |
| `−Z/14500`    | Elevation pushes things closer (smaller depth) | −0.00221 per Z=32 |

Where:

- `400 = sizeX + sizeY` (for 200×200 map)
- `14500` — calibrated so one height unit (Z=32) ≈ 0.88 tiles of perceived isometric distance
- `Z = height × heightScale` — world-space elevation

### ℹ️ Derivation of 14500

One tile step diagonally ≈ 35.5 screen pixels. One height unit (Z=32) displaces
32 pixels vertically. Visual ratio: 32/35.5 ≈ 0.90 tiles per height unit.

Depth change per tile: 1/400 = 0.0025.
Desired depth change per height unit: 0.90 × 0.0025 = 0.00225.
For Z=32: 32 / D = 0.00225 → D ≈ 14222, rounded to 14500 empirically.

### Shader vertex shader (iso_tilemap.vert)

```glsl
vec4 worldPos = modelMatrix * isoMatrix * vec4(vertexPos, 1.0);
worldPos.y -= vertexPos.z;
vec4 clipPos = projectionMatrix * cameraMatrix * worldPos;
float isoDepth = clamp(
    (vertexPos.x - vertexPos.y) / 400.0 + 0.5 - vertexPos.z / 14500.0,
    0.0, 1.0
);
clipPos.z = isoDepth;
gl_Position = clipPos;
```

### CPU-side sprite depth (IsoSprite.rawDraw)

Per-vertex depth capped at anchor depth, passed as `isoDepthCorners` vec4:

```typescript
const baseDepth = (pos[0]-pos[1]) / 400 + 0.5 - pos[2] / 14500 - 0.005;
for (let i = 0; i < 4; i++) {
    const d = (pos[0] + fp[i][0] - pos[1] - fp[i][1]) / 400 + 0.5 - pos[2] / 14500 - 0.005;
    raw[i] = Math.min(d, baseDepth);
}
const minFp = Math.min(raw[0], raw[1], raw[2], raw[3]);
cornerDepths[0] = minFp; // bottom-left (SW)
cornerDepths[1] = minFp; // bottom-right (SE) — flat base edge
cornerDepths[2] = raw[3]; // top-left (NW), capped at baseDepth
cornerDepths[3] = raw[0]; // top-right (NE), capped at baseDepth
```

The `-0.005` bias (not `-0.001`) overcomes GPU-interpolated terrain vertex
depths at adjacent mesh vertices that can dip ~0.0026 below the exact
iso-position depth. See `isometric-tilemap` §4 for the shader-side corner
selection and §17 for footprint format.

### Sort order (IsometricDrawable)

```typescript
order(): number {
    return this.position[0] - this.position[1]
         - this.tilemap.sizeX - this.tilemap.sizeY;
}
```

The `− sizeX − sizeY` offset ensures all isometric sprites have `order < 0`
and draw **after** the tilemap (order = 0). Without it, NE sprites
(`tx > ty`) get positive order and draw before the tilemap — ghost
rendering and depth‑based occlusion break because opaque terrain overpaints
the sprites.

Relative sprite-to-sprite ordering is preserved; depth test LEQUAL handles
per‑pixel occlusion correctly.

---

## 4. VERTEX SHADER Y-OFFSET AND ITS EFFECTS

### The culprit

```glsl
worldPos.y -= vertexPos.z;
```

This pushes elevated vertices **up** on screen. But it has a critical side
effect on the ISO GRID COORDINATES:

### How Z shifts both tx and ty

The world position after Y-offset:

```
worldX = 31.82 × (tx + ty)        ← unchanged
worldY = 15.91 × (ty − tx) − Z    ← shifted up by Z
```

Inverse-solving for the iso coords that produce the shifted world position:

```
tx_shifted = tx + Z / 31.82    ← shifts toward NE (+Z/31.82)
ty_shifted = ty − Z / 31.82    ← shifts toward NW (−Z/31.82)
```

### ℹ️ Elevated tiles appear SHIFTED on screen

A tile at `(tx, ty)` with height `Z` appears on screen as if it were at:

```
(tx + Z/31.82, ty − Z/31.82) — shifted ~1 tile toward NE per 32 Z-units
```

### Implications

- Clicking on an elevated tile's visible surface: the inverse mouse transform
  gives the SHIFTED coordinates. Must compensate by subtracting Z/31.82 from tx
  and adding Z/31.82 to ty.
- Sprites standing on elevated terrain: the IsometricDrawable.modelMatrix()
  subtracts `h × heightScale` from `cartPos[1]` (worldY), which matches the
  vertex shader's Y-offset — no extra compensation needed.
- Default height of 1 (Z=32): everything is shifted by ~1 tile toward NE
  from its grid position.

---

## 5. MOUSE-TO-ISO COORDINATE CONVERSION

### The transform chain

```typescript
updateMousePos(): void {
    this.mouseIsoPos = vec3.clone(this.game.mousePos);          // 1. screen px
    vec3.add(this.mouseIsoPos, this.mouseIsoPos, this.game.camera.getFix());  // 2. screen→world
    vec3.div(this.mouseIsoPos, this.mouseIsoPos, this.game.camera.scale);     // 3. camera scale
    this.cartesianToIso(this.mouseIsoPos);                      // 4. world→iso
    // 5. compensate vertex-shader Y-offset (iterative, see §6 below)
}
```

### Camera getFix()

```typescript
getFix(): vec3 {
    return position * scale − size / 2;
}
```

This returns the world-space position of the **top-left** corner of the
viewport.

### Camera positioning

```typescript
// Correct: viewport center IS camera.position
game.camera.position[0] = worldX_of_target_tile;
game.camera.position[1] = worldY_of_target_tile;

// WRONG: do NOT add size/2 — getFix() already subtracts it
// game.camera.position[0] = worldX + size[0]/2;  ← double-counts
```

### Step-by-step for screen (sx, sy) → iso (tx, ty)

1. `worldX = sx + position[0] * scale[0] − size[0]/2` (from `mouse + getFix()`)
2. `worldY = sy + position[1] * scale[1] − size[1]/2`
3. `wx = worldX / 45` (dividing out tilemap scale from `cartesianToIso`)
4. `wy = worldY / 45`
5. `tx = 0.707 × (wx − wy)` (rotateZ(π/4), scale(1,2))
6. `ty = 0.354 × (wx + wy)`... wait:

cartesianToIso4 = rotateZ(π/4) * scale(1, 2, 1):

For a cartesian point (cx, cy):

- First scale by (1, 2): (cx, 2×cy)
- Then rotate by 45°:
    - tx = cx×cos45 − 2×cy×sin45 = 0.707×(cx − 2×cy)
    - ty = cx×sin45 + 2×cy×cos45 = 0.707×(cx + 2×cy)

After dividing world by 45 for the scale array:

- cx = worldX/45, cy = worldY/45
- tx = 0.707×(worldX/45 − 2×worldY/45)
- ty = 0.707×(worldX/45 + 2×worldY/45)

---

## 6. HEIGHT COMPENSATION (vertex-shader Y-offset reversal)

### Why needed

The vertex shader does `worldPos.y -= vertexPos.z`. This makes elevated tiles
appear at a DIFFERENT screen position than their grid coordinates would suggest.
The mouse-to-iso transform must reverse this.

### Bilinear height read

Read all 4 corners of the tile the mouse hovers over and interpolate:

```typescript
const at = (tx: number, ty: number) =>
    heightData[clamp(floor(tx), 0, sizeX - 1) + clamp(floor(ty), 0, sizeY - 1) * sizeX] ?? 0;

const ftx = Math.floor(px),
    fty = Math.floor(py);
const fx = px - ftx,
    fy = py - fty;
const hNW = at(ftx, fty);
const hNE = at(ftx + 1, fty);
const hSW = at(ftx, fty + 1);
const hSE = at(ftx + 1, fty + 1);
const h = hNW + (hNE - hNW) * fx + (hSW - hNW) * fy + (hNW - hNE - hSW + hSE) * fx * fy;
```

This matches the GPU's vertex interpolation exactly.

### Fixed-point iteration (3 passes)

The height read and position form a circular dependency — the visual position
is already shifted by the terrain height. Three passes converge reliably:

```typescript
const tileStep = scale[0] * 0.7071;  // worldX per tile = 31.82
let isoX = mouseIsoPos[0];
let isoY = mouseIsoPos[1];

for (let i = 0; i < 3; i++) {
    const h = /* bilinear at (isoX, isoY) */;
    if (h <= 0) break;
    const zOffset = (h * heightScale) / tileStep;
    isoX = mouseIsoPos[0] - zOffset;
    isoY = mouseIsoPos[1] + zOffset;
}

mouseIsoPos[0] = isoX;
mouseIsoPos[1] = isoY;
```

### 💀 DO NOT compound the offset

Each pass computes from the **original visual position** (`mouseIsoPos`),
not from the already-compensated position (`isoX`/`isoY` from previous pass).
`isoX` and `isoY` track the current _estimate_; `mouseIsoPos` is always the
_visual_ (uncompensated) position. Compounding (subtracting from the already-
subtracted value) diverges, not converges.

### Tile-step for compensation

The correct conversion is `tileStep = scale[0] × 0.7071` (NOT `scale[0] × 0.3536`):

- The vertex shader Y-offset adds Z pixels to worldY
- 1 tile in tx shifts worldX by 31.82 = scale × √2/2
- 1 tile in ty shifts worldY by 15.91 = scale × √2/4
- 1 pixel of Z offset = 1/31.82 tiles in tx and 1/31.82 tiles in ty

Using 0.3536 (√2/4) instead of 0.7071 (√2/2) produces exactly 2× too much
compensation — the selection jumps to the far side of the map.

### Sign convention

```typescript
isoX = visualX − zOffset;  // subtract from tx
isoY = visualY + zOffset;  // add to ty
```

This moves from the shifted position (tx+LARGER, ty+SMALLER) back to the true
grid position.

---

## 7. SPRITE HEIGHT TRACKING

### Bilinear interpolation in `modelMatrix()`

`IsometricDrawable.modelMatrix()` subtracts terrain height from `cartPos[1]`
to create the visual Y‑offset matching the vertex shader. This MUST use
bilinear interpolation — reading all four corners at the exact float
position — not `Math.floor` of a single corner:

```typescript
// CORRECT: bilinear at exact position
const px = this.position[0],
    py = this.position[1];
const ftx = Math.floor(px),
    fty = Math.floor(py);
const fx = px - ftx,
    fy = py - fty;
const hNW = at(ftx, fty);
const hNE = at(ftx + 1, fty);
const hSW = at(ftx, fty + 1);
const hSE = at(ftx + 1, fty + 1);
const h = hNW + (hNE - hNW) * fx + (hSW - hNW) * fy + (hNW - hNE - hSW + hSE) * fx * fy;
cartPos[1] -= h * this.tilemap.heightScale;

// WRONG: single-corner snapshot
// const tx = Math.floor(this.position[0]);
// const ty = Math.floor(this.position[1]);
// const h = heightData[tx + ty * sizeX] ?? 0;
```

A `Math.floor` read can lag one tile behind the actual terrain surface
when the sprite is between two tiles of different heights — the visual
offset doesn't match the GPU‑interpolated surface.

This applies to ALL `IsometricDrawable` instances (IsoSprite, IsoAgent,
trees, houses, semaphores) — not just the agent.

### Initialise `position[2]` to terrain height

ALL sprites need `position[2]` set to the bilinear terrain height, not just
the agent. Static sprites loaded from `state.json` default to `position[2] = -1`,
which causes base clipping on height>0 terrain. Initialise them in
`initFootprintColliders()`:

```typescript
const px = sprite.position[0];
const py = sprite.position[1];
const ftx = Math.floor(px),
    fty = Math.floor(py);
const fx = px - ftx,
    fy = py - fty;
const hTop = at(ftx, fty) + (at(ftx + 1, fty) - at(ftx, fty)) * fx;
const hBot = at(ftx, fty + 1) + (at(ftx + 1, fty + 1) - at(ftx, fty + 1)) * fx;
sprite.position[2] = (hTop + (hBot - hTop) * fy) * heightScale;
```

### Collider and debug footprint terrain height

Both `IsoSprite.attachCollider()` and the state.ts debug footprint rendering
must apply bilinear terrain height per-vertex to world-space positions:

```typescript
const h = /* bilinear at (position[0] + pt[0], position[1] + pt[1]) */;
v[1] -= h * tilemap.heightScale;
```

Without this, colliders and debug footprints appear `h * heightScale` pixels
below the terrain surface (shifted toward SW / camera by ~1 tile per height unit).

### Anchor placement from visual base

A sprite's anchor Y in the texture should match the visible ground-contact
row (`groundY`). For sprites with transparent pixels below the visible content
(tree trunk, traffic light pole base), the anchor defaults to ~0.997 (bottom
of texture) but the visual base is higher:

```
anchorY = groundY / textureHeight
```

To keep the sprite visually in place after changing the anchor:

```
delta_worldY = (oldAnchorRow - groundY) * scaleY
dty = delta_worldY / 31.82, dtx = -dty
newPosition = oldPosition + [dtx, dty]
```

The 1-tile-up adjustment (anchoring 32 screen‑pixels above groundY to centre
the footprint diamond's SW vertex on the visual base) is unnecessary when
terrain height compensation is correctly applied — the anchor should sit
directly at groundY.

---

## 8. COMMON BUG PATTERNS

| Symptom                                                            | Likely cause                                                                 |
| ------------------------------------------------------------------ | ---------------------------------------------------------------------------- |
| Selection always off by ~1 tile toward top-left                    | No height compensation at all                                                |
| Selection jumps massively toward bottom-right                      | Wrong tile-step (used 15.91 instead of 31.82)                                |
| Selection wildly overshoots                                        | Offset compounding (subtracting from already-compensated position each pass) |
| Selection misses slopes by increasing amounts                      | Single-pass compensation (no iteration on sloped terrain)                    |
| Selection works on flat but not slopes                             | `Math.round` instead of bilinear height read                                 |
| Camera centres on wrong tile                                       | Added `+size/2` to camera position (double-counting)                         |
| NW corner of map appears at viewport center instead of target tile | Camera position was set incorrectly (see §5)                                 |
| Agent visually lagging ~1 tile behind terrain on slopes            | `modelMatrix()` uses `Math.floor` NW-corner read instead of bilinear         |
| Agent rendering behind terrain for first ~0.5 s after spawn        | `position[2]` not initialised to terrain height (starts at 0)                |
| Ghost rendering / sprite invisible in NE quadrant                  | `order()` positive for NE sprites — drawn before tilemap, overpainted by it  |
| Terrain flickering / black artifacts after creating UIText elements | `Text.setText` binds internal FBO, changes viewport — must restore both. Also requires minimal draw-call flush (`drawArrays(LINE_STRIP, 0, 0)`) at frame end to sync GPU pipeline state |
| Sprite clipped at visual base on height>0 terrain                   | `position[2]` not initialized to bilinear terrain height in `initFootprintColliders()` — defaults to -1 |
| Agent ghosted when standing near large sprite                       | Depth buffer conflict between large sprite's per-vertex gradient and small sprite. Fix: `depthMask(false)` on normal pass + renderList sort for sprite-sprite ordering |
| Depth bias too small for terrain interpolation                      | GPU fragment interpolation between adjacent terrain mesh vertices can produce depths ~0.0026 below the exact-position value. Bias must be at least `-0.005` to overcome this |

---

## 9. QUICK REFERENCE

```
                      ISO ↔ WORLD                    DEPTH
                      ===========                  ==========
tx+ty = worldX / scale / 0.7071          depth = (tx−ty)/400 + 0.5 − Z/14500
ty−tx = worldY / scale / 0.3536
tx = ((tx+ty) + (tx−ty)) / 2             tx−ty: −199 closest, +199 farthest
ty = ((tx+ty) − (tx−ty)) / 2             Z = height × heightScale (32 per unit)

                      HEIGHT COMPENSATION
                      ===================
tileStep = scale × 0.7071  (NOT scale × 0.3536!)
zOffset  = height × heightScale / tileStep
isoTx = visualMouseTx − zOffset
isoTy = visualMouseTy + zOffset

                      COMPASS / CARDINALS
                      ===================
iso NE (+tx,−ty) → N (screen UP)      iso (0,−1) → NW (screen left-up 2:1)
iso SE (+tx,+ty) → E (screen RIGHT)   iso (+1, 0) → NE (screen right-up 2:1)
iso SW (−tx,+ty) → S (screen DOWN)    iso (0,+1) → SE (screen right-down 2:1)
iso NW (−tx,−ty) → W (screen LEFT)    iso (−1, 0) → SW (screen left-down 2:1)
```
