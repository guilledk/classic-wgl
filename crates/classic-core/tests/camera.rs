use glam::Vec3;

use classic_core::Camera;

#[test]
fn resize_updates_size() {
    let mut cam = Camera::new(Vec3::ZERO, Vec3::ONE);
    cam.resize(Vec3::new(800.0, 600.0, 1.0));
    assert_eq!(cam.size, Vec3::new(800.0, 600.0, 1.0));
}

#[test]
fn fix_centers_view() {
    let mut cam = Camera::new(Vec3::new(100.0, 50.0, 0.0), Vec3::new(2.0, 2.0, 1.0));
    cam.resize(Vec3::new(800.0, 600.0, 1.0));

    // fix = (position * scale - size) / [2, 2, 1]
    let expected =
        Vec3::new((100.0 * 2.0 - 800.0) / 2.0, (50.0 * 2.0 - 600.0) / 2.0, (0.0 * 1.0 - 1.0) / 1.0);
    assert_eq!(cam.fix(), expected);
}

#[test]
fn fix_with_unit_scale_is_neg_half_size() {
    let mut cam = Camera::new(Vec3::ZERO, Vec3::ONE);
    cam.resize(Vec3::new(200.0, 100.0, 1.0));
    assert_eq!(cam.fix(), Vec3::new(-100.0, -50.0, -1.0));
}

#[test]
fn matrix_translates_and_scales() {
    let mut cam = Camera::new(Vec3::ZERO, Vec3::new(2.0, 3.0, 1.0));
    cam.resize(Vec3::ZERO);

    // At origin with zero-size viewport, fix = (0,0,0)
    let m = cam.matrix();
    let expected =
        glam::Mat4::from_scale(Vec3::new(2.0, 3.0, 1.0)) * glam::Mat4::from_translation(Vec3::ZERO);
    assert_eq!(m, expected);
}

#[test]
fn matrix_reflects_position() {
    let mut cam = Camera::new(Vec3::new(10.0, 20.0, 0.0), Vec3::ONE);
    cam.resize(Vec3::ZERO);

    let fix = cam.fix(); // (10,20,0) / 2 = (5,10,0) for TS formula
    let neg_fix = -fix;

    let m = cam.matrix();
    let expected = glam::Mat4::from_translation(neg_fix) * glam::Mat4::from_scale(Vec3::ONE);
    assert_eq!(m, expected);
}

#[test]
fn matrix_with_nonzero_position_and_size() {
    let mut cam = Camera::new(Vec3::new(100.0, 50.0, 0.0), Vec3::new(2.0, 2.0, 1.0));
    cam.resize(Vec3::new(800.0, 600.0, 1.0));

    let fix = cam.fix();
    // TS: fix = ((200,100,0) - (800,600,1)) / (2,2,1) = (-600,-500,-1) / 2 = (-300,-250,-1)
    assert_eq!(fix, Vec3::new(-300.0, -250.0, -1.0));

    let neg_fix = -fix;
    let expected =
        glam::Mat4::from_translation(neg_fix) * glam::Mat4::from_scale(Vec3::new(2.0, 2.0, 1.0));
    assert_eq!(cam.matrix(), expected);
}
