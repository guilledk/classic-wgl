//! # Skill: `classic-gfx`
//!
//! **Read `.agents/skills/classic-gfx/SKILL.md` before working on this module.**
//!
//! classic-gfx: OpenGL ES 3.0 / WebGL2 graphics layer.
//!
//! Shader compilation, texture upload, shared quad buffers, and draw-call
//! emitters for each `DrawKind` variant.  Pure `glow` — no windowing.

mod shaders;

mod compressed;

mod bloom;
mod buffer;
mod draw;
pub use draw::ModelDraws;
mod framebuffer;
mod light;
mod light_grid;
mod model_pass;
mod particle_pass;
mod particles;
pub mod perf;
mod shader;
mod shadow;
pub use shadow::ShadowModels;
mod sprite_batch;
pub use sprite_batch::{
    outline_delta, plan_batches, plan_shadow_batches, region_uv, BlendTextures, DirBlendRegion,
    GhostIdTarget, IsoSpriteInstance, PlannedBatch, ScreenRect, SpriteBatch, ThirdRegion,
    BLEND_THIRD_UNIT, BLEND_UNITS, GHOST_ALPHA, GHOST_ID_UNIT, PLAN_LOOKBACK,
    SPRITE_INSTANCE_ATTRS, SPRITE_INSTANCE_BYTES,
};
mod texture;

#[cfg(not(target_arch = "wasm32"))]
pub use compressed::{transcode_basis, Caps, DecodedBasis};

#[cfg(target_arch = "wasm32")]
mod basis_web;

use classic_core::components::Light;
use glam::Mat4;
use glow::HasContext;
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use buffer::build_quad;

pub use buffer::{DynamicVb, GlBuffer, ModelMeshGpu, QuadBuffers, MODEL_VERTEX_STRIDE};
pub use framebuffer::{DepthFramebuffer, GlFrameBuffer, ModelTarget};
pub use light::{
    pack_lights, LightBuffer, LIGHT_UBO_BINDING, MAX_LIGHTS, SHADOW_MAP_SIZE, SHADOW_MAP_UNIT,
    SHADOW_SPRITE_SLOPE_OFFSET, SHADOW_SPRITE_UNIT_OFFSET,
};
pub use light_grid::{
    clip_from_world, grid_map, LightGrid, LightGridView, LIGHT_GRID_UNIT, LIGHT_TILE_CAP,
    LIGHT_TILE_PX,
};
pub use model_pass::{
    emissive_shade, model_target_size, EmissiveRect, TeamTint, MAX_EMISSIVE, MODEL_GHOST_GROUP,
};
pub use particle_pass::{particle_pass_size, ParticleTarget, PARTICLE_PASS_DIVISOR};
pub use particles::{
    map_fade, particle_size_floor, ParticleGpu, ParticleLayerUniforms, ParticleRecord,
    ParticleRing, MAP_FADE_M, NO_MAP, PARTICLE_MIN_TEXELS, PARTICLE_RECORD_BYTES,
    RIBBON_VERTEX_FLOATS,
};
pub use shader::{builtin_shaders, BuiltinShader, Shader, ShaderSourceRegistry};
pub use texture::GlTexture;

// ---------------------------------------------------------------------------
// Gfx state
// ---------------------------------------------------------------------------

/// How a sprite's texture region is addressed: a uniform-grid frame index, or
/// a packed-atlas UV rect with trim/anchor metadata.  The packed (UV) form is
/// canonical; the grid form is the non-packed fallback.
pub enum SpriteRegion<'a> {
    /// Uniform-grid frame index + grid dimensions (non-packed fallback).
    Grid { frame: f32, tile_set_size: [f32; 2] },
    /// Packed-atlas UV rect `[u0, v0, u1, v1]` with trim/anchor metadata.
    Uv {
        uv_rect: &'a [f32; 4],
        trim_offset: &'a [f32; 2],
        source_size: &'a [f32; 2],
        content_size: &'a [f32; 2],
    },
}

/// Which pass of the two-phase isometric sprite draw to run.
///
/// The engine draws all [`IsoSpritePass::Normal`] sprites (terrain-occluded,
/// depth-writing) before all [`IsoSpritePass::Ghost`] sprites (40% alpha where
/// behind the depth buffer), so sprite-vs-sprite occlusion resolves via the
/// depth buffer rather than draw order.
pub enum IsoSpritePass {
    Normal,
    Ghost,
}

/// Shared lighting/projection settings passed to the lit draw calls (tilemap
/// and the lit sprite draws).
pub struct RenderSettings {
    pub ambient: [f32; 3],
    pub light_dir: [f32; 3],
    pub light_color: [f32; 3],
    /// Camera view-depth bounds `[near, far]` (metres) for the iso-depth
    /// normalisation `depth = (near - dot(back, world)) / (near - far)`.
    pub depth_span: [f32; 2],
    pub ppm: f32,
    /// Optional directional shadow map.  When `Some`, the lit shaders sample the
    /// depth texture and multiply the **sun diffuse** term by the shadow factor
    /// (ambient + point lights stay unshadowed).  When `None`, `use_shadow` is
    /// 0 and the term is byte-identical to the unshadowed path.
    pub shadow: Option<ShadowSettings>,
}

/// Texture unit the tilemap's material-blend lookup table binds to (units 0/1
/// are the tile data + tileset, 3 the shadow map).
pub const BLEND_LUT_UNIT: u32 = 2;

/// Material-blend uniforms for [`Gfx::draw_tilemap`] (the engine-side mirror
/// of `classic_core::components::TileBlend`).  `None` keeps the classic
/// one-id-per-tile path.  Lengths are in tiles.
#[derive(Clone, Copy)]
pub struct TileBlend {
    /// `256x1` RGBA8 class/priority/hard table
    /// (`classic_core::tilemap::build_blend_lut`), `NEAREST`.
    pub lut: glow::Texture,
    pub width: f32,
    /// Highest tileset mip level the explicit-LOD lookup may use (`0` when the
    /// tileset has no mip chain).
    pub max_lod: f32,
    pub noise_scale: f32,
    pub noise_strength: f32,
    pub height_strength: f32,
}

/// The directional shadow map consumed by the lit shaders.
#[derive(Clone, Copy)]
pub struct ShadowSettings {
    /// Depth texture sampled as a `sampler2D` (manual compare, PCF).
    pub texture: glow::Texture,
    /// `proj * view` mapping world space to light clip space.
    pub view_proj: Mat4,
    /// Depth bias (in light NDC units) added to the stored depth before compare.
    pub bias: f32,
    /// Diffuse fraction kept by a fully-shadowed pixel (`0..=1`); lit pixels
    /// keep `1.0`.  A value `> 0` stops shadows reading as black, so the cast
    /// shadow stays a subtle complement to the Lambertian self-shading.
    pub strength: f32,
    /// One shadow-map texel in UV space (`1 / SHADOW_MAP_SIZE`), for PCF.
    pub texel: [f32; 2],
    /// Distance to push the receiver along its surface normal before sampling,
    /// in world units (normal-offset bias).  Suppresses shadow acne without the
    /// peter-panning a comparable depth bias would cause.
    pub normal_offset: f32,
    /// `CLASSIC_SHADOW_DEBUG`: replace the shaded output with the raw shadow
    /// visibility factor (white = lit, black = occluded), bypassing albedo,
    /// ambient and point lights.  Diagnostic only.
    pub debug: bool,
}

pub struct Gfx {
    pub gl: Rc<glow::Context>,
    pub shaders: HashMap<String, Shader>,
    pub textures: HashMap<String, GlTexture>,
    pub quad: QuadBuffers,
    pub viewport_w: f32,
    pub viewport_h: f32,
    pub render_target: Option<GlFrameBuffer>,
    lights: LightBuffer,
    /// Screen-tile light lists for the lit fragment shaders (see `light_grid`).
    light_grid: light_grid::LightGridGpu,
    /// Size of the framebuffer the lit draws currently render into (the main
    /// target, or the model pass target), for the grid lookup.
    pass_target: Cell<(u32, u32)>,
    shadow_map: Option<DepthFramebuffer>,
    /// Bound in place of the shadow map on draws without one (see
    /// `framebuffer::shadow_placeholder`).
    shadow_placeholder: glow::Texture,
    /// The 3D-model pixelation target (see `model_pass`), created lazily.
    model_target: Option<ModelTarget>,
    /// Bloom post targets (see `bloom`), created lazily.
    bloom: Option<bloom::BloomTargets>,
    /// Baked tileable billow noise for procedural smoke (see `particles`).
    particle_noise: GlTexture,
    /// LINEAR + clamp sampler bound over particle textures (see `particles`).
    particle_sampler: glow::Sampler,
    /// Half-res particle pass (see `particle_pass`).
    particle_pass: particle_pass::ParticlePassState,
    /// Instanced iso sprites (see `sprite_batch`).
    sprite_batch: sprite_batch::SpriteBatchState,
    vao: glow::VertexArray,
}

impl Gfx {
    pub fn new(gl: Rc<glow::Context>) -> Self {
        let quad = build_quad(&gl);
        let vao = unsafe { gl.create_vertex_array() }.expect("create VAO");
        let lights = LightBuffer::new(&gl, MAX_LIGHTS);
        let light_grid = light_grid::LightGridGpu::new(&gl);
        let particle_noise = particles::noise_texture(&gl);
        let particle_sampler = particles::linear_sampler(&gl);
        let shadow_placeholder = framebuffer::shadow_placeholder(&gl);
        Self {
            gl,
            shaders: HashMap::new(),
            textures: HashMap::new(),
            quad,
            viewport_w: 1920.0,
            viewport_h: 1080.0,
            render_target: None,
            lights,
            light_grid,
            pass_target: Cell::new((1, 1)),
            shadow_map: None,
            shadow_placeholder,
            model_target: None,
            bloom: None,
            particle_noise,
            particle_sampler,
            particle_pass: Default::default(),
            sprite_batch: Default::default(),
            vao,
        }
    }

    /// Upload the active dynamic lights into the `LightBlock` UBO and bind it to
    /// [`LIGHT_UBO_BINDING`] (consumed by the lit shaders).  Turns the light
    /// grid off until [`Gfx::upload_light_grid`] bins these lights, so a stale
    /// grid never indexes the new block.
    pub fn upload_lights(&mut self, lights: &[Light]) {
        self.lights.upload(&self.gl, lights);
        self.light_grid.invalidate();
    }

    /// Bin `lights` (the slice just passed to [`Gfx::upload_lights`]) into the
    /// screen-tile light grid for this frame's camera, so the lit fragment
    /// shaders loop over the lights near their tile only.  `camera`,
    /// `world_matrix` and `ppm` are the values the lit draws receive.  The grid
    /// stays off (full loop) without lights or for a non-orthographic camera.
    pub fn upload_light_grid(
        &mut self,
        lights: &[Light],
        camera: &Mat4,
        world_matrix: &Mat4,
        ppm: f32,
    ) {
        let main = self.main_target_size();
        let clip = clip_from_world(&self.projection(), camera, world_matrix, ppm);
        let view = if lights.is_empty() { None } else { LightGridView::new(&clip, main.0, main.1) };
        let grid = view.map(|v| LightGrid::build(lights, &v, self.light_grid.max_words()));
        if let (true, Some(g)) = (crate::perf::enabled(), grid.as_ref()) {
            g.count_stats();
        }
        self.light_grid.upload(&self.gl, grid.as_ref().map(|g| (g, main, clip)));
    }

    /// Create and set an offscreen render target of the given size.
    pub fn set_render_target(&mut self, width: u32, height: u32) {
        let rt = GlFrameBuffer::new(&self.gl, width, height, true);
        self.render_target = Some(rt);
    }

    /// Remove the offscreen render target (back to default framebuffer).
    pub fn clear_render_target(&mut self) {
        self.render_target = None;
    }

    /// Build the orthographic projection matrix.
    pub fn projection(&self) -> Mat4 {
        Mat4::orthographic_rh(0.0, self.viewport_w, self.viewport_h, 0.0, -10000.0, 10000.0)
    }

    /// Resize the viewport (called on window/canvas resize).
    pub fn resize(&mut self, w: f32, h: f32) {
        self.viewport_w = w;
        self.viewport_h = h;
    }

    // -- resource management -----------------------------------------------

    /// Compile and store a shader from a declaration (builtin or override).
    pub fn add_shader(
        &mut self,
        name: &str,
        vs_src: &str,
        fs_src: &str,
        attr: &[&str],
        unif: &[&str],
    ) -> Result<(), String> {
        let s = Shader::compile(&self.gl, vs_src, fs_src, attr, unif)?;
        // Bind the light UBO for any shader that declares `LightBlock` (the two
        // lit shaders); a no-op for every other program.
        s.bind_uniform_block(&self.gl, "LightBlock", LIGHT_UBO_BINDING);
        // Point the light-grid `usampler2D` (lit fragment shaders) at its own
        // unit once: left on unit 0 it would alias a float sampler, which WebGL
        // rejects at draw time.  A no-op for programs without it.
        s.bind(&self.gl);
        s.uniform_1i(&self.gl, "light_grid", LIGHT_GRID_UNIT as i32);
        self.shaders.insert(name.to_string(), s);
        Ok(())
    }

    pub fn add_texture_rgba8(&mut self, name: &str, rgba: &[u8], w: u32, h: u32) {
        self.textures.insert(name.to_string(), GlTexture::from_rgba8(&self.gl, rgba, w, h));
    }

    pub fn add_texture_r8(&mut self, name: &str, r: &[u8], w: u32, h: u32) {
        self.textures.insert(name.to_string(), GlTexture::from_r8(&self.gl, r, w, h));
    }

    pub fn add_texture_rgb8(&mut self, name: &str, rgb: &[u8], w: u32, h: u32) {
        self.textures.insert(name.to_string(), GlTexture::from_rgb8(&self.gl, rgb, w, h));
    }

    /// Upload a GPU-compressed texture from raw block data (see
    /// [`GlTexture::from_compressed`]).
    pub fn add_texture_compressed(
        &mut self,
        name: &str,
        internal_format: u32,
        data: &[u8],
        w: u32,
        h: u32,
    ) {
        self.textures.insert(
            name.to_string(),
            GlTexture::from_compressed(&self.gl, internal_format, data, w, h),
        );
    }

    /// Upload a previously-transcoded `.basis` payload (the off-thread half of
    /// [`Gfx::add_texture_basis`], native only).  Compressed block data goes
    /// through `compressed_tex_image_2d`; the RGBA8 fallback goes through the
    /// raw path.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn upload_decoded_basis(&mut self, name: &str, decoded: &compressed::DecodedBasis) {
        match decoded {
            compressed::DecodedBasis::Compressed { internal_format, width, height, data } => {
                self.add_texture_compressed(name, *internal_format, data, *width, *height);
            }
            compressed::DecodedBasis::Rgba8 { width, height, data } => {
                self.add_texture_rgba8(name, data, *width, *height);
            }
        }
    }

    /// Upload a Basis Universal `.basis` payload: transcode to the target
    /// [`compressed::CompressedFormat`] and upload compressed, or fall back to a
    /// raw RGBA8 transcode.  Returns `false` (and uploads nothing) when the
    /// payload can't be transcoded — the caller treats the texture as missing.
    pub fn add_texture_basis(&mut self, name: &str, bytes: &[u8], format: &str) -> bool {
        if let Some(fmt) = compressed::CompressedFormat::parse(format) {
            if let Some(decoded) = compressed::transcode(&self.gl, bytes, fmt) {
                log::debug!(
                    "texture {name}: basis -> gl 0x{:04X} ({}/{})",
                    decoded.internal_format,
                    decoded.width,
                    decoded.height
                );
                self.add_texture_compressed(
                    name,
                    decoded.internal_format,
                    &decoded.data,
                    decoded.width,
                    decoded.height,
                );
                return true;
            }
        }
        if let Some((w, h, rgba)) = compressed::transcode_rgba8(bytes) {
            log::debug!("texture {name}: basis -> RGBA8 fallback ({w}/{h})");
            self.add_texture_rgba8(name, &rgba, w, h);
            return true;
        }
        false
    }

    /// Web-only async counterpart to [`Gfx::add_texture_basis`]: transcode in
    /// the dedicated worker (awaited here) and upload on the main thread, with
    /// a synchronous main-thread fallback when the worker cannot start.  Returns
    /// the decoded texture dimensions on success (`None` when the payload can't
    /// be transcoded).
    #[cfg(target_arch = "wasm32")]
    pub async fn add_texture_basis_async(
        &mut self,
        name: &str,
        bytes: &[u8],
        format: &str,
    ) -> Option<(u32, u32)> {
        let gl = self.gl.clone();
        if let Some(fmt) = compressed::CompressedFormat::parse(format) {
            if let Some(decoded) = compressed::transcode_async(&gl, bytes, fmt).await {
                log::debug!(
                    "texture {name}: basis -> gl 0x{:04X} ({}/{})",
                    decoded.internal_format,
                    decoded.width,
                    decoded.height
                );
                let dims = (decoded.width, decoded.height);
                self.add_texture_compressed(
                    name,
                    decoded.internal_format,
                    &decoded.data,
                    decoded.width,
                    decoded.height,
                );
                return Some(dims);
            }
        }
        if let Some((w, h, rgba)) = compressed::transcode_rgba8_async(bytes).await {
            log::debug!("texture {name}: basis -> RGBA8 fallback ({w}/{h})");
            self.add_texture_rgba8(name, &rgba, w, h);
            return Some((w, h));
        }
        None
    }

    pub fn shader(&self, name: &str) -> &Shader {
        self.shaders.get(name).unwrap_or_else(|| panic!("shader '{name}' not found"))
    }

    pub fn texture(&self, name: &str) -> &GlTexture {
        self.textures.get(name).unwrap_or_else(|| panic!("texture '{name}' not found"))
    }

    // -- frame begin -------------------------------------------------------

    /// Clear the framebuffer and set up state for the current frame.
    pub fn begin_frame(&self) {
        let gl = &self.gl;
        self.pass_target.set(self.main_target_size());
        unsafe {
            // Flush any accumulated GL errors before starting a new frame.
            while gl.get_error() != 0 {}
            if let Some(ref rt) = self.render_target {
                rt.bind(gl);
                gl.viewport(0, 0, rt.width as i32, rt.height as i32);
            } else {
                gl.bind_framebuffer(glow::FRAMEBUFFER, None);
                gl.viewport(0, 0, self.viewport_w as i32, self.viewport_h as i32);
            }
            gl.bind_vertex_array(Some(self.vao));
            gl.clear_color(0.0, 0.0, 0.0, 1.0);
            // `glClear` respects the stencil write mask, and the ghost pass
            // leaves it at 0x00 — reset it so the stencil buffer actually
            // clears every frame (stale ghost-group ids otherwise suppress the
            // ghost pass as the camera pans).
            gl.stencil_mask(0xFF);
            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT | glow::STENCIL_BUFFER_BIT);
            gl.enable(glow::BLEND);
            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
            gl.depth_func(glow::LEQUAL);
            gl.depth_mask(true);
            gl.disable(glow::SCISSOR_TEST);
            gl.disable(glow::STENCIL_TEST);
        }
    }
}
