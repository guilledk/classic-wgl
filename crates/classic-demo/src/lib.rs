//! classic-demo: Application-specific prefabs, editor tools and UI bootstrap.
//!
//! Provides [`init_engine`] — the single source of truth for the demo's
//! startup sequence (used by both native and web targets).
//!
//! The engine (`classic-engine`) is generic: it owns the world, camera, input,
//! gfx and tilemap/nav plumbing.  Everything here — the editor HUD, tool
//! widgets, light presets, debug overlays and the CLASSIC_TEST scenario — is
//! demo content, built as free functions over `&mut Engine` + shared
//! `DemoState` and registered through the engine's hook surface.

pub mod editor;
pub mod hud;
pub mod lighting;
pub mod prefabs;
pub mod state;
pub mod testing;

use std::cell::RefCell;
use std::rc::Rc;

use classic_core::cl_info;
use classic_engine::Engine;

use crate::state::{DemoState, DemoStateRef};

/// Full demo engine bootstrap.  Asset data is passed in so each target
/// can embed it at compile time via `include_bytes!` / `include_str!`.
#[allow(clippy::too_many_arguments)]
pub fn init_engine(
    gl: Rc<glow::Context>,
    manifest_json: &str,
    state_json: &str,
    tileset_png: &[u8],
    map_data: &str,
    nav_data: &str,
    sdf_atlas_png: &[u8],
    sdf_metrics_json: &str,
    semaphore01_png: &[u8],
    semaphore02_png: &[u8],
    house_png: &[u8],
    cursor_png: &[u8],
    humanoid_png: &[u8],
    cool_snek_png: &[u8],
    tree_png: &[u8],
    editor_icons_png: &[u8],
    nav_tileset_png: &[u8],
) -> Engine {
    let mut e = Engine::new();
    e.init_gfx(gl, manifest_json);
    e.load_state(state_json).expect("load state.json");
    e.init_tilemap("tilemap", tileset_png, map_data);
    e.load_sdf_font("dejavusans-sdf", sdf_metrics_json, sdf_atlas_png);
    e.load_texture_png("semaphore01", semaphore01_png);
    e.load_texture_png("semaphore02", semaphore02_png);
    e.load_texture_png("house", house_png);
    e.load_texture_png("cursor", cursor_png);
    e.load_texture_png("humanoid", humanoid_png);
    e.load_texture_png("coolSnake", cool_snek_png);
    e.load_texture_png("tree", tree_png);
    e.load_texture_png("editorIcons", editor_icons_png);
    e.load_texture_png("navTileset", nav_tileset_png);

    let state: DemoStateRef = Rc::new(RefCell::new(DemoState::default()));

    prefabs::init_cursor(&mut e);
    prefabs::init_camera_wasd(&mut e);
    prefabs::init_animator_system(&mut e);
    prefabs::init_agent_system(&mut e);
    prefabs::init_footprint_colliders(&mut e);
    e.init_navigation(nav_data);
    prefabs::init_debug_toggles(&mut e, &state);
    editor::init_ui(&mut e);
    editor::init_tool_buttons(&mut e, &state);
    editor::init_height_widget(&mut e, &state);
    lighting::init_light_widget(&mut e, &state);
    editor::init_tile_palette(&mut e, &state);
    editor::init_nav_palette(&mut e, &state);
    e.init_nav_mesh_render();
    editor::init_editor_mode_control(&mut e, &state);
    e.measure_all_ui_labels();
    lighting::init_lighting(&mut e, &state);
    hud::init_text_showcase(&mut e, &state);
    hud::init_iso_coord_overlay(&mut e, &state);

    // Engine hooks — the demo owns this behaviour, registered as callbacks.
    {
        let s = state.clone();
        e.on_pre_update(move |engine| hud::route_text_scroll(engine, &s));
    }
    {
        let s = state.clone();
        e.on_selection_end(move |engine| editor::apply_editor_selection(engine, &s));
    }
    {
        let s = state.clone();
        e.add_overlay(move |engine| hud::draw_debug_overlay(engine, &s));
    }
    testing::install(&mut e, &state);

    let mut iso = classic_core::math::cartesian_to_iso_4().inverse();
    iso = glam::Mat4::from_scale(glam::Vec3::new(45.0, 45.0, 1.0)) * iso;
    let origin = iso.transform_point3(glam::Vec3::new(32.0, 13.0, 0.0));
    e.camera.position.x = origin.x;
    e.camera.position.y = origin.y;
    e.show_grid = true;

    cl_info!(classic_core::instrument::Chan::Frame, "classic-demo initialized");
    e
}
