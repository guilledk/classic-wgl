//! The 3D-model pixelation pass: draw every model mesh into a low-res offscreen
//! target, then composite it back onto the main target block-for-block.
//!
//! Sprites are pre-rendered at `PPM_TARGET` (64 px/m) and sampled `NEAREST`,
//! so one sprite texel spans `zoom` screen pixels.  Rendering models at
//! `viewport × min(1, 1/zoom)` gives them the same texel size: sprite-sized
//! blocks when zoomed in, a plain pass-through at zoom ≤ 1.
//!
//! The composite writes `gl_FragDepth`, so terrain and sprites keep occluding
//! models (and vice versa) through the shared depth buffer, and it takes part in
//! the sprite ghost scheme: the normal composite stamps [`MODEL_GHOST_GROUP`]
//! into the stencil, and the ghost composite redraws occluded model pixels at the
//! sprite ghost alpha.

use glow::HasContext;

use crate::framebuffer::ModelTarget;
use crate::{Gfx, IsoSpritePass};
pub use classic_core::model::EmissiveRect;

/// Stencil ghost group reserved for 3D models (all models share one
/// composite, so they share one group).  Sprite/vehicle groups stay in
/// `1..MODEL_GHOST_GROUP`.
pub const MODEL_GHOST_GROUP: u32 = 255;

/// Texture units for the composite's samplers (clear of the sprite maps on
/// 0..2 and the shadow map on [`crate::SHADOW_MAP_UNIT`]).
const COMPOSITE_COLOR_UNIT: u32 = 4;
const COMPOSITE_DEPTH_UNIT: u32 = 5;
/// The ghost mask's unit: the particle texture's, which the particle pass
/// rebinds on every draw.
const COMPOSITE_MASK_UNIT: u32 = 6;

/// The model target size for a main target of `width` x `height` at camera
/// `zoom`: `ceil(size × min(1, 1/zoom))`, at least 1 px.  A non-positive or
/// non-finite zoom renders at full resolution.
pub fn model_target_size(width: u32, height: u32, zoom: f32) -> (u32, u32) {
    let scale = if zoom.is_finite() && zoom > 1.0 { 1.0 / zoom } else { 1.0 };
    let dim = |d: u32| ((d as f32 * scale).ceil() as u32).max(1);
    (dim(width), dim(height))
}

/// A mesh's team tint for [`Gfx::draw_model`]: multiply the sampled albedo by
/// `tint` where the fragment's `TEXCOORD_0` lies inside `rect`.
///
/// `rect` is `[u0, v0, u1, v1]` in glTF texture space (top-left origin, the
/// space `mesh.vert` passes through unflipped), half-open on the far edges:
/// see `classic_core::model::TEAM_RECT_EXTRA` for the asset contract.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TeamTint {
    pub rect: [f32; 4],
    pub tint: [f32; 3],
}

/// Most self-lit rects one mesh draw takes (`MAX_EMISSIVE` in `mesh.frag`).
pub const MAX_EMISSIVE: usize = classic_core::model::MAX_EMISSIVE_RECTS;

/// The CPU reference of `mesh.frag`'s emissive term: the colour of a
/// fragment with `albedo` (after the team tint) lit by `light` (the summed
/// ambient + sun + point-light factor), plus `albedo × strength` of the first
/// of `rects` containing `uv` (strengths already scaled by `Model.emissive`).
pub fn emissive_shade(
    rects: &[EmissiveRect],
    uv: [f32; 2],
    albedo: [f32; 3],
    light: [f32; 3],
) -> [f32; 3] {
    let s = rects.iter().take(MAX_EMISSIVE).find(|r| r.contains(uv)).map_or(0.0, |r| r.strength);
    [0, 1, 2].map(|i| albedo[i] * light[i] + albedo[i] * s)
}

impl TeamTint {
    /// Whether `uv` is inside [`Self::rect`] (`u0 <= u < u1 && v0 <= v < v1`).
    pub fn contains(&self, uv: [f32; 2]) -> bool {
        let [u0, v0, u1, v1] = self.rect;
        u0 <= uv[0] && uv[0] < u1 && v0 <= uv[1] && uv[1] < v1
    }

    /// The CPU reference of `mesh.frag`'s `teamTint`: the albedo `rgba`
    /// sampled at `uv`, tinted inside the rect and returned unchanged
    /// (bit-for-bit) outside it.
    pub fn apply(&self, uv: [f32; 2], rgba: [f32; 4]) -> [f32; 4] {
        if !self.contains(uv) {
            return rgba;
        }
        let [r, g, b, a] = rgba;
        [r * self.tint[0], g * self.tint[1], b * self.tint[2], a]
    }
}

impl Gfx {
    /// The main render target's size (the offscreen FBO when set, else the
    /// default framebuffer's viewport).
    pub(crate) fn main_target_size(&self) -> (u32, u32) {
        match &self.render_target {
            Some(rt) => (rt.width, rt.height),
            None => (self.viewport_w as u32, self.viewport_h as u32),
        }
    }

    /// Rebind the main render target with its full viewport.
    pub(crate) fn bind_main_target(&self) {
        let gl = &self.gl;
        unsafe {
            match &self.render_target {
                Some(rt) => {
                    rt.bind(gl);
                    gl.viewport(0, 0, rt.width as i32, rt.height as i32);
                }
                None => {
                    gl.bind_framebuffer(glow::FRAMEBUFFER, None);
                    gl.viewport(0, 0, self.viewport_w as i32, self.viewport_h as i32);
                }
            }
        }
    }

    /// Start the model pass: size the pixelation target for `zoom`, bind it and
    /// clear it (colour alpha 0, depth 1).  Draw the model meshes with
    /// [`Gfx::draw_model`] afterwards, then call [`Gfx::end_model_pass`].
    pub fn begin_model_pass(&mut self, zoom: f32) {
        let (w, h) = self.main_target_size();
        let (tw, th) = model_target_size(w, h, zoom);
        self.pass_target.set((tw, th));
        match self.model_target.as_mut() {
            Some(t) => t.resize(&self.gl, tw, th),
            None => self.model_target = Some(ModelTarget::new(&self.gl, tw, th)),
        }
        let gl = &self.gl;
        let target = self.model_target.as_ref().expect("model target");
        target.bind(gl);
        unsafe {
            gl.viewport(0, 0, tw as i32, th as i32);
            gl.clear_color(0.0, 0.0, 0.0, 0.0);
            gl.depth_mask(true);
            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
            gl.clear_color(0.0, 0.0, 0.0, 1.0);
        }
    }

    /// End the model pass: rebind the main render target and viewport.
    pub fn end_model_pass(&self) {
        self.pass_target.set(self.main_target_size());
        self.bind_main_target();
    }

    /// Composite the model target onto the main target.
    ///
    /// - [`IsoSpritePass::Normal`]: depth `LEQUAL`, writes depth and stamps
    ///   `ghost_group` into the stencil (like a sprite's normal pass).
    /// - [`IsoSpritePass::Ghost`]: 40% alpha where the model is behind the depth
    ///   buffer (`GREATER`), skipping pixels its own group already covers and
    ///   those of models that opt out (`Model.ghost`, the target's mask).
    ///
    /// No-op before the first [`Gfx::begin_model_pass`].
    pub fn composite_models(&self, pass: IsoSpritePass, ghost_group: u32) {
        let Some(target) = &self.model_target else { return };
        let gl = &self.gl;
        let s = self.shader("modelComposite");
        s.bind(gl);
        unsafe {
            gl.active_texture(glow::TEXTURE0 + COMPOSITE_COLOR_UNIT);
            gl.bind_texture(glow::TEXTURE_2D, Some(target.color_tex));
            gl.active_texture(glow::TEXTURE0 + COMPOSITE_DEPTH_UNIT);
            gl.bind_texture(glow::TEXTURE_2D, Some(target.depth_tex));
            gl.active_texture(glow::TEXTURE0 + COMPOSITE_MASK_UNIT);
            gl.bind_texture(glow::TEXTURE_2D, Some(target.mask_tex));
            gl.active_texture(glow::TEXTURE0);
        }
        s.uniform_1i(gl, "color_tex", COMPOSITE_COLOR_UNIT as i32);
        s.uniform_1i(gl, "depth_tex", COMPOSITE_DEPTH_UNIT as i32);
        s.uniform_1i(gl, "mask_tex", COMPOSITE_MASK_UNIT as i32);
        s.uniform_1f(
            gl,
            "ghost_alpha",
            match pass {
                IsoSpritePass::Normal => 0.0,
                IsoSpritePass::Ghost => 0.4,
            },
        );
        crate::buffer::vertex_attrib_ptr_f32(gl, &self.quad.verts, s.attr("vertex_pos"), 3, 0, 0);
        self.quad.indices.bind(gl);

        unsafe {
            gl.enable(glow::DEPTH_TEST);
            gl.enable(glow::STENCIL_TEST);
            match pass {
                IsoSpritePass::Normal => {
                    gl.depth_func(glow::LEQUAL);
                    gl.depth_mask(true);
                    gl.stencil_func(glow::ALWAYS, ghost_group as i32, 0xFF);
                    gl.stencil_op(glow::KEEP, glow::KEEP, glow::REPLACE);
                    gl.stencil_mask(0xFF);
                }
                IsoSpritePass::Ghost => {
                    gl.depth_func(glow::GREATER);
                    gl.depth_mask(false);
                    gl.stencil_func(glow::NOTEQUAL, ghost_group as i32, 0xFF);
                    gl.stencil_op(glow::KEEP, glow::KEEP, glow::KEEP);
                    gl.stencil_mask(0x00);
                }
            }
            gl.draw_elements(
                glow::TRIANGLES,
                self.quad.index_count as i32,
                glow::UNSIGNED_SHORT,
                0,
            );
            gl.disable(glow::STENCIL_TEST);
            gl.stencil_mask(0xFF);
            gl.depth_mask(true);
            gl.depth_func(glow::LEQUAL);
            gl.disable(glow::DEPTH_TEST);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_target_tracks_zoom_only_when_zoomed_in() {
        // zoom <= 1: sprite texels are <= 1 screen px, render at full res.
        assert_eq!(model_target_size(1280, 720, 0.32), (1280, 720));
        assert_eq!(model_target_size(1280, 720, 1.0), (1280, 720));
        // zoom 2: one model texel = one sprite texel = 2 screen px.
        assert_eq!(model_target_size(1280, 720, 2.0), (640, 360));
        // Non-integer sizes round up so the target always covers the viewport.
        assert_eq!(model_target_size(1281, 721, 2.0), (641, 361));
        assert_eq!(model_target_size(1280, 720, 3.0), (427, 240));
        // Degenerate zooms fall back to full resolution; never 0 px.
        assert_eq!(model_target_size(1280, 720, 0.0), (1280, 720));
        assert_eq!(model_target_size(1280, 720, f32::NAN), (1280, 720));
        assert_eq!(model_target_size(1, 1, 1000.0), (1, 1));
    }

    #[test]
    fn team_tint_only_touches_texels_inside_the_rect() {
        let t = TeamTint { rect: [0.5, 0.25, 0.75, 0.5], tint: [1.0, 0.25, 0.5] };
        let albedo = [0.8, 0.8, 0.8, 1.0];
        // Inside: rgb multiplied, alpha kept.
        assert_eq!(t.apply([0.6, 0.3], albedo), [0.8, 0.2, 0.4, 1.0]);
        // The near edges are inside, the far edges outside (half-open), so a
        // rect on texel edges selects exactly its texels.
        assert!(t.contains([0.5, 0.25]));
        assert!(!t.contains([0.75, 0.3]) && !t.contains([0.6, 0.5]));
        // Outside on every side: bit-identical.
        for uv in [[0.49, 0.3], [0.8, 0.3], [0.6, 0.2], [0.6, 0.6], [0.0, 0.0], [1.0, 1.0]] {
            assert_eq!(t.apply(uv, albedo), albedo, "{uv:?}");
        }
    }

    #[test]
    fn emissive_adds_unlit_albedo_only_inside_its_rects() {
        let rects = [
            EmissiveRect { rect: [0.5, 0.0, 1.0, 0.5], strength: 2.0 },
            EmissiveRect { rect: [0.0, 0.0, 0.75, 0.25], strength: 9.0 },
        ];
        let (albedo, dark) = ([0.5, 0.25, 0.125], [0.0; 3]);
        // In the dark, inside: albedo x strength; the first matching rect wins.
        assert_eq!(emissive_shade(&rects, [0.6, 0.1], albedo, dark), [1.0, 0.5, 0.25]);
        assert_eq!(emissive_shade(&rects, [0.1, 0.1], albedo, dark), [4.5, 2.25, 1.125]);
        // Outside every rect (and on the far edges): lit colour unchanged.
        let light = [0.8, 0.8, 0.8];
        for uv in [[0.1, 0.3], [1.0, 0.1], [0.6, 0.5], [0.75, 0.6]] {
            assert_eq!(emissive_shade(&rects, uv, albedo, light), [0.4, 0.2, 0.1], "{uv:?}");
        }
        // Added on top of the lighting, not multiplied by it.
        assert_eq!(emissive_shade(&rects, [0.6, 0.4], albedo, light), [1.4, 0.7, 0.35]);
        assert_eq!(emissive_shade(&[], [0.6, 0.4], albedo, light), [0.4, 0.2, 0.1]);
    }

    /// `mesh.frag` must implement exactly [`emissive_shade`]: same rect
    /// test, first match wins, added after lighting to the pre-light albedo.
    #[test]
    fn mesh_frag_emissive_matches_the_cpu_reference() {
        let src = crate::shaders::MESH_FRAG;
        assert!(src.contains(&format!("#define MAX_EMISSIVE {MAX_EMISSIVE}")));
        assert!(src.contains("uniform highp vec4 emissive_rect[MAX_EMISSIVE];"));
        assert!(src.contains("if (all(greaterThanEqual(uv, r.xy)) && all(lessThan(uv, r.zw))) {"));
        assert!(src.contains("return emissive_strength[i];"), "first match wins");
        let main = &src[src.find("void main").unwrap()..];
        let albedo = main.find("vec3 albedo = color.rgb;").expect("albedo kept");
        let lit = main.find("color.rgb *= ambient_color").expect("lighting");
        let glow = main.find("color.rgb += albedo * emissiveStrength(vTexCoord);").expect("glow");
        assert!(main.find("teamTint(").unwrap() < albedo, "the glow uses the tinted albedo");
        assert!(albedo < lit && lit < glow, "added after lighting, unlit");
        assert!(main.contains("emissive_count > 0.5 && use_texture > 0.5"));
    }

    /// `mesh.frag` must implement exactly [`TeamTint::apply`]: the same
    /// half-open predicate, gated on a textured mesh with a team tint, and
    /// applied to the albedo before lighting.
    #[test]
    fn mesh_frag_team_tint_matches_the_cpu_reference() {
        let src = crate::shaders::MESH_FRAG;
        assert!(src.contains("uniform highp vec4 team_rect;"), "rect needs highp at 2048 texels");
        assert!(src.contains(
            "all(greaterThanEqual(uv, team_rect.xy)) && all(lessThan(uv, team_rect.zw))"
        ));
        assert!(src.contains("color.rgb *= team_tint;"));
        let main = &src[src.find("void main").unwrap()..];
        let tint = main.find("teamTint(").expect("main applies the tint");
        let lit = main.find("color.rgb *= ambient_color").expect("lighting");
        assert!(tint < lit, "the team tint multiplies the albedo, before lighting");
        assert!(main.contains("use_team > 0.5 && use_texture > 0.5"));
    }
}
