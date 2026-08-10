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

pub fn deg_to_rad(deg: f32) -> f32 {
    deg * std::f32::consts::PI / 180.0
}

pub fn rad_to_deg(rad: f32) -> f32 {
    rad * 180.0 / std::f32::consts::PI
}
