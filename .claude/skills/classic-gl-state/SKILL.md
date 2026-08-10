---
name: classic-gl-state
description: >
    WebGL state management patterns for classic-wgl. Covers frame-start reset
    checklist, GPU pipeline flush, Text element FBO hygiene, draw-call state
    isolation, and debugging visual artifacts from leaked GL state.
    Use when diagnosing black flickering, terrain corruption, missing depth
    writes, wrong viewport, or FBO-bound-after-Text-setText issues.
    Trigger phrases: "GL state", "viewport wrong", "depthMask leak",
    "FBO not restored", "setText leaves FBO", "black flickering",
    "pipeline flush", "drawArrays 0 0", "terrain artifacts", "glitch".
compatibility: WebGL 1.0, gl-matrix. Canvas is single <canvas>, no Web Workers
    touching the GL context.
metadata:
    author: classic-wgl
    version: '0.1'
allowed-tools: Read, Grep, Bash, Edit
---

## Scope

This skill covers WebGL state lifecycle: what must be reset every frame,
what `Text.setText` / `appendText` modifies and must restore, how to debug
state-leak visual artifacts, and the minimal GPU pipeline flush pattern.

---

## 1. FRAME-START RESET CHECKLIST

The draw loop MUST reset these at the start of every frame. Never assume
the previous frame's last drawable left clean state:

```typescript
gl.bindFramebuffer(gl.FRAMEBUFFER, null);
gl.viewport(0, 0, gl.drawingBufferWidth, gl.drawingBufferHeight);
gl.clearColor(0.0, 0.0, 0.0, 1.0);
gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
gl.enable(gl.BLEND);
gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
gl.depthFunc(gl.LEQUAL);
gl.depthMask(true);
```

| State | Reset to | Why it leaks |
|:---|:---|:---|
| Framebuffer | `null` (default) | `Text.setText` binds internal FBO |
| Viewport | `drawingBufferWidth × drawingBufferHeight` | `Text.appendText` sets to texture size |
| Depth function | `LEQUAL` | Debug overlays set `ALWAYS` |
| Depth mask | `true` | Sprites set `false` on normal pass (§4) |
| Blend | `SRC_ALPHA, ONE_MINUS_SRC_ALPHA` | Text rendering may change blend func |

Use `gl.drawingBufferWidth/Height` for viewport, NOT `canvas.width/height`.
On high-DPI displays (devicePixelRatio > 1), the drawing buffer can be
larger than the CSS canvas size. Mismatch causes uncleared framebuffer
regions → flickering.

---

## 2. GPU PIPELINE FLUSH PATTERN

Creating `UIText` elements at init time allocates internal framebuffers and
calls `setText` → `appendText` which binds/modifies GL state. After
initialization completes and the game loop starts, the GPU pipeline may
be in an undefined state that produces terrain rendering artifacts.

The fix is a minimal no-vertex draw call at the END of every frame (when
debug mode is OFF — debug mode's own draws already flush the pipeline):

```typescript
// At end of draw loop, in the else branch of if (debugFootprints):
gl.drawArrays(gl.LINE_STRIP, 0, 0);  // 0 vertices, still executes a pipeline flush
gl.depthFunc(gl.LEQUAL);
gl.depthMask(true);
```

**Why this works:** `drawArrays` with 0 vertices produces no visible output
but forces the GPU to synchronize its internal state machine. Any stale
attribute bindings, buffer references, or shader state from the init phase
are flushed out before the next frame begins.

**When NOT needed:** When debug mode is ON, the debug code already
executes multiple draw calls (footprint LINE_LOOPs, anchor X markers,
compass rose lines) which perform the same flush implicitly.

---

## 3. TEXT ELEMENT FBO HYGIENE

`Text.setText(str)` and the underlying `appendText(str)` method use
render-to-texture to composite glyphs. They MUST restore GL state:

### `Text.setText` — required restores

```typescript
setText(str: string): void {
    this.gl.bindFramebuffer(this.gl.FRAMEBUFFER, this.frameBuffer);
    this.gl.clear(/* ... */);
    this.appendText(str);
    this.gl.bindFramebuffer(this.gl.FRAMEBUFFER, null);  // RESTORE
    this.gl.viewport(0, 0, this.gl.drawingBufferWidth, this.gl.drawingBufferHeight);  // RESTORE
}
```

### `appendText` — what it modifies

```typescript
appendText(str: string): void {
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.frameBuffer);  // binds internal FBO
    gl.enable(gl.BLEND);                                    // changes blend state
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);    // (same as default, but sets anyway)
    gl.viewport(0, 0, textureWidth, textureHeight);         // changes viewport!
    // ... renders glyphs ...
    // DOES NOT restore FBO or viewport
}
```

`appendText` is called by `setText` and by `UIText._recalculateTextElement`.
Both callers must ensure FBO + viewport restore. The `Text.setText` method
handles this at its end. If you call `appendText` directly from other paths,
you must restore FBO + viewport yourself.

### Common symptoms of leaked FBO/Viewport

| Symptom | Cause |
|:---|:---|
| Whole screen black or garbled | `gl.clear` operates on text's internal FBO instead of main canvas |
| Terrain draws on tiny portion of screen | Viewport set to 32×32px (text texture size) |
| Flickering that stops when any debug draw runs | GPU pipeline in stale state; any draw flushes it |

---

## 4. STATE ISOLATION PER DRAWABLE

Every `rawDraw()` implementation must fully set up its own GL state.
Never assume:

- The correct shader program is bound
- Vertex attributes point to your buffer
- Texture units are at the expected slots
- The active texture unit is `TEXTURE0`
- Depth/blend/blit state is as you expect

### Debug overlay hygiene

Debug overlays (compass, XYZ axes, footprint outlines, anchor X markers)
must:

1. Bind the `solid` shader (or their own shader)
2. Set all matrix uniforms (`projectionMatrix`, `cameraMatrix`, `modelMatrix`)
3. Set `depthFunc(ALWAYS)` and `depthMask(false)` so they render on top
4. **Restore** `depthFunc(LEQUAL)` and `depthMask(true)` after ALL debug draws

```typescript
// In debug block:
gl.depthFunc(gl.ALWAYS);
gl.depthMask(false);
// ... draw debug overlays ...
gl.depthFunc(gl.LEQUAL);
gl.depthMask(true);
```

### Ghost pass state

`IsoSprite.rawDraw()` has a ghost pass (ALWAYS, depthMask=false) followed
by a normal pass (LEQUAL, depthMask=false — sprites don't write depth so
sprite-sprite ordering is by renderList sort). The depth state is reset at
frame start, so per-sprite state changes don't accumulate across frames.

### `bindAttribLocation` before `linkProgram` (AMD driver workaround)

AMD drivers (especially Radeon R9 200 series) may drop attribute locations
at link time if the linker determines they're unused — even when the
attribute is read by a varying that the fragment shader consumes. This
returns `getAttribLocation` as `-1` and causes `vTexCoord` to be `(0,0)`
in every fragment.

**Fix:** call `gl.bindAttribLocation(program, index, name)` for every
attribute in the manifest BEFORE `gl.linkProgram()`:

```typescript
for (let i = 0; i < attributes.length; i++) {
    gl.bindAttribLocation(shaderProgram, i, attributes[i]);
}
gl.linkProgram(shaderProgram);
```

This is applied in `initShaderProgram()` in `src/classic/utils.ts`.

### Sampler uniform ↔ texture unit binding order

When binding a sampler uniform:

```typescript
gl.activeTexture(gl.TEXTURE0);
gl.bindTexture(gl.TEXTURE_2D, texture);   // texture FIRST
gl.uniform1i(samplerLoc, 0);              // sampler SECOND
```

Some drivers silently ignore a sampler assignment if the texture isn't
already on the target unit when `uniform1i` is called. The `SdfText.rawDraw()`
method uses this order.

### SDF texture filtering override

The engine's `loadTexture()` sets `gl.NEAREST` min/mag filters on all
textures. SDF font atlases MUST use `gl.LINEAR` for `smoothstep` to produce
anti-aliased edges. The `SdfText` constructor overrides the filter after
retrieving the texture:

```typescript
this.gl.bindTexture(this.gl.TEXTURE_2D, this.atlasTexture.texture);
this.gl.texParameteri(this.gl.TEXTURE_2D, this.gl.TEXTURE_MIN_FILTER, this.gl.LINEAR);
this.gl.texParameteri(this.gl.TEXTURE_2D, this.gl.TEXTURE_MAG_FILTER, this.gl.LINEAR);
```

### Interleaved vertex buffer for text

`SdfText` uses an interleaved buffer: `[posX, posY, uvX, uvY]` per vertex,
6 vertices per quad, stride = 16 bytes. The attribute pointers for the
`Sdf` shader (which expects `vertexPos` at location 0 and `texCoord` at
location 1) are:

```typescript
gl.vertexAttribPointer(shader.attr.vertexPos, 2, gl.FLOAT, false, 16, 0);
gl.vertexAttribPointer(shader.attr.texCoord, 2, gl.FLOAT, false, 16, 8);
```

When binding this buffer to the `solid` shader (which only has `vertexPos`
at location 0), use the same stride and offset — the solid shader ignores
the UV data that follows each position pair.

---

## 5. DEBUGGING STATE LEAKS

### Diagnostic approach

Wrap suspicious code paths with `gl.getError()` to catch the first GL error:

```typescript
const err = gl.getError();
if (err) console.warn(`GL error 0x${err.toString(16)} at ${location}`);
```

Common error codes:

| Code | Name | Meaning |
|:---|:---|:---|
| `0x0500` | INVALID_ENUM | Wrong argument to a GL function |
| `0x0502` | INVALID_OPERATION | Operation illegal in current state |
| `0x8CD6` | FRAMEBUFFER_INCOMPLETE_ATTACHMENT | FBO attachment invalid |
| `0x8CD7` | FRAMEBUFFER_INCOMPLETE_MISSING_ATTACHMENT | No image attached to FBO |

### Binary search for state corruption

When a visual artifact appears:
1. Disable all debug overlays — does it persist? Yes → init-time leak
2. Add `drawArrays(LINE_STRIP, 0, 0)` at various points — where does it fix?
3. Check `Text.setText` restore — add `gl.getError()` before/after
4. Check frame-start reset — is every state listed in §1 being reset?

---

## 6. TEXTURE / FBO LIFECYCLE

`UIText` created via `UI.spawnText()` allocates:
- `targetTexture` (WebGLTexture, `texImage2D` with `null` data)
- `frameBuffer` (WebGLFramebuffer, with `targetTexture` attached to `COLOR_ATTACHMENT0`)

When `setMaxCharSize` resizes the texture, it calls `texImage2D` to reallocate.
The FBO attachment is NOT re-created — it still references the same texture.
This is correct because `texImage2D` resizes the existing texture object.

When the entity is destroyed via `entity.cleanup()`, the texture and
framebuffer are NOT automatically freed. The caller must explicitly delete
them via `gl.deleteTexture` and `gl.deleteFramebuffer`. Currently, this is
not done for UI elements — they persist for the lifetime of the page.

---

## 7. `getComponent` AND SUBCLASSES

`entity.getComponent(IsoSprite)` checks `component.constructor === IsoSprite`.
This does NOT match `IsoAgent` instances (which extend `IsoSprite`). When
iterating entities by a base class, use `instanceof`:

```typescript
let iso: IsoSprite | null = null;
for (const c of entity.components) {
    if (c instanceof IsoSprite) { iso = c as IsoSprite; break; }
}
```

## 8. DEPTH TEST DISABLED FOR UI (Rust-port gotcha)

`begin_frame` in `crates/classic-gfx/src/lib.rs:~428` sets `depthFunc(LEQUAL)` and
`depthMask(true)` but **does not** call `glEnable(DEPTH_TEST)`. The
`draw_tilemap` and `draw_iso_sprite` functions both `glEnable(DEPTH_TEST)`
inside their unsafe blocks then `glDisable(DEPTH_TEST)` at the end, leaving
depth test **off** for the remainder of the frame.

**Consequence**: the entire UiRect + SdfText phase runs with depth test
disabled. UI layering relies **exclusively** on draw order (z-sort) + alpha
blending, not depth occlusion.

**Do NOT globally enable DEPTH_TEST** in `begin_frame` to "fix" UI layering.
The tilemap renders at `z=20000` and UI at `z=-1000`. Under the orthographic
projection (`near=-10000, far=10000`), these map to wildly different depth
values, and with LEQUAL the UI would get completely depth-rejected.

Instead, control layering via explicit z values and the single z-sorted
render list (SDF text merged into the main loop — see classic-sdf-text).
Lower z (more negative) = drawn later = on top.
