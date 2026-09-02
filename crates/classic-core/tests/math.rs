use classic_core::math;

/// The pre-world-metre tile → squashed-cartesian transform, kept here as the
/// zero-drift reference now that it has been deleted from `math.rs`:
/// `diag(1, 0.5, 1) · Rz(-45°)`.
fn old_iso_to_cartesian_4() -> glam::Mat4 {
    glam::Mat4::from_scale(glam::Vec3::new(1.0, 0.5, 1.0))
        * glam::Mat4::from_rotation_z(-std::f32::consts::FRAC_PI_4)
}

/// The pre-world-metre tile → light transform: `Rz(-45°)` (no isometric squash).
fn old_iso_to_light_4() -> glam::Mat4 {
    glam::Mat4::from_rotation_z(-std::f32::consts::FRAC_PI_4)
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
fn iso_world_matrix_reproduces_tile_screen() {
    // The zero-drift bridge: `iso_world_matrix` applied to a world-metre vertex
    // must equal the current `S(scale) · diag(1, 0.5, 1) · Rz(-45°)` applied to
    // the tile vertex `(tx, ty, h·PPM_TARGET)`, bit-for-bit (before the shear).
    let scale = glam::Vec3::new(45.0, 45.0, 1.0);
    let world = math::iso_world_matrix(scale);
    let old_iso = glam::Mat4::from_scale(scale) * old_iso_to_cartesian_4();

    for &(tx, ty, h) in
        &[(0.0, 0.0, 0.0), (1.0, 0.0, 0.0), (0.0, 1.0, 0.0), (3.0, 2.0, 5.0), (200.0, 200.0, 10.0)]
    {
        let new_screen = world.transform_point3(math::iso_world_pos(tx, ty, h));
        let old_screen = old_iso.transform_point3(glam::Vec3::new(
            tx,
            ty,
            h * classic_core::tilemap::PPM_TARGET,
        ));
        for (axis, (a, b)) in ["x", "y", "z"].iter().zip([
            (new_screen.x, old_screen.x),
            (new_screen.y, old_screen.y),
            (new_screen.z, old_screen.z),
        ]) {
            assert!((a - b).abs() < 1e-3, "{axis} drift at ({tx},{ty},{h}): new={a} old={b}");
        }
    }
}

#[test]
fn iso_world_light_matrix_reproduces_tile_light() {
    // Same zero-drift guarantee for light space: `iso_world_light_matrix`
    // applied to world metres equals the current `S(scale) · Rz(-45°)` applied
    // to the tile vertex `(tx, ty, h·PPM_TARGET)`.
    let scale = glam::Vec3::new(45.0, 45.0, 1.0);
    let world_light = math::iso_world_light_matrix(scale);
    let old_light = glam::Mat4::from_scale(scale) * old_iso_to_light_4();

    for &(tx, ty, h) in
        &[(0.0, 0.0, 0.0), (1.0, 0.0, 0.0), (0.0, 1.0, 0.0), (3.0, 2.0, 5.0), (200.0, 200.0, 10.0)]
    {
        let new_light = world_light.transform_point3(math::iso_world_pos(tx, ty, h));
        let old_light_pos = old_light.transform_point3(glam::Vec3::new(
            tx,
            ty,
            h * classic_core::tilemap::PPM_TARGET,
        ));
        for (axis, (a, b)) in ["x", "y", "z"].iter().zip([
            (new_light.x, old_light_pos.x),
            (new_light.y, old_light_pos.y),
            (new_light.z, old_light_pos.z),
        ]) {
            assert!((a - b).abs() < 1e-3, "{axis} drift at ({tx},{ty},{h}): new={a} old={b}");
        }
    }
}

#[test]
fn iso_world_normal_matrix_reproduces_tile_normal() {
    // The world normal is `normalize(D⁻¹ · tile_normal)`; the world normal
    // matrix must map it to the same light-space direction as the current
    // `inverse_transpose(S(scale)·Rz(-45°))` maps the tile normal.  `D` does
    // not commute with the rotation, so the plain
    // `inverse_transpose(iso_world_light_matrix)` would be subtly wrong here.
    use classic_core::tilemap::{PPM_TARGET, TILE_M};

    let scale = glam::Vec3::new(45.0, 45.0, 1.0);
    let old_nm = glam::Mat3::from_mat4(glam::Mat4::from_scale(scale) * old_iso_to_light_4())
        .inverse()
        .transpose();
    let new_nm = math::iso_world_normal_matrix(scale);
    let d_inv = glam::Vec3::new(1.0 / TILE_M, -1.0 / TILE_M, PPM_TARGET);

    for &tile_normal in &[
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.5, 0.5, 1.0],
        [0.0, 0.5, 1.0],
        [0.25, -0.5, 1.0],
    ] {
        let tn = glam::Vec3::from_array(tile_normal);
        let world_normal = (tn * d_inv).normalize();
        let old_light = (old_nm * tn).normalize();
        let new_light = (new_nm * world_normal).normalize();
        assert!(
            (old_light - new_light).length() < 1e-3,
            "tile={tile_normal:?} old={old_light:?} new={new_light:?}"
        );
    }
}
