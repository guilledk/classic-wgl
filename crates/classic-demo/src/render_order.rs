//! GPU pixel assertions for the `render_order` end-to-end scenario.
//!
//! The scenario verifies per-pixel sprite/terrain occlusion (and the per-pixel
//! ghost pass) by sampling framebuffer pixels at screen positions computed from
//! iso tile coordinates.  These assertions are **GPU-only**: they read the
//! framebuffer directly, so they require a real GL context (native or
//! headless-EGL) and are meaningless under a broken depth-test rasterizer.

use glam::Vec3;

use classic_core::components::Tilemap;
use classic_core::math::iso_to_cartesian_4;
use classic_core::tilemap::bilinear_height;
use classic_engine::Engine;

/// Project an iso tile coordinate (at terrain height) to screen pixels
/// (top-left origin), matching the engine's sprite model + camera math.
///
/// Named `iso_to_screen_px` to avoid clashing with the guest SDK's
/// `iso_to_screen` (which returns a cartesian world position, not pixels).
pub fn iso_to_screen_px(engine: &Engine, x: f32, y: f32) -> Option<(f32, f32)> {
    let tm_entity = engine.entity_by_role(classic_core::RoleKind::Tilemap)?;
    let tm = engine.world.get::<&Tilemap>(tm_entity).ok()?;
    let tm_tf = engine.world.get::<&classic_core::components::Transform>(tm_entity).ok()?;

    let iso_to_cart_world = iso_to_cartesian_4() * glam::Mat4::from_scale(tm_tf.scale);
    let mut world = iso_to_cart_world.transform_point3(Vec3::new(x, y, 0.0));
    world += tm_tf.position;
    let h = bilinear_height(&tm.height_data, tm.size_x, tm.size_y, x, y);
    world.y -= h * tm.height_scale;

    let (vw, vh) = engine.viewport_size();
    let size = Vec3::new(vw, vh, 0.0);
    let fix = engine.camera.position * engine.camera.scale - size / Vec3::new(2.0, 2.0, 1.0);
    let camera_space = world * engine.camera.scale - fix;
    Some((camera_space.x, camera_space.y))
}

/// Read the RGBA pixel (normalized `[0, 1]`) at a top-left-origin screen
/// coordinate.  `None` when there is no render target or the coordinate is
/// out of bounds.
pub fn read_pixel_rgba(engine: &Engine, sx: f32, sy: f32) -> Option<[f32; 4]> {
    let gfx = engine.gfx.as_ref()?;
    gfx.read_pixel_rgba(sx as i32, sy as i32)
}

/// Assert a pixel at an entity's iso position, optionally offset in tile units.
///
/// `offset` is a tile-space `(dx, dy)` added to the entity's `(x, y)` before
/// projecting to screen pixels, so callers can sample a sprite corner rather
/// than only its ground anchor.
///
/// When `expected` is `Some(rgba)`, every channel must match within `tol`.
/// When `expected` is `None`, the pixel is checked for opacity (alpha must be
/// at least `tol`, defaulting to a value passed by the caller) — useful for
/// confirming a depth-mapped sprite renders fully where it is in front of
/// terrain rather than ghosting or vanishing.
pub fn assert_pixel_at_entity(
    engine: &Engine,
    name: &str,
    expected: Option<[f32; 4]>,
    tol: f32,
    offset: (f32, f32),
) -> bool {
    let Some(&entity) = engine.names.get(name) else {
        classic_core::cl_info!(
            classic_core::instrument::Chan::Test,
            "  [Pixel] no entity '{name}'"
        );
        return false;
    };
    let Ok(tf) = engine.world.get::<&classic_core::components::Transform>(entity) else {
        classic_core::cl_info!(
            classic_core::instrument::Chan::Test,
            "  [Pixel] '{name}' has no Transform"
        );
        return false;
    };
    let Some((sx, sy)) =
        iso_to_screen_px(engine, tf.position.x + offset.0, tf.position.y + offset.1)
    else {
        classic_core::cl_info!(
            classic_core::instrument::Chan::Test,
            "  [Pixel] no tilemap for '{name}'"
        );
        return false;
    };
    let Some(actual) = read_pixel_rgba(engine, sx, sy) else {
        classic_core::cl_info!(
            classic_core::instrument::Chan::Test,
            "  [Pixel] no framebuffer at ({sx:.0},{sy:.0})"
        );
        return false;
    };
    let ok = match expected {
        Some(exp) => (0..4).all(|i| (actual[i] - exp[i]).abs() <= tol),
        None => actual[3] >= tol,
    };
    if !ok {
        classic_core::cl_info!(
            classic_core::instrument::Chan::Test,
            "  [Pixel] '{name}' @ ({sx:.1},{sy:.1}) offset={offset:?} actual={:?} expected={:?} tol={tol}",
            actual,
            expected
        );
    }
    ok
}
