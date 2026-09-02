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

#[cfg(target_arch = "wasm32")]
mod basis_web;

use classic_core::components::Light;
use glam::{Mat3, Mat4, Vec3};
use glow::HasContext;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

// ---------------------------------------------------------------------------
// Shader
// ---------------------------------------------------------------------------

pub struct Shader {
    program: glow::Program,
    attr: HashMap<String, u32>,
    unif: RefCell<HashMap<String, glow::UniformLocation>>,
}

impl Shader {
    /// Compile a shader program from GLSL 300 es source strings.
    ///
    /// `attr_names` are bound to consecutive attribute locations
    /// (index = location).
    pub fn compile(
        gl: &glow::Context,
        vs_src: &str,
        fs_src: &str,
        attr_names: &[&str],
        unif_names: &[&str],
    ) -> Result<Self, String> {
        let vs = compile_single(gl, glow::VERTEX_SHADER, vs_src)?;
        let fs = compile_single(gl, glow::FRAGMENT_SHADER, fs_src)?;

        let program = unsafe {
            let p = gl.create_program().map_err(|_| "failed to create program")?;
            gl.attach_shader(p, vs);
            gl.attach_shader(p, fs);

            // Bind attribute locations by manifest index (attribute arrays
            // are positional).
            for (i, name) in attr_names.iter().enumerate() {
                gl.bind_attrib_location(p, i as u32, name);
            }

            gl.link_program(p);

            if !gl.get_program_link_status(p) {
                let log = gl.get_program_info_log(p);
                gl.delete_program(p);
                gl.delete_shader(vs);
                gl.delete_shader(fs);
                return Err(format!("link failed: {log}"));
            }

            gl.delete_shader(vs);
            gl.delete_shader(fs);

            p
        };

        let mut attr = HashMap::new();
        for &name in attr_names {
            if let Some(loc) = unsafe { gl.get_attrib_location(program, name) } {
                attr.insert(name.to_string(), loc);
            }
        }

        let mut unif = HashMap::new();
        for &name in unif_names {
            if let Some(loc) = unsafe { gl.get_uniform_location(program, name) } {
                unif.insert(name.to_string(), loc);
            }
        }

        Ok(Self { program, attr, unif: RefCell::new(unif) })
    }

    pub fn bind(&self, gl: &glow::Context) {
        unsafe { gl.use_program(Some(self.program)) }
    }

    pub fn attr(&self, name: &str) -> u32 {
        *self.attr.get(name).unwrap_or_else(|| panic!("attribute '{name}' not found"))
    }

    /// Resolve a uniform location, consulting the compile-time cache first and
    /// lazily querying the linked program on a miss.  Lazy resolution lets a
    /// shader declare uniforms beyond those listed in the manifest's `unif`
    /// array (e.g. the packed-atlas `uv_rect`) without a manifest bump.
    // `UniformLocation` is `Copy` on native GL but `Clone`-only on WebGL, so we
    // clone defensively (cheap either way).
    #[allow(clippy::clone_on_copy)]
    fn unif(&self, gl: &glow::Context, name: &str) -> Option<glow::UniformLocation> {
        if let Some(loc) = self.unif.borrow().get(name) {
            return Some(loc.clone());
        }
        let loc = unsafe { gl.get_uniform_location(self.program, name) }?;
        self.unif.borrow_mut().insert(name.to_string(), loc.clone());
        Some(loc)
    }

    // -- uniform setters ---------------------------------------------------

    pub fn uniform_mat4(&self, gl: &glow::Context, name: &str, m: &Mat4) {
        if let Some(loc) = self.unif(gl, name) {
            unsafe {
                gl.uniform_matrix_4_f32_slice(Some(&loc), false, m.as_ref());
            }
        }
    }

    pub fn uniform_mat3(&self, gl: &glow::Context, name: &str, m: &Mat3) {
        if let Some(loc) = self.unif(gl, name) {
            unsafe {
                gl.uniform_matrix_3_f32_slice(Some(&loc), false, m.as_ref());
            }
        }
    }

    pub fn uniform_vec4(&self, gl: &glow::Context, name: &str, v: &[f32; 4]) {
        if let Some(loc) = self.unif(gl, name) {
            unsafe { gl.uniform_4_f32(Some(&loc), v[0], v[1], v[2], v[3]) }
        }
    }

    pub fn uniform_vec3(&self, gl: &glow::Context, name: &str, v: Vec3) {
        if let Some(loc) = self.unif(gl, name) {
            unsafe { gl.uniform_3_f32(Some(&loc), v[0], v[1], v[2]) }
        }
    }

    pub fn uniform_vec2(&self, gl: &glow::Context, name: &str, v: &[f32; 2]) {
        if let Some(loc) = self.unif(gl, name) {
            unsafe { gl.uniform_2_f32(Some(&loc), v[0], v[1]) }
        }
    }

    pub fn uniform_1f(&self, gl: &glow::Context, name: &str, v: f32) {
        if let Some(loc) = self.unif(gl, name) {
            unsafe { gl.uniform_1_f32(Some(&loc), v) }
        }
    }

    pub fn uniform_1i(&self, gl: &glow::Context, name: &str, v: i32) {
        if let Some(loc) = self.unif(gl, name) {
            unsafe { gl.uniform_1_i32(Some(&loc), v) }
        }
    }

    pub fn uniform_bool(&self, gl: &glow::Context, name: &str, v: bool) {
        if let Some(loc) = self.unif(gl, name) {
            unsafe { gl.uniform_1_i32(Some(&loc), v as i32) }
        }
    }

    /// Bind a named `std140` uniform block to a UBO binding point.  A no-op for
    /// programs that don't declare the block (the index query returns `None`).
    pub fn bind_uniform_block(&self, gl: &glow::Context, name: &str, binding: u32) {
        if let Some(idx) = unsafe { gl.get_uniform_block_index(self.program, name) } {
            unsafe { gl.uniform_block_binding(self.program, idx, binding) };
        }
    }
}

fn compile_single(gl: &glow::Context, ty: u32, src: &str) -> Result<glow::Shader, String> {
    let shader = unsafe { gl.create_shader(ty) }.map_err(|_| "failed to create shader")?;
    unsafe {
        gl.shader_source(shader, src);
        gl.compile_shader(shader);
    }
    let ok = unsafe { gl.get_shader_compile_status(shader) };
    if !ok {
        let log = unsafe { gl.get_shader_info_log(shader) };
        unsafe { gl.delete_shader(shader) };
        return Err(log);
    }
    Ok(shader)
}

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

// ---------------------------------------------------------------------------
// Buffer
// ---------------------------------------------------------------------------

pub struct GlBuffer {
    buffer: glow::Buffer,
    target: u32,
    #[allow(dead_code)]
    count: usize,
}

impl GlBuffer {
    pub fn from_slice<T: bytemuck::Pod>(
        gl: &glow::Context,
        target: u32,
        data: &[T],
        usage: u32,
    ) -> Self {
        let buffer = unsafe { gl.create_buffer() }.expect("create buffer");
        let bytes: &[u8] = bytemuck::cast_slice(data);
        unsafe {
            gl.bind_buffer(target, Some(buffer));
            gl.buffer_data_u8_slice(target, bytes, usage);
        }
        Self { buffer, target, count: data.len() }
    }

    pub fn bind(&self, gl: &glow::Context) {
        unsafe { gl.bind_buffer(self.target, Some(self.buffer)) }
    }

    pub fn sub_data<T: bytemuck::Pod>(&self, gl: &glow::Context, data: &[T]) {
        let bytes: &[u8] = bytemuck::cast_slice(data);
        self.bind(gl);
        unsafe { gl.buffer_sub_data_u8_slice(self.target, 0, bytes) }
    }

    /// Bind this buffer to a numbered indexed-buffer binding point (used for
    /// uniform blocks; the target must be `UNIFORM_BUFFER`).
    pub fn bind_base(&self, gl: &glow::Context, index: u32) {
        unsafe { gl.bind_buffer_base(self.target, index, Some(self.buffer)) }
    }
}

// ---------------------------------------------------------------------------
// Dynamic lights (std140 uniform block)
// ---------------------------------------------------------------------------

/// Maximum number of dynamic lights uploadable to the `LightBlock` UBO.
/// The block is 3 `vec4`s per light, so `256` lights occupy
/// `16 + 256 * 48 = 12304` bytes — comfortably within the WebGL2-guaranteed
/// 16 KB `MAX_UNIFORM_BLOCK_SIZE`.
pub const MAX_LIGHTS: usize = 256;

/// The UBO binding point shared by every shader that declares `LightBlock`.
pub const LIGHT_UBO_BINDING: u32 = 1;

/// Edge length of the square directional shadow map (depth texture).
pub const SHADOW_MAP_SIZE: u32 = 2048;

/// Slope-scaled depth offset applied to sprite billboard shadow casters, in
/// OpenGL polygon-offset factor units.  See `Gfx::set_shadow_sprite_offset`.
pub const SHADOW_SPRITE_SLOPE_OFFSET: f32 = 4.0;

/// Constant depth offset paired with `SHADOW_SPRITE_SLOPE_OFFSET`.
pub const SHADOW_SPRITE_UNIT_OFFSET: f32 = 8.0;

/// Texture unit the directional shadow map is bound to in the lit shaders.
/// Tilemap uses units 0/1, sprites use 0/1/2 — unit 3 is free for both.
pub const SHADOW_MAP_UNIT: u32 = 3;

/// Pack a slice of [`Light`]s into the flat `f32` buffer consumed by the
/// `LightBlock` `std140` uniform block:
///
/// ```text
/// offset 0        : vec4 count            (x = active light count)
/// per light i     : vec4 pos_radius       (xyz = position, w = radius)
///                 : vec4 color_intensity  (rgb = color, a = intensity)
///                 : vec4 dir_cone         (xyz = direction, w = cone_angle)
/// ```
///
/// `cone_angle <= 0` is the point-light sentinel (the shader skips the cone
/// term), so both `LightKind::Point` and `LightKind::Spot` share one layout.
/// The returned buffer is always `(1 + MAX_LIGHTS * 3) * 4` floats; trailing
/// lights beyond `capacity` are silently dropped.
pub fn pack_lights(lights: &[Light], capacity: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; (1 + capacity * 3) * 4];
    out[0] = (lights.len().min(capacity)) as f32;
    for (i, l) in lights.iter().take(capacity).enumerate() {
        let base = (1 + i * 3) * 4;
        out[base..base + 4].copy_from_slice(&[l.position.x, l.position.y, l.position.z, l.radius]);
        out[base + 4..base + 8].copy_from_slice(&[l.color[0], l.color[1], l.color[2], l.intensity]);
        let cone =
            if l.kind == classic_core::components::LightKind::Point { 0.0 } else { l.cone_angle };
        out[base + 8..base + 12].copy_from_slice(&[l.dir.x, l.dir.y, l.dir.z, cone]);
    }
    out
}

/// A host-side UBO backing the `LightBlock` uniform block.  Owns the CPU-side
/// capacity and the GPU buffer; uploaded once per frame by [`Gfx::upload_lights`].
pub struct LightBuffer {
    buffer: GlBuffer,
    capacity: usize,
}

impl LightBuffer {
    pub fn new(gl: &glow::Context, capacity: usize) -> Self {
        let floats = (1 + capacity * 3) * 4;
        let zeros = vec![0.0f32; floats];
        let buffer = GlBuffer::from_slice(gl, glow::UNIFORM_BUFFER, &zeros, glow::DYNAMIC_DRAW);
        Self { buffer, capacity }
    }

    /// Upload the packed light block and bind it to [`LIGHT_UBO_BINDING`].
    pub fn upload(&self, gl: &glow::Context, lights: &[Light]) {
        let data = pack_lights(lights, self.capacity);
        self.buffer.sub_data(gl, &data);
        self.buffer.bind_base(gl, LIGHT_UBO_BINDING);
    }
}

// ---------------------------------------------------------------------------
// Quad buffers — shared by all drawables
// ---------------------------------------------------------------------------

pub struct QuadBuffers {
    pub verts: GlBuffer,
    pub uv: GlBuffer,
    pub indices: GlBuffer,
    pub index_count: usize,
}

fn build_quad(gl: &glow::Context) -> QuadBuffers {
    let verts: [f32; 12] = [
        0.0, 1.0, 0.0, // v0
        1.0, 1.0, 0.0, // v1
        0.0, 0.0, 0.0, // v2
        1.0, 0.0, 0.0, // v3
    ];
    let uvs: [f32; 8] = [
        0.0, 1.0, // uv0
        1.0, 1.0, // uv1
        0.0, 0.0, // uv2
        1.0, 0.0, // uv3
    ];
    let idx: [u16; 6] = [0, 1, 2, 1, 2, 3];

    QuadBuffers {
        verts: GlBuffer::from_slice(gl, glow::ARRAY_BUFFER, &verts, glow::STATIC_DRAW),
        uv: GlBuffer::from_slice(gl, glow::ARRAY_BUFFER, &uvs, glow::STATIC_DRAW),
        indices: GlBuffer::from_slice(gl, glow::ELEMENT_ARRAY_BUFFER, &idx, glow::STATIC_DRAW),
        index_count: idx.len(),
    }
}

// ---------------------------------------------------------------------------
// Vertex attrib setup helpers
// ---------------------------------------------------------------------------

fn vertex_attrib_ptr_f32(
    gl: &glow::Context,
    buffer: &GlBuffer,
    location: u32,
    components: i32,
    stride_bytes: i32,
    offset_bytes: i32,
) {
    buffer.bind(gl);
    unsafe {
        gl.vertex_attrib_pointer_f32(
            location,
            components,
            glow::FLOAT,
            false,
            stride_bytes,
            offset_bytes,
        );
        gl.enable_vertex_attrib_array(location);
    }
}

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
    /// World -> light space (`iso_world_light_matrix`), the metric frame every
    /// lighting quantity lives in.  Deliberately *not* `model_matrix *
    /// iso_matrix`: that carries the isometric `diag(1, 0.5, 1)` squash.  Built
    /// by `classic_engine`; consumed by the tilemap draw only (sprites receive
    /// their own per-tilemap matrix on each draw).
    pub light_matrix: Mat4,
    /// Terrain normal matrix: `iso_world_normal_matrix`.  Unused by the sprite
    /// draws (their normals come from a baked map and ride their own per-tilemap
    /// matrix on each draw).
    pub normal_matrix: Mat3,
    /// Optional directional shadow map.  When `Some`, the lit shaders sample the
    /// depth texture and multiply the **sun diffuse** term by the shadow factor
    /// (ambient + point lights stay unshadowed).  When `None`, `use_shadow` is
    /// 0 and the term is byte-identical to the unshadowed path.
    pub shadow: Option<ShadowSettings>,
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
    shadow_map: Option<DepthFramebuffer>,
    vao: glow::VertexArray,
}

impl Gfx {
    pub fn new(gl: Rc<glow::Context>) -> Self {
        let quad = build_quad(&gl);
        let vao = unsafe { gl.create_vertex_array() }.expect("create VAO");
        let lights = LightBuffer::new(&gl, MAX_LIGHTS);
        Self {
            gl,
            shaders: HashMap::new(),
            textures: HashMap::new(),
            quad,
            viewport_w: 1920.0,
            viewport_h: 1080.0,
            render_target: None,
            lights,
            shadow_map: None,
            vao,
        }
    }

    /// Upload the active dynamic lights into the `LightBlock` UBO and bind it to
    /// [`LIGHT_UBO_BINDING`] (consumed by `sheet.frag` + `iso_tilemap.frag`).
    pub fn upload_lights(&self, lights: &[Light]) {
        self.lights.upload(&self.gl, lights);
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
    /// a synchronous main-thread fallback when the worker cannot start.
    #[cfg(target_arch = "wasm32")]
    pub async fn add_texture_basis_async(
        &mut self,
        name: &str,
        bytes: &[u8],
        format: &str,
    ) -> bool {
        let gl = self.gl.clone();
        if let Some(fmt) = compressed::CompressedFormat::parse(format) {
            if let Some(decoded) = compressed::transcode_async(&gl, bytes, fmt).await {
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
        if let Some((w, h, rgba)) = compressed::transcode_rgba8_async(bytes).await {
            log::debug!("texture {name}: basis -> RGBA8 fallback ({w}/{h})");
            self.add_texture_rgba8(name, &rgba, w, h);
            return true;
        }
        false
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

    /// Draw one terrain mesh into the shadow map in light space
    /// (`light_matrix * vertex`, no camera matrix).
    pub fn draw_shadow_tilemap(
        &self,
        light_matrix: &Mat4,
        view_proj: &Mat4,
        vertex_count: i32,
        vertex_buffer: &GlBuffer,
    ) {
        let gl = &self.gl;
        let s = self.shader("shadowDepth");
        s.bind(gl);
        vertex_attrib_ptr_f32(gl, vertex_buffer, s.attr("vertex_pos"), 3, 36, 0);
        s.uniform_mat4(gl, "light_matrix", light_matrix);
        s.uniform_mat4(gl, "light_view_proj", view_proj);
        unsafe {
            gl.draw_arrays(glow::TRIANGLES, 0, vertex_count);
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
        light_matrix: &Mat4,
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
        s.uniform_mat4(gl, "light_matrix", light_matrix);
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

    // -- draw calls --------------------------------------------------------

    /// Read a single RGBA pixel (normalized `[0, 1]`) from the current render
    /// target (offscreen FBO, or the default framebuffer when none is set).
    /// Screen coordinates are top-left origin, matching the engine's screen
    /// space; the GL bottom-left origin is flipped internally.
    pub fn read_pixel_rgba(&self, sx: i32, sy: i32) -> Option<[f32; 4]> {
        let gl = &self.gl;
        let (w, h) = match &self.render_target {
            Some(rt) => {
                rt.bind(gl);
                (rt.width as i32, rt.height as i32)
            }
            None => {
                unsafe { gl.bind_framebuffer(glow::FRAMEBUFFER, None) };
                (self.viewport_w as i32, self.viewport_h as i32)
            }
        };
        if sx < 0 || sy < 0 || sx >= w || sy >= h {
            return None;
        }
        let mut px = [0u8; 4];
        unsafe {
            gl.finish();
            gl.read_pixels(
                sx,
                h - 1 - sy,
                1,
                1,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut px)),
            );
        }
        Some([
            px[0] as f32 / 255.0,
            px[1] as f32 / 255.0,
            px[2] as f32 / 255.0,
            px[3] as f32 / 255.0,
        ])
    }

    /// Bind the projection/camera/model uniforms shared by every draw call.
    /// `ignore_cam` swaps the camera matrix for identity (screen-space UI).
    fn bind_view(&self, s: &Shader, camera: &Mat4, model: &Mat4, ignore_cam: bool) {
        let gl = &self.gl;
        let proj = self.projection();
        s.uniform_mat4(gl, "projection_matrix", &proj);
        s.uniform_mat4(gl, "camera_matrix", if ignore_cam { &Mat4::IDENTITY } else { camera });
        s.uniform_mat4(gl, "model_matrix", model);
    }

    /// Bind the directional shadow map and set the sampling uniforms on a lit
    /// shader (`isoTilemap` / `imageSheet`).  When `settings.shadow` is `None`,
    /// `use_shadow` is cleared to 0 (the shaders skip the term).
    fn bind_shadow(&self, s: &Shader, settings: &RenderSettings) {
        let gl = &self.gl;
        match &settings.shadow {
            Some(shadow) => {
                unsafe {
                    gl.active_texture(glow::TEXTURE0 + SHADOW_MAP_UNIT);
                    gl.bind_texture(glow::TEXTURE_2D, Some(shadow.texture));
                }
                s.uniform_1i(gl, "shadow_map", SHADOW_MAP_UNIT as i32);
                s.uniform_mat4(gl, "light_view_proj", &shadow.view_proj);
                s.uniform_1f(gl, "shadow_bias", shadow.bias);
                s.uniform_1f(gl, "shadow_strength", shadow.strength);
                s.uniform_vec2(gl, "shadow_texel", &shadow.texel);
                s.uniform_1f(gl, "shadow_normal_offset", shadow.normal_offset);
                s.uniform_1f(gl, "use_shadow", 1.0);
                s.uniform_1f(gl, "shadow_debug", if shadow.debug { 1.0 } else { 0.0 });
            }
            None => {
                s.uniform_1f(gl, "use_shadow", 0.0);
                s.uniform_1f(gl, "shadow_debug", 0.0);
            }
        }
    }

    /// Draw a solid-colour rectangle.
    pub fn draw_rect(&self, model: &Mat4, camera: &Mat4, color: &[f32; 4], ignore_cam: bool) {
        let gl = &self.gl;
        let s = self.shader("solid");

        s.bind(gl);
        self.bind_view(s, camera, model, ignore_cam);
        s.uniform_vec4(gl, "color", color);

        vertex_attrib_ptr_f32(gl, &self.quad.verts, s.attr("vertex_pos"), 3, 0, 0);
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

    /// Draw a sprite from a sprite sheet, addressed by either a uniform-grid
    /// frame or a packed-atlas UV rect (see [`SpriteRegion`]).
    ///
    /// `settings` carries the shared lighting bundle; `sheet.frag` only applies
    /// it when a normal map is bound (`use_normal_map`), so the unlit path is
    /// byte-identical.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_sprite(
        &self,
        model: &Mat4,
        camera: &Mat4,
        texture_name: &str,
        region: SpriteRegion<'_>,
        ignore_cam: bool,
        ghost_alpha: f32,
        settings: &RenderSettings,
    ) {
        let gl = &self.gl;
        let s = self.shader("imageSheet");
        let t = self.texture(texture_name);

        s.bind(gl);
        t.bind(gl, 0);

        s.uniform_1i(gl, "tex_sampler", 0);
        self.bind_view(s, camera, model, ignore_cam);
        match region {
            SpriteRegion::Grid { frame, tile_set_size } => {
                s.uniform_1f(gl, "tile_id_flat", frame);
                s.uniform_vec2(gl, "tile_set_size", &tile_set_size);
                s.uniform_1f(gl, "use_uv_rect", 0.0);
                s.uniform_vec4(gl, "uv_rect", &[0.0, 0.0, 0.0, 0.0]);
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
        s.uniform_1f(gl, "use_iso_depth", 0.0);
        s.uniform_vec4(gl, "iso_depth_corners", &[0.0, 0.0, 0.0, 0.0]);
        s.uniform_1f(gl, "ghost_alpha", ghost_alpha);
        // Non-iso sprites never show the RTS silhouette; reset the uniforms so a
        // previously-selected iso sprite doesn't leak into the UI/Sprite phase.
        s.uniform_1f(gl, "selected", 0.0);
        s.uniform_vec3(gl, "selection_color", Vec3::from_array([0.0, 0.0, 0.0]));
        s.uniform_vec2(gl, "outline_delta", &[0.0, 0.0]);
        s.uniform_1f(gl, "use_lighting", 0.0);
        s.uniform_1f(gl, "use_normal_map", 0.0);
        s.uniform_vec3(gl, "ambient_color", Vec3::from_array(settings.ambient));
        s.uniform_vec3(gl, "light_direction", Vec3::from_array(settings.light_dir));
        s.uniform_vec3(gl, "light_color", Vec3::from_array(settings.light_color));
        s.uniform_vec3(gl, "tint", Vec3::from_array([1.0, 1.0, 1.0]));
        s.uniform_1f(gl, "use_shadow", 0.0);
        s.uniform_1f(gl, "shadow_debug", 0.0);

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

    /// SDF text draw (called 1-3 times per frame for shadow/glow/main).
    #[allow(clippy::too_many_arguments)]
    pub fn draw_sdf(
        &self,
        model: &Mat4,
        camera: &Mat4,
        atlas_name: &str,
        color: &[f32; 4],
        outline_color: &[f32; 4],
        outline_width: f32,
        spread: f32,
        atlas_size: &[f32; 2],
        weight: f32,
        gamma: f32,
        vertex_count: i32,
        vertex_buffer: &GlBuffer,
        ignore_cam: bool,
    ) {
        let gl = &self.gl;
        let s = self.shader("sdf");
        let t = self.texture(atlas_name);

        s.bind(gl);
        t.bind(gl, 0);

        s.uniform_1i(gl, "tex_sampler", 0);
        self.bind_view(s, camera, model, ignore_cam);
        s.uniform_vec4(gl, "color", color);
        s.uniform_vec4(gl, "outline_color", outline_color);
        s.uniform_1f(gl, "outline_width", outline_width);
        s.uniform_1f(gl, "soft_edge", 0.08);
        s.uniform_1f(gl, "spread", spread);
        s.uniform_vec2(gl, "atlas_size", atlas_size);
        s.uniform_1f(gl, "weight", weight);
        s.uniform_1f(gl, "gamma", gamma);

        vertex_attrib_ptr_f32(gl, vertex_buffer, s.attr("vertex_pos"), 2, 16, 0);
        vertex_attrib_ptr_f32(gl, vertex_buffer, s.attr("tex_coord"), 2, 16, 8);

        unsafe {
            gl.draw_arrays(glow::TRIANGLES, 0, vertex_count);
        }
    }

    /// Bind the `imageSheet` shader, sprite texture and uniforms shared by the
    /// normal and ghost passes of the isometric sprite draw.
    ///
    /// When `depth_map` is `Some(name)`, the sprite writes a per-pixel
    /// `gl_FragDepth` sampled from the depth-map texture (which stores camera
    /// view depth directly in window `[0, 1]`), so overlapping sprites occlude
    /// each other per-pixel rather than purely by draw order.  When
    /// `normal_map` is `Some(name)`, the sprite is shaded with a runtime
    /// Lambertian term from `settings`.
    #[allow(clippy::too_many_arguments)]
    fn bind_iso_sprite(
        &self,
        model: &Mat4,
        camera: &Mat4,
        world_matrix: &Mat4,
        light_matrix: &Mat4,
        normal_matrix: &Mat3,
        texture_name: &str,
        region: SpriteRegion<'_>,
        iso_depth_corners: &[f32; 4],
        depth_map: Option<&str>,
        normal_map: Option<&str>,
        tint: &[f32; 3],
        settings: &RenderSettings,
        ghost_alpha: f32,
        selected: bool,
        selection_color: &[f32; 3],
        outline_radius: f32,
    ) {
        let gl = &self.gl;
        let s = self.shader("imageSheet");
        let t = self.texture(texture_name);

        s.bind(gl);
        t.bind(gl, 0);

        s.uniform_1i(gl, "tex_sampler", 0);
        self.bind_view(s, camera, model, false);
        s.uniform_mat4(gl, "world_matrix", world_matrix);
        s.uniform_mat4(gl, "light_matrix", light_matrix);
        s.uniform_mat3(gl, "normal_matrix", normal_matrix);
        s.uniform_1f(gl, "ppm", settings.ppm);

        // Silhouette outline: the sheet-UV offset of `outline_radius` content
        // pixels, so a selected sprite's transparent edge samples its own cell's
        // opaque neighbours without cross-frame bleed.
        let outline_delta: [f32; 2] = match &region {
            SpriteRegion::Grid { .. } => {
                [outline_radius / t.size.0.max(1) as f32, outline_radius / t.size.1.max(1) as f32]
            }
            SpriteRegion::Uv { uv_rect, content_size, .. } => {
                let ext_x = (uv_rect[2] - uv_rect[0]).abs().max(1e-6);
                let ext_y = (uv_rect[3] - uv_rect[1]).abs().max(1e-6);
                [
                    outline_radius * ext_x / content_size[0].max(1.0),
                    outline_radius * ext_y / content_size[1].max(1.0),
                ]
            }
        };

        match region {
            SpriteRegion::Grid { frame, tile_set_size } => {
                s.uniform_1f(gl, "tile_id_flat", frame);
                s.uniform_vec2(gl, "tile_set_size", &tile_set_size);
                s.uniform_1f(gl, "use_uv_rect", 0.0);
                s.uniform_vec4(gl, "uv_rect", &[0.0, 0.0, 0.0, 0.0]);
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
        s.uniform_1f(gl, "use_iso_depth", 1.0);
        s.uniform_vec4(gl, "iso_depth_corners", iso_depth_corners);
        s.uniform_1f(gl, "ghost_alpha", ghost_alpha);
        s.uniform_1f(gl, "selected", if selected { 1.0 } else { 0.0 });
        s.uniform_vec3(gl, "selection_color", Vec3::from_array(*selection_color));
        s.uniform_vec2(gl, "outline_delta", &outline_delta);

        if let Some(depth_tex) = depth_map {
            if let Some(dt) = self.textures.get(depth_tex) {
                dt.bind(gl, 1);
                s.uniform_1i(gl, "depth_sampler", 1);
            }
            s.uniform_1f(gl, "use_depth_map", 1.0);
        } else {
            s.uniform_1f(gl, "use_depth_map", 0.0);
        }

        if let Some(normal_tex) = normal_map {
            if let Some(nt) = self.textures.get(normal_tex) {
                nt.bind(gl, 2);
                s.uniform_1i(gl, "normal_sampler", 2);
                s.uniform_1f(gl, "use_normal_map", 1.0);
            } else {
                s.uniform_1f(gl, "use_normal_map", 0.0);
            }
        } else {
            s.uniform_1f(gl, "use_normal_map", 0.0);
        }
        s.uniform_vec3(gl, "ambient_color", Vec3::from_array(settings.ambient));
        s.uniform_vec3(gl, "light_direction", Vec3::from_array(settings.light_dir));
        s.uniform_vec3(gl, "light_color", Vec3::from_array(settings.light_color));
        s.uniform_vec3(gl, "tint", Vec3::from_array(*tint));
        s.uniform_1f(gl, "use_lighting", 1.0);
        self.bind_shadow(s, settings);

        vertex_attrib_ptr_f32(gl, &self.quad.verts, s.attr("vertex_pos"), 3, 0, 0);
        vertex_attrib_ptr_f32(gl, &self.quad.uv, s.attr("tex_coord"), 2, 0, 0);
        self.quad.indices.bind(gl);
    }

    /// Draw one pass of an isometric sprite.
    ///
    /// The engine drives isometric sprites in two phases (all
    /// [`IsoSpritePass::Normal`] then all [`IsoSpritePass::Ghost`]) so
    /// sprite-vs-sprite occlusion is resolved by the depth buffer, not draw
    /// order.  The stencil buffer records a per-instance `ghost_group` id
    /// during the normal pass (`REPLACE`) so the ghost pass can skip pixels its
    /// own group already occludes (`NOTEQUAL`).
    ///
    /// - **normal** — `LEQUAL`, `depth_mask(depth_map.is_some())` (depth-mapped
    ///   sprites write depth), stencil `ALWAYS`/`REPLACE ghost_group`,
    ///   `stencil_mask(0xFF)`, `ghost_alpha=0`.
    /// - **ghost** — `GREATER`, `depth_mask(false)`, `ghost_alpha=0.4`, stencil
    ///   `NOTEQUAL ghost_group` (`ALWAYS` when group 0), `stencil_mask(0x00)`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_iso_sprite(
        &self,
        model: &Mat4,
        camera: &Mat4,
        world_matrix: &Mat4,
        light_matrix: &Mat4,
        normal_matrix: &Mat3,
        texture_name: &str,
        region: SpriteRegion<'_>,
        iso_depth_corners: &[f32; 4],
        depth_map: Option<&str>,
        normal_map: Option<&str>,
        tint: &[f32; 3],
        settings: &RenderSettings,
        ghost_group: u32,
        pass: IsoSpritePass,
        selected: bool,
        selection_color: &[f32; 3],
        outline_radius: f32,
    ) {
        let gl = &self.gl;
        let ghost_alpha = match pass {
            IsoSpritePass::Normal => 0.0,
            IsoSpritePass::Ghost => 0.4,
        };
        self.bind_iso_sprite(
            model,
            camera,
            world_matrix,
            light_matrix,
            normal_matrix,
            texture_name,
            region,
            iso_depth_corners,
            depth_map,
            normal_map,
            tint,
            settings,
            ghost_alpha,
            selected,
            selection_color,
            outline_radius,
        );

        unsafe {
            gl.enable(glow::DEPTH_TEST);
            match pass {
                IsoSpritePass::Normal => {
                    gl.depth_func(glow::LEQUAL);
                    gl.depth_mask(depth_map.is_some());

                    gl.enable(glow::STENCIL_TEST);
                    gl.stencil_func(glow::ALWAYS, ghost_group as i32, 0xFF);
                    gl.stencil_op(glow::KEEP, glow::KEEP, glow::REPLACE);
                    gl.stencil_mask(0xFF);

                    gl.draw_elements(
                        glow::TRIANGLES,
                        self.quad.index_count as i32,
                        glow::UNSIGNED_SHORT,
                        0,
                    );

                    gl.disable(glow::STENCIL_TEST);
                }
                IsoSpritePass::Ghost => {
                    gl.depth_func(glow::GREATER);
                    gl.depth_mask(false);

                    gl.enable(glow::STENCIL_TEST);
                    if ghost_group != 0 {
                        gl.stencil_func(glow::NOTEQUAL, ghost_group as i32, 0xFF);
                    } else {
                        gl.stencil_func(glow::ALWAYS, 0, 0xFF);
                    }
                    gl.stencil_op(glow::KEEP, glow::KEEP, glow::KEEP);
                    gl.stencil_mask(0x00);

                    gl.draw_elements(
                        glow::TRIANGLES,
                        self.quad.index_count as i32,
                        glow::UNSIGNED_SHORT,
                        0,
                    );

                    gl.disable(glow::STENCIL_TEST);
                    gl.stencil_mask(0xFF);
                }
            }
            gl.depth_mask(true);
            gl.depth_func(glow::LEQUAL);
            gl.disable(glow::DEPTH_TEST);
        }
    }

    /// Draw a polygon outline as `LINE_LOOP` using the solid shader.
    /// Uses `depthFunc(ALWAYS)` + `depthMask(false)` for wireframe-over-terrain.
    pub fn draw_line_loop(
        &self,
        vertex_buffer: &GlBuffer,
        vertex_count: i32,
        model: &Mat4,
        camera: &Mat4,
        color: &[f32; 4],
    ) {
        let gl = &self.gl;
        let s = self.shader("solid");

        s.bind(gl);
        self.bind_view(s, camera, model, false);
        s.uniform_vec4(gl, "color", color);

        vertex_attrib_ptr_f32(gl, vertex_buffer, s.attr("vertex_pos"), 3, 0, 0);

        unsafe {
            gl.depth_func(glow::ALWAYS);
            gl.depth_mask(false);
            gl.draw_arrays(glow::LINE_LOOP, 0, vertex_count);
            gl.depth_mask(true);
            gl.depth_func(glow::LEQUAL);
        }
    }

    /// Draw line-strip segments using the solid shader.
    pub fn draw_line_strip(
        &self,
        vertex_buffer: &GlBuffer,
        first: i32,
        count: i32,
        model: &Mat4,
        camera: &Mat4,
        color: &[f32; 4],
    ) {
        let gl = &self.gl;
        let s = self.shader("solid");

        s.bind(gl);
        self.bind_view(s, camera, model, false);
        s.uniform_vec4(gl, "color", color);

        vertex_attrib_ptr_f32(gl, vertex_buffer, s.attr("vertex_pos"), 3, 0, 0);

        unsafe {
            gl.depth_func(glow::ALWAYS);
            gl.depth_mask(false);
            gl.draw_arrays(glow::LINE_STRIP, first, count);
            gl.depth_mask(true);
            gl.depth_func(glow::LEQUAL);
        }
    }

    // -- shader-source resolution ------------------------------------------

    // draw_tilemap enables DEPTH_TEST locally, then disables it on exit.
    // This is the ONLY place DEPTH_TEST is enabled. begin_frame leaves it off.
    // UI rendering relies on draw-order (not depth) for layering.
    /// Draw the isometric tilemap terrain.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_tilemap(
        &self,
        model: &Mat4,
        camera: &Mat4,
        world_matrix: &Mat4,
        tile_data_tex: &glow::Texture,
        tileset_name: &str,
        tile_set_size: &[f32; 2],
        tile_pixel_size: &[f32; 2],
        map_size: &[f32; 2],
        selected_tile: &[f32; 2],
        selection_begin: &[f32; 2],
        selection_mode: i32,
        selection_color: &[f32; 4],
        settings: &RenderSettings,
        show_grid: bool,
        vertex_count: i32,
        vertex_buffer: &GlBuffer,
    ) {
        let gl = &self.gl;
        let s = self.shader("isoTilemap");
        let tset = self.texture(tileset_name);

        s.bind(gl);

        // Interleaved vertex attribs at offsets 0, 12, 20, 24
        vertex_attrib_ptr_f32(gl, vertex_buffer, s.attr("vertex_pos"), 3, 36, 0);
        vertex_attrib_ptr_f32(gl, vertex_buffer, s.attr("map_coord"), 2, 36, 12);
        vertex_attrib_ptr_f32(gl, vertex_buffer, s.attr("tile_id"), 1, 36, 20);
        vertex_attrib_ptr_f32(gl, vertex_buffer, s.attr("normal"), 3, 36, 24);

        // Texture 0: map data (raw GL texture handle)
        unsafe {
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(*tile_data_tex));
        }
        // Texture 1: tileset
        tset.bind(gl, 1);

        s.uniform_1i(gl, "map_data", 0);
        s.uniform_1i(gl, "tile_set", 1);
        self.bind_view(s, camera, model, false);
        s.uniform_mat4(gl, "world_matrix", world_matrix);
        s.uniform_vec2(gl, "tile_set_size", tile_set_size);
        s.uniform_vec2(gl, "tile_pixel_size", tile_pixel_size);
        s.uniform_vec2(gl, "depth_span", &settings.depth_span);
        s.uniform_1f(gl, "ppm", settings.ppm);
        s.uniform_vec2(gl, "map_size", map_size);
        s.uniform_vec2(gl, "selected_tile", selected_tile);
        s.uniform_vec2(gl, "selection_begin", selection_begin);
        s.uniform_1i(gl, "selection_mode", selection_mode);
        s.uniform_vec4(gl, "selection_color", selection_color);
        s.uniform_vec4(gl, "wall_color", &[0.3, 0.2, 0.15, 1.0]);
        s.uniform_1f(gl, "grid_radius", 3.0);
        s.uniform_1i(gl, "show_grid", if show_grid { 1 } else { 0 });
        s.uniform_vec3(gl, "grid_color", Vec3::ZERO);
        s.uniform_mat3(gl, "normal_matrix", &settings.normal_matrix);
        s.uniform_mat4(gl, "light_matrix", &settings.light_matrix);
        s.uniform_vec3(gl, "ambient_color", Vec3::from_array(settings.ambient));
        s.uniform_vec3(gl, "light_direction", Vec3::from_array(settings.light_dir));
        s.uniform_vec3(gl, "light_color", Vec3::from_array(settings.light_color));
        self.bind_shadow(s, settings);

        unsafe {
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            gl.depth_mask(true);
            gl.draw_arrays(glow::TRIANGLES, 0, vertex_count);
            gl.disable(glow::DEPTH_TEST);
        }
    }
}

// ---------------------------------------------------------------------------
// Named shader source registry
// ---------------------------------------------------------------------------

/// Extract the filename (last `/`-separated segment) from a shader URL such as
/// `/shaders/direct.vert`.
fn shader_filename(url: &str) -> &str {
    url.rsplit('/').next().unwrap_or(url)
}

/// A name-keyed registry of GLSL shader sources.
///
/// Built-in sources (embedded via `include_str!`) are registered by
/// [`ShaderSourceRegistry::builtin`]; a ROM may override any of them by
/// filename, so ROM-owned shaders replace the engine defaults without touching
/// the manifest or the draw layer.
#[derive(Default)]
pub struct ShaderSourceRegistry {
    vertex: std::collections::HashMap<String, String>,
    fragment: std::collections::HashMap<String, String>,
}

impl ShaderSourceRegistry {
    /// Registry seeded with the engine's built-in shader sources.
    pub fn builtin() -> Self {
        let mut r = Self::default();
        r.override_vertex("direct.vert", shaders::DIRECT_VERT);
        r.override_vertex("direct_tex.vert", shaders::DIRECT_TEX_VERT);
        r.override_vertex("iso_tilemap.vert", shaders::ISO_TILEMAP_VERT);
        r.override_vertex("sdf.vert", shaders::SDF_VERT);
        r.override_vertex("shadow_depth.vert", shaders::SHADOW_DEPTH_VERT);
        r.override_vertex("shadow_sprite.vert", shaders::SHADOW_SPRITE_VERT);
        r.override_fragment("solid.frag", shaders::SOLID_FRAG);
        r.override_fragment("image.frag", shaders::IMAGE_FRAG);
        r.override_fragment("image_colorized.frag", shaders::IMAGE_COLORIZED_FRAG);
        r.override_fragment("iso_tilemap.frag", shaders::ISO_TILEMAP_FRAG);
        r.override_fragment("sheet.frag", shaders::SHEET_FRAG);
        r.override_fragment("sdf.frag", shaders::SDF_FRAG);
        r.override_fragment("shadow_depth.frag", shaders::SHADOW_DEPTH_FRAG);
        r.override_fragment("shadow_sprite.frag", shaders::SHADOW_SPRITE_FRAG);
        r
    }

    /// Register (or replace) a vertex shader source by filename.
    pub fn override_vertex(&mut self, filename: &str, source: impl Into<String>) {
        self.vertex.insert(filename.to_string(), source.into());
    }

    /// Register (or replace) a fragment shader source by filename.
    pub fn override_fragment(&mut self, filename: &str, source: impl Into<String>) {
        self.fragment.insert(filename.to_string(), source.into());
    }

    /// Resolve a vertex-shader URL to a source string.
    pub fn resolve_vertex(&self, url: &str) -> String {
        let f = shader_filename(url);
        self.vertex.get(f).cloned().unwrap_or_else(|| panic!("unknown vertex shader URL: {url}"))
    }

    /// Resolve a fragment-shader URL to a source string.
    pub fn resolve_fragment(&self, url: &str) -> String {
        let f = shader_filename(url);
        self.fragment
            .get(f)
            .cloned()
            .unwrap_or_else(|| panic!("unknown fragment shader URL: {url}"))
    }
}

/// A built-in shader declaration: the program name, its vertex/fragment
/// source filenames (resolved through [`ShaderSourceRegistry`]), and the
/// attribute/uniform layout.  The engine compiles the full builtin set by
/// default; a ROM may override any shader by *name* via its manifest
/// `shaders[]` list.
pub struct BuiltinShader {
    pub name: &'static str,
    pub vertex: &'static str,
    pub fragment: &'static str,
    pub attr: &'static [&'static str],
    pub unif: &'static [&'static str],
}

/// The engine's built-in shader catalog, in dependency-free declaration form.
/// Names/filenames/layouts mirror the shared `shaders[]` block the ROM
/// manifests used to carry (now owned by the engine — see the `classic-gfx`
/// skill).
pub fn builtin_shaders() -> Vec<BuiltinShader> {
    vec![
        BuiltinShader {
            name: "solid",
            vertex: "direct.vert",
            fragment: "solid.frag",
            attr: &["vertex_pos"],
            unif: &["model_matrix", "camera_matrix", "projection_matrix", "color"],
        },
        BuiltinShader {
            name: "image",
            vertex: "direct_tex.vert",
            fragment: "image.frag",
            attr: &["vertex_pos", "tex_coord"],
            unif: &["model_matrix", "camera_matrix", "projection_matrix", "tex_sampler"],
        },
        BuiltinShader {
            name: "imageColorize",
            vertex: "direct_tex.vert",
            fragment: "image_colorized.frag",
            attr: &["vertex_pos", "tex_coord"],
            unif: &["model_matrix", "camera_matrix", "projection_matrix", "tex_sampler", "color"],
        },
        BuiltinShader {
            name: "imageSheet",
            vertex: "direct_tex.vert",
            fragment: "sheet.frag",
            attr: &["vertex_pos", "tex_coord"],
            unif: &[
                "model_matrix",
                "camera_matrix",
                "projection_matrix",
                "world_matrix",
                "light_matrix",
                "normal_matrix",
                "ppm",
                "tex_sampler",
                "tile_id_flat",
                "tile_set_size",
                "use_iso_depth",
                "iso_depth_corners",
                "ghost_alpha",
                "use_uv_rect",
                "uv_rect",
                "trim_offset",
                "source_size",
                "content_size",
                "depth_sampler",
                "use_depth_map",
                "normal_sampler",
                "use_normal_map",
                "use_lighting",
                "ambient_color",
                "light_direction",
                "light_color",
                "tint",
                "shadow_map",
                "light_view_proj",
                "shadow_bias",
                "shadow_strength",
                "shadow_texel",
                "use_shadow",
                "shadow_debug",
                "shadow_normal_offset",
            ],
        },
        BuiltinShader {
            name: "sdf",
            vertex: "sdf.vert",
            fragment: "sdf.frag",
            attr: &["vertex_pos", "tex_coord"],
            unif: &[
                "model_matrix",
                "camera_matrix",
                "projection_matrix",
                "tex_sampler",
                "color",
                "outline_color",
                "outline_width",
                "soft_edge",
                "spread",
                "atlas_size",
                "weight",
                "gamma",
            ],
        },
        BuiltinShader {
            name: "shadowDepth",
            vertex: "shadow_depth.vert",
            fragment: "shadow_depth.frag",
            attr: &["vertex_pos"],
            unif: &["light_matrix", "light_view_proj"],
        },
        BuiltinShader {
            name: "shadowSprite",
            vertex: "shadow_sprite.vert",
            fragment: "shadow_sprite.frag",
            attr: &["vertex_pos", "tex_coord"],
            unif: &[
                "model_matrix",
                "light_matrix",
                "light_view_proj",
                "tex_sampler",
                "tile_id_flat",
                "tile_set_size",
                "use_uv_rect",
                "uv_rect",
                "trim_offset",
                "source_size",
                "content_size",
            ],
        },
        BuiltinShader {
            name: "isoTilemap",
            vertex: "iso_tilemap.vert",
            fragment: "iso_tilemap.frag",
            attr: &["vertex_pos", "map_coord", "tile_id", "normal"],
            unif: &[
                "world_matrix",
                "model_matrix",
                "light_matrix",
                "camera_matrix",
                "projection_matrix",
                "normal_matrix",
                "map_data",
                "map_size",
                "tile_set",
                "tile_set_size",
                "tile_pixel_size",
                "depth_span",
                "ppm",
                "selected_tile",
                "selection_begin",
                "selection_mode",
                "selection_color",
                "wall_color",
                "grid_radius",
                "show_grid",
                "grid_color",
                "ambient_color",
                "light_direction",
                "light_color",
                "shadow_map",
                "light_view_proj",
                "shadow_bias",
                "shadow_strength",
                "shadow_texel",
                "use_shadow",
                "shadow_debug",
                "shadow_normal_offset",
            ],
        },
    ]
}

// ---------------------------------------------------------------------------
// Vertex buffer builder for dynamic geometry (SDF text glyph quads)
// ---------------------------------------------------------------------------

pub struct DynamicVb {
    buffer: glow::Buffer,
    capacity_bytes: usize,
    len_bytes: usize,
}

impl DynamicVb {
    pub fn new(gl: &glow::Context, capacity_bytes: usize) -> Self {
        let buffer = unsafe { gl.create_buffer() }.expect("create buffer");
        unsafe {
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
            gl.buffer_data_size(glow::ARRAY_BUFFER, capacity_bytes as i32, glow::DYNAMIC_DRAW);
        }
        Self { buffer, capacity_bytes, len_bytes: 0 }
    }

    pub fn upload<T: bytemuck::Pod>(&mut self, gl: &glow::Context, data: &[T]) {
        let bytes: &[u8] = bytemuck::cast_slice(data);
        self.len_bytes = bytes.len();
        if self.len_bytes > self.capacity_bytes {
            self.capacity_bytes = self.len_bytes;
            unsafe {
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.buffer));
                gl.buffer_data_size(
                    glow::ARRAY_BUFFER,
                    self.capacity_bytes as i32,
                    glow::DYNAMIC_DRAW,
                );
            }
        }
        unsafe {
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.buffer));
            gl.buffer_sub_data_u8_slice(glow::ARRAY_BUFFER, 0, bytes);
        }
    }

    pub fn bind(&self, gl: &glow::Context) {
        unsafe { gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.buffer)) }
    }
}

/// A depth-only framebuffer: a `DEPTH_COMPONENT24` texture attached to
/// `DEPTH_ATTACHMENT` with no color attachment (`draw_buffers([NONE])`).  Used
/// for the directional shadow map; the depth texture is sampled as a
/// `sampler2D` in the lit shaders (manual `step` compare, no PCF).
pub struct DepthFramebuffer {
    fbo: glow::Framebuffer,
    depth_tex: glow::Texture,
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

#[cfg(test)]
mod tests {
    use super::*;
    use classic_core::components::{Light, LightKind};

    #[test]
    fn pack_lights_std140_layout() {
        let lights = vec![Light {
            kind: LightKind::Point,
            position: glam::Vec3::new(1.0, 2.0, 3.0),
            color: [0.1, 0.2, 0.3],
            intensity: 4.0,
            radius: 50.0,
            dir: glam::Vec3::new(5.0, 6.0, 7.0),
            cone_angle: 0.5,
            parent: None,
        }];
        let buf = pack_lights(&lights, MAX_LIGHTS);
        assert_eq!(buf.len(), (1 + MAX_LIGHTS * 3) * 4);
        // count vec4
        assert_eq!(buf[0], 1.0);
        assert_eq!(buf[1], 0.0);
        // light 0: [pos.xyz | radius]
        assert_eq!(&buf[4..8], &[1.0, 2.0, 3.0, 50.0]);
        // light 0: [color.rgb | intensity]
        assert_eq!(&buf[8..12], &[0.1, 0.2, 0.3, 4.0]);
        // light 0: [dir.xyz | cone]; a Point light forces cone_angle to 0.
        assert_eq!(&buf[12..16], &[5.0, 6.0, 7.0, 0.0]);
    }

    #[test]
    fn pack_lights_spot_keeps_cone_angle() {
        let lights = vec![Light {
            kind: LightKind::Spot,
            dir: glam::Vec3::new(0.0, 0.0, 1.0),
            cone_angle: 0.7,
            ..Default::default()
        }];
        let buf = pack_lights(&lights, MAX_LIGHTS);
        assert_eq!(&buf[12..16], &[0.0, 0.0, 1.0, 0.7]);
    }

    #[test]
    fn pack_lights_truncates_beyond_capacity() {
        let lights = vec![Light::default(); MAX_LIGHTS + 5];
        let buf = pack_lights(&lights, MAX_LIGHTS);
        assert_eq!(buf[0], MAX_LIGHTS as f32);
    }
}
