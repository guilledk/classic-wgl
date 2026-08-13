//! # Skill: `classic-gfx`
//!
//! **Read `.claude/skills/classic-gfx/SKILL.md` before working on this module.**
//!
//! classic-gfx: OpenGL ES 3.0 / WebGL2 graphics layer.
//!
//! Shader compilation, texture upload, shared quad buffers, and draw-call
//! emitters for each `DrawKind` variant.  Pure `glow` — no windowing.

mod shaders;

use glam::{Mat3, Mat4, Vec3};
use glow::HasContext;
use std::collections::HashMap;
use std::rc::Rc;

// ---------------------------------------------------------------------------
// Shader
// ---------------------------------------------------------------------------

pub struct Shader {
    program: glow::Program,
    attr: HashMap<String, u32>,
    unif: HashMap<String, glow::UniformLocation>,
}

impl Shader {
    /// Compile a shader program from GLSL 300 es source strings.
    ///
    /// `attr_names` are bound to consecutive attribute locations
    /// (index = location, matching `utils.ts:239-243`).
    pub fn compile(
        gl: &glow::Context,
        vs_src: &str,
        fs_src: &str,
        attr_names: &[String],
        unif_names: &[String],
    ) -> Result<Self, String> {
        let vs = compile_single(gl, glow::VERTEX_SHADER, vs_src)?;
        let fs = compile_single(gl, glow::FRAGMENT_SHADER, fs_src)?;

        let program = unsafe {
            let p = gl.create_program().map_err(|_| "failed to create program")?;
            gl.attach_shader(p, vs);
            gl.attach_shader(p, fs);

            // Bind attribute locations by manifest index (critical for
            // matching the TS behaviour — attribute arrays are positional).
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
        for name in attr_names {
            if let Some(loc) = unsafe { gl.get_attrib_location(program, name) } {
                attr.insert(name.clone(), loc);
            }
        }

        let mut unif = HashMap::new();
        for name in unif_names {
            if let Some(loc) = unsafe { gl.get_uniform_location(program, name) } {
                unif.insert(name.clone(), loc);
            }
        }

        Ok(Self { program, attr, unif })
    }

    pub fn bind(&self, gl: &glow::Context) {
        unsafe { gl.use_program(Some(self.program)) }
    }

    pub fn attr(&self, name: &str) -> u32 {
        *self.attr.get(name).unwrap_or_else(|| panic!("attribute '{name}' not found"))
    }

    fn unif(&self, name: &str) -> Option<&glow::UniformLocation> {
        self.unif.get(name)
    }

    // -- uniform setters ---------------------------------------------------

    pub fn uniform_mat4(&self, gl: &glow::Context, name: &str, m: &Mat4) {
        if let Some(loc) = self.unif(name) {
            unsafe {
                gl.uniform_matrix_4_f32_slice(Some(loc), false, m.as_ref());
            }
        }
    }

    pub fn uniform_mat3(&self, gl: &glow::Context, name: &str, m: &Mat3) {
        if let Some(loc) = self.unif(name) {
            unsafe {
                gl.uniform_matrix_3_f32_slice(Some(loc), false, m.as_ref());
            }
        }
    }

    pub fn uniform_vec4(&self, gl: &glow::Context, name: &str, v: &[f32; 4]) {
        if let Some(loc) = self.unif(name) {
            unsafe { gl.uniform_4_f32(Some(loc), v[0], v[1], v[2], v[3]) }
        }
    }

    pub fn uniform_vec3(&self, gl: &glow::Context, name: &str, v: Vec3) {
        if let Some(loc) = self.unif(name) {
            unsafe { gl.uniform_3_f32(Some(loc), v[0], v[1], v[2]) }
        }
    }

    pub fn uniform_vec2(&self, gl: &glow::Context, name: &str, v: &[f32; 2]) {
        if let Some(loc) = self.unif(name) {
            unsafe { gl.uniform_2_f32(Some(loc), v[0], v[1]) }
        }
    }

    pub fn uniform_1f(&self, gl: &glow::Context, name: &str, v: f32) {
        if let Some(loc) = self.unif(name) {
            unsafe { gl.uniform_1_f32(Some(loc), v) }
        }
    }

    pub fn uniform_1i(&self, gl: &glow::Context, name: &str, v: i32) {
        if let Some(loc) = self.unif(name) {
            unsafe { gl.uniform_1_i32(Some(loc), v) }
        }
    }

    pub fn uniform_bool(&self, gl: &glow::Context, name: &str, v: bool) {
        if let Some(loc) = self.unif(name) {
            unsafe { gl.uniform_1_i32(Some(loc), v as i32) }
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

pub struct GlTexture {
    texture: glow::Texture,
    /// Image pixel dimensions.
    pub size: (u32, u32),
}

impl GlTexture {
    /// Upload RGBA8 pixel data to a new 2D texture.
    pub fn from_rgba8(gl: &glow::Context, rgba: &[u8], width: u32, height: u32) -> Self {
        let texture = unsafe { gl.create_texture() }.expect("create texture");
        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA as i32,
                width as i32,
                height as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(rgba)),
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

pub struct Gfx {
    pub gl: Rc<glow::Context>,
    pub shaders: HashMap<String, Shader>,
    pub textures: HashMap<String, GlTexture>,
    pub quad: QuadBuffers,
    pub viewport_w: f32,
    pub viewport_h: f32,
    pub render_target: Option<GlFrameBuffer>,
    vao: glow::VertexArray,
}

impl Gfx {
    pub fn new(gl: Rc<glow::Context>) -> Self {
        let quad = build_quad(&gl);
        let vao = unsafe { gl.create_vertex_array() }.expect("create VAO");
        Self {
            gl,
            shaders: HashMap::new(),
            textures: HashMap::new(),
            quad,
            viewport_w: 1920.0,
            viewport_h: 1080.0,
            render_target: None,
            vao,
        }
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
    /// Matches `mat4.ortho(m, 0, vw, vh, 0, -10000, 10000)` from `state.ts:355-363`.
    pub fn projection(&self) -> Mat4 {
        Mat4::orthographic_rh(0.0, self.viewport_w, self.viewport_h, 0.0, -10000.0, 10000.0)
    }

    /// Resize the viewport (called on window/canvas resize).
    pub fn resize(&mut self, w: f32, h: f32) {
        self.viewport_w = w;
        self.viewport_h = h;
    }

    // -- resource management -----------------------------------------------

    /// Compile and store a shader from a manifest entry.
    pub fn add_shader(
        &mut self,
        name: &str,
        vs_src: &str,
        fs_src: &str,
        attr: &[String],
        unif: &[String],
    ) -> Result<(), String> {
        let s = Shader::compile(&self.gl, vs_src, fs_src, attr, unif)?;
        self.shaders.insert(name.to_string(), s);
        Ok(())
    }

    pub fn add_texture_rgba8(&mut self, name: &str, rgba: &[u8], w: u32, h: u32) {
        self.textures.insert(name.to_string(), GlTexture::from_rgba8(&self.gl, rgba, w, h));
    }

    pub fn shader(&self, name: &str) -> &Shader {
        self.shaders.get(name).unwrap_or_else(|| panic!("shader '{name}' not found"))
    }

    pub fn texture(&self, name: &str) -> &GlTexture {
        self.textures.get(name).unwrap_or_else(|| panic!("texture '{name}' not found"))
    }

    // -- frame begin -------------------------------------------------------

    /// Clear the framebuffer and set up state for the current frame.
    /// Matches `draw()` at `state.ts:455-465`.
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
            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
            gl.enable(glow::BLEND);
            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
            gl.depth_func(glow::LEQUAL);
            gl.depth_mask(true);
            gl.disable(glow::SCISSOR_TEST);
        }
    }

    // -- draw calls --------------------------------------------------------

    /// Draw a solid-colour rectangle.
    pub fn draw_rect(&self, model: &Mat4, camera: &Mat4, color: &[f32; 4], ignore_cam: bool) {
        let gl = &self.gl;
        let s = self.shader("solid");
        let proj = self.projection();

        s.bind(gl);
        s.uniform_mat4(gl, "projectionMatrix", &proj);
        s.uniform_mat4(gl, "cameraMatrix", if ignore_cam { &Mat4::IDENTITY } else { camera });
        s.uniform_mat4(gl, "modelMatrix", model);
        s.uniform_vec4(gl, "color", color);

        vertex_attrib_ptr_f32(gl, &self.quad.verts, s.attr("vertexPos"), 3, 0, 0);
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

    /// Draw a sprite from a sprite sheet.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_sprite(
        &self,
        model: &Mat4,
        camera: &Mat4,
        texture_name: &str,
        frame: f32,
        tile_set_size: &[f32; 2],
        ignore_cam: bool,
        ghost_alpha: f32,
    ) {
        let gl = &self.gl;
        let s = self.shader("imageSheet");
        let t = self.texture(texture_name);
        let proj = self.projection();

        s.bind(gl);
        t.bind(gl, 0);

        s.uniform_1i(gl, "texSampler", 0);
        s.uniform_mat4(gl, "projectionMatrix", &proj);
        s.uniform_mat4(gl, "cameraMatrix", if ignore_cam { &Mat4::IDENTITY } else { camera });
        s.uniform_mat4(gl, "modelMatrix", model);
        s.uniform_1f(gl, "tileIdFlat", frame);
        s.uniform_vec2(gl, "tileSetSize", tile_set_size);
        s.uniform_1f(gl, "useIsoDepth", 0.0);
        s.uniform_vec4(gl, "isoDepthCorners", &[0.0, 0.0, 0.0, 0.0]);
        s.uniform_1f(gl, "ghostAlpha", ghost_alpha);

        vertex_attrib_ptr_f32(gl, &self.quad.verts, s.attr("vertexPos"), 3, 0, 0);
        vertex_attrib_ptr_f32(gl, &self.quad.uv, s.attr("texCoord"), 2, 0, 0);
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
        let proj = self.projection();

        s.bind(gl);
        t.bind(gl, 0);

        s.uniform_1i(gl, "texSampler", 0);
        s.uniform_mat4(gl, "projectionMatrix", &proj);
        s.uniform_mat4(gl, "cameraMatrix", if ignore_cam { &Mat4::IDENTITY } else { camera });
        s.uniform_mat4(gl, "modelMatrix", model);
        s.uniform_vec4(gl, "color", color);
        s.uniform_vec4(gl, "outlineColor", outline_color);
        s.uniform_1f(gl, "outlineWidth", outline_width);
        s.uniform_1f(gl, "softEdge", 0.08);
        s.uniform_1f(gl, "spread", spread);
        s.uniform_vec2(gl, "atlasSize", atlas_size);
        s.uniform_1f(gl, "weight", weight);
        s.uniform_1f(gl, "gamma", gamma);

        vertex_attrib_ptr_f32(gl, vertex_buffer, s.attr("vertexPos"), 2, 16, 0);
        vertex_attrib_ptr_f32(gl, vertex_buffer, s.attr("texCoord"), 2, 16, 8);

        unsafe {
            gl.draw_arrays(glow::TRIANGLES, 0, vertex_count);
        }
    }

    /// Draw an isometric sprite with two-pass depth rendering (ghost + normal).
    #[allow(clippy::too_many_arguments)]
    pub fn draw_iso_sprite(
        &self,
        model: &Mat4,
        camera: &Mat4,
        texture_name: &str,
        frame: f32,
        tile_set_size: &[f32; 2],
        iso_depth_corners: &[f32; 4],
    ) {
        let gl = &self.gl;
        let s = self.shader("imageSheet");
        let t = self.texture(texture_name);
        let proj = self.projection();

        s.bind(gl);
        t.bind(gl, 0);

        s.uniform_1i(gl, "texSampler", 0);
        s.uniform_mat4(gl, "projectionMatrix", &proj);
        s.uniform_mat4(gl, "cameraMatrix", camera);
        s.uniform_mat4(gl, "modelMatrix", model);
        s.uniform_1f(gl, "tileIdFlat", frame);
        s.uniform_vec2(gl, "tileSetSize", tile_set_size);
        s.uniform_1f(gl, "useIsoDepth", 1.0);
        s.uniform_vec4(gl, "isoDepthCorners", iso_depth_corners);

        vertex_attrib_ptr_f32(gl, &self.quad.verts, s.attr("vertexPos"), 3, 0, 0);
        vertex_attrib_ptr_f32(gl, &self.quad.uv, s.attr("texCoord"), 2, 0, 0);
        self.quad.indices.bind(gl);

        unsafe {
            gl.enable(glow::DEPTH_TEST);

            // Two-pass ghost rendering: PASS 1 draws sprites behind terrain (ALWAYS,
            // depth_mask=off), PASS 2 draws normally (LEQUAL, depth_mask=on).
            // Must restore depth_mask(true) + depth_func(LEQUAL) after both passes.
            // Ghost pass: visible through occluding terrain
            s.uniform_1f(gl, "ghostAlpha", 0.4);
            gl.depth_func(glow::ALWAYS);
            gl.depth_mask(false);
            gl.draw_elements(
                glow::TRIANGLES,
                self.quad.index_count as i32,
                glow::UNSIGNED_SHORT,
                0,
            );

            // Normal pass: on top of terrain
            s.uniform_1f(gl, "ghostAlpha", 0.0);
            gl.depth_func(glow::LEQUAL);
            gl.depth_mask(false);
            gl.draw_elements(
                glow::TRIANGLES,
                self.quad.index_count as i32,
                glow::UNSIGNED_SHORT,
                0,
            );

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
        let proj = self.projection();

        s.bind(gl);
        s.uniform_mat4(gl, "projectionMatrix", &proj);
        s.uniform_mat4(gl, "cameraMatrix", camera);
        s.uniform_mat4(gl, "modelMatrix", model);
        s.uniform_vec4(gl, "color", color);

        vertex_attrib_ptr_f32(gl, vertex_buffer, s.attr("vertexPos"), 3, 0, 0);

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
        let proj = self.projection();

        s.bind(gl);
        s.uniform_mat4(gl, "projectionMatrix", &proj);
        s.uniform_mat4(gl, "cameraMatrix", camera);
        s.uniform_mat4(gl, "modelMatrix", model);
        s.uniform_vec4(gl, "color", color);

        vertex_attrib_ptr_f32(gl, vertex_buffer, s.attr("vertexPos"), 3, 0, 0);

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
        iso_matrix: &Mat4,
        tile_data_tex: &glow::Texture,
        tileset_name: &str,
        tile_set_size: &[f32; 2],
        tile_pixel_size: &[f32; 2],
        map_size: &[f32; 2],
        selected_tile: &[f32; 2],
        selection_begin: &[f32; 2],
        selection_mode: i32,
        selection_color: &[f32; 4],
        normal_matrix: &Mat3,
        ambient: &[f32; 3],
        light_dir: &[f32; 3],
        light_color: &[f32; 3],
        show_grid: bool,
        vertex_count: i32,
        vertex_buffer: &GlBuffer,
    ) {
        let gl = &self.gl;
        let s = self.shader("isoTilemap");
        let tset = self.texture(tileset_name);
        let proj = self.projection();

        s.bind(gl);

        // Interleaved vertex attribs at offsets 0, 12, 20, 24
        vertex_attrib_ptr_f32(gl, vertex_buffer, s.attr("vertexPos"), 3, 36, 0);
        vertex_attrib_ptr_f32(gl, vertex_buffer, s.attr("mapCoord"), 2, 36, 12);
        vertex_attrib_ptr_f32(gl, vertex_buffer, s.attr("tileId"), 1, 36, 20);
        vertex_attrib_ptr_f32(gl, vertex_buffer, s.attr("normal"), 3, 36, 24);

        // Texture 0: map data (raw GL texture handle)
        unsafe {
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(*tile_data_tex));
        }
        // Texture 1: tileset
        tset.bind(gl, 1);

        s.uniform_1i(gl, "mapData", 0);
        s.uniform_1i(gl, "tileSet", 1);
        s.uniform_mat4(gl, "projectionMatrix", &proj);
        s.uniform_mat4(gl, "cameraMatrix", camera);
        s.uniform_mat4(gl, "modelMatrix", model);
        s.uniform_mat4(gl, "isoMatrix", iso_matrix);
        s.uniform_vec2(gl, "tileSetSize", tile_set_size);
        s.uniform_vec2(gl, "tilePixelSize", tile_pixel_size);
        s.uniform_vec2(gl, "mapSize", map_size);
        s.uniform_vec2(gl, "selectedTile", selected_tile);
        s.uniform_vec2(gl, "selectionBegin", selection_begin);
        s.uniform_1i(gl, "selectionMode", selection_mode);
        s.uniform_vec4(gl, "selectionColor", selection_color);
        s.uniform_vec4(gl, "wallColor", &[0.3, 0.2, 0.15, 1.0]);
        s.uniform_1f(gl, "gridRadius", 3.0);
        s.uniform_1i(gl, "showGrid", if show_grid { 1 } else { 0 });
        s.uniform_vec3(gl, "gridColor", Vec3::ZERO);
        s.uniform_mat3(gl, "normalMatrix", normal_matrix);
        s.uniform_vec3(gl, "ambientColor", Vec3::from_array(*ambient));
        s.uniform_vec3(gl, "lightDirection", Vec3::from_array(*light_dir));
        s.uniform_vec3(gl, "lightColor", Vec3::from_array(*light_color));

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
        r.override_fragment("solid.frag", shaders::SOLID_FRAG);
        r.override_fragment("image.frag", shaders::IMAGE_FRAG);
        r.override_fragment("image_colorized.frag", shaders::IMAGE_COLORIZED_FRAG);
        r.override_fragment("iso_tilemap.frag", shaders::ISO_TILEMAP_FRAG);
        r.override_fragment("sheet.frag", shaders::SHEET_FRAG);
        r.override_fragment("sdf.frag", shaders::SDF_FRAG);
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
                    glow::DEPTH_COMPONENT16,
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
                    glow::DEPTH_ATTACHMENT,
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
                    glow::DEPTH_COMPONENT16,
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
