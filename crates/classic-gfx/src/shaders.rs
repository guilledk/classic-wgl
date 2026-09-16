//! # Skill: `classic-gfx`
//!
//! **Read `.agents/skills/classic-gfx/SKILL.md` before working on this module.**
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
pub const MESH_VERT: &str = include_str!("shaders/mesh.vert");
pub const MESH_FRAG: &str = include_str!("shaders/mesh.frag");
pub const MODEL_COMPOSITE_VERT: &str = include_str!("shaders/model_composite.vert");
pub const MODEL_COMPOSITE_FRAG: &str = include_str!("shaders/model_composite.frag");

#[cfg(test)]
mod tests {
    use super::*;

    /// The `BEGIN SHARED LIGHTING` .. `END SHARED LIGHTING` span of a shader.
    fn lighting_block(src: &str) -> &str {
        let begin = src.find("// --- BEGIN SHARED LIGHTING").expect("BEGIN marker");
        let end = src.find("// --- END SHARED LIGHTING ---").expect("END marker");
        &src[begin..end]
    }

    /// Every lit shader evaluates the dynamic lights with the exact same code,
    /// so a light reads identically on terrain, sprites and 3D models.
    #[test]
    fn lit_shaders_share_the_lighting_block() {
        let sheet = lighting_block(SHEET_FRAG);
        assert_eq!(sheet, lighting_block(ISO_TILEMAP_FRAG), "iso_tilemap.frag drifted");
        assert_eq!(sheet, lighting_block(MESH_FRAG), "mesh.frag drifted");
    }
}
