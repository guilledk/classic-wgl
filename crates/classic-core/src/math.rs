//! # Skill: `classic-iso`
//!
//! **Read `.claude/skills/classic-iso/SKILL.md` before working on this module.**
//!
use glam::{Mat3, Mat4, Vec3};

/// Cartesian-to-isometric transformation matrix (3x3).
/// Identity rotated by π/4 around Z, then scaled by [1, 2].
pub fn cartesian_to_iso_3() -> Mat3 {
    Mat3::from_rotation_z(std::f32::consts::FRAC_PI_4) * Mat3::from_scale(glam::Vec2::new(1.0, 2.0))
}

/// Isometric-to-cartesian transformation matrix (3x3).
/// Inverse of `cartesian_to_iso_3`.
pub fn iso_to_cartesian_3() -> Mat3 {
    cartesian_to_iso_3().inverse()
}

/// Cartesian-to-isometric transformation matrix (4x4).
pub fn cartesian_to_iso_4() -> Mat4 {
    Mat4::from_rotation_z(std::f32::consts::FRAC_PI_4) * Mat4::from_scale(Vec3::new(1.0, 2.0, 1.0))
}

/// Isometric-to-cartesian transformation matrix (4x4).
pub fn iso_to_cartesian_4() -> Mat4 {
    cartesian_to_iso_4().inverse()
}

/// Isometric-to-**light-space** transformation matrix (4x4): the same `-45°`
/// yaw as [`iso_to_cartesian_4`] but **without** the `diag(1, 0.5, 1)` 2:1
/// vertical foreshortening.
///
/// # Why this exists — the third space
///
/// `iso_to_cartesian_4() = diag(1, 0.5, 1) · Rz(-45°)`.  The `0.5` is the
/// isometric *drawing* squash, and it makes that space **non-metric**: at the
/// standard 45 px tile / 64 px-per-metre setup, one tile spans 45 px along x
/// (= 0.703 m, 64 px/m ✓) but only 22.5 px along y (= 0.703 m at 32 px/m ✗).
///
/// Every lighting operation — `length()`, `normalize()`, `dot(n, L)` — is
/// Euclidean and is therefore **invalid** in a space with a 2× axis
/// compression.  Consequences that shipped: point-light pools rendered as
/// screen-space circles instead of ground-plane ellipses, and sprite normal
/// maps (baked in metric Blender world space, consumed untransformed) were
/// wrong by up to 153°.
///
/// Dropping the squash leaves a pure rotation+reflection, so light space is
/// isotropic and metric: `light = ppm · Rz(-45°) · diag(1,-1,1) · blender`
/// (see [`blender_to_light_3`]).  Every lighting quantity — `Light::position`,
/// `light_dir`, `vNormal`, `vLightPos`, the shadow map — lives here.
///
/// Do **not** use this for rasterisation; the renderer still draws through
/// [`iso_to_cartesian_4`] plus the `y -= z` shear.
pub fn iso_to_light_4() -> Mat4 {
    Mat4::from_rotation_z(-std::f32::consts::FRAC_PI_4)
}

/// Blender world space → light space, as a rotation (3x3).
///
/// The asset pipeline bakes sprite normal maps in **Blender world space**
/// (`render/materials.py`, `ShaderNodeNewGeometry.Normal`), with the tile axes
/// mapped `+tx → +X` and `+ty → −Y` (`render/iso.py`).  Light space is that
/// same metric frame yawed by `-45°`, so the conversion is
/// `Rz(-45°) · diag(1, -1, 1)` — orthogonal, hence its own inverse-transpose,
/// so it applies unchanged to normals.
///
/// Scale is deliberately excluded: this maps *directions*, and the shader uses
/// it only on normal-map values.
pub fn blender_to_light_3() -> Mat3 {
    Mat3::from_rotation_z(-std::f32::consts::FRAC_PI_4)
        * Mat3::from_diagonal(Vec3::new(1.0, -1.0, 1.0))
}

/// Cartesian (pre-shear screen) y → light-space y.
///
/// The two spaces differ only by the isometric `diag(1, 0.5, 1)` squash, and
/// only on y — so `light.x == cart.x`, `light.z == cart.z`, and
/// `light.y == cart.y * 2`.  Used where a value is only available already
/// projected into cartesian space (e.g. a sprite's animated `frame_offset`).
/// `iso_and_light_differ_only_by_the_y_squash` pins this against the matrices.
pub fn cartesian_y_to_light(y: f32) -> f32 {
    y * 2.0
}

pub fn deg_to_rad(deg: f32) -> f32 {
    deg * std::f32::consts::PI / 180.0
}

pub fn rad_to_deg(rad: f32) -> f32 {
    rad * 180.0 / std::f32::consts::PI
}
