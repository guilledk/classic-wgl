//! classic-demo: Application-specific prefabs and UI bootstrap.
//!
//! Provides `init_engine()` — the single source of truth for the demo's
//! startup sequence (used by both native and web targets).

use std::rc::Rc;

use classic_core::cl_info;
use classic_engine::Engine;

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
    e.init_cursor();
    e.init_camera_wasd();
    e.init_animator_system();
    e.init_agent_system();
    e.init_footprint_colliders();
    e.init_navigation(nav_data);
    e.init_debug_toggles();
    e.init_ui();
    e.init_tool_buttons();
    e.init_height_widget();
    e.init_light_widget();
    e.init_tile_palette();
    e.init_nav_palette();
    e.init_nav_mesh_render();
    e.init_editor_mode_control();
    e.measure_all_ui_labels();
    e.init_lighting();
    e.init_text_showcase();
    e.init_iso_coord_overlay();

    let mut iso = classic_core::math::cartesian_to_iso_4().inverse();
    iso = glam::Mat4::from_scale(glam::Vec3::new(45.0, 45.0, 1.0)) * iso;
    let origin = iso.transform_point3(glam::Vec3::new(32.0, 13.0, 0.0));
    e.camera.position.x = origin.x;
    e.camera.position.y = origin.y;
    e.show_grid = true;

    cl_info!(classic_core::instrument::Chan::Frame, "classic-demo initialized");
    e
}
