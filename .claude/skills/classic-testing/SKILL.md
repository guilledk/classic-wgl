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
  finished, `test_should_close` is set.  On assertion failure, it is
  also set immediately.  On headless, this terminates the `run_loop`.
- **CLASSIC_TEST_FILE**: if set, the scenario is loaded from that JSON file
  instead of the hardcoded default.  Takes precedence.
- **Editor-state persistence**: `test_editor_state` holds the most recent
  `SetEditor` action so it can be re-applied every frame, compensating for
  `tool_buttons` `on_update` closures that reset `editor_target` via `Rc` sync.
- **CLASSIC_TEST_FAILFAST**: when set, the first assertion failure causes an
  immediate `panic!` instead of just setting `test_failed=true`.  This is
  useful for debugging with a backtrace.

## 2. Test Actions

Each `TestStep` carries a `Vec<TestAction>`.  All nine actions are mapped in
`run_test_frame` and directly modify `Engine` input/editor state:

| Action | JSON key | Description |
|---|---|---|
| `SetEditor` | `setEditor` | Sets the editor target (`"height"` or `"tilemap"`), height delta, height mode (`"blend"` / `"set"`), and tile id. Equivalent to clicking a tool button. |
| `Drag` | `drag` | Simulates a mouse drag from `(from)` to `(to)` over `hold_frames` frames. Interpolates `mouse_iso_pos` linearly, then calls `apply_editor_selection` on the final frame. |
| `OpenMenu` | `openMenu` | Opens the dev menu panel by setting `panel_menu_open=true` and enabling the `menu_panel_e` entity. |
| `EnableTextDemo` | `enableTextDemo` | Activates the text showcase panel. Sets `editor_target="textDemo"` and enables the `text_showcase_e` entity. |
| `MouseMove` | `mouseMove` | Sets `input.mouse_pos` and `input.mouse_axis` (normalized to `[-1,1]` using the last viewport dimensions). |
| `MouseClick` | `mouseClick` | Sets `input.mouse_pos`, `mouse_down[button]`, `mouse_pressed[button]`, and `frame_had_click` (for button 0). |
| `KeyPress` | `keyPress` | Inserts into `input.keys_down` and `input.keys_pressed` maps. Key strings follow winit `VirtualKeyCode` naming (`"F9"`, `"Space"`, etc.). |
| `Wheel` | `wheel` | Sets `input.mouse_wheel`. The engine's wheel-decay logic runs after the test frame, so wheel values may need to be set immediately before assertions that depend on them. |
| `Wait` | `wait` | No-op; only useful as a sentinel in the JSON. Frame-based waiting is achieved by scheduling a step on a later frame. |

Drag simulation detail: the drag is processed by `run_test_frame`'s drag state
machine.  On frame `start+0` it sets `selection_iso_begin=mouse_iso_pos=from`
and `selection_mode=1`.  On interim frames (`rel > 0 && rel < hold`) it
interpolates `mouse_iso_pos` by `from.lerp(to, rel/hold)`.  On frame
`start+hold` it sets `selection_iso_end=to`, `selection_mode=-1`, and calls
`apply_editor_selection()`.  The drag state is then cleared along with
`test_editor_state`.

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

On failure, each assertion logs a diagnostic line via the `Test` instrument
channel.  The test result string is pushed to `test_results` for the
completion summary (`"=== CLASSIC_TEST COMPLETE: X/Y assertions passed ==="`).

## 4. Writing a Test Scenario

Scenarios are `Vec<TestStep>` values, defined either in Rust (hardcoded in
`build_test_scenario`) or as a JSON file pointed to by `CLASSIC_TEST_FILE`.

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

### Interactive debugging

Set `CLASSIC_TEST_FAILFAST=1` to panic on the first assertion failure,
which gives a backtrace for the failing assertion.  Combine with
`CLASSIC_UI_DEBUG=1` to dump UI entity positions for the first 120 frames.
Use `CLASSIC_LOG=Test,golden` for per-frame test output and golden comparison
details.

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
name), `model` (16-element row-major matrix), `camera_ignored` (bool),
and optional `texture`, `frame`, `color`.

### Operation

- **Capture timing**: `golden_capture_frame` defaults to `last_test_step.frame + 1`.
  On that frame, a `TraceCollector` is created and every draw call pushes a
  `TraceItem` with its model matrix, texture, and metadata.
- **CLASSIC_GOLDEN=check**: after rendering the capture frame, the trace is
  serialized and compared line-by-line against
  `tests/golden/<scenario>/<tag>.trace.jsonl`.  On mismatch, the actual
  trace is written to `target/classic-test/<scenario>/<tag>.actual.trace.jsonl`
  for CI artifact upload.
- **CLASSIC_GOLDEN=update**: overwrites the reference file with the current
  output.
- **Baseline location**: `tests/golden/baseline/baseline.trace.jsonl`
  (70 lines, covering 6 `IsoSprite`, 54 `SdfText`, 1 `Sprite`, 1
  `Tilemap`, 7 `UiRect`, 1 `UiSprite`).

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
    loads `libEGL.so.1` and creates a pbuffer surface + GLES 3.0 context.
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

### `--test-threads=1`

All unit and integration tests must run with `--test-threads=1`.  This is
required because the global component registry (`ComponentReg`) is backed
by a `RwLock<HashMap>` that is shared across tests via `LazyLock`.  Parallel
test execution causes races on registry registration and lookup.

### `instrument::reset_for_test`

Tests that interact with the `CLASSIC_LOG` channel system should call
`classic_core::instrument::reset_for_test()` to zero out the atomic level
table.  This prevents test ordering from leaking channel levels.  Currently
used in `classic-core/tests/instrument.rs`; not needed by integration tests
that don't touch logging.

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

- **`build_test_scenario(name)` ignores the `name` parameter**: the
  parameter is accepted but discarded.  The only hardcoded scenario is
  always loaded.  `CLASSIC_TEST=all` and `CLASSIC_TEST=1` are equivalent.
  Named scenario support requires `CLASSIC_TEST_FILE` for custom scenarios
  or extending `build_test_scenario` with a match arm.

- **`Wait` action is a no-op**: the `Wait { frames }` action does nothing
  in `run_test_frame`.  To wait, schedule a `TestStep` on a later frame
  number with no actions.  The `Wait` variant exists in the type definitions
  for forward compatibility with a potential wait-until render-complete
  mechanism.

- **No pixel golden in CI**: `CLASSIC_GOLDEN_PNG=1` is not set in
  `.github/workflows/ci.yml`.  Pixel comparison is sensitive to the Mesa
  llvmpipe version and produces false positives across Ubuntu image
  updates.  The trace golden provides adequate coverage.

- **No headless on macOS or Windows**: `HeadlessPlatform` dynamically loads
  `libEGL.so.1` and is Linux-only.  CI golden tests only run on
  `ubuntu-latest`.  Local golden development on macOS requires a native
  window (omit `CLASSIC_HEADLESS`).

- **No incremental trace diffing**: mismatched traces produce a
  line-by-line diff capped at 40 lines.  For large diffs, the full actual
  trace in `target/classic-test/` must be examined manually.

- **test_editor_state is cleared after drag completion**: after
  `apply_editor_selection()`, both `test_drag_state` and `test_editor_state`
  are set to `None`.  A subsequent `SetEditor` action is required before
  another drag.

- **No multi-scenario support**: at most one scenario runs per invocation.
  The runner processes `STEPS` as a `LazyLock`, computed once per process
  lifetime.  Running multiple scenarios requires separate invocations.

- **No headless assets check**: CI runs `npm run assets` before building,
  but there is no automated check that `public/res/` matches the committed
  asset submodule.  A stale `public/res/` (missing regenerated assets)
  causes runtime errors only, not build-time errors.
