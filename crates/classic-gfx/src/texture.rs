//! GPU textures (`GlTexture`).

use glow::HasContext;

// ---------------------------------------------------------------------------
// Texture
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct GlTexture {
    texture: glow::Texture,
    /// Image pixel dimensions.
    pub size: (u32, u32),
}

impl GlTexture {
    /// Upload pixel data to a new 2D texture with the given internal format,
    /// data format, and unpack alignment (1 for R8/RGB8, 4 for RGBA8).
    fn upload(
        gl: &glow::Context,
        internal: u32,
        format: u32,
        data: &[u8],
        width: u32,
        height: u32,
        alignment: i32,
    ) -> Self {
        let texture = unsafe { gl.create_texture() }.expect("create texture");
        unsafe {
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, alignment);
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                internal as i32,
                width as i32,
                height as i32,
                0,
                format,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(data)),
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
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
            gl.bind_texture(glow::TEXTURE_2D, None);
        }
        Self { texture, size: (width, height) }
    }

    /// Upload RGBA8 pixel data to a new 2D texture.
    pub fn from_rgba8(gl: &glow::Context, rgba: &[u8], width: u32, height: u32) -> Self {
        Self::upload(gl, glow::RGBA, glow::RGBA, rgba, width, height, 4)
    }

    /// Upload R8 (single-channel) pixel data to a new 2D texture.  Used for
    /// grayscale depth maps and the SDF font atlas (sampled as `.r`).
    pub fn from_r8(gl: &glow::Context, r: &[u8], width: u32, height: u32) -> Self {
        Self::upload(gl, glow::R8, glow::RED, r, width, height, 1)
    }

    /// Upload RGB8 pixel data to a new 2D texture.  Used for world-space normal
    /// maps (sampled as `.rgb`).
    pub fn from_rgb8(gl: &glow::Context, rgb: &[u8], width: u32, height: u32) -> Self {
        Self::upload(gl, glow::RGB8, glow::RGB, rgb, width, height, 1)
    }

    /// Upload GPU-compressed block data (e.g. BC7/BC4, from a transcoded Basis
    /// `.basis` payload) via `compressed_tex_image_2d`.  `width`/`height` are
    /// the texture's texel dimensions (the block data is implicitly padded).
    pub fn from_compressed(
        gl: &glow::Context,
        internal_format: u32,
        data: &[u8],
        width: u32,
        height: u32,
    ) -> Self {
        let texture = unsafe { gl.create_texture() }.expect("create texture");
        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.compressed_tex_image_2d(
                glow::TEXTURE_2D,
                0,
                internal_format as i32,
                width as i32,
                height as i32,
                0,
                data.len() as i32,
                data,
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
        }
        Self { texture, size: (width, height) }
    }

    /// Set LINEAR filtering (used for SDF atlas).
    pub fn set_linear(&self, gl: &glow::Context) {
        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
            gl.bind_texture(glow::TEXTURE_2D, None);
        }
    }

    /// Bind to a texture unit (0 = TEXTURE0, etc.).
    pub fn bind(&self, gl: &glow::Context, unit: u32) {
        unsafe {
            gl.active_texture(glow::TEXTURE0 + unit);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
        }
    }
}
