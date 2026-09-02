//! # Skill: `classic-iso`
//!
//! **Read `.agents/skills/classic-iso/SKILL.md` before working on this module.**
//!
//! World-metre coordinate helpers.  After the coordinate-system unification the
//! renderer lives in a single Blender-canonical world space; the whole
//! isometric projection is folded into [`iso_world_matrix`] (screen),
//! [`iso_world_light_matrix`] (light) and the `y -= ppm·z` height shear.

use glam::{Mat3, Mat4, Vec3};

/// Convert a tile coordinate + world height (metres) to Blender-canonical
/// world space: `(tx·TILE_M, −ty·TILE_M, h)`.
///
/// `h` is a real world-Z coordinate in metres (never a pixel offset), matching
/// `height_data` and animation `rig_location.z`.  The `+tx → +X`, `+ty → −Y`
/// flip is Blender's clockwise top-down convention (`render/iso.py`).
pub fn iso_world_pos(tx: f32, ty: f32, h: f32) -> Vec3 {
    Vec3::new(tx * crate::tilemap::TILE_M, -ty * crate::tilemap::TILE_M, h)
}

/// The fixed isometric **view** matrix (45° yaw + 30° elevation, 2:1 dimetric),
/// mapping Blender world space → camera view space.
///
/// The camera basis matches `classic-assets` `render/iso.py::iso_basis(30°)`:
///
/// ```text
/// right = (√½, −√½, 0)
/// up    = (sin30°·√½, sin30°·√½, cos30°) = (0.3536, 0.3536, 0.8660)
/// back  = right × up = (−0.6124, −0.6124, 0.5)
/// ```
///
/// `view · world` yields the camera-frame coordinate
/// `(dot(right, w), dot(up, w), dot(back, w))`: the first two components are the
/// 2D isometric image (before ortho projection and pan/zoom), the third is view
/// depth.  The entire 45° yaw + 30° elevation + `ty → −Y` flip collapses into
/// this one matrix — there is no separate squash, shear, or light-space
/// rotation.
pub fn iso_camera_matrix() -> Mat4 {
    let half = std::f32::consts::FRAC_1_SQRT_2; // √½
    let sin_el = 0.5; // sin(30°)
    let cos_el = 0.866_025_4; // cos(30°) = √3/2
    let right = Vec3::new(half, -half, 0.0);
    let up = Vec3::new(sin_el * half, sin_el * half, cos_el);
    let back = right.cross(up);
    // Camera-to-world has the basis as columns; its inverse (== transpose for
    // an orthonormal basis) is the world-to-camera view matrix.
    Mat4::from_cols(right.extend(0.0), up.extend(0.0), back.extend(0.0), glam::Vec4::W).transpose()
}

/// World metres → squashed-cartesian **screen pixels** (before the `y -= ppm·z`
/// shear), reproducing the current `iso_matrix` path for world-metre vertices.
///
/// The current renderer builds screen space as `S(scale) · diag(1, 0.5, 1) ·
/// Rz(-45°)` applied to *tile* vertices `(tx, ty, z_px)`.  A world-metre vertex
/// is `v = (tx·TILE_M, −ty·TILE_M, h_m)` with `z_px = h_m·PPM_TARGET`, i.e.
/// `tile = D⁻¹ · v` for `D⁻¹ = diag(1/TILE_M, −1/TILE_M, PPM_TARGET)`.  So the
/// world-metre screen transform is `S(scale) · diag(1, 0.5, 1) · Rz(-45°) ·
/// D⁻¹`.  The caller still applies the `y -= ppm·z` shear (or folds it in)
/// afterwards.
///
/// This is the **zero-visual-drift** bridge used to move the tilemap and sprite
/// pipelines to world metres without changing a single rendered pixel.
pub fn iso_world_matrix(scale: Vec3) -> Mat4 {
    let d_inv = Mat4::from_scale(Vec3::new(
        1.0 / crate::tilemap::TILE_M,
        -1.0 / crate::tilemap::TILE_M,
        crate::tilemap::PPM_TARGET,
    ));
    // The 45° yaw plus the 2:1 dimetric squash: `diag(1, 0.5, 1) · Rz(-45°)`.
    let iso_to_cartesian = Mat4::from_scale(Vec3::new(1.0, 0.5, 1.0))
        * Mat4::from_rotation_z(-std::f32::consts::FRAC_PI_4);
    Mat4::from_scale(scale) * iso_to_cartesian * d_inv
}

/// World metres → **light space** (px, metric +Z up), reproducing the current
/// `light_matrix` path for world-metre vertices (origin handled by the caller).
///
/// Light space is `Rz(-45°) · diag(1,-1,1) · world · ppm`; re-expressing the
/// current `S(scale) · Rz(-45°)` (applied to tile vertices) for world metres
/// gives `S(scale) · Rz(-45°) · D⁻¹` (see [`iso_world_matrix`] for `D⁻¹`).
///
/// Dropping the `diag(1, 0.5, 1)` squash makes light space metric: at the
/// standard 45 px tile / 64 px-per-metre setup, one tile spans 45 px along both
/// axes, so `length()`, `normalize()` and `dot(n, L)` mean the same thing in
/// every direction.  Every lighting quantity — `Light::position`, `light_dir`,
/// `vNormal`, `vLightPos`, the shadow map — lives here.
pub fn iso_world_light_matrix(scale: Vec3) -> Mat4 {
    let d_inv = Mat4::from_scale(Vec3::new(
        1.0 / crate::tilemap::TILE_M,
        -1.0 / crate::tilemap::TILE_M,
        crate::tilemap::PPM_TARGET,
    ));
    let iso_to_light = Mat4::from_rotation_z(-std::f32::consts::FRAC_PI_4);
    Mat4::from_scale(scale) * iso_to_light * d_inv
}

/// Normal matrix for **world-metre** terrain normals: transforms a metric world
/// normal into light space, reproducing the current
/// `inverse_transpose(S(scale)·Rz(−45°))` applied to *tile* normals.
///
/// The world normal is `normalize(D⁻¹ · tile_normal)` with
/// `D⁻¹ = diag(1/TILE_M, −1/TILE_M, PPM_TARGET)`, so the correct matrix is
/// `inverse_transpose(S(scale)·Rz(−45°)) · D` — **not**
/// `inverse_transpose(mat3(iso_world_light_matrix))` (which would be
/// `D · inverse_transpose(...)`; `D` does not commute with the rotation, and
/// that subtly re-axes slope lighting).
pub fn iso_world_normal_matrix(scale: Vec3) -> Mat3 {
    let d = Mat3::from_diagonal(Vec3::new(
        crate::tilemap::TILE_M,
        -crate::tilemap::TILE_M,
        1.0 / crate::tilemap::PPM_TARGET,
    ));
    let iso_to_light = Mat4::from_rotation_z(-std::f32::consts::FRAC_PI_4);
    Mat3::from_mat4(Mat4::from_scale(scale) * iso_to_light).inverse().transpose() * d
}

/// World-metre iso-depth scale for a tilemap of `size_x × size_y`, consumed by
/// the tilemap vertex shader and the sprite depth corners:
/// `depth = 0.5 + (v.x + v.y)/scale[0] + v.z/scale[1]` (window space `[0, 1]`).
///
/// `scale[0]` is the horizontal divisor in world metres.  The sprite depth
/// sheets bake their per-pixel grayscale against a fixed 400-tile horizontal
/// divisor (classic-assets `render/materials.py`), so the divisor is floored at
/// `TILE_M · 400` and grown to the map diagonal (`TILE_M · 2·max(size)`) for
/// maps larger than 200×200, whose `tx − ty` span would otherwise clip the
/// NE/SW corners.
///
/// `scale[1]` is the height divisor in metres, derived from the 30°-elevation
/// camera back axis (`back = (−√(3/8), −√(3/8), +1/2)`): one metre of height
/// contributes `back.z = 0.5` of view depth while one tile of `tx − ty`
/// contributes `√(3/8)·TILE_M`, so the height divisor is
/// `2·√(3/8)·TILE_M·400` (`344.46`).
///
/// Both divisors are re-baked to plain camera view depth in step E; until then
/// they keep the legacy depth-sheet interlock, so depth stays bit-for-bit
/// stable across the world-metre refactor.
pub fn iso_world_depth_scale(size_x: i32, size_y: i32) -> [f32; 2] {
    // Horizontal divisor: floored at the 400-tile divisor the sprite depth
    // sheets bake against, then grown to the map diagonal for large maps.
    let horizontal_tiles = (2.0 * size_x.max(size_y).max(1) as f32).max(400.0);
    [crate::tilemap::TILE_M * horizontal_tiles, 344.46]
}

pub fn deg_to_rad(deg: f32) -> f32 {
    deg * std::f32::consts::PI / 180.0
}

pub fn rad_to_deg(rad: f32) -> f32 {
    rad * 180.0 / std::f32::consts::PI
}
