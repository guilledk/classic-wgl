---
name: classic-sdf-text
description: >
    Signed Distance Field (SDF) font rendering system for classic-wgl.
    Covers `SdfText` (base direct-GL renderer) and `UISdfText` (word-wrap,
    justification, outline, shadow, multi-line). Includes the build-time
    atlas generator, coordinate system conventions, GL requirements,
    manifest integration, and common pitfalls. Trigger phrases: "SDF
    text", "SdfText", "UISdfText", "font rendering", "sdf.frag", "sdfText",
    "atlas generator", "make-font-atlas", "setOutline", "setShadow",
    "setJustify", "spawnSdfText", "dejavusans", "wrapTextAtPixelWidth",
    "glyph metrics".
---

# classic-wgl SDF Font Rendering

## 1. ARCHITECTURE

```
Drawable (transforms.ts)
  └─ SdfText (sdfText.ts)              direct vertex-buffer rendering; no FBO
       └─ UISdfText (ui.ts)            word-wrap, justification, multi-line
```

`SdfText` renders each glyph as a textured quad from a pre-generated SDF atlas.
It builds an interleaved `Float32Array` vertex buffer (position + UV per vertex,
6 vertices per glyph) and issues one `drawArrays(gl.TRIANGLES)` call per text
string per frame (plus an optional second pass for drop-shadow).

`UISdfText` extends `SdfText` with word-wrapping (pixel-width driven, not
character-count), explicit `\n` line breaks, and per-line justification
(`left` / `center` / `right`).

All data (metrics + atlas texture) is loaded synchronously during
`loadResources()` — no runtime `fetch`.

### Key files

| File | Role |
|------|------|
| `scripts/make-font-atlas.mjs` | Build-time SDF atlas + metrics JSON generator |
| `src/classic/sdfText.ts` | `SdfText` class: glyph layout, vertex buffer, rawDraw |
| `src/classic/ui.ts` (UISdfText) | `UISdfText` class: word-wrap, justify, multi-line |
| `src/shaders/sdf.vert` | Vertex shader: `texCoord` varying passthrough |
| `src/shaders/sdf.frag` | Fragment shader: `smoothstep` + outline + shadow |
| `src/classic/utils.ts` | `initSdfFonts()` — loads metrics JSON at startup |
| `src/classic/state.ts` | `game.sdfFonts`, `game.getSdfFont()` |
| `public/manifest.json` | `"sdfFonts": [{ "name": "...", "metrics": "..." }]` |

---

## 2. BUILD-TIME ATLAS GENERATOR

`scripts/make-font-atlas.mjs` uses `@napi-rs/canvas` (devDependency only) to
rasterise a `.ttf`/`.otf` file and produce a grayscale SDF texture atlas.

### Key constants

| Constant | Value | Meaning |
|---|---|---|
| `GLYPH_SIZE` | 64 | Target cell size in output pixels |
| `RENDER_SCALE` | 16 | High-res render multiplier |
| `SOURCE_W` | `HIGH_RES + padding*2` = 3072 | Source canvas pixel dimensions |
| `CELL_SCALE` | `SOURCE_W / GLYPH_SIZE` = 48 | Source pixels per output cell pixel |
| `ORIGIN_CELL` | `padding / CELL_SCALE` ≈ 21.33 | Font baseline in cell-pixel coordinates |
| `maxDist` | `GLYPH_SIZE` = 64 | SDF distance-field spread (in source pixels) |

### Algorithm

1. Render each glyph at `fontSize * RENDER_SCALE` px, `textBaseline='alphabetic'`, `textAlign='left'`
2. Compute SDF: detect edges (inside→outside transitions), brute-force Manhattan distances, normalize to `[0..255]` with `maxDist` spread
3. Crop to bounding box of non-background pixels (`byte ≠ 1`)
4. Shelf-pack cropped glyphs into power-of-two atlas (typically 256×256)
5. Output `public/res/<font>-sdf.png` + `public/res/<font>-sdf.json`

### Metrics JSON format

```json
{
  "name": "dejavusans",
  "family": "DejaVuSans",
  "atlasSize": [256, 256],
  "glyphSize": 64,
  "baseline": 19.968,
  "lineHeight": 33.28,
  "glyphs": {
    "A": { "x": 232, "y": 0, "w": 20, "h": 22, "xOffset": -1.33, "yOffset": -20.33, "xAdvance": 17.51 }
  }
}
```

All numeric quantities are in **cell pixels** — the universal unit for SDF
text metrics. One cell pixel = one pixel in the original 64×64 SDF output grid.

---

## 3. COORDINATE SYSTEM

Everything is in **cell pixels**, multiplied by `_scale` to get screen pixels.

| Quantity | Computation |
|---|---|
| Glyph top in local coords | `gy = baseline * scale + yOffset * scale` |
| Glyph bottom | `gy + h * scale` |
| Line offset (multi-line) | `gy += lineIndex * lineHeight * scale` |
| Text block extent | `min(gy)` to `max(gy + h*scale)` across all glyphs |
| `textHeight` | `glyphExtentMax - glyphExtentMin` (not `lineHeight * scale`!) |
| `textWidth` | Max line advance from `xAdvance * scale` sums |

**Critical:** `textHeight` must reflect the glyph *extent*, not the line box.
The extent is recomputed immediately after the perLine collection loop
(before the vertex buffer build) so that both the vertex local coordinates
and `modelMatrix()` use the same value. If `textHeight` is changed *after*
the vertex loop, the coordinates mismatch and glyphs appear squashed.

Local → screen transform via `modelMatrix()`:
```typescript
translate(position) * scale(textWidth, textHeight, 1)
```

Positioned by the UI anchor system (e.g., `mid-center`) just like any `UIDrawable`.

### UV mapping

```typescript
ux0 = g.x / atlasW;          // left edge of glyph in atlas
ux1 = (g.x + g.w) / atlasW;  // right edge
uy0 = g.y / atlasH;          // top edge — NO "1 -" inversion
uy1 = (g.y + g.h) / atlasH;
```

**Do NOT use `1 - g.y/atlasH`.** This engine does not set
`UNPACK_FLIP_Y_WEBGL`; `texImage2D` puts image row 0 at texture t=0.
The engine's own `sheet.frag` shader follows the same convention.

---

## 4. SHADERS

### Vertex (`sdf.vert`)

```glsl
attribute vec4 vertexPos;     // location 0 (bound before linkProgram!)
attribute vec2 texCoord;      // location 1
varying mediump vec2 vTexCoord;
// standard model * camera * projection transform
```

### Fragment (`sdf.frag`)

```glsl
float distance = texture2D(texSampler, vTexCoord).r;
float edge = 0.5;
float alpha = smoothstep(edge - softEdge, edge + softEdge, distance);
// Outline: smoothstep in band [edge - outlineWidth, edge]
// Shadow: second drawPass with offset model matrix + blurred outline
```

`softEdge = 0.08` is a good fixed value for the 64-px `GLYPH_SIZE`.

---

## 5. GL REQUIREMENTS

### `bindAttribLocation` before `linkProgram`

In `initShaderProgram()` (`src/classic/utils.ts`), call
`gl.bindAttribLocation(program, i, attributes[i])` **before**
`gl.linkProgram()`. AMD drivers (notably Radeon R9 200) may drop attribute
locations that appear unused to the linker, returning `getAttribLocation`
as `-1`. This causes `vTexCoord` to be `(0,0)`, making `texture2D` sample
the origin of the atlas (usually black).

### Sampler binding order

```typescript
gl.activeTexture(gl.TEXTURE0);
gl.bindTexture(gl.TEXTURE_2D, atlasTexture.texture);
gl.uniform1i(shader.unif.texSampler, 0);  // AFTER texture is bound
```

Some drivers silently ignore a sampler if the texture isn't on the unit
when `uniform1i` is called.

### LINEAR filtering on SDF atlas texture

The atlas texture must use `gl.LINEAR` (min + mag) for `smoothstep` to
produce anti-aliased edges. The engine's default `initTextures()` sets
`gl.NEAREST`, so `SdfText`'s constructor overrides this:

```typescript
this.gl.bindTexture(this.gl.TEXTURE_2D, this.atlasTexture.texture);
this.gl.texParameteri(this.gl.TEXTURE_2D, this.gl.TEXTURE_MIN_FILTER, this.gl.LINEAR);
this.gl.texParameteri(this.gl.TEXTURE_2D, this.gl.TEXTURE_MAG_FILTER, this.gl.LINEAR);
```

### Interleaved vertex buffer

Format: `[posX, posY, uvX, uvY]` per vertex, 6 vertices per glyph,
stride = 16 (4 floats × 4 bytes). Attribute pointers:

```typescript
gl.vertexAttribPointer(attr.vertexPos, 2, gl.FLOAT, false, 16, 0);
gl.vertexAttribPointer(attr.texCoord, 2, gl.FLOAT, false, 16, 8);
```

---

## 6. MANIFEST INTEGRATION

Font metrics are loaded synchronously during `loadResources()`, matching
the engine pattern for shaders/textures/animations.

```json
// public/manifest.json
{
  "sdfFonts": [
    { "name": "dejavusans", "metrics": "/res/dejavusans-sdf.json" }
  ],
  "textures": [
    { "name": "dejavusans-sdf", "src": "/res/dejavusans-sdf.png" }
  ]
}
```

```typescript
// Loaded in state.ts loadResources():
this.sdfFonts = await initSdfFonts(this.manifest.sdfFonts || [], ...);

// Accessed synchronously in SdfText constructor:
this.metrics = this.game.getSdfFont(atlasName);
```

No runtime `fetch`, no `metricsPromise`, no `async` code path. If the JSON
fails to load, `loadResources()` throws before the game starts.

---

## 7. UISdfText FEATURES

### Word wrapping

`UISdfText.wrapTextAtPixelWidth()`:

- Splits on `\n` first (hard line breaks)
- Within each segment, word-wraps on spaces
- Measures each word's pixel width via `measureWord()` (sums per-glyph `xAdvance * scale`)
- Pushes words to successive lines when `linePx + gap + wordPx > maxPx`

The `maxWidth` argument to `spawnSdfText()` is in **screen pixels** (not
character count — unlike the legacy `UIText`).

### Multi-line support

`_buildGlyphBuffer` tracks a `lineIndex` counter. Each glyph's `y` field
in `perLine` stores its line index. The Y offset is computed as:

```typescript
gy = baseline * scale + yOffset * scale + lineIndex * lineHeight * scale;
```

`textHeight` accounts for the line count: `textHeight = maxExtent * lineCount`.

### Justification

```typescript
infoText.setJustify('center');  // 'left' (default) | 'center' | 'right'
```

Per-line: computes `lineWidth = sum(advances)` for each line, then shifts
glyph `x` positions by `(maxWidth - lineWidth) / 2` (center) or
`(maxWidth - lineWidth)` (right).

### Scale conversion from legacy UIText

`UIText` uses a 32px glyph grid. `UISdfText` uses cell-pixel metrics (~18–25
cell px per character). To get equivalent visual size, multiply the legacy
scale by approximately **2.5**:

```typescript
// UIText at scale 0.5 ≈ UISdfText at scale 1.25
// UIText at scale 0.4 ≈ UISdfText at scale 1.0
```

### `spawnButton` with SDF text

```typescript
UI.spawnButton(w, h, color, onClick, {
    sdfText: true,
    text: 'Label',
    textScale: 0.4 * _uiScale,  // legacy scale — auto-converted ×2.5
});
```

The `sdfText` option causes `spawnButton` to create a `UISdfText` child
instead of `UIText`, with internal scale multiplication.

### Outline and shadow

```typescript
title.setOutline(0.12, [0.1, 0.05, 0, 1]);   // width in SDF units, RGBA color
title.setShadow(2, 3, [0, 0, 0, 0.5], 1);      // offsetX, offsetY, color, blur
```

Outline renders as a band around the edge in the fragment shader. Shadow
is a second `drawPass` with an offset model matrix and wider outline blur.

---

## 8. COMMON PITFALLS

| Symptom | Cause | Fix |
|---|---|---|
| **No text visible, magenta rect appears** | SDF pipeline broken; green quads with solid shader confirm vertex pipeline | Proceed to shader/texture diagnostics |
| **No text, green solid quads visible** | Fragment shader outputs transparent; check texCoord attribute location | `bindAttribLocation` before `linkProgram`; check `shader.attr.texCoord >= 0` |
| **All black quads** | `texture2D` returns 0 | Atlas UVs sample empty background: verify UV range maps to glyph data region; check for `1 - g.y/atlasH` inversion bug |
| **VertexCount = 0 after construction** | Metrics not loaded | Ensure `game.getSdfFont(name)` is available synchronously; `initSdfFonts` runs in `loadResources()` |
| **Text squashed vertically** | `textHeight` changed after vertex loop | Compute glyph extent *before* building vertex data, not after |
| **Glyphs rendered upside-down** | UV Y-axis inverted | Use `g.y/atlasH` (no `1 -`); this engine uses `texImage2D` without `UNPACK_FLIP_Y` |
| **Glyphs on one pixel row** | `lineIndex` not applied | `gy += lineIndex * lineHeight * scale` in `_buildGlyphBuffer` |
| **Text rendered ~3× too small** | Unit mismatch: old font-size units vs new cell-pixel units | Multiply scale by ~2.5 when migrating from `UIText` |
| **Lines overlap (no vertical offset)** | `pg.y` stored but ignored | `gy` must include `pg.y * lineHeight * scale` |
| **Justification has no effect** | Justify block runs before perLine is fully collected | Place justify block after the `\n`/char loop, before `textWidth` assignment |
| **Atlas has all values = 1** | `maxDist` too large relative to glyph size | Use `maxDist = GLYPH_SIZE` (64), not canvas half-diagonal |
| **Atlas has no glyph data** | BBox detection threshold wrong | Check `findBbox` — look for `byte ≠ 1` (not `byte > 128`) |
| **Title clipped at top of screen** | `textHeight` too large, `gy` negative | Compute `textHeight` from glyph extent, not `lineHeight`; fix anchor to `mid-center` |

---

## 9. DEBUGGING PIPELINE

Systematically isolate each stage:

1. **Check `vertexCount`**: `console.log` in `rawDraw` — if 0, text was never built
2. **Check attribute locations**: `shader.attr.texCoord` should be ≥ 0 (not -1)
3. **Swap to solid shader with glyph buffer**: green quads → vertex pipeline works
4. **Swap to solid shader with full-bounds quad**: confirms `modelMatrix()` is correct
5. **Hardcode fragment output to solid color**: confirms fragment shader executes
6. **Output `vTexCoord` as color**: confirms varying interpolation (red/green gradient per glyph)
7. **Sample atlas at fixed `vec2(0.5, 0.5)`**: confirms texture binding works globally
8. **Output `texture2D(texSampler, vTexCoord).r` as grayscale**: glyph shapes should appear bright if UVs map correctly
9. **Check `gl.getError()`** around `drawArrays`: `0x0501` = INVALID_VALUE, `0x0502` = INVALID_OPERATION
10. **Check `CURRENT_PROGRAM`**: `gl.getParameter(gl.CURRENT_PROGRAM) === shader.program`
