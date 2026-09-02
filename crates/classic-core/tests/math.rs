use classic_core::math;

#[test]
fn cartesian_to_iso_and_back_round_trip() {
    let cart = math::cartesian_to_iso_4();
    let iso_inv = math::iso_to_cartesian_4();
    let round = iso_inv * cart;
    let p = glam::Vec3::new(32.0, 13.0, 0.0);
    let result = round.transform_point3(p);
    assert!((result.x - p.x).abs() < 0.001);
    assert!((result.y - p.y).abs() < 0.001);
    assert!((result.z - p.z).abs() < 0.001);
}

#[test]
fn matrices_are_inverses() {
    let cti = math::cartesian_to_iso_4();
    let itc = math::iso_to_cartesian_4();
    let product = cti * itc;
    // Product should be approximately identity
    let id = glam::Mat4::IDENTITY;
    for r in 0..4 {
        for c in 0..4 {
            assert!((product.col(c)[r] - id.col(c)[r]).abs() < 0.001);
        }
    }
}

#[test]
fn iso_transform_identity_lies_on_correct_axis() {
    // cartesian (1, 0) → S(1,2,1) → (1,0,0) → R(π/4) → (cos45, sin45, 0) ≈ (0.707, 0.707, 0)
    let p = glam::Vec3::new(1.0, 0.0, 0.0);
    let iso = math::cartesian_to_iso_4().transform_point3(p);
    let s = std::f32::consts::FRAC_1_SQRT_2; // cos(45°) = sin(45°) = 1/√2
    assert!((iso.x - s).abs() < 0.001, "iso.x={} expected={}", iso.x, s);
    assert!((iso.y - s).abs() < 0.001, "iso.y={} expected={}", iso.y, s);
    assert!((iso.z - 0.0).abs() < 0.001, "iso.z={}", iso.z);
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
        glam::Mat4::from_scale(glam::Vec3::new(TILE_PX, TILE_PX, 1.0)) * math::iso_to_cartesian_4();

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
