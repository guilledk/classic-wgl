---
name: classic-test
description: >
    CLASSIC_TEST=1 end-to-end testing framework for classic-wgl's Rust port.
    Covers the test DSL (TestStep, TestAction, AssertKind, TileAssertion),
    `run_test_frame()` lifecycle and frame timing, assertion implementations,
    CLASSIC_UI_DEBUG logging for diagnosis, extending actions/assertions,
    and common pitfalls (editor_target Rc sync, test_editor_state re-application,
    drag timing, entity visibility checks).
    Use when writing, debugging, extending, or analyzing end-to-end tests
    triggered by `CLASSIC_TEST=1 CLASSIC_FRAMES=N cargo run -p classic-desktop`.
    Trigger phrases: "CLASSIC_TEST", "end-to-end test", "test step",
    "TestAction", "AssertKind", "debug_frame", "CLASSIC_UI_DEBUG",
    "run_test_frame", "build_test_scenario", "test_should_close",
    "test_editor_state", "test_drag_state", "UiTextCentered",
    "UiEnabled", "TestStep", "TileAssertion".
compatibility: vitest 4.x (existing), RUST_LOG=info
metadata:
    author: classic-wgl
    version: '1.0'
allowed-tools: Read, Grep, Glob, Bash(git *), Edit, Write
---

# Skill: classic-test

## Scope

Covers the `CLASSIC_TEST=1` headless end-to-end testing framework in
`crates/classic-engine/src/lib.rs`.  The framework injects simulated
user actions (clicks, drags, menu opens, editor mode changes) and
asserts game state (height data, tile data, UI entity positions,
visibility) at scheduled frames.

---

## 1. Architecture Overview

### Frame Loop Position

`CLASSIC_TEST` runs at **line ~530** in `frame()`, immediately AFTER
all `on_update` closures but BEFORE the render list is built:

```
1. resize handling → ui.resize()
2. refresh_layout() + sync_colliders()
3. physics.begin_frame() + perform_calls()
4. mouse wheel routing (text demo scroll vs camera zoom)
5. *** ALL on_update CLOSURES EXECUTE HERE ***
6. *** CLASSIC_TEST RUNS HERE ***      ← env var gated
7. SYNTH_DRAG (separate debug helper)
8. wheel decay, CLASSIC_UI_DEBUG log
9. debug_frame += 1
10. selection end / paint
11. render list built + drawn
```

**Critical**: on_update closures (tool_buttons, editor_mode_control,
camera WASD, cursor, agent system, etc.) have already executed when
the test step for the current frame is processed.  Actions in a test
step override state AFTER closures; assertions verify state that was
set by PREVIOUS frames' actions.

### Env Vars

| Var | Effect |
|---|---|
| `CLASSIC_TEST=1` | Enables test runner in `frame()` |
| `CLASSIC_FRAMES=N` | Desktop binary exits after N frames (must exceed last step) |
| `CLASSIC_UI_DEBUG=1` | Dumps all UI entity positions every frame (first 120 frames) |
| `RUST_LOG=info` | Shows test step logs, PASS/FAIL, and assertion detail |

Run command:
```bash
CLASSIC_TEST=1 CLASSIC_FRAMES=80 RUST_LOG=info cargo run -p classic-desktop
```

Use nix shell for X11 linking:
```bash
nix develop --command bash -c 'CLASSIC_TEST=1 CLASSIC_FRAMES=80 RUST_LOG=info cargo run -p classic-desktop'
```

### Early Return

When an assertion fails (`!passed`), `self.test_should_close = true` is set and
`run_test_frame()` returns.  `frame()` checks this flag and returns early —
the render list is SKIPPED.  The next frame still runs via the event loop.

When all steps are done and no drag is in progress, the test reports
`=== CLASSIC_TEST COMPLETE: N/M assertions passed ===` and sets
`test_should_close = true`.

### `debug_frame` Lifecycle

- Initialized to `0` at Engine construction.
- Incremented once per frame at **line ~630** (near end of `frame()`).
- The test uses it as the frame counter — each `TestStep.frame` is
  matched against `self.debug_frame` to determine which step to run.
- **CLASSIC_UI_DEBUG** reads `debug_frame` before increment.

---

## 2. Test Types

### `TestAction`

```rust
enum TestAction {
    SetEditor { target: String, height_delta: i32, height_mode: String, tile_id: u32 },
    Drag { from: (f32, f32), to: (f32, f32), hold_frames: u64 },
    OpenMenu,
    EnableTextDemo,
}
```

| Action | What it does |
|---|---|
| `SetEditor` | Sets `editor_target`, `editor_height`, `height_edit_mode`, `editor_tile`, AND stores `test_editor_state` for per-frame re-application |
| `Drag` | Initiates a synthetic drag: press at `from`, lerp toward `to` over `hold_frames` frames, release at `hold` frame, call `apply_editor_selection()` |
| `OpenMenu` | Sets `panel_menu_open = true`, calls `set_enabled(menu_panel_e, true)` |
| `EnableTextDemo` | Sets `editor_target = "textDemo"`, stores `test_editor_state`, calls `set_enabled(text_showcase_e, true)` |

### `AssertKind`

```rust
enum AssertKind {
    Height,         // Check height_data in region matches expected (tolerance 0.01)
    Tile,           // Check tile data in region matches expected (exact equality)
    UiTextCentered, // Check menu row labels are centered (expected = tolerance in px)
    UiEnabled,      // Check text_showcase_e entity has no Disabled marker (expected != 0 means enabled)
}
```

### `TileAssertion`

```rust
struct TileAssertion {
    kind: AssertKind,
    region: (i32, i32, i32, i32),  // (x0, y0, x1, y1) — exclusive upper bound
    expected: f32,                  // Interpretation depends on AssertKind
    log: &'static str,              // Human-readable assertion description
}
```

### `TestStep`

```rust
struct TestStep {
    frame: u64,                             // Frame number to execute
    actions: Vec<TestAction>,               // Actions executed at start of step
    assertions: Vec<TileAssertion>,         // Assertions checked AFTER actions
    log: &'static str,                      // Log message for this step
}
```

**Semantics**: At the scheduled `frame`, first all `actions` are executed,
then all `assertions` are checked.  A step with `assertions` but no
`actions` is a pure-check step (used after waiting for side effects).

---

## 3. Test Scenario Design

Tests are defined in `build_test_scenario()`, which returns a `Vec<TestStep>`.

### Frame Spacing Conventions

| Operation | Recommended wait | Reason |
|---|---|---|
| SetEditor → Drag | Same frame OK | Drag starts after editor is configured |
| Drag → Assertion | 2-4 frames after release | Time for mesh rebuild / GPU upload |
| OpenMenu / EnableTextDemo → Assertion | 2 frames | Time for `on_update` to enable entity, measure text, position children |
| SetEditor (no drag) → UiEnabled | 2 frames | `editor_mode_control` runs in on_update on NEXT frame |

### Drag Hold Timing

`hold_frames` is the number of frames from press to release (inclusive).
A drag with `hold_frames=4`:
- Frame N:   `SetEditor` + `Drag` action → sets `test_drag_state` with `from, to, hold=4, start=N`
- Frame N:   `rel=0` → press at `from`
- Frame N+1: `rel=1` → lerp toward `to` (25% progress)
- Frame N+2: `rel=2` → lerp toward `to` (50% progress)
- Frame N+3: `rel=3` → lerp toward `to` (75% progress)
- Frame N+4: `rel=4 == hold` → release at `to`, call `apply_editor_selection()`

Assertions should be scheduled at N+4+3 = N+7 or later to allow for mesh rebuild.

### Current Test Sequence (16 steps, frames 2–54)

| Step | Frame | Actions | Assertions | What it validates |
|---|---|---|---|---|
| 0 | 2 | `OpenMenu` | — | Open dev menu panel |
| 1 | 4 | — | `UiTextCentered(2.0)` | Menu row labels are centered within containers |
| 2 | 5 | `SetEditor(height, blend, 2)` | — | Configure height editor |
| 3 | 5 | `Drag(10,10)→(14,14) hold=4` | — | Start blend drag |
| 4 | 13 | — | Height(10,10-14,14)=3.0, (0,0-2,2)=1.0, (16,10-18,12)=1.0 | Blend region correct, adjacent untouched |
| 5 | 16 | `SetEditor(height, set, 5)` | — | Switch to set mode |
| 6 | 16 | `Drag(10,10)→(14,14) hold=4` | — | Set mode drag |
| 7 | 24 | — | Height(10,10-14,14)=5.0, (0,0-2,2)=1.0 | Set values applied, untouched remains |
| 8 | 27 | `SetEditor(height, blend, 3)` | — | Blend mode |
| 9 | 27 | `Drag(20,10)→(22,12) hold=4` | — | Different region |
| 10 | 35 | — | Height(20,10-22,12)=4.0, (10,10-11,11)=5.0 | Blend vs set regions independent |
| 11 | 38 | `SetEditor(tilemap, ... tile_id=7)` | — | Tile editor mode |
| 12 | 38 | `Drag(8,8)→(9,9) hold=3` | — | Single tile paint |
| 13 | 45 | — | Tile(8,8-9,9)=7, (10,10-11,11)=9 | Tile paint correct, adjacent untouched |
| 14 | 48 | `SetEditor+`+`Drag (25,10)→(26,10) `hold=2` | Height(25,10-26,11)=1.0 | Zero-delta blend preserves value |
| 15 | 52 | `EnableTextDemo` | — | Enable text demo panel |
| 16 | 54 | — | `UiEnabled=1.0` | Text showcase entity is enabled |

---

## 4. Frame Timing Deep Dive

### `run_test_frame()` Lifecycle

```
1. Re-apply test_editor_state (BEFORE steps)
   ├─ sets editor_target, editor_height, height_edit_mode, editor_tile
   └─ if target=="textDemo", calls set_enabled(text_showcase_e, true)

2. Process matching steps (while step.frame == debug_frame)
   ├─ Execute actions (SetEditor, Drag, OpenMenu, EnableTextDemo)
   └─ Execute assertions (Height, Tile, UiTextCentered, UiEnabled)

3. Re-apply test_editor_state (AFTER steps, every frame)
   └─ Ensures editor_target persists even on frames without steps

4. Process active drag
   ├─ rel==0: press → set selection_iso_begin, selection_mode=1
   ├─ 0<rel<hold: drag → lerp mouse_iso_pos toward `to`
   ├─ rel==hold: release → set selection_iso_end, call apply_editor_selection()
   └─ rel>hold: clear test_drag_state

5. Check completion
   └─ If all steps done + no active drag → report, set test_should_close
```

### Why test_editor_state is Re-Applied Twice

**Problem**: `tool_buttons` on_update closure runs EVERY frame and syncs
`editor_target` from an Rc value (deaulting to `"none"`).  Any test-set
`editor_target` is overwritten by the closure on subsequent frames.

**Solution**: `test_editor_state` stores the test's desired editor config.
It is re-applied:

1. **At the TOP of `run_test_frame()`** (before step processing) — so
   assertions on THIS frame see the corrected state.
2. **After step processing** (every frame) — so subsequent frames
   (including non-step frames between steps) retain the test state.

The `EnableTextDemo` action additionally calls `self.set_enabled(e, true)`
directly because the re-application only sets `editor_target` — the
`editor_mode_control` on_update (which toggles visibility) has already
run at that point.  On subsequent frames, the re-application checks
`target == "textDemo"` and also calls `set_enabled`.

---

## 5. Assertion Implementations

### `assert_heights(region, expected)` → bool

Iterates height data in the region.  Height data uses **stride `size_x + 1`**
(one extra sample per row for edge vertices).  Uses tolerance `0.01`.

```rust
fn assert_heights(&self, region: (i32, i32, i32, i32), expected: f32) -> bool {
    let tm = self.world.get::<&Tilemap>(tilemap_entity).unwrap();
    for y in region.1..region.3 {
        for x in region.0..region.2 {
            let idx = (y * (tm.size_x + 1) + x) as usize;
            let actual = tm.height_data.get(idx).copied().unwrap_or(-999.0);
            if (actual - expected).abs() > 0.01 {
                return false;
            }
        }
    }
    true
}
```

### `assert_tiles(region, expected)` → bool

Iterates tile data in the region.  Tile data uses **stride `size_x`**
(tile cells, not vertices).  Uses exact equality.  Out-of-bounds returns `999`.

### `assert_ui_text_centered(tolerance)` → bool

Walks the menu panel entity's children (row buttons).  For each row:
1. Gets the first child entity (the SDF text label)
2. Reads child `UiNode.size` for actual text dimensions
3. Computes expected position: `row_pos + row_size/2 - text_size/2`
4. Checks both `dx` and `dy` against `tolerance` in pixels

Uses `self.menu_panel_e` to find the menu, then `UiNode.children` to walk rows.

### `UiEnabled` (inline assertion)

Checks `is_disabled(self.text_showcase_e.unwrap())` — returns true when
the entity has NO `Disabled` marker AND no parent-chain ancestor has one.
`expected != 0.0` means "should be enabled".

---

## 6. CLASSIC_UI_DEBUG Diagnostic Logging

Enable with `CLASSIC_UI_DEBUG=1`.  Logs per-frame entity state for the
first 120 frames (gated by `self.debug_frame < 120`).

**Timing**: Runs after CLASSIC_TEST and SYNTH_DRAG, but BEFORE
`debug_frame += 1`.  So the frame number matches the test step's `frame`.

**Output per frame**:
```
=== frame 4 vp=1280x720 ===
  [Container] 0 pos=(0,0) size=(1280,720) z=-1000 enabled=true parent=None children=3
  [Array] 1 pos=(64,456) size=(128,208) z=-1000 enabled=true parent=None children=2
  [Container] 2 pos=(64,528) size=(128,128) z=-1000 enabled=true parent=Some(1) children=1
  [SdfText] 3 pos=(80,553) size=(96,20) z=-1000 enabled=true parent=Some(2) children=0
```

**Interpretation**:
- `z` = layer for z-sorted draw order (more negative = on top)
- `enabled` = `true` means NO `Disabled` component is present
- `parent` = entity ID of the container (from `UiNode.parent`)
- `children` = count of `UiNode.children` entries

**Combined with CLASSIC_TEST**: Add `CLASSIC_UI_DEBUG=1` to the env vars
to see exact entity positions on assertion failure frames.  This reveals
whether a positioning bug is in the layout system or in the assertion
formula itself.

---

## 7. Adding New Assertions

**Step 1**: Add variant to `AssertKind`:
```rust
enum AssertKind {
    Height,
    Tile,
    UiTextCentered,
    UiEnabled,
    MyNewAssertion,  // ← add here
}
```

**Step 2**: Add assertion check in `run_test_frame()`:
```rust
for a in &step.assertions {
    let passed = match a.kind {
        AssertKind::Height => self.assert_heights(a.region, a.expected),
        AssertKind::MyNewAssertion => self.assert_my_new_thing(a.region, a.expected),
        // ...
    };
    // PASS/FAIL log and test_results.push handled generically below
}
```

**Step 3**: Implement the assertion method on `Engine`:
```rust
fn assert_my_new_thing(&self, region: (i32, i32, i32, i32), expected: f32) -> bool {
    // Query entity, check property, return true/false
    // MUST log::info!() on failure with actual vs expected values
    true
}
```

**Rules**:
- Return `false` on failure (triggers `test_should_close = true`)
- Use `log::info!()` for detailed failure diagnostics (actual values, entity IDs)
- Use `region` for spatial scoping (interpret flexibly — coordinates, entity indices, etc.)
- Use `expected` for the expected value (default `0.0` for boolean checks)

---

## 8. Adding New Actions

**Step 1**: Add variant to `TestAction`:
```rust
enum TestAction {
    // ... existing variants ...
    MyNewAction { some_param: String },
}
```

**Step 2**: Add action handling in `run_test_frame()`:
```rust
for action in &step.actions {
    match action {
        // ... existing actions ...
        TestAction::MyNewAction { some_param } => {
            self.my_field = some_param.clone();
            self.test_editor_state = Some(("myTarget".into(), 0, "set".into(), 0));
        }
    }
}
```

**Step 3**: If the action sets editor state that must persist across frames,
store it in `test_editor_state`.  The per-frame re-application (both at
the top and after steps) will restore it on subsequent frames, countering
the `tool_buttons` Rc sync overwrite.

**Rules**:
- Store in `test_editor_state` if the action changes `editor_target` or related fields
- Call `self.set_enabled()` directly if enabling/disabling entities
- Don't set `test_drag_state` unless the action is a synthetic drag
- Re-apply logic in the `test_editor_state` re-application block if needed (e.g., `if target == "textDemo"`)

---

## 9. Common Pitfalls

### 1. `tool_buttons` Rc Sync Overwrites `editor_target`

**Symptom**: `SetEditor` works but subsequent frames revert to `"none"`.

**Cause**: The `tool_buttons` on_update runs every frame and sets
`engine.editor_target = et2.borrow().clone()`, where `et2` is an
`Rc<RefCell<String>>` defaulting to `"none"`.  This runs AFTER
`SetEditor` in the same frame but BEFORE the next frame's test assertions.

**Fix**: Always store in `test_editor_state`.  The per-frame re-application
(at the TOP of `run_test_frame`) restores the value before assertions run.
For direct entity enables (like `text_showcase_e`), also call
`set_enabled()` in both the action handler AND the re-application block.

### 2. `EnableTextDemo` Must Call `set_enabled()` Directly

**Symptom**: `UiEnabled` assertion fails even though `editor_target == "textDemo"`.

**Cause**: The test sets `editor_target` but `editor_mode_control` on_update
(which toggles visibility) has already run for this frame.  Setting the
target doesn't retroactively enable the entity.

**Fix**: `EnableTextDemo` calls `self.set_enabled(e, true)` directly.
The per-frame re-application block also checks `target == "textDemo"`
and re-calls `set_enabled(e, true)` on subsequent frames.

### 3. `test_should_close` Early Return Skips Render

**Symptom**: Test passes but no visual output on the last frame.

**Cause**: `test_should_close = true` causes `frame()` to `return` early,
skipping the render list.  This is by design — the test is headless.

**Fix**: None needed.  If you need visual output for debugging, use
`CLASSIC_FRAMES=80` (no `CLASSIC_TEST=1`) with `RUST_LOG=info` to
run interactively.

### 4. Drag Assertion Frame Timing

**Symptom**: Height/Tile assertion fails with unexpected values.

**Cause**: Assertions fire too soon after drag release — mesh hasn't
been rebuilt yet.  `apply_editor_selection()` calls mesh rebuild, but
the GPU upload happens in the render path of the SAME frame.

**Fix**: Wait 2-4 frames after the release frame.  For example, a drag
starting at frame 5 with `hold_frames=4` releases at frame 9.
Schedule the assertion at frame 13 (9 + 4).

### 5. CLASSIC_FRAMES Must Exceed Last Step

**Symptom**: Test appears to hang or exits without completion.

**Cause**: `CLASSIC_FRAMES=50` but last step is at frame 54 — the event
loop exits before all steps execute.

**Fix**: Set `CLASSIC_FRAMES` ≥ last_step_frame + 5.  For the current
test scenario, use `CLASSIC_FRAMES=80`.

### 6. `UiNode.size` Must Be Pre-Measured for Text Assertions

**Symptom**: `UiTextCentered` assertion fails on first visible frame.

**Cause**: `spawn_sdf_text` creates text with `UiNode.size = (max_width, 0)`.
The size is only updated when the SDF render pass builds the glyph buffer.
`measure_all_ui_labels()` pre-measures all text at init time, ensuring
correct sizes from frame 0.

**Fix**: Ensure `measure_all_ui_labels()` is called after all UI init
(in `apps/desktop/src/main.rs` and `apps/web/src/lib.rs`).

---

## 10. Debug Workflow

### Standard Investigation

```bash
# 1. Run the test suite
CLASSIC_TEST=1 CLASSIC_FRAMES=80 RUST_LOG=info cargo run -p classic-desktop 2>&1 | grep -E "PASS|FAIL|COMPLETE"

# 2. If FAIL: find the failing assertion
CLASSIC_TEST=1 CLASSIC_FRAMES=80 RUST_LOG=info cargo run -p classic-desktop 2>&1 | grep "FAIL"

# 3. Look at the frame and assertion log
#    [FRAME 13] FAIL: height(10,10-14,14)=3.0 (region ... expected=3)
#    → Something is wrong with heights at (10,10)-(14,14)

# 4. Add CLASSIC_UI_DEBUG=1 for entity position dump
CLASSIC_UI_DEBUG=1 CLASSIC_TEST=1 CLASSIC_FRAMES=80 RUST_LOG=info cargo run -p classic-desktop 2>&1 | grep -A5 "FAIL"

# 5. If the assertion is about UI (position, centering, visibility):
#    Look at the CLASSIC_UI_DEBUG output for the FAIL frame
#    → Check entity positions, sizes, enabled states, parent/child chains

# 6. Narrow the failure:
#    - Is the value completely wrong? → Logic bug in paint/rebuild
#    - Is it close but off by a margin? → Tolerance too tight or timing issue
#    - Is the entity even present? → Check entity creation + set_enabled
```

### Interactive Debugging

For visual inspection without test automation:
```bash
RUST_LOG=info cargo run -p classic-desktop
```

Then manually trigger the failing interaction path (e.g., open menu,
select editor mode, perform drag).

### Adding Targeted Logs

Add `log::info!("  [DEBUG] my_entity pos=({:.1},{:.1})", ...)` in the
assertion method or in the `on_update` closure to trace specific values.

---

## 11. Reference

### Execution Order Diagram

```
Engine::frame() called once per frame
│
├─ Begin: clone input, set delta
├─ Resize handling (viewport change detection)
├─ UI refresh_layout() + sync_colliders()   ← positions root-tree entities
├─ Physics begin_frame() + perform_calls()  ← click handlers fire
├─ UI update_hover()                        ← hover blending
├─ Mouse wheel routing                      ← scroll text demo OR pass to camera
├─ *** ALL on_update() CLOSURES ***         ← camera zoom, cursor, tool_buttons, etc.
│   ├─ tool_buttons: sync Rc→engine, position array, menu panel, backdrop
│   ├─ editor_mode_control: set_enabled based on editor_target
│   ├─ height_widget: position buttons, labels
│   ├─ light_widget: position panel, labels
│   ├─ text_showcase: position texts, scrollbar
│   └─ navigation: pathfind on click
├─ *** CLASSIC_TEST ***
│   ├─ Re-apply test_editor_state ← counters tool_buttons Rc overwrite
│   ├─ Process matching step actions
│   ├─ Run step assertions ← checks game state
│   ├─ Re-apply test_editor_state (every frame)
│   ├─ Process active drag (press/drag/release)
│   └─ Check completion → test_should_close
├─ SYNTH_DRAG (separate debug helper)
├─ Mouse wheel decay + clamp
├─ CLASSIC_UI_DEBUG log ← retired; use CLASSIC_LOG=ui=trace
├─ debug_frame += 1
├─ Selection end / apply_editor_selection (real user drag)
├─ Build render list (collect Sprites/Tilemaps/IsoSprites/UiRect/SdfText)
│   → SdfText is now in the main z-sorted list, not a separate pass
├─ Sort by z (descending) + pre-compute debug names
└─ Draw: single z-sorted pass with SdfText inline
```

### Env Var Cheat Sheet

| Env Var | Value | Purpose |
|---|---|---|
| `CLASSIC_TEST` | `1` or scenario name | Enable test runner (specific scenario or all) |
| `CLASSIC_FRAMES` | `80` | Frame limit (must exceed last test step) |
| `CLASSIC_UI_DEBUG` | `1` | Retired — use `CLASSIC_LOG=ui=trace` instead |
| `CLASSIC_SYNTH_DRAG` | start_frame | Retired — subsumed by `DragIso` test action |
| `CLASSIC_FIXED_DT` | e.g. `0.0167` | Fixed timestep (auto `1/60` under CLASSIC_TEST) |
| `CLASSIC_WIDTH` / `CLASSIC_HEIGHT` | `640` / `360` | Forced viewport size |
| `CLASSIC_LOG` | `all=info,gfx=trace` | Channel-gated logging (see classic-logging skill) |
| `CLASSIC_GOLDEN` | `check` \| `update` | Golden test mode |
| `CLASSIC_GOLDEN_PNG` | `1` | Enable pixel capture for golden tests |
| `CLASSIC_DUMP_DIR` | path | Native state-dump output dir (default `./dump/`) |
| `CLASSIC_DUMP_ON_EXIT` | `1` | Auto-dump state on shutdown |
| `RUST_LOG` | `info` | Show test step/assertion logs |

### Test Types Quick Reference

| AssertKind | `region` meaning | `expected` meaning |
|---|---|---|
| `Height` | (x0, y0, x1, y1) tile coords | height value (f32, tolerance 0.01) |
| `Tile` | (x0, y0, x1, y1) tile coords | tile ID (u32, exact) |
| `UiTextCentered` | (not used) | tolerance in pixels (f32) |
| `UiEnabled` | (not used) | 0.0 = should be disabled, else enabled |

## 12. CI Exit Code

`Engine::test_failed: bool` is set to `true` on any assertion failure
(`classic-engine/src/lib.rs:~4225`). The desktop binary checks this
after the run loop exits and calls `std::process::exit(1)` on failure
(`apps/desktop/src/main.rs:~96`).

Previously, test failures set `test_should_close = true` but the process
still exited 0, meaning CI could not detect failures.

## 13. LazyLock Scenario Cache

`build_test_scenario()` is now cached via `std::sync::LazyLock` at
`classic-engine/src/lib.rs:~571`:

```rust
static STEPS: std::sync::LazyLock<Vec<TestStep>> =
    std::sync::LazyLock::new(Engine::build_test_scenario);
```

Previously the scenario was rebuilt **every frame** (perf waste).
The `LazyLock` computes it once on first access and reuses it.

## 14. DebugName Component

Every named entity (from `state.json`) gets a `DebugName(String)` component
at load time. UI entities get path-style names from the UIManager factories.
`Engine::debug_name(e: hecs::Entity) -> String` reads `DebugName`, falls
back to `e#<id>`. This replaces the previous O(n) `self.names.iter()`
reverse lookups and provides stable identity for logs, golden traces, and
CLASSIC_UI_DEBUG output.
