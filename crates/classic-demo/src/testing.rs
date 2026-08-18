//! CLASSIC_TEST e2e framework: scenario types, builder, and the per-frame
//! assertion runner.  Moved out of `classic-engine` because the scenario
//! exercises demo/editor behaviour (height/tile edits, tool panels, text
//! showcase), not generic engine behaviour.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::LazyLock;

use serde::Deserialize;

use classic_core::components::{Tilemap, Transform, UiNode};
use classic_core::instrument::Chan;
use classic_engine::env_config::EnvConfig;
use classic_engine::Engine;

use crate::editor::apply_editor_selection;
use crate::state::DemoStateRef;

/// Simulated user action in a test step.
#[derive(Clone, Deserialize)]
pub enum TestAction {
    #[serde(rename = "setEditor")]
    SetEditor {
        target: String,
        #[serde(rename = "heightDelta")]
        height_delta: i32,
        #[serde(rename = "heightMode")]
        height_mode: String,
        #[serde(rename = "tileId")]
        tile_id: u32,
    },
    #[serde(rename = "drag")]
    Drag {
        from: (f32, f32),
        to: (f32, f32),
        #[serde(rename = "holdFrames")]
        hold_frames: u64,
    },
    #[serde(rename = "openMenu")]
    OpenMenu,
    #[serde(rename = "enableTextDemo")]
    EnableTextDemo,
    #[serde(rename = "mouseMove")]
    MouseMove { x: f32, y: f32 },
    #[serde(rename = "mouseClick")]
    MouseClick { x: f32, y: f32, button: u32 },
    #[serde(rename = "keyPress")]
    KeyPress { key: String, pressed: bool },
    #[serde(rename = "wheel")]
    Wheel { amount: f32 },
    #[serde(rename = "wait")]
    Wait { frames: u64 },
}

/// Kinds of assertions the test runner supports.
#[derive(Clone, Copy, Debug, Deserialize)]
pub enum AssertKind {
    #[serde(rename = "height")]
    Height,
    #[serde(rename = "tile")]
    Tile,
    #[serde(rename = "uiTextCentered")]
    UiTextCentered,
    #[serde(rename = "uiEnabled")]
    UiEnabled,
    #[serde(rename = "cameraAt")]
    CameraAt,
    #[serde(rename = "entityVisible")]
    EntityVisible,
    #[serde(rename = "entityPos")]
    EntityPos,
}

/// A single assertion against a region of tile/height data or a UI property.
#[derive(Clone, Deserialize)]
pub struct TileAssertion {
    pub kind: AssertKind,
    pub region: (i32, i32, i32, i32),
    pub expected: f32,
    pub log: String,
}

/// A scheduled test step: at the given frame, execute actions then run assertions.
#[derive(Clone, Deserialize)]
pub struct TestStep {
    pub frame: u64,
    pub actions: Vec<TestAction>,
    pub assertions: Vec<TileAssertion>,
    pub log: String,
}

/// Mutable per-run state for the test runner.
#[derive(Default)]
struct TestRunner {
    step_index: usize,
    results: Vec<String>,
    drag_state: Option<(glam::Vec2, glam::Vec2, u64, u64)>,
    editor_state: Option<(String, i32, String, u32)>,
    complete_reported: bool,
}

/// Register the CLASSIC_TEST runner on the engine's test hook (no-op unless
/// `CLASSIC_TEST` is active).
pub fn install(engine: &mut Engine, state: &DemoStateRef) {
    if !EnvConfig::get().test_active() {
        return;
    }
    let state = Rc::clone(state);
    let runner = Rc::new(RefCell::new(TestRunner::default()));
    engine.set_test_runner(move |engine| {
        static STEPS: LazyLock<Vec<TestStep>> = LazyLock::new(|| {
            let name = EnvConfig::get().test.clone();
            build_test_scenario(&name)
        });
        // Set golden capture frame to 1 frame after the last test step.
        if let Some(last) = STEPS.last() {
            engine.golden_capture_frame = last.frame + 1;
        }
        run_frame(engine, &state, &mut runner.borrow_mut(), &STEPS);
    });
}

fn build_test_scenario(_name: &str) -> Vec<TestStep> {
    let config = EnvConfig::get();
    if !config.test_file.is_empty() {
        match std::fs::read_to_string(&config.test_file) {
            Ok(json) => {
                return serde_json::from_str(&json)
                    .unwrap_or_else(|e| panic!("CLASSIC_TEST_FILE {}: {}", config.test_file, e));
            }
            Err(e) => panic!("cannot read CLASSIC_TEST_FILE {}: {}", config.test_file, e),
        }
    }
    serde_json::from_str(include_str!("../../../tests/scenarios/default.test.json"))
        .expect("deserialize default test scenario")
}

fn run_frame(
    engine: &mut Engine,
    state: &DemoStateRef,
    runner: &mut TestRunner,
    steps: &[TestStep],
) {
    let frame = engine.frame_number();

    // Re-apply editor state before step processing so assertions
    // on this frame see the corrected state (tool_buttons on_update
    // resets editor_target via Rc sync earlier in the frame).
    if let Some((ref target, hd, ref mode, tid)) = runner.editor_state {
        {
            let mut s = state.borrow_mut();
            s.editor.target = target.clone();
            s.editor.height = hd;
            s.editor.height_mode = mode.clone();
            s.editor.tile = tid;
        }
        if target == "textDemo" {
            if let Some(e) = state.borrow().text_showcase_e {
                engine.set_enabled(e, true);
            }
        }
    }

    // Process any step scheduled for this frame
    while runner.step_index < steps.len() && steps[runner.step_index].frame == frame {
        let step = &steps[runner.step_index];
        classic_core::cl_info!(Chan::Test, "[FRAME {}] STEP: {}", frame, step.log);

        // Execute actions
        for action in &step.actions {
            match action {
                TestAction::SetEditor { target, height_delta, height_mode, tile_id } => {
                    {
                        let mut s = state.borrow_mut();
                        s.editor.target = target.clone();
                        s.editor.height = *height_delta;
                        s.editor.height_mode = height_mode.clone();
                        s.editor.tile = *tile_id;
                    }
                    runner.editor_state =
                        Some((target.clone(), *height_delta, height_mode.clone(), *tile_id));
                }
                TestAction::Drag { from, to, hold_frames } => {
                    let hold = *hold_frames;
                    runner.drag_state = Some((
                        glam::Vec2::new(from.0, from.1),
                        glam::Vec2::new(to.0, to.1),
                        hold,
                        frame,
                    ));
                }
                TestAction::OpenMenu => {
                    state.borrow_mut().editor.panel_menu_open = true;
                    if let Some(mp) = state.borrow().menu_panel_e {
                        engine.set_enabled(mp, true);
                    }
                }
                TestAction::EnableTextDemo => {
                    state.borrow_mut().editor.target = "textDemo".into();
                    runner.editor_state = Some(("textDemo".into(), 0, "set".into(), 0));
                    if let Some(e) = state.borrow().text_showcase_e {
                        engine.set_enabled(e, true);
                    }
                }
                TestAction::MouseMove { x, y } => {
                    let (vw, vh) = engine.viewport_size();
                    engine.input.mouse_pos = glam::Vec2::new(*x, *y);
                    engine.input.mouse_axis.x = ((*x / vw) - 0.5) * 2.0;
                    engine.input.mouse_axis.y = ((*y / vh) - 0.5) * 2.0;
                }
                TestAction::MouseClick { x, y, button } => {
                    engine.input.mouse_pos = glam::Vec2::new(*x, *y);
                    let b = *button as usize;
                    if b < 3 {
                        engine.input.mouse_down[b] = true;
                        engine.input.mouse_pressed[b] = true;
                    }
                    if b == 0 {
                        engine.input.frame_had_click = true;
                    }
                }
                TestAction::KeyPress { key, pressed } => {
                    engine.input.keys_down.insert(key.clone(), *pressed);
                    if *pressed {
                        engine.input.keys_pressed.insert(key.clone(), true);
                    }
                }
                TestAction::Wheel { amount } => {
                    engine.input.mouse_wheel = *amount;
                }
                TestAction::Wait { frames: _wait_frames } => {}
            }
        }

        // Run assertions
        for a in &step.assertions {
            let passed = match a.kind {
                AssertKind::Height => assert_heights(engine, a.region, a.expected),
                AssertKind::Tile => assert_tiles(engine, a.region, a.expected as u32),
                AssertKind::UiTextCentered => assert_ui_text_centered(engine, state, a.expected),
                AssertKind::UiEnabled => {
                    let should_be_enabled = a.expected != 0.0;
                    let is_enabled = state
                        .borrow()
                        .text_showcase_e
                        .map(|e| !engine.is_disabled(e))
                        .unwrap_or(false);
                    if is_enabled != should_be_enabled {
                        classic_core::cl_info!(
                            Chan::Test,
                            "  [UI] text showcase enabled={} expected={}",
                            is_enabled,
                            should_be_enabled
                        );
                    }
                    is_enabled == should_be_enabled
                }
                AssertKind::CameraAt => {
                    let pos_tol = if a.expected <= 0.0 { 1.0 } else { a.expected };
                    let scale_tol = if a.expected <= 0.0 { 0.01 } else { a.expected };
                    let ex = a.region.0 as f32;
                    let ey = a.region.1 as f32;
                    let ez = a.region.2 as f32;
                    let es = if a.region.3 != 0 { a.region.3 as f32 } else { 1.0 };
                    let dx = (engine.camera.position.x - ex).abs();
                    let dy = (engine.camera.position.y - ey).abs();
                    let dz = (engine.camera.position.z - ez).abs();
                    let ds = (engine.camera.scale.x - es).abs();
                    if dx > pos_tol || dy > pos_tol || dz > pos_tol || ds > scale_tol {
                        classic_core::cl_info!(
                            Chan::Test,
                            "  [Camera] pos=({:.1},{:.1},{:.1}) scale={:.2} expected pos=({},{},{}) scale={}",
                            engine.camera.position.x,
                            engine.camera.position.y,
                            engine.camera.position.z,
                            engine.camera.scale.x,
                            ex,
                            ey,
                            ez,
                            es,
                        );
                    }
                    dx <= pos_tol && dy <= pos_tol && dz <= pos_tol && ds <= scale_tol
                }
                AssertKind::EntityVisible => {
                    let name = if a.log.is_empty() { "entity" } else { &a.log };
                    let should_be_visible = a.expected != 0.0;
                    let is_visible =
                        engine.names.get(name).map(|&e| !engine.is_disabled(e)).unwrap_or(false);
                    if is_visible != should_be_visible {
                        classic_core::cl_info!(
                            Chan::Test,
                            "  [Visible] '{}' visible={} expected={}",
                            name,
                            is_visible,
                            should_be_visible
                        );
                    }
                    is_visible == should_be_visible
                }
                AssertKind::EntityPos => {
                    let name = if a.log.is_empty() { "entity" } else { &a.log };
                    let tol = if a.expected <= 0.0 { 1.0 } else { a.expected };
                    let ex = a.region.0 as f32;
                    let ey = a.region.1 as f32;
                    let passes = engine
                        .names
                        .get(name)
                        .and_then(|&e| engine.world.get::<&Transform>(e).ok())
                        .map(|tf| {
                            (tf.position.x - ex).abs() <= tol && (tf.position.y - ey).abs() <= tol
                        })
                        .unwrap_or(false);
                    if !passes {
                        if let Some(&e) = engine.names.get(name) {
                            if let Ok(tf) = engine.world.get::<&Transform>(e) {
                                classic_core::cl_info!(
                                    Chan::Test,
                                    "  [Pos] '{}' pos=({:.1},{:.1}) expected=({},{}) tol={:.1}",
                                    name,
                                    tf.position.x,
                                    tf.position.y,
                                    ex,
                                    ey,
                                    tol,
                                );
                            }
                        }
                    }
                    passes
                }
            };
            let result = format!(
                "[FRAME {}] {}: {} (region=({},{})-({},{}) expected={})",
                frame,
                if passed { "PASS" } else { "FAIL" },
                a.log,
                a.region.0,
                a.region.1,
                a.region.2,
                a.region.3,
                a.expected,
            );
            classic_core::cl_info!(Chan::Test, "{}", result);
            runner.results.push(result);
            if !passed {
                engine.test_failed = true;
                if EnvConfig::get().failfast {
                    engine.test_should_close = true;
                }
            }
        }

        runner.step_index += 1;
    }

    // Re-apply editor state every frame (tool_buttons on_update resets it via Rc sync)
    if let Some((ref target, hd, ref mode, tid)) = runner.editor_state {
        let mut s = state.borrow_mut();
        s.editor.target = target.clone();
        s.editor.height = hd;
        s.editor.height_mode = mode.clone();
        s.editor.tile = tid;
    }

    // Process active drag
    if let Some((from, to, hold, start)) = runner.drag_state {
        let rel = (frame - start) as i64;
        if rel == 0 {
            // press
            if let Some(e) = engine.entity_by_role(classic_core::RoleKind::Tilemap) {
                if let Ok(mut tm) = engine.world.get::<&mut Tilemap>(e) {
                    tm.selection_iso_begin = from.extend(0.0);
                    tm.mouse_iso_pos = from.extend(0.0);
                    engine.selection_mode = 1;
                }
            }
        } else if rel > 0 && (rel as u64) < hold {
            // drag
            if let Some(e) = engine.entity_by_role(classic_core::RoleKind::Tilemap) {
                if let Ok(mut tm) = engine.world.get::<&mut Tilemap>(e) {
                    let t = rel as f32 / hold as f32;
                    let cur = from.lerp(to, t);
                    tm.mouse_iso_pos = cur.extend(0.0);
                }
            }
        } else if (rel as u64) == hold {
            // release
            if let Some(e) = engine.entity_by_role(classic_core::RoleKind::Tilemap) {
                if let Ok(mut tm) = engine.world.get::<&mut Tilemap>(e) {
                    tm.selection_iso_end = to.extend(0.0);
                    tm.mouse_iso_pos = to.extend(0.0);
                    engine.selection_mode = -1;
                }
            }
            apply_editor_selection(engine, state);
            runner.drag_state = None;
            runner.editor_state = None;
        } else {
            runner.drag_state = None;
        }
    }

    // Report completion when all steps done and no drag in progress
    if runner.step_index >= steps.len() && runner.drag_state.is_none() {
        if !runner.complete_reported {
            let total = runner.results.len();
            let passed = runner.results.iter().filter(|r| r.contains("PASS")).count();
            classic_core::cl_info!(
                Chan::Test,
                "=== CLASSIC_TEST COMPLETE: {}/{} assertions passed ===",
                passed,
                total
            );
            runner.complete_reported = true;

            if EnvConfig::get().dump_on_exit {
                let _ = engine.dump_state();
            }
        }
        engine.test_should_close = true;
    }
}

fn assert_tiles(engine: &Engine, region: (i32, i32, i32, i32), expected: u32) -> bool {
    let Some(e) = engine.entity_by_role(classic_core::RoleKind::Tilemap) else { return false };
    let Ok(tm) = engine.world.get::<&Tilemap>(e) else {
        return false;
    };
    for y in region.1..region.3 {
        for x in region.0..region.2 {
            let idx = (y * tm.size_x + x) as usize;
            let actual = tm.data.get(idx).copied().unwrap_or(999);
            if actual != expected {
                classic_core::cl_info!(
                    Chan::Test,
                    "  tile({x},{y}) actual={actual} expected={expected}"
                );
                return false;
            }
        }
    }
    true
}

fn assert_heights(engine: &Engine, region: (i32, i32, i32, i32), expected: f32) -> bool {
    let Some(e) = engine.entity_by_role(classic_core::RoleKind::Tilemap) else { return false };
    let Ok(tm) = engine.world.get::<&Tilemap>(e) else {
        return false;
    };
    for y in region.1..region.3 {
        for x in region.0..region.2 {
            let idx = (y * (tm.size_x + 1) + x) as usize;
            let actual = tm.height_data.get(idx).copied().unwrap_or(-999.0);
            if (actual - expected).abs() > 0.01 {
                classic_core::cl_info!(
                    Chan::Test,
                    "  height({x},{y}) actual={actual:.1} expected={expected:.1}"
                );
                return false;
            }
        }
    }
    true
}

/// Verify that SDF text children of menu panel rows are correctly centered.
fn assert_ui_text_centered(engine: &Engine, state: &DemoStateRef, tolerance: f32) -> bool {
    let Some(mp) = state.borrow().menu_panel_e else {
        classic_core::cl_info!(Chan::Test, "  [UI] no menu panel entity");
        return false;
    };
    let Some(menu_node) = engine.world.get::<&UiNode>(mp).ok() else {
        return false;
    };
    let rows: Vec<hecs::Entity> = menu_node.children.iter().map(|c| c.entity).collect();
    for row_e in &rows {
        let Some(row_node) = engine.world.get::<&UiNode>(*row_e).ok() else {
            continue;
        };
        let Some(first_child) = row_node.children.first() else {
            continue;
        };
        let child_e = first_child.entity;
        let Ok(row_tf) = engine.world.get::<&Transform>(*row_e) else {
            continue;
        };
        let Ok(child_tf) = engine.world.get::<&Transform>(child_e) else {
            continue;
        };
        let (child_w, child_h) = engine
            .world
            .get::<&UiNode>(child_e)
            .ok()
            .map(|n| (n.size.x, n.size.y))
            .unwrap_or((0.0, 0.0));

        let expected_x = row_tf.position.x + row_node.size.x / 2.0 - child_w / 2.0;
        let expected_y = row_tf.position.y + row_node.size.y / 2.0 - child_h / 2.0;
        let dx = (child_tf.position.x - expected_x).abs();
        let dy = (child_tf.position.y - expected_y).abs();

        if dx > tolerance || dy > tolerance {
            classic_core::cl_info!(
                Chan::Test,
                "  [UI] row {:?} text child @ ({:.1},{:.1}) expected ({:.1},{:.1}) \
                 child_size=({:.1},{:.1}) row_size=({:.1},{:.1}) dx={:.1} dy={:.1}",
                row_e.id(),
                child_tf.position.x,
                child_tf.position.y,
                expected_x,
                expected_y,
                child_w,
                child_h,
                row_node.size.x,
                row_node.size.y,
                dx,
                dy,
            );
            return false;
        }
    }
    true
}
