//! Lunar scene: installs a procedurally generated lunar surface onto the
//! demo's reused entity names.
//!
//! `classic-core::terrain` does all the generation; this module installs the
//! result on the ECS entities and GPU, and owns the boot-time state the camera
//! needs.  The scene reuses the demo entity names (`tilemap`,
//! `tilemapNavigation`, ...) so the whole editor toolchain works on a
//! generated map with no further changes.  Runtime re-rolling is driven by the
//! ROM guest (`guest/lunar-guest`) through the generic `generate_terrain` SDK
//! import, not by a host widget.

use classic_core::cl_info;
use classic_core::components::Tilemap;
use classic_core::instrument::Chan;
use classic_core::terrain::lunar::{generate_lunar, LunarParams, LunarTerrain};
use classic_core::terrain::tileset::{build_lunar_tileset, DEFAULT_COLS, DEFAULT_ROWS};
use classic_core::terrain::{GeneratedTerrain, Terrain, Tileset};
use classic_engine::Engine;

use crate::state::DemoStateRef;

/// Runtime state for the lunar scene, kept so `focus_camera_on_spawn` can
/// centre the view on a landing zone at boot.
#[derive(Clone, Debug)]
pub struct LunarScene {
    pub params: LunarParams,
    pub terrain: LunarTerrain,
    /// Vertical exaggeration applied to the height field, overriding the
    /// tilemap default of `tile_pixel_size[0]`.
    pub height_scale: f32,
}

/// Monotonic milliseconds, or `None` where no clock is available.
///
/// `std::time::Instant::now()` panics on `wasm32-unknown-unknown`, and this
/// module runs during startup on the web target, so timing has to be optional
/// rather than merely inaccurate.
#[cfg(not(target_arch = "wasm32"))]
fn now_ms() -> Option<f64> {
    use std::sync::LazyLock;
    static START: LazyLock<std::time::Instant> = LazyLock::new(std::time::Instant::now);
    Some(START.elapsed().as_secs_f64() * 1000.0)
}

#[cfg(target_arch = "wasm32")]
fn now_ms() -> Option<f64> {
    None
}

/// Human-readable elapsed time since `start`, for log lines.
fn since(start: Option<f64>) -> String {
    match (start, now_ms()) {
        (Some(a), Some(b)) => format!("{:.1} ms", b - a),
        _ => String::from("n/a"),
    }
}

/// Vertical scale for generated terrain.
///
/// The flat demo map uses `tile_pixel_size[0]` (32) with heights of exactly
/// 1.0.  Generated terrain spans roughly 7 height units, so the same scale
/// would give ~225px of relief against 45px tiles — dramatic, but it also
/// stretches the 3-pass mouse-picking parallax solve to ~7 tiles of
/// correction, where it converges poorly.  14 keeps both in a sane range.
pub const LUNAR_HEIGHT_SCALE: f32 = 14.0;

/// Generate a lunar surface and install it on the `tilemap` /
/// `tilemapNavigation` entities, including a procedurally painted tileset.
///
/// Must be called after `load_state`, and before `init_navigation` /
/// `init_nav_mesh_render` (which read the data this installs).
pub fn init_lunar_terrain(engine: &mut Engine, state: &DemoStateRef, params: LunarParams) {
    let t0 = now_ms();
    let terrain = generate_lunar(&params);
    let gen_ms = since(t0);

    cl_info!(
        Chan::Terrain,
        "lunar '{}' {}x{}: {} craters, relief {:.2}, {:.0}% walkable, {:.0}% buildable, \
         {} corridors, {} relax iters ({})",
        params.seed,
        terrain.size_x,
        terrain.size_y,
        terrain.stats.craters,
        terrain.stats.max_height - terrain.stats.min_height,
        terrain.stats.walkable_fraction * 100.0,
        terrain.stats.buildable_fraction * 100.0,
        terrain.stats.corridors_carved,
        terrain.stats.relax_iterations_used,
        gen_ms
    );

    let (rgba, tw, th) =
        build_lunar_tileset(&format!("{}:tileset", params.seed), 32, DEFAULT_COLS, DEFAULT_ROWS);

    let gen = GeneratedTerrain {
        terrain: Terrain {
            size_x: terrain.size_x,
            size_y: terrain.size_y,
            tiles: terrain.tiles.clone(),
            heights: terrain.heights.clone(),
            nav: terrain.nav.clone(),
        },
        tileset: Tileset { rgba, width: tw, height: th },
        nav_slope_threshold: params.nav_max_slope,
    };
    engine.install_generated_terrain(&gen, LUNAR_HEIGHT_SCALE);

    state.borrow_mut().lunar =
        Some(LunarScene { params, terrain, height_scale: LUNAR_HEIGHT_SCALE });
}

/// Centre the camera on the first landing zone.
pub fn focus_camera_on_spawn(engine: &mut Engine, state: &DemoStateRef) {
    let Some(scene) = state.borrow().lunar.clone() else { return };
    let Some(&(sx, sy)) = scene.terrain.spawn_points.first() else { return };
    let Some(tm_entity) = engine.entity_by_role(classic_core::RoleKind::Tilemap) else { return };
    let scale = engine
        .world
        .get::<&Tilemap>(tm_entity)
        .map(|tm| tm.scale)
        .unwrap_or(glam::Vec3::new(45.0, 45.0, 1.0));

    let iso = glam::Mat4::from_scale(scale) * classic_core::math::cartesian_to_iso_4().inverse();
    let origin = iso.transform_point3(glam::Vec3::new(sx as f32, sy as f32, 0.0));
    engine.camera.position.x = origin.x;
    engine.camera.position.y = origin.y;
}

/// Install the generated navigation grid.
///
/// The generator already derived walkability from real terrain slope and
/// guaranteed every spawn is mutually reachable; re-deriving it from the
/// coarse height rule here would undo that.
pub fn hydrate_nav(engine: &mut Engine, state: &DemoStateRef) {
    let nav = state.borrow().lunar.as_ref().map(|s| s.terrain.nav.clone()).unwrap_or_default();
    engine.init_navigation_data(nav);
}

/// Zoom out for the generated terrain, centre on the first spawn, and apply
/// the airless lunar light preset.
pub fn setup_view(engine: &mut Engine, state: &DemoStateRef) {
    // Zoom out: at scale 1.0 a 45px tile fills the view with ~28 tiles, which
    // shows none of the terrain the generator produces.
    engine.camera.scale = glam::Vec3::new(0.32, 0.32, 1.0);
    focus_camera_on_spawn(engine, state);
    // Airless lighting: near-zero ambient and a hard low sun, which is what
    // makes the crater relief legible.
    crate::lighting::apply_light_preset(engine, state, "lunar");
    // The editor grid overlay fights the natural surface.
    engine.show_grid = false;
}
