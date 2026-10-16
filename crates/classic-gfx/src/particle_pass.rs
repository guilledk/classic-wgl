//! The particle pass: particle layers draw into an offscreen target whose
//! resolution follows the camera zoom, then composite back onto the scene.
//!
//! - **Zoomed in** (`zoom > 1`, see [`crate::pixelates`]): the target is the
//!   model pixelation target's size ([`crate::model_target_size`],
//!   `viewport × 1/zoom`) and composites `NEAREST` over the same fullscreen
//!   quad, so a particle texel is a sprite texel is a model texel and the
//!   particle blocks sit on exactly the model pass's pixel grid.  Ribbons
//!   (arcs, lightning) draw into it too.
//! - **Zoomed out** (`zoom ≤ 1`, a sprite texel is at most one screen pixel,
//!   so blocks cannot read): a soft half-resolution target, upscaled
//!   `LINEAR`.  Particles are soft and overlap heavily, so their fill cost
//!   dominates a big effect (a rocket's ground cloud); half resolution
//!   quarters the shaded pixels.  Ribbons stay full-res outside it.
//!
//! A pixelated target holds `1/zoom²` of the screen's pixels, so it shades
//! as many pixels per world area as full resolution at zoom 1, and no more
//! than the half-res target from zoom 2 up.
//!
//! [`Gfx::begin_particle_pass`] blits the scene depth (NEAREST, scaled) into
//! the target's `DEPTH24_STENCIL8` renderbuffer, so particles still
//! depth-test against terrain, models and depth-mapped sprites, then clears
//! its colour to transparent.  Particles draw with their usual premultiplied
//! blend (`ONE, ONE_MINUS_SRC_ALPHA`), so the target holds premultiplied
//! colour + coverage, and [`Gfx::end_particle_pass`] composites it back with
//! the same blend — exact "over" compositing.
//!
//! The depth blit needs a single-sample main framebuffer with a matching
//! depth format.  A multisampled main target (a browser canvas with
//! `antialias`) or a blit GL error switches the pass off for good, and
//! particles draw at full resolution as before.

use std::cell::Cell;

use glow::HasContext;

use crate::model_pass::{model_target_size, pixelates};
use crate::Gfx;

/// Divisor of the main target size when zoomed out (2 = half resolution per
/// axis).
pub const PARTICLE_PASS_DIVISOR: u32 = 2;

/// Texture unit the composite samples (clear of every scene binding).
const COMPOSITE_UNIT: u32 = 10;

/// How the particle pass renders this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticlePass {
    /// No offscreen pass (unsupported here): draw at full resolution.
    Off,
    /// Soft half-res target, `LINEAR` upscale (zoomed out).
    Soft,
    /// Model-grid target, `NEAREST` composite (zoomed in): sprite-sized
    /// blocks.  Ribbons draw into it as well.
    Pixelated,
}

/// The particle target for a main target of `width` x `height` at camera
/// `zoom`: `(width, height, pixelated)`.  Zoomed in it is exactly the model
/// pixelation target ([`model_target_size`]); otherwise
/// `ceil(size / PARTICLE_PASS_DIVISOR)`.  Never 0 px.
pub fn particle_pass_target(width: u32, height: u32, zoom: f32) -> (u32, u32, bool) {
    if pixelates(zoom) {
        let (w, h) = model_target_size(width, height, zoom);
        (w, h, true)
    } else {
        let d = PARTICLE_PASS_DIVISOR;
        (width.div_ceil(d).max(1), height.div_ceil(d).max(1), false)
    }
}

/// The particle target: colour texture (`LINEAR` or `NEAREST` per pass) +
/// depth/stencil renderbuffer (the depth blit's destination).
pub struct ParticleTarget {
    fbo: glow::Framebuffer,
    color: glow::Texture,
    depth: glow::Renderbuffer,
    filter: u32,
    pub width: u32,
    pub height: u32,
}

impl ParticleTarget {
    fn new(gl: &glow::Context, width: u32, height: u32) -> Self {
        unsafe {
            let fbo = gl.create_framebuffer().expect("particle fbo");
            let color = gl.create_texture().expect("particle color");
            let depth = gl.create_renderbuffer().expect("particle depth");
            let mut t = Self { fbo, color, depth, filter: 0, width: 0, height: 0 };
            gl.bind_texture(glow::TEXTURE_2D, Some(color));
            for p in [glow::TEXTURE_WRAP_S, glow::TEXTURE_WRAP_T] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, p, glow::CLAMP_TO_EDGE as i32);
            }
            gl.bind_texture(glow::TEXTURE_2D, None);
            t.set_filter(gl, glow::LINEAR);
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

    /// Set the composite's sampling filter (`LINEAR` / `NEAREST`).
    fn set_filter(&mut self, gl: &glow::Context, filter: u32) {
        if filter == self.filter {
            return;
        }
        self.filter = filter;
        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(self.color));
            for p in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, p, filter as i32);
            }
            gl.bind_texture(glow::TEXTURE_2D, None);
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

/// Pass state kept on `Gfx`: the lazily created target and the sticky
/// "unsupported here" flag.
#[derive(Default)]
pub(crate) struct ParticlePassState {
    target: Option<ParticleTarget>,
    unsupported: Cell<bool>,
}

impl Gfx {
    /// Start the particle pass for camera `zoom`.  Returns
    /// [`ParticlePass::Off`] (and leaves the main target bound) when the pass
    /// is unsupported here; draw the particles at full resolution then.
    /// Otherwise draw them (and, when [`ParticlePass::Pixelated`], the
    /// ribbons), then call [`Gfx::end_particle_pass`].
    pub fn begin_particle_pass(&mut self, zoom: f32) -> ParticlePass {
        if self.particle_pass.unsupported.get() {
            return ParticlePass::Off;
        }
        let (mw, mh) = self.main_target_size();
        let (w, h, pixelated) = particle_pass_target(mw, mh, zoom);
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
            return ParticlePass::Off;
        }
        let t = self.particle_pass.target.get_or_insert_with(|| ParticleTarget::new(gl, w, h));
        t.resize(gl, w, h);
        t.set_filter(gl, if pixelated { glow::NEAREST } else { glow::LINEAR });
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
                return ParticlePass::Off;
            }
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(t.fbo));
            gl.viewport(0, 0, w as i32, h as i32);
            gl.clear_color(0.0, 0.0, 0.0, 0.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            gl.clear_color(0.0, 0.0, 0.0, 1.0);
        }
        if pixelated {
            ParticlePass::Pixelated
        } else {
            ParticlePass::Soft
        }
    }

    /// End the particle pass: rebind the main target and composite the
    /// particle colour over it (premultiplied "over"; the target's filter
    /// makes it soft or blocky).
    pub fn end_particle_pass(&self) {
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
    fn zoomed_out_is_soft_half_res_and_never_hits_zero() {
        for zoom in [0.1, 0.32, 0.5, 1.0] {
            assert_eq!(particle_pass_target(1280, 720, zoom), (640, 360, false), "zoom {zoom}");
        }
        assert_eq!(particle_pass_target(1281, 721, 0.32), (641, 361, false));
        assert_eq!(particle_pass_target(1, 1, 1.0), (1, 1, false));
        assert_eq!(particle_pass_target(0, 0, 1.0), (1, 1, false));
        // Degenerate zooms never pixelate.
        assert_eq!(particle_pass_target(1280, 720, 0.0), (640, 360, false));
        assert_eq!(particle_pass_target(1280, 720, f32::NAN), (640, 360, false));
        assert_eq!(particle_pass_target(1280, 720, f32::INFINITY), (640, 360, false));
    }

    #[test]
    fn zoomed_in_shares_the_model_pixel_grid() {
        // Every zoom > 1 pixelates on exactly the model target, so a plume's
        // blocks line up with the rocket mesh's (same size, same fullscreen
        // mapping) and with the sprite texel size.
        for zoom in [1.01, 1.5, 2.0, 2.5, 3.0, 4.0, 7.3] {
            let (w, h) = model_target_size(1280, 720, zoom);
            assert_eq!(particle_pass_target(1280, 720, zoom), (w, h, true), "zoom {zoom}");
        }
        assert_eq!(particle_pass_target(1280, 720, 2.0), (640, 360, true));
        assert_eq!(particle_pass_target(1280, 720, 3.0), (427, 240, true));
        assert_eq!(particle_pass_target(1, 1, 1000.0), (1, 1, true));
    }

    #[test]
    fn pixelated_target_never_costs_more_than_zoom_one_full_res() {
        // Fill per world area: target px / (screen px × zoom²) — at most the
        // zoom-1 full-res cost, and at most the half-res cost from zoom 2 up.
        let (vw, vh) = (1280u32, 720u32);
        for zoom in [1.01f32, 1.5, 2.0, 3.0, 4.0] {
            let (w, h, _) = particle_pass_target(vw, vh, zoom);
            let per_world = (w * h) as f32 * zoom * zoom / (vw * vh) as f32;
            assert!(per_world <= 1.02, "zoom {zoom}: {per_world}");
            if zoom >= 2.0 {
                assert!((w * h) as f32 <= (vw * vh) as f32 / 4.0 + vw as f32, "zoom {zoom}");
            }
        }
    }
}
