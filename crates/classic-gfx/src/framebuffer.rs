//! Offscreen framebuffers: the colour render target and the depth-only shadow map.

use glow::HasContext;

/// A depth-only framebuffer: a `DEPTH_COMPONENT24` texture attached to
/// `DEPTH_ATTACHMENT` with no color attachment (`draw_buffers([NONE])`).  Used
/// for the directional shadow map; the depth texture is sampled as a
/// `sampler2D` in the lit shaders (manual `step` compare, no PCF).
pub struct DepthFramebuffer {
    pub(crate) fbo: glow::Framebuffer,
    pub(crate) depth_tex: glow::Texture,
    pub width: u32,
    pub height: u32,
}

impl DepthFramebuffer {
    pub fn new(gl: &glow::Context, width: u32, height: u32) -> Self {
        let fbo = unsafe { gl.create_framebuffer() }.expect("create fbo");
        let depth_tex = unsafe { gl.create_texture() }.expect("create texture");
        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(depth_tex));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::DEPTH_COMPONENT24 as i32,
                width as i32,
                height as i32,
                0,
                glow::DEPTH_COMPONENT,
                glow::UNSIGNED_INT,
                glow::PixelUnpackData::Slice(None),
            );
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::NEAREST as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::NEAREST as i32);
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.bind_texture(glow::TEXTURE_2D, None);

            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::DEPTH_ATTACHMENT,
                glow::TEXTURE_2D,
                Some(depth_tex),
                0,
            );
            // No color attachment: mask color writes so the FBO is complete.
            gl.draw_buffers(&[glow::NONE]);
            gl.read_buffer(glow::NONE);
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        }
        Self { fbo, depth_tex, width, height }
    }

    pub fn bind(&self, gl: &glow::Context) {
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
        }
    }
}

pub struct GlFrameBuffer {
    fbo: glow::Framebuffer,
    depth_rb: Option<glow::Renderbuffer>,
    pub texture: glow::Texture,
    pub width: u32,
    pub height: u32,
}

impl GlFrameBuffer {
    /// Create an RGBA framebuffer with an attached color texture and optional depth renderbuffer.
    pub fn new(gl: &glow::Context, width: u32, height: u32, with_depth: bool) -> Self {
        let fbo = unsafe { gl.create_framebuffer() }.expect("create fbo");
        let tex = unsafe { gl.create_texture() }.expect("create texture");
        let depth_rb = if with_depth {
            let rb = unsafe { gl.create_renderbuffer() }.expect("create renderbuffer");
            unsafe {
                gl.bind_renderbuffer(glow::RENDERBUFFER, Some(rb));
                gl.renderbuffer_storage(
                    glow::RENDERBUFFER,
                    glow::DEPTH24_STENCIL8,
                    width as i32,
                    height as i32,
                );
                gl.bind_renderbuffer(glow::RENDERBUFFER, None);
            }
            Some(rb)
        } else {
            None
        };

        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
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
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::NEAREST as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::NEAREST as i32);
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );

            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(tex),
                0,
            );
            if let Some(rb) = depth_rb {
                gl.framebuffer_renderbuffer(
                    glow::FRAMEBUFFER,
                    glow::DEPTH_STENCIL_ATTACHMENT,
                    glow::RENDERBUFFER,
                    Some(rb),
                );
            }
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.bind_texture(glow::TEXTURE_2D, None);
        }

        Self { fbo, depth_rb, texture: tex, width, height }
    }

    pub fn bind(&self, gl: &glow::Context) {
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
        }
    }

    pub fn unbind(gl: &glow::Context) {
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        }
    }

    /// Clear the framebuffer with the given color.
    pub fn clear(&self, gl: &glow::Context, rgba: &[f32; 4]) {
        unsafe {
            gl.clear_color(rgba[0], rgba[1], rgba[2], rgba[3]);
            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
        }
    }

    /// Resize the framebuffer and its attachments.
    pub fn resize(&mut self, gl: &glow::Context, width: u32, height: u32) {
        if width == self.width && height == self.height {
            return;
        }
        self.width = width;
        self.height = height;

        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
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

            if let Some(rb) = self.depth_rb {
                gl.bind_renderbuffer(glow::RENDERBUFFER, Some(rb));
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

    /// Read RGBA pixels from the framebuffer. Caller must bind this FBO first.
    pub fn read_pixels_rgba(&self, gl: &glow::Context) -> Vec<u8> {
        let len = (self.width * self.height * 4) as usize;
        let mut pixels = vec![0u8; len];
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
            gl.read_pixels(
                0,
                0,
                self.width as i32,
                self.height as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut pixels)),
            );
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        }
        pixels
    }
}

impl Drop for GlFrameBuffer {
    fn drop(&mut self) {
        // Resources are leaked intentionally — this struct lives for the
        // process lifetime and Drop can't access the GL context.
    }
}

/// The 3D-model pixelation target: an RGBA8 colour texture + a
/// `DEPTH_COMPONENT24` depth texture, both `NEAREST`-sampled so the composite
/// pass (`model_composite`) can upscale colour **and** depth block-for-block
/// and write the depth back with `gl_FragDepth` (WebGL2 cannot scale-blit a
/// depth buffer).
pub struct ModelTarget {
    fbo: glow::Framebuffer,
    pub(crate) color_tex: glow::Texture,
    pub(crate) depth_tex: glow::Texture,
    pub width: u32,
    pub height: u32,
}

impl ModelTarget {
    pub fn new(gl: &glow::Context, width: u32, height: u32) -> Self {
        let fbo = unsafe { gl.create_framebuffer() }.expect("create fbo");
        let color_tex = unsafe { gl.create_texture() }.expect("create texture");
        let depth_tex = unsafe { gl.create_texture() }.expect("create texture");
        let target = Self { fbo, color_tex, depth_tex, width, height };
        target.allocate(gl);
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(color_tex),
                0,
            );
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::DEPTH_ATTACHMENT,
                glow::TEXTURE_2D,
                Some(depth_tex),
                0,
            );
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        }
        target
    }

    /// (Re)allocate both textures at the current `width` x `height`.
    fn allocate(&self, gl: &glow::Context) {
        let (w, h) = (self.width as i32, self.height as i32);
        unsafe {
            for (tex, internal, format, ty) in [
                (self.color_tex, glow::RGBA8, glow::RGBA, glow::UNSIGNED_BYTE),
                (
                    self.depth_tex,
                    glow::DEPTH_COMPONENT24,
                    glow::DEPTH_COMPONENT,
                    glow::UNSIGNED_INT,
                ),
            ] {
                gl.bind_texture(glow::TEXTURE_2D, Some(tex));
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    internal as i32,
                    w,
                    h,
                    0,
                    format,
                    ty,
                    glow::PixelUnpackData::Slice(None),
                );
                for (param, value) in [
                    (glow::TEXTURE_MIN_FILTER, glow::NEAREST),
                    (glow::TEXTURE_MAG_FILTER, glow::NEAREST),
                    (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                    (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
                ] {
                    gl.tex_parameter_i32(glow::TEXTURE_2D, param, value as i32);
                }
            }
            gl.bind_texture(glow::TEXTURE_2D, None);
        }
    }

    /// Resize both attachments (no-op when unchanged).
    pub fn resize(&mut self, gl: &glow::Context, width: u32, height: u32) {
        if width == self.width && height == self.height {
            return;
        }
        self.width = width;
        self.height = height;
        self.allocate(gl);
    }

    pub fn bind(&self, gl: &glow::Context) {
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
        }
    }
}
