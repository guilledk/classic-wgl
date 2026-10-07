---
name: classic-testing
description: >
    Automated testing infrastructure for classic-wgl's Rust port.
    Covers CLASSIC_TEST e2e framework, golden trace/pixel harness,
    headless EGL CI, test scenario authoring, and test workflow
    patterns for AI agents.  Use when writing end-to-end tests,
    debugging golden trace mismatches, adding test assertions, or
    diagnosing CI failures.
    Trigger phrases: "CLASSIC_TEST", "end-to-end test", "golden trace",
    "golden check", "headless EGL", "test scenario", "TestAction",
    "AssertKind", "build_test_scenario", "CLASSIC_TEST_FILE",
    "mock GL", "test_should_close", "golden_capture_frame".
---

# classic-testing

## 1. CLASSIC_TEST Overview

The `CLASSIC_TEST` env-var triggers the automated end-to-end test runner inside
the normal desktop binary.  When `CLASSIC_TEST` is set to any non-empty
non-zero value, `classic-demo` registers a per-frame runner via
`Engine::set_test_runner` (see `classic-demo/src/testing.rs`): it loads a test
scenario (a sequence of scheduled `TestStep` values), begins executing actions
and assertions frame-by-frame, and sets `test_should_close` when the scenario
completes or the first assertion fails.

Key lifecycle properties:

- **Delta time**: automatically fixed to `1/60` when `CLASSIC_TEST` is active
  (unless `CLASSIC_FIXED_DT` overrides it).  This makes frame scheduling
  deterministic.
- **Frame counter**: `Engine::frame_number()` increments every `frame()` call,
  starting from 0.  Test steps fire when `frame_number()` equals `step.frame`.
- **Completion**: once all steps are processed and any active drag has
  finished, `test_should_close` is set.  On assertion failure only
  `test_failed` is set (and `test_should_close` immediately **only** when
  `CLASSIC_TEST_FAILFAST` is set).  On headless, `test_should_close`
  terminates the `run_loop`.
- **CLASSIC_TEST_FILE**: if set, the scenario is loaded from that JSON file
  instead of the hardcoded default.  Takes precedence.
- **Editor-state persistence**: `editor_state` holds the most recent
  `SetEditor` action so it can be re-applied every frame, compensating for
  `tool_buttons` `on_update` closures that reset `editor_target` via `Rc` sync.
- **CLASSIC_TEST_FAILFAST**: when set, the first assertion failure sets
  `test_should_close` (early exit) instead of just setting `test_failed=true`.
  There is no `panic!` on assertion failure; the desktop main loop turns
  `test_failed` into `process::exit(1)`.

## 2. Test Actions

Each `TestStep` carries a `Vec<TestAction>`.  All actions are mapped in
`run_test_frame` and directly modify `Engine` input/editor/entity state:

| Action | JSON key | Description |
|---|---|---|
| `SetEditor` | `setEditor` | Sets the editor target (`"height"` or `"tilemap"`), height delta, height mode (`"blend"` / `"set"`), and tile id. Equivalent to clicking a tool button. |
| `Drag` | `drag` | Simulates a mouse drag from `(from)` to `(to)` over `hold_frames` frames. Interpolates `mouse_iso_pos` linearly, then calls `apply_editor_selection` on the final frame. |
| `OpenMenu` | `openMenu` | Opens the dev menu panel by setting `panel_menu_open=true` and enabling the `menu_panel_e` entity. |
| `EnableTextDemo` | `enableTextDemo` | Activates the text showcase panel. Sets `editor_target="textDemo"` and enables the `text_showcase_e` entity. |
| `MouseMove` | `mouseMove` | Sets `input.mouse_pos` and `input.mouse_axis` (normalized to `[-1,1]` using the last viewport dimensions). |
| `MouseClick` | `mouseClick` | Sets `input.mouse_pos`, `mouse_down[button]`, `mouse_pressed[button]`, and `frame_had_click` (for button 0). |
| `KeyPress` | `keyPress` | Inserts into `input.keys_down` and `input.keys_pressed` maps. Key strings follow winit 0.30 `PhysicalKey::Code` debug naming (`"F9"`, `"Space"`, `"KeyW"`, etc.). |
| `Wheel` | `wheel` | Sets `input.mouse_wheel`. The engine's wheel-decay logic runs after the test frame, so wheel values may need to be set immediately before assertions that depend on them. |
| `Wait` | `wait` | No-op; only useful as a sentinel in the JSON. Frame-based waiting is achieved by scheduling a step on a later frame. |
| `SetCameraIso` | `setCameraIso` | Centers the camera on iso tile `(tx, ty)` at the given `scale` (via `Engine::iso_to_screen` + `set_camera`). Useful for framing a sprite for pixel assertions. |
| `SetMouseIso` | `setMouseIso` | Sets `input.mouse_pos` to the screen position of iso tile `(tx, ty)` via `render_order::iso_to_screen_px`. Note: in a native (winit) window the OS cursor overwrites `input.mouse_pos` every `CursorMoved`, so this action is only reliable headless — prefer `setEntityPos` for deterministic native placement. |
| `SetEntityPos` | `setEntityPos` | Sets a named entity's `Transform.position` to `(x, y, z)` **and arms a persistent per-frame override** (`runner.pos_override`), so a mouse-following guest (which re-writes the position every frame) can't steal the placement. `z` is in px (metres × `PPM_TARGET`). |
| `SetSpriteFrame` | `setSpriteFrame` | Calls `Engine::set_sprite_frame(name, frame)` **and arms a persistent per-frame override** (`runner.frame_override`), keeping a packed-atlas sprite on a chosen pitch/roll frame even though the guest recomputes it from the terrain each frame. |

Drag simulation detail: the drag is processed by `run_test_frame`'s drag state
machine.  On frame `start+0` it sets `selection_iso_begin=mouse_iso_pos=from`
and `selection_mode=1`.  On interim frames (`rel > 0 && rel < hold`) it
interpolates `mouse_iso_pos` by `from.lerp(to, rel/hold)`.  On frame
`start+hold` it sets `selection_iso_end=to`, `selection_mode=-1`, and lets the
`on_selection_end` hook (demo `apply_editor_selection`) run.  The drag state is
then cleared along with `editor_state`.

## 3. Assertions

Each `TestStep` carries a `Vec<TileAssertion>`, a struct with `kind`,
`region`, `expected`, and `log` fields.  `region` is `(x1, y1, x2, y2)` in
tile coordinates for spatial assertions; its meaning varies by assertion kind.

| AssertKind | JSON key | Semantics |
|---|---|---|
| `Height` | `height` | Iterates `region` (exclusive on `x2`, `y2`) and checks `tilemap.height_data[index] == expected` with tolerance ±0.01.  Height data is `(size_x+1) × (size_y+1)` vertices. |
| `Tile` | `tile` | Iterates `region` and checks `tilemap.data[index] == expected` (exact match, `u32`).  Tile data is `size_x × size_y`. |
| `UiTextCentered` | `uiTextCentered` | Walks `menu_panel_e` children, checks each row's first child `SdfText` position vs `row.pos + row.size/2 - child.size/2`, with tolerance = `expected` pixels. |
| `UiEnabled` | `uiEnabled` | Checks whether `text_showcase_e` is enabled (matching `expected != 0.0`). |
| `CameraAt` | `cameraAt` | `region = (ex, ey, ez, expected_scale)`.  Checks `camera.position` against `(ex, ey, ez)` with default tolerance 1.0 in position and 0.01 in scale.  `expected` overrides the tolerance if > 0.  If `region.3 == 0`, scale defaults to 1.0. |
| `EntityVisible` | `entityVisible` | Uses `log` as the entity name lookup key in `Engine::names`.  Checks `is_disabled` matches `expected != 0.0`. |
| `EntityPos` | `entityPos` | Uses `log` as the entity name.  `region = (ex, ey, ...)`.  Checks `Transform::position.x/y` within tolerance (default 1.0) of `(ex, ey)`.  `expected` overrides tolerance if > 0. |
| `PixelAtEntity` | `pixelAtEntity` | **GPU-only.** Uses `log` as the entity name; projects its iso position (plus an optional tile-space `offset: [dx, dy]`) to screen pixels (`render_order::iso_to_screen_px`) and reads the framebuffer pixel (`Gfx::read_pixel_rgba`). With `color` set, checks all 4 channels within `expected` tolerance; with `color` absent/null, checks alpha `>= expected`. The `offset` lets a test sample a sprite's footprint corner (e.g. `[-1.735, 1.735]` = front-bottom) rather than only its ground anchor — this is how the shipping-container corner-ghost regression was pinned down. |

`PixelAtEntity` is the render-order / depth-occlusion assertion (per-texture
depth maps, ghost pass).  It reads the previous frame's framebuffer (the test
runner runs before `begin_frame`), so schedule it after the scene settles.
It needs a real GL depth-test driver — under Mesa llvmpipe the depth test is
broken, so ghost/occlusion assertions cannot be validated headless.

On failure, each assertion logs a diagnostic line via the `Test` instrument
channel.  The test result string is pushed to the runner's `results` vector
for the completion summary (`"=== CLASSIC_TEST COMPLETE: X/Y assertions
passed ==="`).

## 4. Writing a Test Scenario

Scenarios are `Vec<TestStep>` values.  Each lives in
`tests/scenarios/<name>.test.json` and is registered in the `SCENARIOS` table
in `classic-demo/src/testing.rs`, which `include_str!`s it so a run never
depends on the process cwd.  An ad-hoc scenario can still be passed by path
through `CLASSIC_TEST_FILE`, which takes precedence over everything below.

### 4a. Selecting a scenario

`CLASSIC_TEST=<name>` picks `tests/scenarios/<name>.test.json`.  An unknown
name panics with the list of known ones — it does **not** silently fall back
to the default, which is how three scenarios sat unrun.

`CLASSIC_TEST=1` / `true` / `all` / `default` are all aliases for the
`default` scenario.  `all` is a misnomer kept for compatibility: CI,
AGENTS.md and the golden runbooks all use it, and the demo baseline's golden
capture frame is derived from the default scenario's last step, so
repointing `all` would silently invalidate that baseline.

**A scenario is bound to one ROM**, because it names entities and those names
are namespaced by the ROM that declares them.  There is no "run everything in
one process":

| scenario | ROM | invocation |
|---|---|---|
| `default` | `demo` | `CLASSIC_TEST=all` (CI golden) |
| `render_order` | `lrvtest` | `CLASSIC_ROM=rom:lrvtest CLASSIC_TEST=render_order CLASSIC_FIXED_DT=0.016666668 CLASSIC_WIDTH=1280 CLASSIC_HEIGHT=720` (CI) |
| `rocket` | `lunar` | `CLASSIC_ROM=rom:lunar CLASSIC_TEST=rocket CLASSIC_FIXED_DT=0.05 CLASSIC_FRAMES=225 CLASSIC_WIDTH=1280 CLASSIC_HEIGHT=720` (CI) |

Two `classic-demo` unit tests keep the table honest:
`scenario_table_covers_the_directory` (a file not in `SCENARIOS` is
unreachable) and `every_registered_scenario_parses` (a scenario whose JSON has
rotted past the `TestStep` schema is a test that cannot run).

**Entity names in assertions must be namespaced** (`lunar::rocket`,
`lunar-common::lrv`), matching what the ROM rewrite passes produce.  Dump the
real names with
`CLASSIC_GOLDEN=update CLASSIC_GOLDEN_DIR=<tmp> CLASSIC_FRAMES=60` and grep
`"name"` out of the emitted trace — note the default capture frame is 55, so
`CLASSIC_FRAMES` must exceed it or nothing is written.

### JSON format

The JSON file is an array of step objects:

```json
[
  {
    "frame": 2,
    "actions": [{"openMenu": null}],
    "assertions": [],
    "log": "open dev menu"
  },
  {
    "frame": 5,
    "actions": [{"setEditor": {"target": "height", "heightDelta": 2, "heightMode": "blend", "tileId": 0}}],
    "assertions": [],
    "log": "set height editor"
  },
  {
    "frame": 13,
    "actions": [],
    "assertions": [
      {"kind": "height", "region": [10, 10, 14, 14], "expected": 3.0, "log": "height blend applied"}
    ],
    "log": "verify height"
  }
]
```

### Step scheduling

`frame` field is relative to `debug_frame` (starting at 0 after all `init_*`
calls).  Actions execute immediately on that frame.  Assertions run after
actions, before rendering.  Drags span multiple frames — schedule the
assertion step for after `hold_frames` elapses, plus a few frames for mesh
rebuild (height/tile changes need 2-3 frames for the mesh rebuild to
complete).

### TileAssertion fields

- `kind`: one of `"height"`, `"tile"`, `"uiTextCentered"`, `"uiEnabled"`,
  `"cameraAt"`, `"entityVisible"`, `"entityPos"`.
- `region`: `[x1, y1, x2, y2]`.  For spatial assertions, exclusive on `x2`,
  `y2` (iterates `y` from `y1` to `y2-1`, `x` from `x1` to `x2-1`).
  For `CameraAt`: `(ex, ey, ez, scale)` in world units.  For `EntityPos`:
  `(ex, ey, 0, 0)` in world units.
- `expected`: `f32`.  Used as the target value for height/tile assertions,
  tolerance for `UiTextCentered`/`CameraAt`/`EntityPos`, or boolean intent
  for `UiEnabled`/`EntityVisible`.
- `color`: `[r, g, b, a]` (optional) — expected RGBA for `PixelAtEntity`;
  absent/null = opacity-only (alpha `>= expected`).
- `offset`: `[dx, dy]` (optional, default `[0,0]`) — tile-space offset added to
  the entity's position for `PixelAtEntity`, so a test can sample a footprint
  corner instead of the ground anchor.
- `log`: free-form description, emitted on pass/fail.

## 5. Scenario Authoring Workflow

When adding a new end-to-end test scenario, follow this workflow:

1. **Instrument the target feature** with `CLASSIC_LOG=Test` to see step and
   assertion output during manual runs.  This confirms the feature is
   reachable via test actions.

2. **Write the JSON scenario file** with conservative frame numbers.  Leave
   at least 2-3 frames between a drag action and its assertion step to allow
   mesh rebuild.  For UI assertions, leave 1-2 frames for layout refresh.

3. **Run locally with CLASSIC_GOLDEN=check** to verify the scenario passes:
   ```
   CLASSIC_TEST=1 CLASSIC_FRAMES=60 CLASSIC_TEST_FILE=path/to/scenario.json cargo run -p classic-desktop
   ```

4. **Update the golden trace** if the scenario changes the render output:
   ```
   CLASSIC_HEADLESS=1 CLASSIC_FRAMES=60 CLASSIC_TEST_FILE=path/to/scenario.json CLASSIC_GOLDEN=update cargo run -p classic-desktop
   ```

5. **Verify headless** — the scenario must also pass headless:
   ```
   CLASSIC_HEADLESS=1 CLASSIC_FRAMES=60 CLASSIC_TEST=all CLASSIC_GOLDEN=check cargo run -p classic-desktop
   ```

6. **Commit the scenario file and updated golden baselines** together.

7. **Check CI** — the golden job in CI runs with libEGL + Mesa llvmpipe.
   Flaky tests that pass locally but fail in CI usually indicate an
   uninitialized default or nondeterminism in the render loop (check
   that `begin_frame` always resets state, and that entity spawning
   order is deterministic across init stages).

8. **Read the layout map** — a `CLASSIC_GOLDEN=update` run writes
   `{golden_dir}/baseline.layout.txt`; a `check` run writes
   `target/classic-test/baseline.layout.txt`.  Skim it to confirm a
   sprite/text landed in the expected screen rect without pixel vision
   (see §6 "Layout map").

### Interactive debugging

Set `CLASSIC_TEST_FAILFAST=1` to exit on the first assertion failure (there is
no `panic!` — the desktop main loop turns `test_failed` into a non-zero exit
code).  Combine with `CLASSIC_UI_DEBUG=1` to dump UI entity positions for the
first 120 frames.  Use `CLASSIC_LOG=Test,golden` for per-frame test output and
golden comparison details.

## 5b. The scene / golden matrix

There are two committed golden baselines.

| `CLASSIC_GOLDEN_DIR` | Scene | Distinguishing flags | Guards |
|---|---|---|---|
| `tests/golden/baseline` | demo | `CLASSIC_TEST=all` | e2e assertions + demo render |
| `tests/golden/lunar` | lunar | `CLASSIC_ROM=rom:lunar CLASSIC_FIXED_DT=0.016666668` | procedural terrain, rocket anim |

**There is no committed lighting reference.**  `basetest-lit` used to be one
(a 30° sun and two containers casting long shadows), but its containers had
stopped rendering (static `frame_name`s were never namespace-qualified,
fixed in #117) and, under `CLASSIC_NO_UI`, the nav-mesh overlay painted
the whole map blue — so it was guarding neither; `basetest` was retired.
For lighting or shadow work, capture a clean lit frame yourself and **look
at it**: `CLASSIC_NO_UI=1` now hides the nav overlay as well as the
editor/HUD, and the `render_order` scenario frames the LRV as a caster.

```bash
# a clean lit frame with a caster in shot (lrvtest's sun is 60°, so
# shadows are short: compare against your own pre-change capture)
CLASSIC_ROM=rom:lrvtest CLASSIC_TEST=render_order CLASSIC_NO_UI=1 \
CLASSIC_HEADLESS=1 CLASSIC_FRAMES=45 CLASSIC_FIXED_DT=0.016666668 \
CLASSIC_WIDTH=1280 CLASSIC_HEIGHT=720 \
CLASSIC_GOLDEN=update CLASSIC_GOLDEN_DIR=/tmp/lit CLASSIC_GOLDEN_PNG=1 \
LIBGL_ALWAYS_SOFTWARE=1 LP_NUM_THREADS=0 cargo run -p classic-desktop
```

Whenever you re-baseline a trace golden, regenerate and look at its PNG as
well: `basetest-lit`'s trace was re-baselined twice after its containers
vanished, and the trace could not see it.

`LIBGL_ALWAYS_SOFTWARE=1 LP_NUM_THREADS=0` are required for goldens (llvmpipe's
multithreaded rasteriser races on the sprite ghost-pass depth rendering) and
must **not** be used for interactive runs.

## 5c. Writing assertions that can actually fail

A cautionary tale worth generalising: the directional shadow map shipped
completely non-functional while five unit tests and every golden passed.

- **The tests asserted a weak invariant.**  They checked that the light-space
  box corners "land inside NDC `[-1,1]`" — which is also true of a fully
  degenerate projection.  The tests that caught the bug assert *physical*
  contracts: the sun's elevation as the shadow map sees it (2.7° vs the
  authored 30°), and that a caster lands within one shadow-map texel of the
  ground it shadows (73.7 texels vs <1).
- **Beware assertions that are true by construction.**  The first attempt built
  the receiver as `caster - light_dir * t`; an orthographic projection along
  `light_dir` maps any such pair to a single texel *regardless of the bug*.
  Both endpoints must go through the real vertex transform.
- **Expect existing tests to encode the bug.**  Three tests had asserted the
  buggy behaviour as correct and had to be rewritten.  When a test fails after
  a fix, check whether it was testing the defect before "fixing" the fix.
- **The framebuffer alpha channel is blend bookkeeping, not visibility.**  A
  sprite's semi-transparent edge texels leave `srcA² + (1 − srcA)` in the
  alpha channel over opaque terrain (0.7607843 for a 0.4 texel), so an
  alpha-only `pixelAtEntity` on a silhouette edge flips between drivers,
  and one over terrain passes even when the sprite is gone.  Assert
  *colour* at a point a captured PNG shows is solid and distinct from what
  lies behind it, check a 2–3 px radius holds on hardware GL and on
  llvmpipe, and run a negative control (entity moved, frame changed, or a
  frame where it is not yet in shot) that must fail.
- **Goldens do not protect an effect you cannot see.**  A partial-strength
  effect (shadow strength `0.65`) makes "broken" and "subtle" numerically
  similar.  Turn the effect to full strength during bring-up, and use a debug
  view (`CLASSIC_SHADOW_DEBUG=1`) that isolates it.

## 6. Golden Trace Harness

The golden trace harness captures a deterministic, frame-by-frame record
of every draw call — model matrices, textures, sort order, camera state,
and per-kind draw counts — for comparison against committed reference files.

### JSONL format

Each golden capture produces one JSON lines file.  The first line is a
header object containing `tag`, `frame`, `viewport`, `camera` (position,
scale, matrix), and `counts` (per-kind draw call count).  Each subsequent
line is a `TraceItem`: `order` (z-sort depth), `kind` (e.g. `Tilemap`,
`IsoSprite`, `SdfText`, `UiRect`, `UiSprite`, `Sprite`), `name` (debug
name), `model` (16-element matrix via `glam::Mat4::to_cols_array()`, i.e.
column-major), `camera_ignored` (bool), and optional `texture`, `frame`,
`color`.

Each `TraceItem` also carries a `screen` rect (`[x, y, w, h]`, top-left
origin pixels) computed in pure Rust via `golden::project_rect` — but it is
`#[serde(skip)]`'d, so it never appears in the JSONL and never affects the
line-by-line golden comparison.  It only feeds the layout map below.

### Operation

- **Capture timing**: `golden_capture_frame` defaults to `last_test_step.frame + 1`.
  On that frame, a `TraceCollector` is created and every draw call pushes a
  `TraceItem` with its model matrix, texture, and metadata.
- **CLASSIC_GOLDEN=check**: after rendering the capture frame, the trace is
  serialized and compared line-by-line against
  `{CLASSIC_GOLDEN_DIR}/baseline.trace.jsonl` (default
  `tests/golden/baseline`; see the scene/golden matrix in §5b for the four
  committed baselines).  On mismatch, the actual trace is
  written to `target/classic-test/baseline.actual.trace.jsonl` for CI artifact
  upload.
- **CLASSIC_GOLDEN=update**: overwrites the reference file with the current
  output.
- **Baseline location**: `tests/golden/baseline/baseline.trace.jsonl`
  (70 lines, covering 5 `IsoSprite`, 54 `SdfText`, 1 `Sprite`, 1
  `Tilemap`, 7 `UiRect`, 1 `UiSprite`).

### Layout map (CLASSIC_GOLDEN_LAYOUT)

Alongside the trace, the harness emits a **layout map** — a deterministic,
GPU-free text rendering of *what is drawn where* — so a text-only model can
"look at" the frame without reading pixels.  It is on by default whenever
`CLASSIC_GOLDEN` is set (disable with `CLASSIC_GOLDEN_LAYOUT=0`).

- **On `update`**: written to `{CLASSIC_GOLDEN_DIR}/baseline.layout.txt`.
- **On `check`**: written to `target/classic-test/baseline.layout.txt`
  (always, not only on mismatch), so an author can read it on a passing run.

Format: a `#`-prefixed header line (frame, viewport, camera pos/scale), a
`#`-prefixed column header, then one fixed-width line per draw item sorted by
`order` — `name`, `kind`, `order`, `x y w h` (screen rect), `texture`, `color`.
The `screen` rect is the axis-aligned bounding box of the item's unit quad
after `(camera or identity) · model`; for `Tilemap` it is the projected
iso-extent diamond (`(0,0)..(size_x,size_y)` through `iso_camera_px`).
This is not part of `compare_traces`, so it cannot fail a golden run.

### Pixel golden (CLASSIC_GOLDEN_PNG)

An additional pixel-comparison mode that captures an RGBA framebuffer via
`CLASSIC_OFFSCREEN=1` (implied by `CLASSIC_HEADLESS`).  After `glFinish`,
the render target is read, vertically flipped, and compared pixel-by-pixel
against `tests/golden/baseline/baseline.png` with per-channel tolerance
controlled by `CLASSIC_GOLDEN_TOL` (default 2).  A match is accepted if
pixel differences exceed 0.1% or less of total pixels.

This mode is NOT run in CI by default because software rasterizer pixel
output depends on the Mesa version.  It remains available for manual use
and can detect large-scale rendering regressions (missing draw calls,
wrong textures) even with the tolerance.

### Common golden mismatch causes

- **Mesh rebuild timing**: tile/height edits need 2-3 frames for the
  tilemap mesh to regenerate.  If `golden_capture_frame` is too soon, the
  trace will miss updated geometry.
- **UI layout timing**: UI layout runs on `refresh_layout()` which is
  called at the start of `frame()`.  If a test action opens a panel on
  frame N, the layout won't reflect it until frame N+1.
- **Entity naming**: trace items use `DebugName` component as the `name`
  field.  If an entity lacks `DebugName`, the name falls back to a hex
  entity ID string, which is nondeterministic between runs and platforms.
  Always ensure traced entities have deterministic `DebugName` components.
- **Entity-ID renumbering (expected re-baseline, not a regression)**: when a
  scene gains or loses entities, the golden mismatch is often *pure* `name` /
  `e#N` entity-ID renumbering — geometry, color, and model matrices are all
  unchanged, only the per-entity `name` shifts because hecs reuses entity IDs
  in spawn order.  This is the expected re-baseline, not a rendering
  regression.  How to tell: the `expected` vs `actual` lines differ only in
  the `"name":"e#N"` field (everything else matches byte-for-byte).
- **Mesa version drift**: pixel golden is sensitive to Mesa version.
  Match your local `LIBGL_ALWAYS_SOFTWARE=1` renderer to CI's llvmpipe
  version.  When updating baselines, always regenerate both trace and
  pixel baselines from the same run.

## 7. Headless EGL CI

The CI golden job runs in a headless environment with no window system:

- **Binary**: `cargo build -p classic-desktop` (the native binary).
- **System packages**: `libegl1-mesa-dev`, `libgl1-mesa-dri`, `libgbm-dev`,
  `libx11-dev` (libx11 is needed at link time even though headless never
  opens an X11 window — the winit crate links against it).
- **Environment**:
  - `LIBGL_ALWAYS_SOFTWARE=1` — forces Mesa's llvmpipe software rasterizer.
  - `EGL_PLATFORM=surfaceless` — enables surfaceless EGL contexts, no
    display server required.
  - `CLASSIC_HEADLESS=1` — selects the `HeadlessPlatform`, which dynamically
    loads `libEGL.so.1` and creates a pbuffer surface + desktop OpenGL 3.x
    context (`EGL_RENDERABLE_TYPE = EGL_OPENGL_BIT`, not GLES).
  - `CLASSIC_FRAMES=60` — limits the headless run loop to 60 frames.
    The headless `run_loop` ignores `should_close` from test completion
    and waits for this limit, giving golden capture time to occur after
    the last test step.
  - `CLASSIC_TEST=all` — loads the hardcoded scenario.
  - `CLASSIC_GOLDEN=check` — compares trace (and pixel if enabled)
    against committed baselines.
- **Artifacts**: on failure, `target/classic-test/` is uploaded as a CI
  artifact, containing `baseline.actual.trace.jsonl`.

### Running locally

Install Mesa development libraries, then:

```
CLASSIC_HEADLESS=1 CLASSIC_FRAMES=60 CLASSIC_TEST=all CLASSIC_GOLDEN=check cargo run -p classic-desktop
```

To update baselines:

```
CLASSIC_HEADLESS=1 CLASSIC_FRAMES=60 CLASSIC_TEST=all CLASSIC_GOLDEN=update cargo run -p classic-desktop
CLASSIC_HEADLESS=1 CLASSIC_FRAMES=60 CLASSIC_TEST=all CLASSIC_GOLDEN=update CLASSIC_GOLDEN_PNG=1 cargo run -p classic-desktop
```

## 8. Unit Test Patterns

### Test parallelism

Tests run in parallel (`cargo test`, no `--test-threads=1`).  The component
registry is a populate-once `OnceLock<Vec<ComponentReg>>` (read-only after
`register_all_components()`), and the `CLASSIC_LOG` channel table is guarded
by a per-test mutex in `classic-core/tests/instrument.rs`.

### `instrument::reset_for_test`

Tests that interact with the `CLASSIC_LOG` channel system should call
`classic_core::instrument::reset_for_test()` to zero out the atomic level
table, and hold the `SETUP_LOCK` guard across setup + assertions (see
`classic-core/tests/instrument.rs`).  This prevents test ordering from
leaking channel levels.

### Mock GL approach

`classic-gfx` and `classic-engine` currently have **no unit tests with mock
GL**.  The render layer (`Gfx`, shaders, draw functions) is covered
indirectly through the golden trace harness, which captures model matrices
and draw metadata without requiring pixel readback.  The design intent is
that a future mock GL backend would implement the `glow::HasContext` trait
with a recording proxy, allowing unit tests to verify GL call sequences.

### Test module layout

- `classic-core/tests/` — integration tests (instrument, registry).
- `classic-demo/src/testing.rs` — test types, scenario builder, and the runner.
- `classic-gfx/src/golden.rs` — trace types, serialization, comparison.
- `tests/golden/` — committed baselines (trace JSONL, PNG).

## 9. Known-divergent / non-functional

- **`render_order` and `rocket` are revived and in CI.**  Both were re-derived from captured frames (hardware Mesa, and
  llvmpipe single- and multi-threaded) with a freshly fetched ROM set.
  - `render_order` (ROM `lrvtest`) used to read alpha 0.7607843 at the `lrv`
    and `lrvWheelFl` ground origins under llvmpipe and 1.0 on hardware GL.
    The frame shows **no ghosting anywhere**: every alpha-below-1 pixel is a
    one-texel outline along a sprite silhouette (section 5c), and both old
    sample points sat on such an edge.  `lrvWheelFl`'s ground origin is
    moreover *behind the chassis* on screen (its `lrvTireFl` fender is the
    upper, far-side one), so "opaque at the ground origin" was ill-posed.
    It now asserts colour: the chassis's dark instrument box
    (`lrv` + `[-0.58, -0.85]`, `[0.094, 0.125, 0.18] ± 0.12`) and the FL
    fender's orange (`lrvWheelFl` + `[0.7, -1.4]`, `[0.7, 0.42, 0.16] ± 0.2`).
    Negative controls fail: the body on another heading frame, and the FL
    tire moved away (the sample reads bare regolith).
  - `rocket` (ROM `lunar`): the lunar guest relocates the rocket to a
    generated landing zone at **(317.5, 277.5)** (not `state.json`'s
    (100.5, 100.5)), and the `landing` clip holds it ~100 m up early on;
    it touches down at t ≈ 10 s and stays put (a cargo box appears from
    t ≈ 13 s).  The scenario frames that anchor at scale 0.3 and asserts at
    frame 220 **under `CLASSIC_FIXED_DT=0.05`** (t = 11 s; the same landed
    pose as frame ~620 at 1/60, at a third of the llvmpipe cost): the black
    base band (`+ [4.0, -6.75]`, `[0.039, 0.039, 0.047] ± 0.1`) and a light
    body panel (`+ [10.25, -7.0]`, `[0.722, 0.71, 0.741] ± 0.1`).  Both
    offsets stay on the flat stamped pad — `iso_to_screen_px` projects at
    the terrain height of the *offset* tile, so a large up-screen offset
    would couple the sample to off-pad terrain.  Run at 1/60 it fails
    (frame 220 is mid-descent; both samples read regolith).  A `lunar`
    republish that moves the landing zone needs these re-measured.
  - `container_ghost` was deleted along with the `container` scene it
    targeted (a one-off, never-published classic-roms test scene).  Its
    only runnable retarget, the retired `basetest`'s `containerA`, passed
    6/6 *vacuously* — those containers rendered no pixels, so every sample
    read opaque terrain.

- **`basetest` was retired.**  Its two containers (`lunar-common::
  shippingContainerBody` frame 56, a non-empty atlas rect) were drawn every
  frame but produced no pixels against the current ROM set on both HEAD and
  the `a5fb6e3` engine, while its `basetest-lit` golden PNG (`4aee082`) still
  showed them.  Separately, its CI step ran under `CLASSIC_NO_UI=1`, which
  until then left the nav-mesh overlay visible and painted the map flat
  `common::navTileset` blue.  Without its containers the scene was a near-
  flat plain (12.5k px of shadow vs ~110k with them), so it was removed
  rather than repaired.  The container loss was an engine bug, not ROM
  content: hydration never namespace-qualified a static `IsoSprite.frame_name`,
  so the bare `shippingContainerBody_56` missed the frame table's
  `lunar-common::` key (fixed in #117).

- **`Wait` action is a no-op**: the `Wait { frames }` action does nothing
  in `run_test_frame`.  To wait, schedule a `TestStep` on a later frame
  number with no actions.  The `Wait` variant exists in the type definitions
  for forward compatibility with a potential wait-until render-complete
  mechanism.

- **No pixel golden in CI**: `CLASSIC_GOLDEN_PNG=1` is not set in
  `.github/workflows/ci.yml`.  Pixel comparison is sensitive to the Mesa
  llvmpipe version and produces false positives across Ubuntu image
  updates.  The trace golden provides adequate coverage.  Tolerant *point*
  assertions are different: `render_order` and `rocket` each sample two
  pixels chosen from a captured frame to sit inside a 5–7 px solid patch, with
  a per-channel tolerance of 0.1–0.2, and hold on hardware GL and llvmpipe
  alike.

- **No headless on macOS or Windows**: `HeadlessPlatform` dynamically loads
  `libEGL.so.1` and is Linux-only.  CI golden tests only run on
  `ubuntu-latest`.  Local golden development on macOS requires a native
  window (omit `CLASSIC_HEADLESS`).

- **No incremental trace diffing**: mismatched traces produce a
  line-by-line diff capped at 40 lines.  For large diffs, the full actual
  trace in `target/classic-test/` must be examined manually.

- **`editor_state` is cleared after drag completion**: after the selection
  end (which runs the demo's `apply_editor_selection` hook), the runner's
  `drag_state` and `editor_state` are set to `None`.  A subsequent `SetEditor`
  action is required before another drag.

- **No multi-scenario support**: at most one scenario runs per invocation.
  The runner processes `STEPS` as a `LazyLock`, computed once per process
  lifetime.  Running multiple scenarios requires separate invocations.

- **No headless assets check**: CI runs `cargo xtask fetch-roms` before
  building, which verifies each ROM against the published `roms.json` sha256
  index (and now fails loudly when the index is absent, unless
  `--skip-verify`).  A stale `roms/out/` is caught by that sha256 check, not a
  build-time error.

## 10. Cross-repo integration checks (classic-roms ↔ classic-wgl)

The asset→ROM→engine boundary is validated outside this repo by `classic-roms`
(`cargo xtask check` / `cargo xtask diff`) and by a manual smoke boot:

- **`cargo xtask check`** (classic-roms) validates every name a scene
  references (textures/animations/grids/fonts/vehicles/entities) against the
  `dist.json` catalog and fails loudly on a dangling reference, plus the
  catalog staleness check (`manifest_version` + `catalog_source_hash` vs the
  assets checkout).  This is the check class that would have caught the
  rocket-landing regression at build time.
- **`cargo xtask diff [--url <base>]`** (classic-roms) diffs the local
  `roms/out/*.rom` size/sha256 against the published `roms.json` — see what
  changed before republishing.

### Smoke test (manual)

Boot a locally-built fixture ROM headless; a clean exit means the asset
rendered → packed → bundled → loaded → drawn without error:

```bash
# classic-roms (inside the classic-assets `nix develop` shell):
CLASSIC_ASSETS_DIR=$PWD/../classic-assets cargo xtask all      # builds roms/out/packtest.rom

# classic-wgl:
CLASSIC_ROM=$PWD/../classic-roms/roms/out/packtest.rom \
  CLASSIC_HEADLESS=1 CLASSIC_FRAMES=60 cargo run -p classic-desktop
```

The `packtest` ROM is a minimal fixture (`corner_building` IsoSprite + a
tilemap), so it exercises the whole pack path without a guest.  Extending this
to a `pixelAtEntity` assertion on `cornerBuilding` needs a `CLASSIC_TEST`
scenario referencing that entity (the hardcoded scenario is demo-specific) —
see §3 for the `pixelAtEntity` semantics and §4 for scenario authoring.
