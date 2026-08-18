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
