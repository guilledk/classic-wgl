use classic_core::math;

/// The pre-world-metre tile → squashed-cartesian transform, kept here as the
/// zero-drift reference now that it has been deleted from `math.rs`:
/// `diag(1, 0.5, 1) · Rz(-45°)`.
fn old_iso_to_cartesian_4() -> glam::Mat4 {
    glam::Mat4::from_scale(glam::Vec3::new(1.0, 0.5, 1.0))
        * glam::Mat4::from_rotation_z(-std::f32::consts::FRAC_PI_4)
}

#[test]
fn deg_to_rad_and_back() {
    let deg: f32 = 90.0;
    let rad = math::deg_to_rad(deg);
    let back = math::rad_to_deg(rad);
    assert!((back - deg).abs() < 0.001);
}

#[test]
fn rad_to_deg_and_back() {
    let rad: f32 = std::f32::consts::PI;
    let deg = math::rad_to_deg(rad);
    let back = math::deg_to_rad(deg);
    assert!((back - rad).abs() < 0.001);
}

#[test]
fn iso_camera_matrix_is_orthonormal() {
    let view = math::iso_camera_matrix();
    // An orthonormal view's transpose is its inverse.
    let product = view * view.transpose();
    let id = glam::Mat4::IDENTITY;
    for r in 0..4 {
        for c in 0..4 {
            assert!(
                (product.col(c)[r] - id.col(c)[r]).abs() < 1e-5,
                "view is not orthonormal at ({r},{c}): {:?}",
                product
            );
        }
    }
}

#[test]
fn iso_camera_matrix_preserves_horizontal_dimetric_shape() {
    // The new world-space iso camera must reproduce the current squashed
    // cartesian + `y -= z` shear pipeline for the *horizontal* plane (h = 0):
    // the 2:1 dimetric tile footprint must land on the same screen pixels.
    //
    // The height term is deliberately NOT asserted here.  The current shear
    // lifts height at 1:1 (`up.z = 1.0`), whereas the true 30° basis uses
    // `up.z = cos(30°) ≈ 0.866`; folding in that correction is a separate,
    // clearly-scoped commit ("32 vs 45").  This test guards the *shape* (the
    // 2:1 dimetric angle/scale), not bit-identity — see the coordinate-system
    // plan, ratified decision #1.
    use classic_core::tilemap::{PPM_TARGET, TILE_PX};

    let view = math::iso_camera_matrix();
    let iso_matrix =
        glam::Mat4::from_scale(glam::Vec3::new(TILE_PX, TILE_PX, 1.0)) * old_iso_to_cartesian_4();

    for &(tx, ty) in
        &[(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0), (3.0, 2.0), (10.0, -4.0), (200.0, 200.0)]
    {
        // Current screen position (pixels, top-left origin, h = 0).
        let cart = iso_matrix.transform_point3(glam::Vec3::new(tx, ty, 0.0));
        // New world (metres) → view (right/up) → pixels, top-left origin.
        let v = view.transform_point3(math::iso_world_pos(tx, ty, 0.0));
        assert!(
            (v.x * PPM_TARGET - cart.x).abs() < 0.5,
            "x drift at ({tx},{ty}): new={} current={}",
            v.x * PPM_TARGET,
            cart.x
        );
        assert!(
            (-v.y * PPM_TARGET - cart.y).abs() < 0.5,
            "y drift at ({tx},{ty}): new={} current={}",
            -v.y * PPM_TARGET,
            cart.y
        );
    }
}

#[test]
fn iso_camera_px_round_trips_ground_points() {
    // The camera-view pixel projection and its ground-plane inverse must
    // round-trip world points on the `z = 0` plane.
    for &(tx, ty) in &[(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (3.0, 2.0), (200.0, 200.0)] {
        let world = math::iso_world_pos(tx, ty, 0.0);
        let px = math::iso_camera_px(world);
        let back = math::iso_camera_px_inverse(glam::Vec2::new(px.x, px.y));
        assert!(
            (back - world).length() < 1e-3,
            "ground round-trip at ({tx},{ty}): {world:?} vs {back:?}"
        );
    }
}

#[test]
fn iso_view_depth_normalizes_over_near_far() {
    // `iso_view_depth` is `dot(back, world)`; the normalised window depth is
    // `(DEPTH_NEAR - dot)/(DEPTH_NEAR - DEPTH_FAR)` (0 = nearest, 1 = farthest).
    let back = math::iso_basis().2;
    for &(tx, ty) in &[(0.0, 0.0), (100.0, 0.0), (0.0, 100.0), (200.0, 200.0)] {
        let world = math::iso_world_pos(tx, ty, 0.0);
        let dot = math::iso_view_depth(world);
        assert!((dot - back.dot(world)).abs() < 1e-6);
        let depth = (math::DEPTH_NEAR - dot) / (math::DEPTH_NEAR - math::DEPTH_FAR);
        assert!((0.0..=1.0).contains(&depth), "depth {depth} out of [0,1]");
    }
}
