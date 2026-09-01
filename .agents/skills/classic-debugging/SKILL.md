---
name: classic-debugging
description: >
    Debugging tooling and diagnostic workflows for classic-wgl's Rust port.
    Covers CLASSIC_LOG channel-gated logging, log levels and macros,
    channel reference, golden trace diffing, state dump inspection,
    headless CI debugging, and a step-by-step debugging playbook.
    Trigger phrases: "CLASSIC_LOG", "cl_info", "cl_debug", "cl_trace",
    "cl_scope", "cl_every", "channel logging", "golden diff",
    "state dump", "headless", "debug playbook", "frame counter".
compatibility: log 0.4, env_logger 0.11, web_sys::console
metadata:
    author: classic-wgl
    version: '1.0'
allowed-tools: Read, Grep, Glob, Bash(git *), Edit
---

# Skill: classic-debugging

Definitive reference for diagnosing problems in `classic-wgl`. Covers logging
infrastructure (`CLASSIC_LOG`), golden-trace comparison, state dump inspection,
headless CI runs, and a structured playbook for common issues.

For macro-level details and adding new channels, see
`crates/classic-core/src/instrument.rs`. This document focuses on diagnostic
workflows and runtime tooling.

---

## 1. CLASSIC_LOG Quick Reference

### Grammar

```
CLASSIC_LOG=ui,collision=trace          # ui=info (default), collision=trace
CLASSIC_LOG=all=info,gfx=trace,-nav     # everything info, gfx trace, nav off
CLASSIC_LOG=help                        # prints channel list + grammar, continues
CLASSIC_LOG=<unset>                     # all channels at Info (default gate)
```

Tokens are comma-separated. Each token is `chan`, `chan=LEVEL`, `-chan`, `all`,
or `all=LEVEL`. Order matters: `all` directives apply first, then `-all`,
then per-channel overrides. Unknown channel names emit a `log::warn!` telling
you to run `CLASSIC_LOG=help` (it does not enumerate the names).

### All 25 Channels

| Channel | Description |
|---|---|
| `frame` | Frame begin/end, FPS, delta timing |
| `input` | Raw input events (key presses, mouse/wheel, cursor lock) |
| `ui` | UI element creation, hover, enabled state, draw submission |
| `layout` | Layout tree walks, dirty propagation, anchor resolution |
| `collision` | Quadtree insert/query, collider registration, enter/exit events |
| `click` | Click dispatch, priority prescan, consumed-click guards |
| `render` | Render-list construction, sort order, draw-kind selection |
| `gfx` | Shader binds, texture upload, buffer allocation, draw calls |
| `glstate` | GL state transitions (depth, blend, cull, viewport) |
| `text` | SDF glyph buffer rebuilds, font load, text GpuCache invalidations |
| `iso` | Iso coordinate transforms, depth calculations, sprite occlusion |
| `nav` | Nav mesh queries, tile walkability checks |
| `terrain` | Terrain generation/upload summary (alias `gen`) |
| `path` | A* pathfinder execution (open/closed sets, cost) |
| `ecs` | Entity spawn/despawn, component insert/remove, world queries |
| `state` | State serialization (dump/load), registry lookups |
| `editor` | Editor mode transitions, tool activation, tile painting |
| `asset` | Texture/manifest loading, shader compilation |
| `camera` | Camera matrix recalc, zoom/pan, fix-point math |
| `anim` | Animator frame advance, direction lookup |
| `test` | CLASSIC_TEST step execution, assertions |
| `golden` | Golden trace capture, comparison, mismatch reporting |
| `dump` | State-dump file I/O (inlined into `state.json`, no sidecar files) |
| `platform` | Platform backend lifecycle, context creation, swap buffers |
| `guest` | ROM guest runtime: init/update/start, SDK bridge, fuel/memory errors |

### Convenience Aliases

| Alias | Expands to |
|---|---|
| `physics` | `collision` + `click` |
| `draw` / `render-all` | `render` + `gfx` + `glstate` |
| `editor-all` | `editor` + `camera` |
| `anim` / `animation` / `animator` | `anim` |

### Web Equivalent

On wasm, `?classic_log=` query parameter on the page URL serves the same
role as the `CLASSIC_LOG` env var. Example:

```
http://localhost:8080/?classic_log=all=trace,-frame,-render
```

The WebLogger installed in `apps/web/src/lib.rs` sets `log::max_level` to
`Trace` so all channel output reaches the console.

---

## 2. Log Macros

Every macro emits `[fNNNNNN] prefix:\n` with the current frame counter and
the channel name as the log target.

### Level Macros

```rust
cl_error!(Chan::Gfx, "shader compile failed: {err}");
cl_warn!(Chan::Ecs, "unknown component '{name}'");
cl_info!(Chan::Asset, "loaded texture '{name}' {w}x{h}");
cl_debug!(Chan::Ui, "layout dirty, refreshing");
cl_trace!(Chan::Render, "draw sprite '{name}' at z={z}");
```

### Throttle Macros

```rust
cl_every!(Chan::Frame, 60, log::Level::Info, "fps={fps}");  // every 60th frame
cl_first!(Chan::Ui, 120, log::Level::Debug, "size={size:?}"); // first 120 frames
cl_once!(Chan::Gfx, log::Level::Warn, "unusual GL state");    // once ever
```

Note: `cl_every!` and `cl_first!` take `log::Level` (the standard crate enum),
not `instrument::Level`.

### Scope Macro

```rust
let _s = cl_scope!(Chan::Render, "draw_items");
// → [f000042] → draw_items
// ... (scope body) ...
// → [f000042] ⤷ draw_items (124μs)
```

Returns `Option<ClScope>` — `None` when the channel is disabled (zero
allocation). The guard logs elapsed microseconds on drop.

---

## 3. Log Levels and Conventions

| Level | Guidelines |
|---|---|
| `Error` | Unrecoverable errors: missing resources, shader compilation failure, GL context loss |
| `Warn` | Recoverable anomalies: unknown channel name, GL error drain, unexpected-but-handled state |
| `Info` | One-shot lifecycle events: texture load, state load, dump save, FPS report (throttled) |
| `Debug` | State transitions: editor target change, resize, set_enabled, UI tree rebuild |
| `Trace` | Per-frame detail: draw calls, collision queries, glyph buffer rebuilds, individual input events |

**Default gate**: when `CLASSIC_LOG` is unset, all channels are at `Info`
level. This means `cl_info!` works as a drop-in diagnostic tool without
any env-var configuration.

**Noise rule**: Per-frame `Trace`-level logs must use `cl_every!` or
`cl_first!` to avoid flooding the console.

### Typical Diagnostic Env-Var Patterns

```bash
# Click not firing: trace entire dispatch pipeline
CLASSIC_LOG=physics=trace,-frame

# Entity not visible: check ECS + render list
CLASSIC_LOG=ecs=trace,render=trace,-frame

# Layout broken: trace layout tree walk
CLASSIC_LOG=layout=trace,ui=trace,-frame

# Pathfinder not finding route: dump A* state
CLASSIC_LOG=path=trace,nav=debug,-frame

# Texture missing / black screen: trace asset + GFX
CLASSIC_LOG=asset=debug,gfx=trace,glstate=trace,-frame

# Shadows/lighting wrong: isolate sun visibility, drop the UI, A/B the pass
CLASSIC_SHADOW_DEBUG=1 CLASSIC_NO_UI=1        # white = lit, black = occluded
CLASSIC_SHADOWS=0                              # disable the shadow pass entirely
```

`CLASSIC_SHADOW_DEBUG` / `CLASSIC_NO_UI` / `CLASSIC_SHADOWS` are not
`CLASSIC_LOG` channels — they change what is rendered.  See the shadow bring-up
playbook in §8.

---

## 4. Runtime Control

### Native

`CLASSIC_LOG` is read once at `Engine::new()` time via `init_from_env()`.
No runtime toggles exist — the atomic level table is set at startup and
can only be reset in tests via `reset_for_test()`.

When `CLASSIC_LOG` is set, `env_logger`'s max level is bumped to `Trace`
so the channel-gated output passes through. If you use `RUST_LOG`, filter by
the **channel name string** (the log target is `chan_name($chan)`, e.g. `gfx`),
not a Rust module path:

```bash
RUST_LOG=info,gfx=trace CLASSIC_LOG=gfx=trace cargo run -p classic-desktop
```

### Web

`?classic_log=` query param is parsed in `apps/web/src/lib.rs` before
`Engine::new()`. Change the URL and reload to toggle channels.

---

## 5. Golden Trace Diffing

### Overview

Golden traces capture a deterministic JSONL record of every draw call:
model matrices, textures, sort order, camera state, per-kind draw counts.

- **Reference**: `{CLASSIC_GOLDEN_DIR}/baseline.trace.jsonl` (default
  `tests/golden/baseline`; per-scene by setting `CLASSIC_GOLDEN_DIR`)
- **Actual**: `target/classic-test/baseline.actual.trace.jsonl`

Traces are structural, not pixel-based — they compare the logic, not the
GPU output. This means golden trace checks pass identically on any platform
(native, headless EGL, CI).

### Running

```bash
# Compare: fail if actual deviates from baseline
CLASSIC_HEADLESS=1 CLASSIC_FRAMES=60 CLASSIC_TEST=all CLASSIC_GOLDEN=check \
  cargo run -p classic-desktop

# Update: overwrite baseline with current output
CLASSIC_HEADLESS=1 CLASSIC_FRAMES=60 CLASSIC_TEST=all CLASSIC_GOLDEN=update \
  cargo run -p classic-desktop
```

### JSONL Format

One JSON object per line. The first line is the header:

```json
{"tag":"baseline","frame":42,"viewport":[1280.0,720.0],"camera":{...},"counts":{"IsoSprite":3,"Tilemap":1}}
```

Subsequent lines are `TraceItem` records:

```json
{"order":-12.5,"kind":"IsoSprite","name":"tree_01","model":[...16 floats...],"camera_ignored":false,"texture":"tree.png","frame":0.0}
```

### Interpreting Mismatches

Diffs are emitted as `- expected[n]` / `+ actual[n]` line pairs. Common
causes:

| Symptom | Likely Cause |
|---|---|
| `order` values shifted by a constant | Iso sort formula changed (depth bias, tilemap size in sort key) |
| `model` matrix values differ | Camera matrix order change, coordinate transform refactor, fix-point math |
| Item missing from trace | Entity not spawned, `Disabled` component set, render-list filter changed |
| Extra item in trace | New entity added, old one no longer properly excluded |
| `name` changed | `DebugName` component missing or renamed |
| `counts` mismatch (e.g. `IsoSprite: 2` vs `3`) | Extra or missing `IsoSprite` in render list |
| Header-only mismatch (viewport or camera) | CLASSIC_WIDTH/HEIGHT changed, camera init changed |

When updating the baseline (`CLASSIC_GOLDEN=update`), review the `.actual.trace.jsonl`
carefully before committing the new baseline — golden tests are a contract: the
committed baseline is authoritative.

### Pixel Golden (CLASSIC_GOLDEN_PNG=1)

Optional per-pixel comparison against
`{CLASSIC_GOLDEN_DIR}/baseline.png`.  Per-channel tolerance is
`CLASSIC_GOLDEN_TOL` (default 2). Not enabled by default in CI because
software-rasteriser output is version-dependent.

---

## 6. State Dump Inspection

### Triggering

- **F9**: dump `state.json` (entity/components registry dump; tile/nav/height
  data is inlined in `state.json` — no sidecar files)
- **F10**: save the current world as a packed ROM archive (`<entrypoint>.rom`)

### Files Produced

| File | Content |
|---|---|
| `state.json` | Full entity state: `{"entities": {"<name>": {"components": [...]}}}` (tile/nav/height data inlined) |

### Output Directory

- **Native**: `./dump/` (overridable via `CLASSIC_DUMP_DIR`)
- **Web**: triggers a `Blob` download in the browser

### Structure of state.json

```json
{
  "entities": {
    "tilemap": {
      "components": [
        {"type": "Tilemap", "position": [0,0,0], "scale": [1,1,1], "size_x": 20, "size_y": 20, ...}
      ]
    }
  }
}
```

Component field keys are snake_case, matching the Rust field names exactly.
The `"type"` key is emitted first as a convention, but key order is not
load-bearing (the loader is registry-driven).

### Diagnostic Uses

1. **Entity not visible**: check the dumped state for `Disabled` component
   or missing/zeroed `IsoSprite`/`Sprite` component
2. **Tilemap corrupted**: inspect the inlined `Tilemap.data` / `height_data`
   fields in the dumped `state.json`
3. **Navigation broken**: verify the inlined `IsometricNavMesh.data` grid has
   walkable tiles where expected

### CLASSIC_UI_DEBUG

Set `CLASSIC_UI_DEBUG=1` to dump UI entity positions every frame (first 120
frames). Each line is a compact JSON object with elem ID, position, size,
anchor, parent chain, and visibility. Useful when diagnosing UI layout
positioning issues.

---

## 7. Headless CI Debugging

### Key Env Vars

| Var | Effect |
|---|---|
| `CLASSIC_HEADLESS=1` | Surfaceless EGL render path, no window, dynamic libEGL load |
| `CLASSIC_OFFSCREEN=1` | Render to FBO (implied by headless) |
| `CLASSIC_FRAMES=60` | Exit after N frames (prevents infinite loop in CI) |
| `CLASSIC_FIXED_DT=0.0166` | Fixed delta time (auto-set to 1/60 under CLASSIC_TEST) |
| `CLASSIC_WIDTH` / `CLASSIC_HEIGHT` | Force logical viewport dimensions |

### Headless Command

```bash
CLASSIC_HEADLESS=1 CLASSIC_FRAMES=60 CLASSIC_TEST=all CLASSIC_GOLDEN=check \
  cargo run -p classic-desktop
```

### Common Issues

| Symptom | Cause |
|---|---|
| `libEGL.so: cannot open shared object file` | No EGL library in LD_LIBRARY_PATH; use `nix develop` |
| No GL context created | Headless requires a working EGL implementation on the host |
| Golden mismatch in CI but local passes | CI runs a different viewport size or mesa version; check CLASSIC_WIDTH/HEIGHT |
| CI exits with exit code 0 despite mismatch | Golden mode must be `check` (not unset); CI should set `CLASSIC_GOLDEN=check` |

---

## 8. Debugging Playbook

### Click Not Firing

1. Enable `CLASSIC_LOG=physics=trace` to trace collision queries and click
   dispatch end-to-end.
2. Check that the target entity has a `Collider` component and the collider's
   `enabled` flag is `true`.
3. Confirm no parent entity has `Disabled` (disabled propagates down the
   hierarchy).
4. Verify `consumesClick` — if a higher-priority entity consumed the click,
   lower ones never see it.  The `ui_consumed_click` flag gates dispatch.
5. Check that the mouse position correctly maps to iso coordinates (use
   `iso=trace` channel; verify `mouseIsoPos` is within expected tile range).
6. Confirm the entity is in the quadtree — disabled colliders are skipped
   in `begin_frame()`.

### Entity Not Visible

1. Enable `CLASSIC_LOG=ecs=trace,render=trace`.
2. Check for `Disabled` component on the entity or any ancestor.
3. Verify `IsoSprite` / `Sprite` has correct `position` (not off-screen
   or behind camera) and non-zero `scale`.
4. Check the render list sort order — entity may be behind terrain if
   its `order` value is lower than tiles at the same iso depth.
5. Confirm the texture is loaded (`asset=debug` channel).
6. For `IsoSprite`: verify `tilemap` field references a valid `Tilemap`
   entity name — the render loop uses this to compute iso position.

### Layout Wrong (UI Element Misplaced)

1. Enable `CLASSIC_LOG=layout=trace,ui=debug`.
2. Enable `CLASSIC_UI_DEBUG=1` to inspect per-frame positions.
3. Verify the anchor constraints align with expectations:
   - `LeftAnchor` + `RightAnchor` → width-constrained centering
   - `TopAnchor` + `BottomAnchor` → height-constrained centering
   - Single anchor only → element positioned at that edge
4. Check `parent` reference — layout tree must form a connected hierarchy.
   Only the top bar is attached to root; other widgets position themselves
   independently.
5. Confirm `refresh_layout()` is called and `dirty` flag propagates up
   the layout tree.
6. SDF text uses `text_height` for vertical centering — verify the font
   atlas is loaded and glyph metrics are non-zero.

### Path Empty (No Route Found)

1. Enable `CLASSIC_LOG=path=trace,nav=debug`.
2. Verify `IsometricNavMesh` component exists on `tilemapNavigation` entity
   and `data` array has correct dimensions.
3. Check the start and end tiles are walkable (nav mesh value allows
   traversal). Blocked tiles block A* entirely.
4. Confirm the nav mesh uses the same coordinate convention as the tilemap
   (both share `size_x` / `size_y`).
5. Check for out-of-bounds start/end — pathfinder returns empty path for
   coordinates outside the nav mesh.

### Texture Missing / Black Screen

1. Enable `CLASSIC_LOG=asset=debug,gfx=trace,glstate=trace`.
2. Verify `roms/out/` was generated: run `cargo xtask all`. Missing assets
   cause `include_bytes!` compile errors, but stale assets may embed zero-byte
   or outdated files.
3. Check GL state contract: `begin_frame` does NOT enable `DEPTH_TEST`
   globally. Tilemap and iso_sprite shaders enable it within their scopes.
   Enabling it globally depth-rejects UI under ortho projection.
4. For missing textures: confirm the texture name in the manifest matches
   the filename in `roms/out/res/`.
5. Verify the shader compiles: check for GLSL 300 es syntax in the
   vertex/fragment sources (no GLSL 100 `attribute`/`varying`/`texture2D`).
6. On web: check browser console for `WebGL 2.0 not supported` — the engine
   requires WebGL 2.

### Golden Trace Mismatch

1. Run with `CLASSIC_GOLDEN=update` to regenerate the actual trace.
2. Diff the files: `diff tests/golden/baseline/baseline.trace.jsonl target/classic-test/...`.
3. Identify the first mismatched line — all subsequent lines are often
   cascading failures from the first deviation.
4. Common root causes: sort order formula change (affects `order` field
   for every item), camera matrix order change (affects all `model`
   matrices), entity count change in render list.

### Shadows / lighting look wrong (bring-up playbook)

This is a distinct workflow because a partial-strength lighting effect can be
**completely broken and still look plausible**.  A non-functional shadow map
survived a full session of "verification" this way.

1. **Turn the effect to full strength.**  Set
   `SHADOW_STRENGTH = 0.0` (`classic-engine/src/shadow.rs`) so a shadowed pixel
   is hard black.  At the shipped `0.4` — and especially the old `0.65` —
   "broken" and "subtle" are indistinguishable by eye *and* by pixel diff.
2. **Isolate the term.**  `CLASSIC_SHADOW_DEBUG=1` renders the raw sun
   visibility factor (white = lit, black = occluded) with no albedo, ambient,
   Lambert term or point lights to hide behind.  Add `CLASSIC_NO_UI=1` to drop
   the editor/HUD layer.
3. **Render to a scratch dir and look at the PNG.**  Aggregate metrics cannot
   distinguish "subtle" from "absent":
   ```bash
   CLASSIC_ROM=rom:basetest CLASSIC_HEADLESS=1 CLASSIC_FRAMES=60 \
   CLASSIC_FIXED_DT=0.016666668 CLASSIC_WIDTH=1280 CLASSIC_HEIGHT=720 \
   CLASSIC_NO_UI=1 CLASSIC_SHADOW_DEBUG=1 \
   CLASSIC_GOLDEN=update CLASSIC_GOLDEN_DIR=/tmp/shot CLASSIC_GOLDEN_PNG=1 \
   LIBGL_ALWAYS_SOFTWARE=1 LP_NUM_THREADS=0 cargo run -p classic-desktop
   ```
4. **A/B against `CLASSIC_SHADOWS=0`** and count changed pixels.  Use
   `-alpha off`, or ImageMagick silently under-reports:
   ```bash
   magick a.png b.png -alpha off -compose difference -composite -colorspace Gray \
     -threshold 2% -format "%[fx:int(mean*w*h)] px changed\n" info:
   ```
   Healthy `basetest` is ~110k px (12%).  Under ~5k means it is not working.
5. **Verify impressions numerically.**  Simultaneous contrast is real: adding a
   large shadow makes unchanged ground *look* darker.  Probe exact pixels with
   `magick f.png -format '%[pixel:p{X,Y}]' info:` before believing your eyes.
6. **Sweep the sun.**  Use the light widget's elevation `-` buttons
   interactively.  Low sun angles are the regime that was previously
   degenerate; bugs that hide at 60° are obvious at 20°.

Read `classic-gfx` §17 first — the light-space vs screen-space distinction is
the root cause of every shadow bug found so far.  Regression signatures:

| Symptom | Cause |
|---|---|
| Speckled / diagonally-striped ground | acne; receiver normal-offset bias regressed |
| Sprites stippled in the debug view | billboard self-shadowing; sprite slope-scaled offset regressed |
| Shadows detached from caster bases | peter-panning; bias too large |
| Shadows vanish as sun elevation drops | positions are being projected in screen space again |
| Sprite shadows collapse to a puddle at their feet | billboard unprojection broke |

---

## 9. Known-Divergent / Non-Functional

Items below are deliberately incomplete and will produce no diagnostics when
broken:

| Item | Status |
|---|---|
| **SDF shadow/glow passes** | Only a single SDF draw runs (main + outline); there is no shadow/glow pass. |
| **No bitmap text** | All text is SDF; there is no glyph-map text renderer. |
| **`consumes_click` dispatch** | `consumed_click` is set directly on the dispatch path; there is no separate pre-scan. |
| **Entity destruction** | `world.despawn` is called via `Engine::despawn_named`, exposed to ROM guests through the `despawn` SDK import. Entities are also soft-disabled with a `Disabled` component in the UI layer. |
| **Collider in quadtree (disabled)** | Disabled colliders are skipped at insertion time in `begin_frame()`. This can affect click dispatch when toggling collider enabled state within a frame. |
| **Camera matrix order** | `T(-fix) * S(scale)`. The fix-point formula compensates so the visible area stays centred. |
| **heightData stride** | `(size_x + 1) * (size_y + 1)` (vertex grid, one height per vertex). The `height_data` grid is a binary sidecar resource (`ResourceKind::Grid`), not inlined in `state.json`. |
| **Root UI tree** | Only the top bar is attached to root; other panels position themselves independently. `CLASSIC_UI_DEBUG` shows fewer elements in the layout tree walk. |
| **Web Worker pathfinder** | A* runs on a host `PathfinderWorker` (native thread / web `Worker`) via async `request_path`/`poll_path`, with a synchronous fallback under the deterministic harness. |
| **`classic_log` hot-reload** | Channels are parsed once at startup. There is no runtime reload or live toggle — changing channels requires a process restart (or page reload on web). |

---

## 10. Cross-repo staleness & validation

Two fail-loud checks guard the asset→ROM→engine boundary; both emit actionable
diagnostics:

| Check | Where | What it catches |
|---|---|---|
| `cargo xtask check` | classic-roms | A `scene.json` referencing a texture/anim/grid/font/vehicle/entity not in the `dist.json` catalog (bails with the dangling name), and a `dist.json.manifest_version` / `catalog_source_hash` mismatch against the assets checkout (the `assets/` submodule is stale or the tree is uncommitted). |
| `cargo xtask fetch-roms` | classic-wgl | A ROM whose sha256 doesn't match the published `roms.json`, or a missing `roms.json` index (now a hard error unless `--skip-verify`). |

Diagnosing a "stale" failure:

- **`catalog_source_hash` mismatch** — `dist.json` was built from a different
  assets commit than the checkout being bundled.  Either the `assets` submodule
  is not at the pinned rev (`git submodule update`), or you're building from a
  dirty standalone checkout (set `CLASSIC_ASSETS_DIR` to make it a warning).
- **`manifest_version` unsupported** — the classic-assets catalog schema is
  newer than this classic-roms; bump the supported version in `xtask` before
  consuming it.
- **`roms.json` missing / sha256 mismatch** — the R2 bucket was republished
  without its index, or a stale/partial upload.  Republish via `publish.sh`,
  or (local dev only) pass `--skip-verify`.

