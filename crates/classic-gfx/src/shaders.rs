//! # Skill: `classic-gfx`
//!
//! **Read `.claude/skills/classic-gfx/SKILL.md` before working on this module.**
//!
/// Embedded shader sources.
pub const DIRECT_VERT: &str = include_str!("shaders/direct.vert");
pub const DIRECT_TEX_VERT: &str = include_str!("shaders/direct_tex.vert");
pub const ISO_TILEMAP_VERT: &str = include_str!("shaders/iso_tilemap.vert");
pub const SDF_VERT: &str = include_str!("shaders/sdf.vert");
pub const SHADOW_DEPTH_VERT: &str = include_str!("shaders/shadow_depth.vert");
pub const SHADOW_SPRITE_VERT: &str = include_str!("shaders/shadow_sprite.vert");

pub const IMAGE_FRAG: &str = include_str!("shaders/image.frag");
pub const IMAGE_COLORIZED_FRAG: &str = include_str!("shaders/image_colorized.frag");
pub const ISO_TILEMAP_FRAG: &str = include_str!("shaders/iso_tilemap.frag");
pub const SHEET_FRAG: &str = include_str!("shaders/sheet.frag");
pub const SDF_FRAG: &str = include_str!("shaders/sdf.frag");
pub const SOLID_FRAG: &str = include_str!("shaders/solid.frag");
pub const SHADOW_DEPTH_FRAG: &str = include_str!("shaders/shadow_depth.frag");
pub const SHADOW_SPRITE_FRAG: &str = include_str!("shaders/shadow_sprite.frag");
