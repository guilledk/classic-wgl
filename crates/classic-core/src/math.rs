//! # Skill: `classic-iso`
//!
//! **Read `.agents/skills/classic-iso/SKILL.md` before working on this module.**
//!
//! World-metre coordinate helpers.  After the coordinate-system unification the
//! renderer lives in a single Blender-canonical world space; the whole
//! isometric projection is folded into [`iso_camera_matrix`] (the single
//! orthographic camera), and depth is the camera view depth
//! [`iso_view_depth`] normalised over [`DEPTH_NEAR`]/[`DEPTH_FAR`].

use glam::{Mat4, Vec2, Vec3};

/// Convert a tile coordinate + world height (metres) to Blender-canonical
/// world space: `(tx·TILE_M, −ty·TILE_M, h)`.
///
/// `h` is a real world-Z coordinate in metres (never a pixel offset), matching
/// `height_data` and animation `rig_location.z`.  The `+tx → +X`, `+ty → −Y`
/// flip is Blender's clockwise top-down convention (`render/iso.py`).
pub fn iso_world_pos(tx: f32, ty: f32, h: f32) -> Vec3 {
    Vec3::new(tx * crate::tilemap::TILE_M, -ty * crate::tilemap::TILE_M, h)
}

/// The orthographic camera basis `(right, up, back)` (all unit length),
/// matching `classic-assets` `render/iso.py::iso_basis(30°)`:
///
/// ```text
/// right = (√½, −√½, 0)
/// up    = (sin30°·√½, sin30°·√½, cos30°) = (0.3536, 0.3536, 0.8660)
/// back  = right × up = (−√(3/8), −√(3/8), 0.5)
/// ```
pub fn iso_basis() -> (Vec3, Vec3, Vec3) {
    let half = std::f32::consts::FRAC_1_SQRT_2; // √½
    let sin_el = 0.5; // sin(30°)
    let cos_el = 0.866_025_4; // cos(30°) = √3/2
    let right = Vec3::new(half, -half, 0.0);
    let up = Vec3::new(sin_el * half, sin_el * half, cos_el);
    let back = right.cross(up);
    (right, up, back)
}

/// The fixed isometric **view** matrix (45° yaw + 30° elevation, 2:1 dimetric),
/// mapping Blender world space → camera view space.
///
/// `view · world` yields the camera-frame coordinate
/// `(dot(right, w), dot(up, w), dot(back, w))`: the first two components are the
/// 2D isometric image (before ortho projection and pan/zoom), the third is view
/// depth.  The entire 45° yaw + 30° elevation + `ty → −Y` flip collapses into
/// this one matrix — there is no separate squash, shear, or light-space
/// rotation.
pub fn iso_camera_matrix() -> Mat4 {
    let (right, up, back) = iso_basis();
    // Camera-to-world has the basis as columns; its inverse (== transpose for
    // an orthonormal basis) is the world-to-camera view matrix.
    Mat4::from_cols(right.extend(0.0), up.extend(0.0), back.extend(0.0), glam::Vec4::W).transpose()
}

/// Camera view depth of a world point, in metres: `dot(back, world)`.
///
/// `back` points toward the camera, so view depth *decreases* with distance:
/// the nearest map corner (SW) has the most positive value, the farthest (NE)
/// the most negative.  Normalise to window `[0, 1]` with
/// [`DEPTH_NEAR`]/[`DEPTH_FAR`].
pub fn iso_view_depth(world: Vec3) -> f32 {
    iso_basis().2.dot(world)
}

/// Project a world point to **camera-view screen pixels** (before pan/zoom):
/// `(right·w, −up·w)` scaled by `PPM_TARGET`.  The screen y is negated because
/// the camera `up` axis projects to the negative old-cartesian y.
pub fn iso_camera_px(world: Vec3) -> Vec3 {
    let view = iso_camera_matrix().transform_point3(world);
    Vec3::new(view.x * crate::tilemap::PPM_TARGET, -view.y * crate::tilemap::PPM_TARGET, 0.0)
}

/// Inverse of [`iso_camera_px`]: map camera-view screen pixels to the world
/// point on the ground plane (`z = 0`) that projects to them.
pub fn iso_camera_px_inverse(px: Vec2) -> Vec3 {
    let (right, up, back) = iso_basis();
    let view_x = px.x / crate::tilemap::PPM_TARGET;
    let view_y = -px.y / crate::tilemap::PPM_TARGET;
    // Solve `world.z = up.z·view_y + back.z·view_z == 0`.
    let view_z = -up.z * view_y / back.z;
    right * view_x + up * view_y + back * view_z
}

/// The orthographic camera ray through a camera-view pixel `px` (before
/// pan/zoom), for terrain/collider picking.
///
/// The ray starts at the near depth plane ([`DEPTH_NEAR`]) and points into the
/// scene (`−back`), so marching from its origin returns the *first* surface the
/// camera sees — a slope in front correctly occludes terrain behind it.
pub fn iso_camera_ray(px: Vec2) -> Ray {
    let (right, up, back) = iso_basis();
    let view_x = px.x / crate::tilemap::PPM_TARGET;
    let view_y = -px.y / crate::tilemap::PPM_TARGET;
    let near = right * view_x + up * view_y + back * DEPTH_NEAR;
    Ray::new(near, -back)
}

/// The closest view depth (metres): `dot(back, world)` at the nearest point.
///
/// Fixed so every scene shares one depth range — a 400×400 map spans
/// `±√(3/8)·400·TILE_M ≈ ±172`, and the tallest sprite (the ~47 m rocket)
/// adds `0.5·47 ≈ 24` toward the near side.  `220` covers both with margin;
/// smaller maps use a sub-range.  Mirrored by classic-assets
/// `render/presets.py::DEPTH_NEAR`.
pub const DEPTH_NEAR: f32 = 220.0;

/// The farthest view depth (metres): `dot(back, world)` at the farthest point.
///
/// See [`DEPTH_NEAR`].  Mirrored by classic-assets
/// `render/presets.py::DEPTH_FAR`.
pub const DEPTH_FAR: f32 = -220.0;

pub fn deg_to_rad(deg: f32) -> f32 {
    deg * std::f32::consts::PI / 180.0
}

pub fn rad_to_deg(rad: f32) -> f32 {
    rad * 180.0 / std::f32::consts::PI
}

/// A world-space ray: an `origin` and a unit `direction`.
///
/// The terrain (and, later, collider) raycast marches along `direction` from
/// `origin` and returns the first intersection, so a "camera → mouse" ray is
/// built with the origin on the camera side and `direction` pointing into the
/// scene.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    pub origin: Vec3,
    pub dir: Vec3,
}

impl Ray {
    /// A ray from `origin` along `dir` (normalised).
    pub fn new(origin: Vec3, dir: Vec3) -> Self {
        Self { origin, dir: dir.normalize() }
    }

    /// The point `origin + dir · t`.
    pub fn at(&self, t: f32) -> Vec3 {
        self.origin + self.dir * t
    }
}
