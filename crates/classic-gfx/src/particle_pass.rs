//! Half-resolution particle pass: particles are soft and overlap heavily, so
//! their fill cost dominates a big effect (a rocket's ground cloud).  Drawing
//! them at half resolution quarters the shaded pixels.
//!
//! [`Gfx::begin_particle_pass`] blits the scene depth (NEAREST, scaled) into
//! the half-res target's `DEPTH24_STENCIL8` renderbuffer, so particles still
//! depth-test against terrain, models and depth-mapped sprites, then clears
//! its colour to transparent.  Particles draw with their usual premultiplied
//! blend (`ONE, ONE_MINUS_SRC_ALPHA`), so the target holds premultiplied
//! colour + coverage, and [`Gfx::end_particle_pass`] composites it back with
//! the same blend — exact "over" compositing, upscaled LINEAR.
//!
//! The depth blit needs a single-sample main framebuffer with a matching
//! depth format.  A multisampled main target (a browser canvas with
//! `antialias`) or a blit GL error switches the pass off for good, and
//! particles draw at full resolution as before.

use std::cell::Cell;

use glow::HasContext;

use crate::Gfx;

/// Divisor of the main target size (2 = half resolution per axis).
pub const PARTICLE_PASS_DIVISOR: u32 = 2;

/// Texture unit the composite samples (clear of every scene binding).
const COMPOSITE_UNIT: u32 = 10;

/// The half-res particle target: LINEAR colour texture + depth/stencil
/// renderbuffer (the depth blit's destination).
pub struct ParticleTarget {
    fbo: glow::Framebuffer,
    color: glow::Texture,
    depth: glow::Renderbuffer,
    pub width: u32,
    pub height: u32,
}

impl ParticleTarget {
    fn new(gl: &glow::Context, width: u32, height: u32) -> Self {
        unsafe {
            let fbo = gl.create_framebuffer().expect("particle fbo");
            let color = gl.create_texture().expect("particle color");
            let depth = gl.create_renderbuffer().expect("particle depth");
            let t = Self { fbo, color, depth, width: 0, height: 0 };
            gl.bind_texture(glow::TEXTURE_2D, Some(color));
            for (p, v) in [
                (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
                (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
                (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
            ] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, p, v as i32);
            }
            gl.bind_texture(glow::TEXTURE_2D, None);
            let mut t = t;
            t.resize(gl, width, height);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(color),
                0,
            );
            gl.framebuffer_renderbuffer(
                glow::FRAMEBUFFER,
                glow::DEPTH_STENCIL_ATTACHMENT,
                glow::RENDERBUFFER,
                Some(depth),
            );
            t
        }
    }

    fn resize(&mut self, gl: &glow::Context, width: u32, height: u32) {
        if (width, height) == (self.width, self.height) {
            return;
        }
        self.width = width;
        self.height = height;
        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(self.color));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA as i32,
                width as i32,
                height as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(self.depth));
            gl.renderbuffer_storage(
                glow::RENDERBUFFER,
                glow::DEPTH24_STENCIL8,
                width as i32,
                height as i32,
            );
            gl.bind_renderbuffer(glow::RENDERBUFFER, None);
        }
    }
}

/// The half-res target size for a main target (`ceil(size / divisor)`).
pub fn particle_pass_size(width: u32, height: u32) -> (u32, u32) {
    let d = PARTICLE_PASS_DIVISOR;
    (width.div_ceil(d).max(1), height.div_ceil(d).max(1))
}

/// Pass state kept on `Gfx`: the lazily created target and the sticky
/// "unsupported here" flag.
#[derive(Default)]
pub(crate) struct ParticlePassState {
    target: Option<ParticleTarget>,
    unsupported: Cell<bool>,
    /// Between a successful `begin_particle_pass` and its end.
    active: Cell<bool>,
}

impl Gfx {
    /// Start the half-res particle pass.  Returns `false` (and leaves the main
    /// target bound) when the pass is unsupported here; draw the particles at
    /// full resolution then.  On `true`, draw them, then call
    /// [`Gfx::end_particle_pass`].
    pub fn begin_particle_pass(&mut self) -> bool {
        if self.particle_pass.unsupported.get() {
            return false;
        }
        let (mw, mh) = self.main_target_size();
        let (w, h) = particle_pass_size(mw, mh);
        let main_fbo = self.render_target.as_ref().map(|rt| rt.fbo());
        let gl = &self.gl;
        // A multisampled main framebuffer cannot be scale-blitted.
        let samples = unsafe {
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, main_fbo);
            gl.get_parameter_i32(glow::SAMPLES)
        };
        if samples > 0 {
            log::info!("particle pass: main framebuffer has {samples} samples; full resolution");
            self.particle_pass.unsupported.set(true);
            self.bind_main_target();
            return false;
        }
        match self.particle_pass.target.as_mut() {
            Some(t) => t.resize(gl, w, h),
            None => self.particle_pass.target = Some(ParticleTarget::new(gl, w, h)),
        }
        let t = self.particle_pass.target.as_ref().expect("particle target");
        unsafe {
            // Isolate the blit's own error from earlier, unrelated ones.
            while gl.get_error() != glow::NO_ERROR {}
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, main_fbo);
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(t.fbo));
            gl.blit_framebuffer(
                0,
                0,
                mw as i32,
                mh as i32,
                0,
                0,
                w as i32,
                h as i32,
                glow::DEPTH_BUFFER_BIT,
                glow::NEAREST,
            );
            let err = gl.get_error();
            if err != glow::NO_ERROR {
                log::warn!("particle pass: depth blit failed (0x{err:04X}); full resolution");
                self.particle_pass.unsupported.set(true);
                self.bind_main_target();
                return false;
            }
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(t.fbo));
            gl.viewport(0, 0, w as i32, h as i32);
            gl.clear_color(0.0, 0.0, 0.0, 0.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            gl.clear_color(0.0, 0.0, 0.0, 1.0);
        }
        self.particle_pass.active.set(true);
        true
    }

    /// The size (texels) of the target particles currently draw into: the
    /// half-res target inside the pass, else the main target.
    pub(crate) fn particle_draw_size(&self) -> (u32, u32) {
        match self.particle_pass.target.as_ref() {
            Some(t) if self.particle_pass.active.get() => (t.width, t.height),
            _ => self.main_target_size(),
        }
    }

    /// End the half-res pass: rebind the main target and composite the
    /// particle colour over it (premultiplied "over", LINEAR upscale).
    pub fn end_particle_pass(&self) {
        self.particle_pass.active.set(false);
        self.bind_main_target();
        let Some(t) = self.particle_pass.target.as_ref() else { return };
        let gl = &self.gl;
        let s = self.shader("particleComposite");
        s.bind(gl);
        unsafe {
            gl.active_texture(glow::TEXTURE0 + COMPOSITE_UNIT);
            gl.bind_texture(glow::TEXTURE_2D, Some(t.color));
            gl.active_texture(glow::TEXTURE0);
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::STENCIL_TEST);
            gl.depth_mask(false);
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
        }
        s.uniform_1i(gl, "color_tex", COMPOSITE_UNIT as i32);
        crate::buffer::vertex_attrib_ptr_f32(gl, &self.quad.verts, s.attr("vertex_pos"), 3, 0, 0);
        self.quad.indices.bind(gl);
        unsafe {
            gl.draw_elements(
                glow::TRIANGLES,
                self.quad.index_count as i32,
                glow::UNSIGNED_SHORT,
                0,
            );
            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
            gl.depth_mask(true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pass_size_rounds_up_and_never_hits_zero() {
        assert_eq!(particle_pass_size(1280, 720), (640, 360));
        assert_eq!(particle_pass_size(1281, 721), (641, 361));
        assert_eq!(particle_pass_size(1, 1), (1, 1));
        assert_eq!(particle_pass_size(0, 0), (1, 1));
    }
}
