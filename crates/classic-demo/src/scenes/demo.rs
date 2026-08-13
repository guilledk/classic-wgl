//! Demo scene: the hand-authored map (`state.json`, with inlined tile/nav
//! data) installed over the engine's generic `load_rom` hydration.
//!
//! The scene reuses the `tilemap` / `tilemapNavigation` / `navAgent` entity
//! names so the whole editor toolchain works with no further changes.

use classic_engine::Engine;

/// Build the flat tilemap mesh + tile-data texture for the Tilemap-role entity.
pub fn hydrate_terrain(engine: &mut Engine) {
    engine.init_tilemap();
}

/// Install the navigation overlay from the inlined nav data.
pub fn hydrate_nav(engine: &mut Engine) {
    engine.init_navigation();
}

/// Position the camera for the hand-authored map and enable the editor grid.
pub fn setup_view(engine: &mut Engine) {
    let mut iso = classic_core::math::cartesian_to_iso_4().inverse();
    iso = glam::Mat4::from_scale(glam::Vec3::new(45.0, 45.0, 1.0)) * iso;
    let origin = iso.transform_point3(glam::Vec3::new(32.0, 13.0, 0.0));
    engine.camera.position.x = origin.x;
    engine.camera.position.y = origin.y;
    engine.show_grid = true;
}
