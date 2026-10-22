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
pub const PARTICLE_VERT: &str = include_str!("shaders/particle.vert");
pub const PARTICLE_FRAG: &str = include_str!("shaders/particle.frag");
pub const RIBBON_VERT: &str = include_str!("shaders/ribbon.vert");
pub const RIBBON_FRAG: &str = include_str!("shaders/ribbon.frag");
pub const POST_VERT: &str = include_str!("shaders/post.vert");
pub const BLOOM_THRESHOLD_FRAG: &str = include_str!("shaders/bloom_threshold.frag");
pub const BLOOM_BLUR_FRAG: &str = include_str!("shaders/bloom_blur.frag");
pub const BLOOM_COMPOSITE_FRAG: &str = include_str!("shaders/bloom_composite.frag");
pub const PARTICLE_COMPOSITE_FRAG: &str = include_str!("shaders/particle_composite.frag");

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
        // Lit particles evaluate the same block per vertex.
        assert_eq!(sheet, lighting_block(PARTICLE_VERT), "particle.vert drifted");
    }

    /// The `BEGIN SUN SHADOW` .. `END SUN SHADOW` span of a shader.
    fn shadow_block(src: &str) -> &str {
        let begin = src.find("// --- BEGIN SUN SHADOW").expect("BEGIN marker");
        let end = src.find("// --- END SUN SHADOW ---").expect("END marker");
        &src[begin..end]
    }

    /// Terrain, sprites and models shadow through the exact same lookup: a
    /// hardware-compared `sampler2DShadow`, four bilinear taps.
    #[test]
    fn lit_shaders_share_the_sun_shadow_block() {
        let sheet = shadow_block(SHEET_FRAG);
        assert_eq!(sheet, shadow_block(ISO_TILEMAP_FRAG), "iso_tilemap.frag drifted");
        assert_eq!(sheet, shadow_block(MESH_FRAG), "mesh.frag drifted");
        assert_eq!(sheet.matches("texture(shadow_map, vec3(").count(), 4, "four taps");
        assert!(sheet.contains("float ref = ndc.z * 0.5 + 0.5 - shadow_bias;"));
        for src in [SHEET_FRAG, ISO_TILEMAP_FRAG, MESH_FRAG] {
            assert!(src.contains("uniform highp sampler2DShadow shadow_map;"));
            assert!(!src.contains("shadowSample"), "no manual compare left");
        }
    }

    /// CPU model of the four `LINEAR` compare taps at `(+-0.5, +-0.5)` texel
    /// from a texel centre: each lands on a texel corner and averages the 2x2
    /// compares around it, so together they weigh the 3x3 texels around the
    /// fragment `[1, 2, 1] x [1, 2, 1] / 16` -- a tent over the footprint the
    /// old 9-tap box PCF covered, so soft edges stay comparable.
    #[test]
    fn four_bilinear_taps_make_a_3x3_tent() {
        let mut w = [[0.0f32; 3]; 3];
        for (cx, cy) in [(-0.5f32, -0.5f32), (0.5, -0.5), (-0.5, 0.5), (0.5, 0.5)] {
            // Bilinear weights of the 2x2 texels around a corner (all 1/4).
            for dx in [cx - 0.5, cx + 0.5] {
                for dy in [cy - 0.5, cy + 0.5] {
                    w[(dy + 1.0) as usize][(dx + 1.0) as usize] += 0.25 * 0.25;
                }
            }
        }
        let tent = [[1.0, 2.0, 1.0], [2.0, 4.0, 2.0], [1.0, 2.0, 1.0]].map(|r| r.map(|v| v / 16.0));
        assert_eq!(w, tent);
        assert_eq!(w.iter().flatten().sum::<f32>(), 1.0);
    }

    /// Spot lights shipped with an inverted cone (`dot(L, dir)`, so a spot
    /// only lit what sat *behind* it) and a soft edge reaching
    /// `acos(0.6 cos a)` -- ~56 deg for a 20 deg cone.  Nothing noticed until
    /// the `lighttest` ROM declared spots.  `dir` is the beam direction.
    #[test]
    fn spot_cone_faces_along_the_beam() {
        let block = lighting_block(SHEET_FRAG);
        assert!(block.contains("dot(-L, normalize(l.dir_cone.xyz))"), "spot dir must be the beam");
        assert!(block.contains("smoothstep(cos(l.dir_cone.w * 1.25), cos(l.dir_cone.w)"));
    }
}
