//! # Skill: `classic-ecs`
//!
//! **Read `.claude/skills/classic-ecs/SKILL.md` before working on this module.**
//!
use glam::{Mat4, Vec3};

/// 2D camera for an orthographic projection.
///
/// Port of `src/classic/camera.ts`.
#[derive(Clone, Debug)]
pub struct Camera {
    pub position: Vec3,
    pub scale: Vec3,
    pub size: Vec3,
}

impl Camera {
    pub fn new(position: Vec3, scale: Vec3) -> Self {
        Self { position, scale, size: Vec3::ZERO }
    }

    /// Set viewport size (called on canvas resize).
    pub fn resize(&mut self, size: Vec3) {
        self.size = size;
    }

    /// Compute the "fix point" — the scaled position re-centred on the viewport.
    /// This is what the camera-matrix translation negates.
    ///
    /// Port of `camera.ts:getFix()`.
    /// Formula: `fix = position * scale - size / [2, 2, 1]`
    /// (This makes `position * scale` map to the viewport centre `size/2`.)
    pub fn fix(&self) -> Vec3 {
        self.position * self.scale - self.size / Vec3::new(2.0, 2.0, 1.0)
    }

    /// Build the camera view matrix.
    ///
    /// port of `camera.ts:matrix()`.
    pub fn matrix(&self) -> Mat4 {
        let fix = self.fix();
        Mat4::from_translation(-fix) * Mat4::from_scale(self.scale)
    }
}
