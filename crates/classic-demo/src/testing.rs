//! CLASSIC_TEST e2e framework: scenario types, builder, and the per-frame
//! assertion runner.  Moved out of `classic-engine` because the scenario
//! exercises demo/editor behaviour (height/tile edits, tool panels, text
//! showcase), not generic engine behaviour.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::LazyLock;

use serde::Deserialize;

use classic_core::components::{IsoVehicle, Tilemap, Transform, UiNode};
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
    #[serde(rename = "setCameraIso")]
    SetCameraIso { tx: f32, ty: f32, scale: f32 },
    #[serde(rename = "setMouseIso")]
    SetMouseIso { tx: f32, ty: f32 },
    #[serde(rename = "setEntityPos")]
    SetEntityPos { name: String, x: f32, y: f32, z: f32 },
    #[serde(rename = "setSpriteFrame")]
    SetSpriteFrame { name: String, frame: f32 },
    /// Pin the camera from this frame on, re-applied every frame (the scene
    /// guest frames its own view, and a followed entity moves).
    #[serde(rename = "camera")]
    Camera(CameraPin),
    /// Spawn a grid of vehicles (stress and review scenes).
    #[serde(rename = "spawnVehicles")]
    SpawnVehicles(SpawnVehicles),
}

/// A `spawnVehicles` step: `count` copies of the vehicle def `vehicle` (a
/// registered name, or the bare part of an `ns::name`) on a `cols`-wide grid
/// from tile `origin`, `spacing` tiles apart, named `{prefix}{i}`.  Each
/// starts at heading `i · heading_step_deg`; a nonzero `spin_deg_per_sec`
/// turns them all in place every frame from then on.
#[derive(Clone, Debug, Deserialize)]
pub struct SpawnVehicles {
    pub vehicle: String,
    pub count: u32,
    pub origin: (f32, f32),
    #[serde(default = "default_cols")]
    pub cols: u32,
    #[serde(default = "default_spacing")]
    pub spacing: f32,
    #[serde(default = "default_prefix")]
    pub prefix: String,
    #[serde(default)]
    pub heading_step_deg: f32,
    #[serde(default)]
    pub spin_deg_per_sec: f32,
}

fn default_cols() -> u32 {
    16
}

fn default_spacing() -> f32 {
    3.0
}

fn default_prefix() -> String {
    "stress".into()
}

/// A `camera` step: follow a named entity (`follow`) or hold an iso tile
/// (`tile`), centred [`CAMERA_LIFT_M`] above the point at `zoom`.  Neither
/// releases the pin.
#[derive(Clone, Debug, Deserialize)]
pub struct CameraPin {
    /// An exact entity name, else the bare part of an `ns::name` (`rocket`
    /// finds `lunar::rocket`); emitters and lights frame their resolved
    /// attachment point (`Engine::named_world_point`).
    #[serde(default)]
    pub follow: Option<String>,
    /// Iso tile `(x, y)` to hold.
    #[serde(default)]
    pub tile: Option<(f32, f32)>,
    #[serde(default = "unit_zoom")]
    pub zoom: f32,
}

fn unit_zoom() -> f32 {
    1.0
}

/// Metres above the pinned point the camera centres on.
pub const CAMERA_LIFT_M: f32 = 1.5;

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
    #[serde(rename = "pixelAtEntity")]
    PixelAtEntity,
}

/// A single assertion against a region of tile/height data or a UI property.
#[derive(Clone, Deserialize)]
pub struct TileAssertion {
    pub kind: AssertKind,
    pub region: (i32, i32, i32, i32),
    pub expected: f32,
    pub log: String,
    /// Expected RGBA (normalized `[0, 1]`) for `PixelAtEntity`.
    #[serde(default)]
    pub color: Option<[f32; 4]>,
    /// Tile-space `(dx, dy)` offset from the entity anchor for `PixelAtEntity`.
    #[serde(default)]
    pub offset: (f32, f32),
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
    pos_override: Option<(String, glam::Vec3)>,
    frame_override: Option<(String, f32)>,
    /// The active `camera` pin and whether its target has resolved yet.
    camera: Option<(CameraPin, bool)>,
    /// Vehicles a `spawnVehicles` step turns every frame, and the rate (rad/s).
    spin: Option<(Vec<String>, f32)>,
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

/// Every scenario under `tests/scenarios/`, compiled in so a run never depends
/// on the process cwd.  The key is both the file stem and the `CLASSIC_TEST`
/// value that selects it.
///
/// One table, N views (AGENTS.md "Patterns" #1): the `scenario_table_*` tests
/// assert this covers the directory exactly, so a new scenario file cannot go
/// silently unrun the way `render_order`, `container_ghost` and `rocket` did
/// while `build_test_scenario` discarded its argument.
pub const SCENARIOS: &[(&str, &str)] = &[
    ("default", include_str!("../../../tests/scenarios/default.test.json")),
    ("render_order", include_str!("../../../tests/scenarios/render_order.test.json")),
    ("rocket", include_str!("../../../tests/scenarios/rocket.test.json")),
    ("lighttest", include_str!("../../../tests/scenarios/lighttest.test.json")),
];

/// `CLASSIC_TEST` values that only mean "turn the runner on".  Kept because CI,
/// AGENTS.md and the golden runbooks all invoke `CLASSIC_TEST=all`, and the
/// demo baseline's capture frame is derived from the default scenario's last
/// step — repointing `all` would silently invalidate it.
const DEFAULT_ALIASES: &[&str] = &["1", "true", "all", "default"];

fn build_test_scenario(name: &str) -> Vec<TestStep> {
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
    let key = if DEFAULT_ALIASES.contains(&name) { "default" } else { name };
    let Some((_, json)) = SCENARIOS.iter().find(|(k, _)| *k == key) else {
        let known: Vec<&str> = SCENARIOS.iter().map(|(k, _)| *k).collect();
        panic!(
            "CLASSIC_TEST={name:?}: no such scenario.  Known: {known:?} \
             (or point CLASSIC_TEST_FILE at a scenario JSON)"
        );
    };
    serde_json::from_str(json).unwrap_or_else(|e| panic!("scenario {key}: {e}"))
}

/// Centre the camera on a `camera` pin's point, if it resolves this frame (a
/// followed entity may only appear once the scene's terrain is committed).
fn apply_camera_pin(engine: &mut Engine, pin: &CameraPin, found: &mut bool) {
    let point = match (&pin.follow, pin.tile) {
        (Some(name), _) => engine.named_world_point(name),
        (None, Some((x, y))) => engine.iso_to_world(x, y, 0.0),
        (None, None) => None,
    };
    let Some(p) = point else { return };
    if !*found {
        *found = true;
        classic_core::cl_info!(Chan::Test, "  [camera] framing {p:?} at zoom {}", pin.zoom);
    }
    // `Camera::fix` puts `position` (pre-zoom px) at the viewport centre.
    let px = classic_core::math::iso_camera_px(p + glam::Vec3::Z * CAMERA_LIFT_M);
    engine.set_camera(px.x, px.y, pin.zoom);
}

fn run_frame(
    engine: &mut Engine,
    state: &DemoStateRef,
    runner: &mut TestRunner,
    steps: &[TestStep],
) {
    let frame = engine.frame_number();

    // Re-apply any persistent entity-position override (a `setEntityPos`
    // action) before step processing, so the entity stays put even though the
    // guest's update closure (which runs earlier in the frame) follows the
    // mouse every frame.
    if let Some((ref name, pos)) = runner.pos_override {
        if let Some(&e) = engine.names.get(name) {
            if let Ok(mut tf) = engine.world.get::<&mut Transform>(e) {
                tf.position = pos;
            }
        }
    }
    if let Some((ref name, frame)) = runner.frame_override {
        engine.set_sprite_frame(name, frame);
    }
    if let Some((names, rate)) = runner.spin.as_ref() {
        let dt = engine.time.delta;
        for name in names {
            if let Some(&e) = engine.names.get(name) {
                if let Ok(mut v) = engine.world.get::<&mut IsoVehicle>(e) {
                    v.heading += rate * dt;
                }
            }
        }
    }
    // Re-apply the `camera` pin over the guest's own framing.
    if let Some((pin, found)) = runner.camera.as_mut() {
        apply_camera_pin(engine, pin, found);
    }

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
                // Delivered next frame: the runner runs after the update
                // closures, and `frame` rebuilds `input` from the platform.
                TestAction::KeyPress { key, pressed } => {
                    if *pressed {
                        engine.scripted_input.press(key);
                    } else {
                        engine.scripted_input.release(key);
                    }
                }
                TestAction::Wheel { amount } => {
                    engine.input.mouse_wheel = *amount;
                }
                TestAction::Wait { frames: _wait_frames } => {}
                TestAction::SetCameraIso { tx, ty, scale } => {
                    if let Some((cx, cy)) = engine.iso_to_screen(*tx, *ty) {
                        engine.set_camera(cx, cy, *scale);
                    }
                }
                TestAction::SetMouseIso { tx, ty } => {
                    if let Some((sx, sy)) = crate::render_order::iso_to_screen_px(engine, *tx, *ty)
                    {
                        let (vw, vh) = engine.viewport_size();
                        engine.input.mouse_pos = glam::Vec2::new(sx, sy);
                        engine.input.mouse_axis.x = ((sx / vw) - 0.5) * 2.0;
                        engine.input.mouse_axis.y = ((sy / vh) - 0.5) * 2.0;
                    }
                }
                TestAction::SetEntityPos { name, x, y, z } => {
                    runner.pos_override = Some((name.clone(), glam::Vec3::new(*x, *y, *z)));
                    let ok = engine
                        .names
                        .get(name)
                        .and_then(|&e| engine.world.get::<&mut Transform>(e).ok())
                        .map(|mut tf| {
                            tf.position = glam::Vec3::new(*x, *y, *z);
                            true
                        })
                        .unwrap_or(false);
                    if !ok {
                        classic_core::cl_info!(Chan::Test, "  [SetEntityPos] no entity '{name}'");
                    }
                }
                TestAction::SetSpriteFrame { name, frame } => {
                    runner.frame_override = Some((name.clone(), *frame));
                    engine.set_sprite_frame(name, *frame);
                }
                TestAction::SpawnVehicles(sv) => {
                    let names = spawn_vehicles(engine, sv);
                    classic_core::cl_info!(
                        Chan::Test,
                        "  [spawnVehicles] {} of {} '{}'",
                        names.len(),
                        sv.count,
                        sv.vehicle
                    );
                    if sv.spin_deg_per_sec != 0.0 {
                        runner.spin = Some((names, sv.spin_deg_per_sec.to_radians()));
                    }
                }
                TestAction::Camera(pin) => {
                    let active = pin.follow.is_some() || pin.tile.is_some();
                    runner.camera = active.then(|| (pin.clone(), false));
                    if let Some((pin, found)) = runner.camera.as_mut() {
                        apply_camera_pin(engine, pin, found);
                    }
                }
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
                AssertKind::PixelAtEntity => {
                    let name = if a.log.is_empty() { "entity" } else { &a.log };
                    let tol = if a.expected <= 0.0 { 0.02 } else { a.expected };
                    crate::render_order::assert_pixel_at_entity(
                        engine, name, a.color, tol, a.offset,
                    )
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

/// Resolve a vehicle def name: exact, else the def whose bare (post-`::`)
/// name matches.
fn resolve_vehicle_def(engine: &Engine, name: &str) -> Option<String> {
    if engine.vehicles.contains_key(name) {
        return Some(name.to_string());
    }
    let mut keys: Vec<&String> = engine.vehicles.keys().collect();
    keys.sort();
    keys.into_iter().find(|k| k.rsplit("::").next() == Some(name)).cloned()
}

/// Run a `spawnVehicles` step; returns the names that spawned.
fn spawn_vehicles(engine: &mut Engine, sv: &SpawnVehicles) -> Vec<String> {
    let Some(def) = resolve_vehicle_def(engine, &sv.vehicle) else {
        classic_core::cl_info!(Chan::Test, "  [spawnVehicles] no vehicle def '{}'", sv.vehicle);
        return Vec::new();
    };
    let cols = sv.cols.max(1);
    let mut names = Vec::new();
    for i in 0..sv.count {
        let (cx, cy) = ((i % cols) as f32, (i / cols) as f32);
        let (x, y) = (sv.origin.0 + cx * sv.spacing, sv.origin.1 + cy * sv.spacing);
        let name = format!("{}{i}", sv.prefix);
        if !engine.spawn_vehicle(&def, &name, x, y) {
            continue;
        }
        if let Some(&e) = engine.names.get(&name) {
            if let Ok(mut v) = engine.world.get::<&mut IsoVehicle>(e) {
                v.heading = (i as f32 * sv.heading_step_deg).to_radians();
            }
        }
        names.push(name);
    }
    names
}

#[cfg(test)]
mod tests {
    use super::{CameraPin, TestAction, TestStep, DEFAULT_ALIASES, SCENARIOS};

    fn scenarios_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/scenarios")
    }

    /// The gate that was missing: a scenario file that is not in `SCENARIOS` is
    /// unreachable via `CLASSIC_TEST=<name>`, and a table entry with no file
    /// would not compile.  Keep the two in lockstep.
    #[test]
    fn scenario_table_covers_the_directory() {
        let mut on_disk: Vec<String> = std::fs::read_dir(scenarios_dir())
            .expect("tests/scenarios must exist")
            .filter_map(|entry| {
                let name = entry.ok()?.file_name().to_string_lossy().into_owned();
                name.strip_suffix(".test.json").map(str::to_string)
            })
            .collect();
        on_disk.sort();

        let mut registered: Vec<String> = SCENARIOS.iter().map(|(k, _)| k.to_string()).collect();
        registered.sort();

        assert_eq!(
            on_disk, registered,
            "every tests/scenarios/<name>.test.json must appear in SCENARIOS, else \
             CLASSIC_TEST=<name> cannot reach it"
        );
    }

    /// A scenario whose JSON has rotted past the `TestStep` schema is a test
    /// that cannot run.  Parse them all, every `cargo test`.
    #[test]
    fn every_registered_scenario_parses() {
        for (name, json) in SCENARIOS {
            let steps: Vec<TestStep> =
                serde_json::from_str(json).unwrap_or_else(|e| panic!("scenario {name}: {e}"));
            assert!(!steps.is_empty(), "scenario {name} has no steps");
            assert!(
                steps.windows(2).all(|w| w[0].frame <= w[1].frame),
                "scenario {name}: steps must be in non-decreasing frame order, the runner 
                 only ever advances `step_index` (equal frames are fine, its inner loop 
                 drains every step scheduled for the current frame)"
            );
        }
    }

    /// The `camera` step's two targets and its release.
    #[test]
    fn camera_steps_parse() {
        let steps: Vec<TestStep> = serde_json::from_str(
            r#"[{ "frame": 2, "assertions": [], "log": "pin", "actions": [
                { "camera": { "follow": "rocket", "zoom": 0.3 } },
                { "camera": { "tile": [317.5, 277.5] } },
                { "camera": {} }
            ] }]"#,
        )
        .expect("camera steps parse");
        let pins: Vec<&CameraPin> = steps[0]
            .actions
            .iter()
            .map(|a| match a {
                TestAction::Camera(pin) => pin,
                _ => panic!("not a camera step"),
            })
            .collect();
        assert_eq!(pins[0].follow.as_deref(), Some("rocket"));
        assert_eq!(pins[0].zoom, 0.3);
        assert_eq!(pins[1].tile, Some((317.5, 277.5)));
        assert_eq!(pins[1].zoom, 1.0, "zoom defaults to 1");
        assert!(pins[2].follow.is_none() && pins[2].tile.is_none(), "empty releases");
    }

    /// A scenario `keyPress` reaches the update closures (the guest) on the
    /// next frame, once, and a held key stays down until released.  Before
    /// the scripted queue, `frame` overwrote the runner's write to
    /// `engine.input` with the platform snapshot, so no closure ever saw it.
    #[test]
    fn scenario_key_press_reaches_update_closures_next_frame() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let steps: Vec<TestStep> = serde_json::from_str(
            r#"[{ "frame": 2, "assertions": [], "log": "press", "actions": [
                    { "keyPress": { "key": "KeyN", "pressed": true } } ] },
                { "frame": 5, "assertions": [], "log": "release", "actions": [
                    { "keyPress": { "key": "KeyN", "pressed": false } } ] }]"#,
        )
        .unwrap();
        let mut engine = classic_engine::Engine::new_for_test();
        let seen: Rc<RefCell<Vec<(u64, bool, bool)>>> = Rc::default();
        let log = Rc::clone(&seen);
        // The guest's update closure, then the runner (the real frame order).
        engine.on_update(move |e| {
            let f = e.frame_number();
            log.borrow_mut().push((
                f,
                e.input.was_key_pressed("KeyN"),
                e.input.is_key_down("KeyN"),
            ));
        });
        let state: super::DemoStateRef = Rc::default();
        let runner = Rc::new(RefCell::new(super::TestRunner::default()));
        engine.on_update(move |e| super::run_frame(e, &state, &mut runner.borrow_mut(), &steps));
        let mut platform = engine.input.clone(); // an idle platform snapshot
        for _ in 0..8 {
            engine.frame(&mut platform, 640.0, 360.0, 1.0 / 60.0);
        }
        let pressed: Vec<u64> = seen.borrow().iter().filter(|s| s.1).map(|s| s.0).collect();
        let down: Vec<u64> = seen.borrow().iter().filter(|s| s.2).map(|s| s.0).collect();
        assert_eq!(pressed, [3], "pressed once, the frame after the step");
        assert_eq!(down, [3, 4, 5], "held until the release step's next frame");
    }

    #[test]
    fn default_aliases_all_resolve() {
        for alias in DEFAULT_ALIASES {
            assert!(
                *alias == "default" || !SCENARIOS.iter().any(|(k, _)| k == alias),
                "alias {alias:?} collides with a real scenario name"
            );
        }
        assert!(SCENARIOS.iter().any(|(k, _)| *k == "default"), "no `default` scenario");
    }
}
