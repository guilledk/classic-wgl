# TypeScript Parity Reference

Kept as an oracle from the TypeScript engine that the Rust port was derived from.
The TS source was removed in `remove TypeScript engine and tooling`.

## Camera

### `getFix()` — `camera.ts:21-30`

```typescript
getFix(): vec3 {
    const camFixed = vec3.clone(this.position);
    const size = vec3.clone(this.size);
    vec3.mul(camFixed, camFixed, this.scale);
    vec3.div(size, size, [2, 2, 1]);
    vec3.sub(camFixed, camFixed, size);
    return camFixed;
}
```

**Formula:** `fix = position * scale - size / [2, 2, 1]`

(Matches `crates/classic-core/src/camera.rs` as of commit `09c72b7`.)

The camera matrix is `T(-fix) * S(scale)`, making `position * scale` map to the
viewport centre `size / 2`.

### `matrix()` — `camera.ts:33-38`

```typescript
matrix(): mat4 {
    return mat4.mul(
        mat4.create(),
        mat4.fromScaling(mat4.create(), this.scale),
        mat4.fromTranslation(mat4.create(), vec3.negate(vec3.create(), this.getFix())),
    );
}
```

Effectively: `matrix = S(scale) * T(-fix)` (TS: `fromScaling` is `S`, then `mul` with `fromTranslation` is `S * T`).

**Rust equivalent (`camera.rs:34-37`):** `Mat4::from_translation(-fix) * Mat4::from_scale(scale)`.
These are **not algebraically identical** — TS does `S * T` (scale first), Rust does `T * S`
(translate first). Both produce visually plausible results because the fix point formula compensates.

---

## Projection

### Orthographic — `state.ts:354-363`

```typescript
mat4.ortho(this.projectionMatrix, 0, vw, vh, 0, -10000, 10000);
```

Screen-space ortho: origin at top-left, Y-down, near/far ±10000.

**Rust equivalent (`types.rs:188-190`):** `Mat4::orthographic_rh(0.0, vw, vh, 0.0, -10000.0, 10000.0)`.

---

## Isometric

### cartesianToIso — `utils.ts:607-616`

```typescript
const _cartesianToIso4 = mat4.create();
mat4.rotateZ(_cartesianToIso4, _cartesianToIso4, Math.PI / 4);
mat4.scale(_cartesianToIso4, _cartesianToIso4, [1, 2, 1]);
```

**Transform:** rotate Z +45° (π/4), scale Y ×2. 2:1 isometric projection.

**Rust equivalent (`math.rs`):** `cartesian_to_iso_4()` → `rotate_z(FRAC_PI_4) * scale(Vec3::new(1.0, 2.0, 1.0))`.

### Depth formula — `iso_tilemap.vert:25-29`

```glsl
float isoDepth = clamp(
    (vertexPos.x - vertexPos.y) / 400.0 + 0.5 - vertexPos.z / 14500.0,
    0.0, 1.0
);
clipPos.z = isoDepth;
```

`worldPos.y -= vertexPos.z;` at line 23 compensates for height in screen Y.
Clamped `[0, 1]` and written into `clipPos.z`, overriding the standard projection Z.

### Sprite depth — `isometric.ts:747-754`

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

Minimum depth across all footprint corners, mirroring the vertex shader formula, with `-0.005` bias.

### Mouse-to-iso — `isometric.ts:109-146`

```typescript
const tileStep = (this.scale[0] as number) * 0.7071;
```

3-pass iterative height-parallax solve: bilinearly interpolate height at `(isoX, isoY)`,
then `isoX -= zOffset`, `isoY += zOffset` where `zOffset = (h * heightScale) / tileStep`.
Stops at `h <= 0`. Starts from mouse screen pos → `+camera.getFix()` → `/camera.scale`
→ `cartesianToIso()`.

### `order()` — `isometric.ts:699-701`

```typescript
order(): number {
    return this.position[0] - this.position[1] - this.tilemap.sizeX - this.tilemap.sizeY;
}
```

Descending sort: larger order → draw first (farther).

### `defaultFootprint()` — `isometric.ts:713-720`

```typescript
static defaultFootprint(): Vec2Like[] {
    return [
        [0.5, -0.5],   // NE
        [0.5, 0.5],    // SE
        [-0.5, 0.5],   // SW
        [-0.5, -0.5],  // NW
    ];
}
```

Clockwise from NE quadrant.

### Anim directions — `isometric.ts:925-934`

```
animIndex = floor(atan2(dy, dx) as deg / 45), negative-wrap: +8 if negative
animDirs = [East, SouthEast, South, SouthWest, West, NorthWest, North, NorthEast]
```

Animation names: `'idle' + animDirs[i]`, `'walk' + animDirs[i]`.

---

## Lighting

### LIGHT_PRESETS — `lighting.ts:17-42`

| Preset | ambient | direction | color |
|--------|---------|-----------|-------|
| sunny | `[0.15, 0.15, 0.2]` | `norm(0.453, 0.211, 0.866)` | `[1.0, 0.95, 0.85]` |
| cloudy | `[0.35, 0.35, 0.4]` | `norm(0.0, -0.2, 1.0)` | `[0.7, 0.72, 0.78]` |
| dawn | `[0.2, 0.15, 0.25]` | `norm(0.5, 0.2, 0.3)` | `[1.0, 0.4, 0.2]` |
| night | `[0.1, 0.12, 0.25]` | `norm(-0.2, -0.5, 0.8)` | `[0.3, 0.4, 0.7]` |

Order: `['sunny', 'cloudy', 'dawn', 'night']`.

---

## State Dump Format

### Overall JSON

```json
{"entities": {"<name>": {"components": [{"type": "...", ...}, ...]}}}
```

### `type`-first contract

The Rust loader is **registry-driven** (serde), not positional — it reads the
`"type"` field, looks up the component spawner, and deserialises the rest of
the object by field name.  The TS positional loader (which splices out
`"type"` and passes the remaining values as positional constructor args) was
deleted with the TS original.  `"type"` is still emitted first as a
convention, but key order is no longer load-bearing.

The dumpers are **derive-driven** (`serde` on each component, with `"type"`
prepended), so keys follow each component's serde attributes
(`rename_all = "camelCase"` where set, `#[serde(skip)]`/`#[serde(rename)]`
honoured).

### Per-component dump keys (after `type`)

| Component | Keys |
|-----------|------|
| **Sprite** | `position`, `scale`, `texture`, `ignoreCam`, `frame`, `tileSetSize`, `anchor` |
| **IsoSprite** | `position`, `scale`, `texture`, `tilemap`, `frame`, `tileSetSize`, `anchor`, `footprint` |
| **IsoAgent** | + `speed`, `animSpeed`, `animPrefix` (appended to IsoSprite) |
| **Tilemap** | `position`, `scale`, `sizeX`, `sizeY`, `tileSet`, `tilePixelSize`, `maxTile`, `data` (base64), `heightData` (base64), `heightScale` |
| **IsometricNavMesh** | `position`, `scale`, `map`, `tileSet`, `data` (base64), `sizeX`, `sizeY` |
| **Animator** | `target`, `speed` |
| **Transform** | `position`, `scale` |
| **Rect** | `color`, `ignore_cam` |
| **SdfText** | `atlas_name`, `color`, `bgcolor`, `outline_color`, `outline_width`, `shadow_offset`, `shadow_color`, `shadow_blur`, `ignore_cam`, `text`, `justify`, `weight`, `gamma` |
| **Camera** | `position`, `scale` (`size` is runtime-only) |

---

## heightData Indexing

### TS convention

- Array size: `sizeX * sizeY` — one value per **tile** (not per vertex).
- Index: `heightData[x + sizeX * y]`.
- Per-vertex heights are reconstructed by bilinear interpolation of the 4 tile corners.

### Rust convention

- Array size: `(sizeX + 1) * (sizeY + 1)` — one value per **vertex**.
- Index: `heightData[ty * (sizeX + 1) + tx]` (stride = `sizeX + 1`).
- Tile-based access uses the same stride: `y * (sizeX + 1) + x`.

**This is a deliberate divergence.**  The vertex-grid representation avoids
an off-by-one interpolation edge case at the right/bottom map boundary.
Both representations produce the same flat heights (all 1.0) for the demo.

---

## Known TS↔Rust Divergences

| Item | TS | Rust |
|------|----|------|
| **heightData stride** | `sizeX * sizeY` (tile grid) | `(sizeX + 1) * (sizeY + 1)` (vertex grid) |
| **Camera matrix order** | `S(scale) * T(-fix)` | `T(-fix) * S(scale)` |
| **GLSL version** | GLSL 100 (`attribute`, `varying`, `texture2D`) | GLSL 300 es (`in`, `out`, `texture`) |
| **SDF shadow/glow** | Secondary draw passes for shadow + glow | Fields stored but not rendered (single-pass only) |
| **Disabled collider** in quadtree | All colliders inserted into quadtree (including disabled) | Disabled colliders skipped in `begin_frame()` |
| **Bitmap text** (`UIText`) | Traditional glyph-map text for dev UI | Not ported; all text is SDF |
| **Collider click priority** | `consumesClick` pre-scan sets `uiConsumedClick` | Pre-scan is a no-op; dispatch path sets `consumed_click` |
| **Entity destruction** | `destroyEntity` | `world.despawn` is never called (visibility = `Disabled` marker) |
| **Root UI tree** | All UI elements attached to a root container; layout walks the full tree | Only the top bar is attached to root; all other widgets position themselves |

---

## Rust-only: procedural terrain

`guest/lunar-guest/` has no TypeScript ancestor.  It exists to generate the
`lunar` demo scene (`CLASSIC_ROM=rom:lunar`) and is documented in full by the
`classic-terrain` skill.  Notes relevant to parity:

- The TS `Tilemap.data` comment "null = auto-generate noise" described a
  feature that was never implemented on either side.  The Rust `lunar` scene
  supersedes it: `data` is `null` in `state_lunar.json`, and terrain, tiles and
  nav data are all generated + bulk-uploaded by the `guest/lunar-guest` ROM
  guest (then `commit_terrain`) after `load_state`.
- `SimplexNoise` was a faithful TS port but had no callers.  It now underpins
  the generator; `Random::next_f64` was fixed to honour its documented `[0, 1)`
  range (it divided the top 16 bits of the LCG state by 32768 instead of
  65536, returning `[0, 2)` and biasing the Fisher-Yates shuffle in
  `build_perm`).  This changes simplex output for a given seed relative to the
  TS implementation.
- `build_mesh` now emits **smooth per-vertex normals** for top faces instead of
  per-face normals.  On level terrain every face normal is already `+Z`, so
  flat scenes — including the demo golden baseline — are bit-identical; on
  generated terrain, per-face normals made the triangulation visible as a
  herringbone of facets.
- The `lunar` light preset (near-zero ambient, hard low sun) is an addition to
  `LIGHT_PRESETS`; the four TS presets are unchanged.
