//! Bloom (M4 of the particle plan): emissive particles, arcs and flashes glow.
//!
//! Applied after the particle phase and before the UI: the main target's
//! colour is bright-passed into a half-res target (soft-knee luminance
//! threshold), blurred (separable Gaussian) at half and quarter resolution,
//! and added back onto the main target.  An offscreen main target (headless,
//! golden runs) is sampled directly through a `LINEAR` sampler object; only
//! the default framebuffer (a window, a canvas), which cannot be sampled, is
//! first blitted to a texture.  Everything is
//! RGBA8 (no float render targets, so WebGL2 needs no extension); the engine
//! runs it only on frames with something emissive on screen, so scenes
//! without effects are pixel-identical.

use glow::HasContext;

use crate::{Gfx, GlFrameBuffer};

/// Bloom render targets: two ping-pong pairs, plus the full-res scene copy
/// when the main target is the default framebuffer (created on first use).
pub struct BloomTargets {
    width: u32,
    height: u32,
    src: Option<GlFrameBuffer>,
    half: [GlFrameBuffer; 2],
    quarter: [GlFrameBuffer; 2],
}

/// Texture units for the bloom samplers (clear of every scene binding).
const BLOOM_UNIT_A: u32 = 7;
const BLOOM_UNIT_B: u32 = 8;

fn linear_target(gl: &glow::Context, w: u32, h: u32) -> GlFrameBuffer {
    let t = GlFrameBuffer::new(gl, w.max(1), h.max(1), false);
    set_linear(gl, t.texture);
    t
}

fn set_linear(gl: &glow::Context, tex: glow::Texture) {
    unsafe {
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
        gl.bind_texture(glow::TEXTURE_2D, None);
    }
}

impl BloomTargets {
    fn new(gl: &glow::Context, w: u32, h: u32) -> Self {
        Self {
            width: w,
            height: h,
            src: None,
            half: [linear_target(gl, w / 2, h / 2), linear_target(gl, w / 2, h / 2)],
            quarter: [linear_target(gl, w / 4, h / 4), linear_target(gl, w / 4, h / 4)],
        }
    }

    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

impl Gfx {
    /// Run the bloom post-pass on the main target (see the module docs).
    /// `threshold` is the bright-pass level (max channel, `0..1`), `knee` the
    /// width over which it ramps to full weight (`smoothstep(threshold,
    /// threshold + knee, max channel)`), `intensity` the composite gain.
    /// Leaves the main target bound with blending restored to `SRC_ALPHA,
    /// ONE_MINUS_SRC_ALPHA` and depth/stencil off.
    pub fn apply_bloom(&mut self, threshold: f32, knee: f32, intensity: f32) {
        let (w, h) = self.main_target_size();
        if w < 4 || h < 4 {
            return;
        }
        if self.bloom.as_ref().map(|b| b.size()) != Some((w, h)) {
            self.bloom = Some(BloomTargets::new(&self.gl, w, h));
        }
        // The scene colour: the offscreen main target's own texture, else a
        // copy of the default framebuffer.
        let direct = self.render_target.as_ref().map(|rt| rt.texture);
        if direct.is_none() {
            let b = self.bloom.as_mut().expect("bloom targets");
            if b.src.is_none() {
                b.src = Some(linear_target(&self.gl, w, h));
            }
        }
        let gl = &self.gl;
        let b = self.bloom.as_ref().expect("bloom targets");
        let scene = match (direct, b.src.as_ref()) {
            (Some(tex), _) => tex,
            (None, Some(src)) => {
                unsafe {
                    // Default framebuffer -> `src` (a resolve when it is
                    // multisampled; same size, so that is allowed).
                    gl.bind_framebuffer(glow::READ_FRAMEBUFFER, None);
                    gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(src.fbo()));
                    gl.blit_framebuffer(
                        0,
                        0,
                        w as i32,
                        h as i32,
                        0,
                        0,
                        w as i32,
                        h as i32,
                        glow::COLOR_BUFFER_BIT,
                        glow::NEAREST,
                    );
                }
                src.texture
            }
            (None, None) => unreachable!("created above"),
        };

        unsafe {
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, None);
            gl.disable(glow::BLEND);
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::STENCIL_TEST);
            gl.depth_mask(false);
        }

        let thr = self.shader("bloomThreshold");
        let pass = |target: &GlFrameBuffer, src: glow::Texture, (sw, sh): (u32, u32), t: f32| {
            target.bind(gl);
            unsafe {
                gl.viewport(0, 0, target.width as i32, target.height as i32);
                gl.active_texture(glow::TEXTURE0 + BLOOM_UNIT_A);
                gl.bind_texture(glow::TEXTURE_2D, Some(src));
            }
            thr.bind(gl);
            thr.uniform_1i(gl, "src_tex", BLOOM_UNIT_A as i32);
            thr.uniform_vec2(gl, "src_texel", &[0.5 / sw as f32, 0.5 / sh as f32]);
            thr.uniform_1f(gl, "threshold", t);
            thr.uniform_1f(gl, "knee", knee);
            self.draw_post_quad(thr);
        };
        // The main target's texture keeps its own (NEAREST) filtering for
        // every other use; a sampler object makes this one read LINEAR, as
        // the copy's texture is (the bright pass's 4 taps cover 16 texels).
        crate::perf::phase("bloom_bright");
        unsafe { gl.bind_sampler(BLOOM_UNIT_A, Some(self.particle_sampler)) };
        pass(&b.half[0], scene, (w, h), threshold);
        unsafe { gl.bind_sampler(BLOOM_UNIT_A, None) };
        crate::perf::phase("bloom_blur");
        self.blur(&b.half[0], &b.half[1]);
        let half = &b.half[0];
        pass(&b.quarter[0], half.texture, (half.width, half.height), -1.0);
        self.blur(&b.quarter[0], &b.quarter[1]);
        self.blur(&b.quarter[0], &b.quarter[1]);

        crate::perf::phase("bloom_composite");
        // Composite onto the main target.
        self.bind_main_target();
        let c = self.shader("bloomComposite");
        unsafe {
            gl.active_texture(glow::TEXTURE0 + BLOOM_UNIT_A);
            gl.bind_texture(glow::TEXTURE_2D, Some(b.half[0].texture));
            gl.active_texture(glow::TEXTURE0 + BLOOM_UNIT_B);
            gl.bind_texture(glow::TEXTURE_2D, Some(b.quarter[0].texture));
            gl.active_texture(glow::TEXTURE0);
            gl.enable(glow::BLEND);
            gl.blend_func(glow::ONE, glow::ONE);
        }
        c.bind(gl);
        c.uniform_1i(gl, "near_tex", BLOOM_UNIT_A as i32);
        c.uniform_1i(gl, "far_tex", BLOOM_UNIT_B as i32);
        c.uniform_1f(gl, "intensity", intensity);
        self.draw_post_quad(c);
        unsafe {
            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
            gl.depth_mask(true);
        }
    }

    /// Horizontal blur `a -> b`, then vertical `b -> a`.
    fn blur(&self, a: &GlFrameBuffer, b: &GlFrameBuffer) {
        let gl = &self.gl;
        let s = self.shader("bloomBlur");
        s.bind(gl);
        s.uniform_1i(gl, "src_tex", BLOOM_UNIT_A as i32);
        for (src, dst, dir) in
            [(a, b, [1.0 / a.width as f32, 0.0]), (b, a, [0.0, 1.0 / b.height as f32])]
        {
            dst.bind(gl);
            unsafe {
                gl.viewport(0, 0, dst.width as i32, dst.height as i32);
                gl.active_texture(glow::TEXTURE0 + BLOOM_UNIT_A);
                gl.bind_texture(glow::TEXTURE_2D, Some(src.texture));
            }
            s.uniform_vec2(gl, "blur_dir", &dir);
            self.draw_post_quad(s);
        }
    }

    /// Draw the shared unit quad with a post shader (`vertex_pos` attribute).
    fn draw_post_quad(&self, s: &crate::Shader) {
        let gl = &self.gl;
        crate::buffer::vertex_attrib_ptr_f32(gl, &self.quad.verts, s.attr("vertex_pos"), 3, 0, 0);
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
}
