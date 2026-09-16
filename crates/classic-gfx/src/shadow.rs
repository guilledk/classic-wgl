//! `Gfx` directional shadow pass.

use glam::Mat4;
use glow::HasContext;

use crate::buffer::vertex_attrib_ptr_f32;
use crate::{
    DepthFramebuffer, Gfx, GlBuffer, ModelMeshGpu, SpriteRegion, MODEL_VERTEX_STRIDE,
    SHADOW_MAP_SIZE, SHADOW_SPRITE_SLOPE_OFFSET, SHADOW_SPRITE_UNIT_OFFSET,
};

impl Gfx {
    // -- shadow pass -------------------------------------------------------

    /// The depth texture of the directional shadow map (raw GL handle for
    /// sampling in the lit shaders).  Lazily created on first use.
    pub fn shadow_map_texture(&mut self) -> Option<glow::Texture> {
        self.ensure_shadow_map();
        self.shadow_map.as_ref().map(|s| s.depth_tex)
    }

    fn ensure_shadow_map(&mut self) {
        if self.shadow_map.is_none() {
            self.shadow_map =
                Some(DepthFramebuffer::new(&self.gl, SHADOW_MAP_SIZE, SHADOW_MAP_SIZE));
        }
    }

    /// Begin the directional shadow pass: bind the depth-only shadow FBO, size
    /// the viewport to the shadow map, and clear depth to 1.0 (far).  The caller
    /// then emits shadow casters via [`Gfx::draw_shadow_tilemap`] and finishes
    /// with [`Gfx::end_shadow_pass`].
    pub fn begin_shadow_pass(&mut self) {
        self.ensure_shadow_map();
        let gl = &self.gl;
        let Some(sm) = &self.shadow_map else { return };
        sm.bind(gl);
        unsafe {
            gl.viewport(0, 0, sm.width as i32, sm.height as i32);
            gl.clear_depth_f32(1.0);
            gl.clear(glow::DEPTH_BUFFER_BIT);
            gl.disable(glow::BLEND);
            gl.disable(glow::STENCIL_TEST);
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            gl.depth_mask(true);
            // Push casters slightly away from the light so coplanar terrain
            // Constant-depth offset only.  A slope-scaled factor (the old
            // `polygon_offset(2.0, 4.0)`) blows up as the depth slope grows,
            // which pushed every occluder behind every receiver and produced
            // exactly zero shadows.  Acne is handled by normal-offset bias on
            // the receive side instead — see `SHADOW_NORMAL_OFFSET`.
            gl.enable(glow::POLYGON_OFFSET_FILL);
            gl.polygon_offset(0.0, 1.0);
        }
    }

    /// Draw one terrain mesh into the shadow map in world space (no camera
    /// matrix; `light_view_proj` maps world metres straight to light clip).
    pub fn draw_shadow_tilemap(
        &self,
        model: &Mat4,
        view_proj: &Mat4,
        vertex_count: i32,
        vertex_buffer: &GlBuffer,
    ) {
        let gl = &self.gl;
        let s = self.shader("shadowDepth");
        s.bind(gl);
        vertex_attrib_ptr_f32(gl, vertex_buffer, s.attr("vertex_pos"), 3, 36, 0);
        s.uniform_mat4(gl, "model_matrix", model);
        s.uniform_mat4(gl, "light_view_proj", view_proj);
        unsafe {
            gl.draw_arrays(glow::TRIANGLES, 0, vertex_count);
        }
    }

    /// Draw one 3D model mesh instance into the shadow map (depth only, real
    /// geometry — no alpha silhouette).  `model` is the instance's world
    /// transform.  Draw model casters with the terrain's constant offset, i.e.
    /// before [`Gfx::set_shadow_sprite_offset`] switches to slope scaling.
    pub fn draw_shadow_model(&self, model: &Mat4, view_proj: &Mat4, mesh: &ModelMeshGpu) {
        let gl = &self.gl;
        let s = self.shader("shadowDepth");
        s.bind(gl);
        vertex_attrib_ptr_f32(gl, &mesh.vbo, s.attr("vertex_pos"), 3, MODEL_VERTEX_STRIDE, 0);
        s.uniform_mat4(gl, "model_matrix", model);
        s.uniform_mat4(gl, "light_view_proj", view_proj);
        mesh.indices.bind(gl);
        unsafe {
            gl.draw_elements(glow::TRIANGLES, mesh.index_count as i32, glow::UNSIGNED_INT, 0);
        }
    }

    /// Switch the shadow pass to slope-scaled depth offset for sprite casters.
    ///
    /// A sprite is a *plane*, and it is both caster and receiver: every sprite
    /// fragment samples the very texels its own billboard wrote.  Because the
    /// plane is slanted relative to the light, stored depth varies across the
    /// PCF kernel, neighbouring taps disagree, and the sprite stipples itself
    /// with ~50% self-shadow.  Normal-offset bias cannot fix this — the offset
    /// stays inside the billboard's own (large) footprint in the shadow map.
    ///
    /// Slope-scaled offset is the right tool here precisely because the error
    /// being corrected *is* proportional to the depth slope.  It is safe now
    /// that the light-space geometry is correct; it was catastrophic before
    /// only because the degenerate 2.7° sun made every slope enormous.
    ///
    /// Terrain casters keep the constant offset from [`Gfx::begin_shadow_pass`].
    pub fn set_shadow_sprite_offset(&self) {
        unsafe {
            self.gl.polygon_offset(SHADOW_SPRITE_SLOPE_OFFSET, SHADOW_SPRITE_UNIT_OFFSET);
        }
    }

    /// Draw one sprite world quad into the shadow map.  The colour texture's
    /// alpha is the silhouette (transparent pixels discard), so the sprite casts
    /// a shaped shadow rather than a full quad.
    pub fn draw_shadow_sprite(
        &self,
        model: &Mat4,
        view_proj: &Mat4,
        texture_name: &str,
        region: SpriteRegion<'_>,
    ) {
        let gl = &self.gl;
        let s = self.shader("shadowSprite");
        let t = self.texture(texture_name);

        s.bind(gl);
        t.bind(gl, 0);
        s.uniform_1i(gl, "tex_sampler", 0);
        s.uniform_mat4(gl, "model_matrix", model);
        s.uniform_mat4(gl, "light_view_proj", view_proj);
        match region {
            SpriteRegion::Grid { frame, tile_set_size } => {
                s.uniform_1f(gl, "tile_id_flat", frame);
                s.uniform_vec2(gl, "tile_set_size", &tile_set_size);
                s.uniform_1f(gl, "use_uv_rect", 0.0);
                s.uniform_vec4(gl, "uv_rect", &[0.0, 0.0, 0.0, 0.0]);
                s.uniform_vec2(gl, "trim_offset", &[0.0, 0.0]);
                s.uniform_vec2(gl, "source_size", &[1.0, 1.0]);
                s.uniform_vec2(gl, "content_size", &[1.0, 1.0]);
            }
            SpriteRegion::Uv { uv_rect, trim_offset, source_size, content_size } => {
                s.uniform_1f(gl, "tile_id_flat", 0.0);
                s.uniform_vec2(gl, "tile_set_size", &[1.0, 1.0]);
                s.uniform_1f(gl, "use_uv_rect", 1.0);
                s.uniform_vec4(gl, "uv_rect", uv_rect);
                s.uniform_vec2(gl, "trim_offset", trim_offset);
                s.uniform_vec2(gl, "source_size", source_size);
                s.uniform_vec2(gl, "content_size", content_size);
            }
        }

        vertex_attrib_ptr_f32(gl, &self.quad.verts, s.attr("vertex_pos"), 3, 0, 0);
        vertex_attrib_ptr_f32(gl, &self.quad.uv, s.attr("tex_coord"), 2, 0, 0);
        self.quad.indices.bind(gl);

        unsafe {
            gl.draw_elements(
                glow::TRIANGLES,
                self.quad.index_count as i32,
                glow::UNSIGNED_SHORT,
                0,
            );
        }
    }

    /// End the shadow pass: restore the depth state and rebind the main render
    /// target (offscreen FBO or the default framebuffer) with the main viewport.
    pub fn end_shadow_pass(&self) {
        let gl = &self.gl;
        unsafe {
            gl.disable(glow::POLYGON_OFFSET_FILL);
            gl.polygon_offset(0.0, 0.0);
            gl.depth_mask(true);
            gl.depth_func(glow::LEQUAL);
            gl.disable(glow::DEPTH_TEST);
            gl.enable(glow::BLEND);
            if let Some(ref rt) = self.render_target {
                rt.bind(gl);
                gl.viewport(0, 0, rt.width as i32, rt.height as i32);
            } else {
                gl.bind_framebuffer(glow::FRAMEBUFFER, None);
                gl.viewport(0, 0, self.viewport_w as i32, self.viewport_h as i32);
            }
        }
    }
}
