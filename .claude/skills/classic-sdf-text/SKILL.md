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
string per frame (plus optional passes for shadow and glow).

`UISdfText` extends `SdfText` with word-wrapping (pixel-width driven, not
character-count), explicit `\n` line breaks, and per-line justification
(`left` / `center` / `right`).

All data (metrics + atlas texture) is loaded synchronously during
`loadResources()` — no runtime `fetch`.

### Key files

| File | Role |
|------|------|
| `scripts/make-font-atlas.mjs` | Build-time SDF atlas + metrics JSON generator |
| `src/classic/sdfText.ts` | `SdfText` class: glyph layout, vertex buffer, `rawDraw` |
| `src/classic/ui.ts` (UISdfText) | `UISdfText` class: word-wrap, justify, height/layout |
| `src/shaders/sdf.vert` | Vertex shader: `texCoord` varying passthrough |
| `src/shaders/sdf.frag` | Fragment shader: derivative AA, weight, gamma, outline, alpha comp |
| `src/classic/utils.ts` | `initSdfFonts()` — loads metrics JSON at startup, warns if `spread` missing |
| `src/classic/state.ts` | `game.sdfFonts`, `game.getSdfFont()`, scissor-test disable |
| `public/manifest.json` | `"sdfFonts": [{ "name": "...", "metrics": "..." }]` + shader uniforms |

---

## 2. BUILD-TIME ATLAS GENERATOR

`scripts/make-font-atlas.mjs` uses `@napi-rs/canvas` (devDependency only) to
rasterise a `.ttf`/`.otf` file and produce a grayscale SDF texture atlas.

### Key constants

| Constant | Value | Meaning |
|---|---|---|
| `FONT_CELL_SIZE` | `GLYPH_SIZE * 0.4` = 25.6 | Font size in cell pixels (engine unit) |
| `PAD` | 2 | Texel gutter between glyphs (prevents LINEAR filter bleed) |
| `SS` (supersampling factor) | 12 | High-res render multiplier per cell px |
| `spread` (default) | 4 | SDF distance-field spread in cell pixels |
| `GLYPH_SIZE` | 64 | Preserved for metric compatibility |

### Algorithm

**Per-glyph:**
1. Size the raster: `cellW = ceil(inkW) + 2·(spread + pad)`, `source = cellW · SS`
2. Render at `renderSize = FONT_CELL_SIZE · SS` px, `textBaseline='alphabetic'`, `textAlign='left'`
3. Build inside/outside binary masks from alpha channel
4. Compute squared-distance transform: separable Felzenszwalb EDT (`dt1d` / `edt2d`) — O(n)
5. Point-sample at cell centres, normalize to `[0..255]` with `maxDist = spread · SS`
6. Detect `.notdef`/blank via bitmap hash comparison against known-absent code points

**Atlas:**
- Shelf-pack cropped glyph cells into power-of-two atlas (typically 512² or 1024²)
- `PAD=2` gutter between glyphs; background is `#000` (far-outside)
- Packer retries with `size *= 2` on overflow; hard-fails at `--max-size` (default 4096)

**Caching:**
- Key = `sha256(font bytes + charset + SS + spread + glyphSize + PAD)`
- Stored next to outputs as `<name>-sdf.sig`; cache hit skips regeneration
- Cache invalidates itself if `.png` or `.json` output files are missing

**CLI:**
```
node scripts/make-font-atlas.mjs <font.ttf> [--family name] [--ss 12] [--spread 4]
    [--max-size 4096] [--charset groups] [--no-cache]
```

### Charset groups

Charset is specified via `--charset` (default: `all`). Named groups can be combined with `+` and excluded with `-`:

| Group | Glyphs | Contents |
|---|---|---|
| `ascii` | 95 | `0020-007E` |
| `latin1` | 96 | `00A0-00FF` accented names, `¡¿«»°±×÷£¥¢§©®µ¶·` |
| `punct` | 26 | curly quotes, en/em dash, ellipsis, bullet, prime, dagger |
| `supsub` | 28 | `⁰¹²³⁻ⁿ ₀₁₂` formulas |
| `fractions` | 16 | `½ ⅓ ¼ ⅔ ⅛` health/stack fractions |
| `currency` | 25 | `€ ₽ ₹ ₩ ¤ ƒ` shops/economy |
| `roman` | 32 | `Ⅰ Ⅱ Ⅲ Ⅳ` tiers |
| `arrows` | 112 | `← ↑ → ↓ ↔ ⇧ ⇥ ↵` movement, tooltips |
| `math` | 36 | `≈ ≠ ≤ ≥ ∞ √ ∑ ∆ ⊕` stat displays |
| `box` | 128 | `┌─┬─┐ │ ├─┼─┤ └─┴─┘` frames, panels |
| `blocks` | 29 | `█ ▌ ▄ ▀` progress bars (excludes ░▒▓ dither) |
| `geometric` | 96 | `■ ▲ ● ◆ ◇` markers, minimap icons |
| `symbols` | 84 | `☠ ☢ ⚠ ⚡ ♔♕ ♠♥♦♣ ⚀-⚅ ♪♫` game icons |
| `dingbats` | 44 | `✓ ✔ ✗ ✘ ❤ ➔` toggles, rarity, lives |
| `enclosed` | 30 | `① ➀ ❶` numbered lists |
| `keys` | 13 | `⌘ ⌥ ⌃ ⏎ ⌫ ⌦ ␣` key prompts |
| `greek` | 50 | `α β γ Δ Σ Ω π μ` stats/formulas |

Default `all` produces ~935 glyphs on a 2048² atlas. Characters not covered by the
font are silently dropped (detected via `.notdef` bitmap hash matching).

### Metrics JSON format

```json
{
  "name": "dejavusans",
  "family": "DejaVuSans",
  "atlasSize": [2048, 2048],
  "glyphSize": 64,
  "spread": 4,
  "baseline": 19.968,
  "lineHeight": 33.28,
  "glyphs": {
    "A": { "x": 108, "y": 77, "w": 30, "h": 31, "xOffset": -6, "yOffset": -25.968, "xAdvance": 17.512 }
  }
}
```

All numeric quantities are in **cell pixels** — the universal unit for SDF
text metrics (`FONT_CELL_SIZE = 25.6`). Floats are rounded to 3 decimal places.

`spread` is required — `initSdfFonts()` warns if absent (stale atlas).

---

## 3. COORDINATE SYSTEM

Everything is in **cell pixels**, multiplied by `_scale` to get screen pixels.

| Quantity | Computation |
|---|---|
| Glyph top in local coords | `gy = baseline * scale + yOffset * scale` |
| Glyph bottom | `gy + h * scale` |
| Line offset (multi-line) | `gy += lineIndex * lineHeight * scale` |

### `textHeight` vs `_layoutHeight`

The SDF cell includes `spread + PAD` margins around the visible ink:

- `textHeight` = full cell height including spread margins (used for vertex
  coordinate normalization and `modelMatrix()` scale).
- `_layoutHeight` = `textHeight - 2·(spread + PAD)·scale` → visible ink
  height. Returned by `UISdfText.get height()` so the UI anchor system
  (`setChildrenPos`) uses visible ink bounds, not padded cell bounds.

### `textWidth` vs `_layoutWidth`

- `textWidth` = maximum rendered line width (used normally).
- `_layoutWidth` = column width for justification. Set to `maxWidth` only
  when `justify !== 'left'`. When set, `textWidth` is replaced with the
  column width so the text block spans the full intended column — center
  and right alignment then shift glyphs into the correct position.

### `advanceFor(ch)`

Shared by `_buildGlyphBuffer`, word-wrap measurement, and layout. Returns:
- Known glyph → `g.xAdvance`
- Space → space advance (or fallback)
- Tab (`\t`) → 4 × space advance
- Unknown → `glyphSize * 0.5`

Local → screen transform via `modelMatrix(offset?)`:
```typescript
translate(position + offset) * scale(textWidth, textHeight, z)
```

The optional `offset` parameter is folded into the translation **before** the
scale, ensuring shadow/glow offsets are in screen pixels, not scaled by
`textWidth/textHeight`.

### UV mapping

```typescript
ux0 = g.x / atlasW;          // left edge of glyph in atlas
ux1 = (g.x + g.w) / atlasW;  // right edge
uy0 = g.y / atlasH;          // top edge — NO "1 -" inversion
uy1 = (g.y + g.h) / atlasH;
```

**Do NOT use `1 - g.y/atlasH`.** This engine does not set
`UNPACK_FLIP_Y_WEBGL`; `texImage2D` puts image row 0 at texture t=0.

---

## 4. SHADERS

### Vertex (`sdf.vert`)

```glsl
attribute vec4 vertexPos;     // location 0 (bound before linkProgram!)
attribute vec2 texCoord;      // location 1
varying mediump vec2 vTexCoord;
```

### Fragment (`sdf.frag`)

#### Uniforms

| Uniform | Type | Meaning |
|---|---|---|
| `texSampler` | `sampler2D` | SDF atlas texture |
| `color` | `vec4` | Fill color (a = opacity) |
| `outlineColor` | `vec4` | Outline color (a = opacity) |
| `outlineWidth` | `float` | Outline width in **cell pixels** |
| `softEdge` | `float` | Manual AA fallback when derivatives unavailable |
| `spread` | `float` | SDF spread from metrics JSON |
| `atlasSize` | `vec2` | Atlas dimensions for UV→px conversion |
| `weight` | `float` | Faux bold/light: `edge = 0.5 - weight` |
| `gamma` | `float` | Perceptual gamma on final alpha |

#### Derivative AA

```glsl
#ifdef GL_OES_standard_derivatives
    vec2 uvPx = fwidth(vTexCoord) * atlasSize;
    float pxRange = (2.0 * spread) / max(length(uvPx), 1e-5);
    w = clamp(0.5 / pxRange, 0.0001, 0.5);
#else
    w = softEdge;
#endif
```

`OES_standard_derivatives` is already requested at `state.ts:177`. The
`softEdge` uniform is a fallback only.

#### Alpha compositing

```glsl
float fillAlpha = alpha * color.a;
float outAlpha = outlineAlpha * outlineColor.a;
result.a = outAlpha + fillAlpha * (1.0 - outAlpha);
```

Both `color.a` and `outlineColor.a` are respected. Outline width in cell
pixels is converted to SDF units internally as `outlineWidth / (2.0 * spread)`.

#### Draw passes

Shadow: second `drawPass` with offset model matrix and `shadowBlur` as outline width.
Glow: zero-offset pass with `glowRadius` as outline width.
Main: primary fill with `outlineWidth` as outline width.

All passes go through `SdfText.rawDraw()` → `drawPass()`.

---

## 5. GL REQUIREMENTS

### `bindAttribLocation` before `linkProgram`

In `initShaderProgram()` (`src/classic/utils.ts`), call
`gl.bindAttribLocation(program, i, attributes[i])` **before**
`gl.linkProgram()`. AMD drivers (notably Radeon R9 200) may drop attribute
locations that appear unused to the linker, returning `getAttribLocation`
as `-1`.

### Sampler binding order

```typescript
gl.activeTexture(gl.TEXTURE0);
gl.bindTexture(gl.TEXTURE_2D, atlasTexture.texture);
gl.uniform1i(shader.unif.texSampler, 0);  // AFTER texture is bound
```

### LINEAR filtering on SDF atlas texture

The atlas texture must use `gl.LINEAR` (min + mag) — `SdfText`'s constructor
overrides this after the engine default `gl.NEAREST`:

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

### `_bufferDirty` flag

`bufferData` is only called when the glyph buffer changed (after `setText`),
not on every frame. Re-rendering unchanged text skips the GPU upload.

### Scissor-test clipping

Used by `UIContainer.clipChildren` to clip overflowing children (scroll,
word-wrap overflow):

- `state.ts` disables `SCISSOR_TEST` at frame start
- `UIContainer.setChildrenPos()` writes `_uiClipRect` on all children when `clipChildren` is set
- `SdfText.rawDraw()` checks `_uiClipRect`: enables scissor with flipped-Y
  rect, draws, disables scissor — all within the single draw call
- No state leak to other drawables

---

## 6. MANIFEST INTEGRATION

### Font metrics

```json
{
  "sdfFonts": [
    { "name": "dejavusans", "metrics": "/res/dejavusans-sdf.json" }
  ],
  "textures": [
    { "name": "dejavusans-sdf", "src": "/res/dejavusans-sdf.png" }
  ]
}
```

Loaded synchronously during `loadResources()`:
```typescript
this.sdfFonts = await initSdfFonts(this.manifest.sdfFonts || [], ...);
this.metrics = this.game.getSdfFont(atlasName);  // in SdfText constructor
```

`initSdfFonts` warns if any loaded metrics are missing the `spread` field.

### Shader uniforms

```json
{
  "name": "sdf",
  "vertex": "/shaders/sdf.vert",
  "fragment": "/shaders/sdf.frag",
  "attr": ["vertexPos", "texCoord"],
  "unif": [
    "modelMatrix", "cameraMatrix", "projectionMatrix",
    "texSampler", "color", "outlineColor", "outlineWidth",
    "softEdge", "spread", "atlasSize", "weight", "gamma"
  ]
}
```

Adding a new uniform requires updating **both** the manifest AND the shader
source — `shader.unif.X` is derived from `getUniformLocation`, which returns
`null` for undeclared names in the manifest.

---

## 7. UISdfText FEATURES

### Word wrapping

`_wrapAtPixelWidth(str, maxPx)` — instance method using `this.advanceFor()`:

- Splits on `\n` first (hard line breaks)
- Within each segment, word-wraps on spaces (ASCII space only; NBSP U+00A0
  does NOT break)
- Measures each word's pixel width via `this.advanceFor() * scale`
- Pushes words to successive lines when `linePx + gap + wordPx > maxPx`

`maxWidth` argument to `spawnSdfText()` is in **screen pixels**.

### Justification

```typescript
text.setJustify('center');  // 'left' (default) | 'center' | 'right'
```

Justification references the **column width** (`_layoutWidth = this.maxWidth`)
rather than the longest rendered line. This means single-line text with
`setJustify('center')` correctly centers within the column. Left-justify
keeps the natural rendered width (`_layoutWidth = 0`).

Per-line: computes line width, then shifts glyph positions by
`(columnWidth - lineWidth) / 2` (center) or `(columnWidth - lineWidth)` (right).
The `textWidth` is set to the column width, and `modelMatrix()` scales local
coordinates so empty space to the column edge renders as transparent.

### Scale conversion from legacy UIText

```typescript
// UIText at scale 0.5 ≈ UISdfText at scale 1.25
// UIText at scale 0.4 ≈ UISdfText at scale 1.0
```

`spawnButton` with `sdfText: true` auto-converts:
```typescript
UI.spawnButton(w, h, color, onClick, {
    sdfText: true,
    text: 'Label',
    textScale: 0.4 * _uiScale,  // legacy scale — auto-converted ×2.5
});
```

### Outline, shadow, glow

```typescript
title.setOutline(1, [0.1, 0.05, 0, 1]);        // 1 cell px, RGBA
title.setShadow(2, 3, [0, 0, 0, 0.5], 2);       // offsetX, offsetY, color, blur
title.setGlow(1.5, [1, 0.8, 0, 0.3]);           // radius, color
text.setWeight(-0.08);                            // faux bold
text.setGamma(1.4);                               // perceptual gamma
```

All dimensional values (outline width, shadow offset/blur, glow radius) are in
**screen pixels** — the shader converts to SDF units internally using `spread`.

### `snapToPixel`

When `snapToPixel = true` and `ignoreCam = true`, `modelMatrix()` rounds the
translation to whole device pixels. Eliminates sub-pixel blur for UI text
positioned through fractional `_uiScale`.

### Children with `_uiFixed`

`UIElement._uiFixed = true` exempts a child from `UIContainer.setChildrenPos()`
repositioning. Used for scrollbar track/thumb — they manage their own positions
in the per-frame update callback.

---

## 8. COMMON PITFALLS

| Symptom | Cause | Fix |
|---|---|---|
| **No text visible, vertexCount=0** | Metrics not loaded | `game.getSdfFont(name)` available synchronously |
| **All SDF text shifted up** | `_layoutHeight` not computed; anchor uses padded cell height | Set `_layoutHeight = textHeight - 2·(spread+2)·scale` |
| **Left-justified button text gone / title off-center** | `_layoutWidth` forced `textWidth = columnW` for all text | Guard `_layoutWidth` to `justify !== 'left'` only |
| **Justification has no visible effect** | Uses longest rendered line width, not column width | Use `_layoutWidth = maxWidth` as column reference |
| **Camera zooms when scrolling panel** | `game.mouseWheel` is global | Add `_scrollContainers` guard in camera controller |
| **Scrollbar moves with content** | `setChildrenPos` applies `scrollY` to all children | Set `_uiFixed = true` on scrollbar children |
| **Atlas LINEAR filter bleeds adjacent glyphs** | Zero gutter between glyphs | `PAD=2` in generator |
| **Shelf packer silently overflows** | No overflow check | Retry with `size *= 2`; hard-fail at `--max-size` |
| **Shadow offset scaled by text size** | Post-translate after `S(textWidth, textHeight)` | Fold offset into `modelMatrix(offset?)` before scale |
| **Wrap vs layout advance mismatch** | Different fallback values in `measureWord` vs `_buildGlyphBuffer` | Use shared `advanceFor()` |
| **Box-drawing strokes thin at spread=4** | Interior field peaks at ~152/255 | Stay above `weight > 0.09` for box chars |
| **Dither chars (░▒▓) render as mush** | SDF can't represent high-frequency checkerboards | Excluded; gate behind `--charset +dither` |
| **Atlas .sig cache misses pointlessly** | Cache key doesn't account for missing outputs | Cache hit also verifies `.png`/`.json` exist |
| **Generated atlas at 2048² instead of 1024²** | Cell sizes grew with spread=4 | Normal; ~340KB PNG at 2048² is acceptable |
| **Text squashed vertically** | `textHeight` changed after vertex loop | Compute glyph extent before building vertex data |
| **Glyphs rendered upside-down** | UV Y-axis inverted | `g.y/atlasH` (no `1 -`); no `UNPACK_FLIP_Y` |
| **Glyphs on one pixel row** | `lineIndex` not applied | `gy += lineIndex * lineHeight * scale` |
| **Text rendered ~3× too small** | Unit mismatch | Multiply scale by ~2.5 when migrating from `UIText` |
