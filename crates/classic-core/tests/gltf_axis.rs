//! The +90°-about-X glTF axis fix + metre-scale reconcile into the engine's
//! single world-metre space (`iso_world_pos` / `iso_camera_matrix`).

use classic_core::math;
use classic_core::model::gltf_to_world;
use classic_core::tilemap::{PPM_TARGET, TILE_M};
use glam::Vec3;

#[test]
fn gltf_axes_map_to_world_axes() {
    let m = gltf_to_world();
    // glTF +Y (up) → world +Z (up).
    assert!((m.transform_point3(Vec3::Y) - Vec3::Z).length() < 1e-5);
    // glTF +Z (forward) → world −Y (the engine's +ty direction).
    assert!((m.transform_point3(Vec3::Z) - Vec3::NEG_Y).length() < 1e-5);
    // glTF +X → world +X (unchanged).
    assert!((m.transform_point3(Vec3::X) - Vec3::X).length() < 1e-5);
}

#[test]
fn gltf_to_world_inverts_the_blender_exporter() {
    // Blender's glTF exporter writes a Blender (+Z up) point `(x, y, z)` as
    // glTF `(x, z, -y)`; `gltf_to_world` must hand back the Blender world
    // point verbatim, since the engine world *is* Blender world
    // (`+tx → +X`, `+ty → −Y`).
    for b in [Vec3::new(-4.0, 4.0, 64.0), Vec3::new(1.0, -2.0, 3.0)] {
        let gltf = Vec3::new(b.x, b.z, -b.y);
        assert!((gltf_to_world().transform_point3(gltf) - b).length() < 1e-4, "{b:?}");
    }
    // Blender world (−4, 4) — the landing clip's start drift — is tile offset
    // (−4/TILE_M, −4/TILE_M) ≈ (−5.69, −5.69) from the pad, and projects
    // straight to the **left** of the pad on screen (4·√2 ≈ 5.66 m, 362 px at
    // zoom 1) with no vertical screen offset: the drift runs along the camera
    // `right` axis, so the settle reads as a pure right-ward slide.
    let (tx, ty) = (-4.0 / TILE_M, -4.0 / TILE_M);
    let start = math::iso_world_pos(tx, ty, 0.0);
    assert!((start - Vec3::new(-4.0, 4.0, 0.0)).length() < 1e-4);
    let px = math::iso_camera_px(start);
    assert!((px.x + 4.0 * std::f32::consts::SQRT_2 * PPM_TARGET).abs() < 0.5, "{px:?}");
    assert!(px.y.abs() < 1e-3, "{px:?}");
}

#[test]
fn rocket_metre_scale_reconciles_to_world_units() {
    // The US Rocket is ~47 m tall; its nose in glTF local space sits near
    // (0, 47, 0) (+Y up), which after +90°-X lands at world (0, 0, 47).
    let nose_gltf = Vec3::new(0.0, 47.0, 0.0);
    let nose_world = gltf_to_world().transform_point3(nose_gltf);
    assert!((nose_world - Vec3::new(0.0, 0.0, 47.0)).length() < 1e-3);

    // Metre scale: 47 m = 47 / TILE_M ≈ 66.8 tiles; the height projects to a
    // screen rise of 47·cos(30°)·PPM ≈ 2606 px through the single ortho camera.
    let tiles_tall = 47.0 / TILE_M;
    assert!((tiles_tall - 66.84).abs() < 0.1, "tiles tall {tiles_tall}");
    let px = math::iso_camera_px(nose_world);
    let expected_rise = -(47.0 * math::iso_basis().1.z * PPM_TARGET);
    assert!((px.y - expected_rise).abs() < 1.0, "px {px:?} vs {expected_rise}");

    // The ~47 m model still sits comfortably inside the fixed depth range.
    let depth = (math::DEPTH_NEAR - math::iso_view_depth(nose_world))
        / (math::DEPTH_NEAR - math::DEPTH_FAR);
    assert!((0.0..=1.0).contains(&depth), "depth {depth} out of [0,1]");
}
