//! Shader programs, uniform setters, and the named shader source registry.

use glam::{Mat3, Mat4, Vec3};
use glow::HasContext;
use std::cell::RefCell;
use std::collections::HashMap;

use crate::shaders;

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
            unif: &["model_matrix", "light_view_proj"],
        },
        BuiltinShader {
            name: "shadowSprite",
            vertex: "shadow_sprite.vert",
            fragment: "shadow_sprite.frag",
            attr: &["vertex_pos", "tex_coord"],
            unif: &[
                "model_matrix",
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
                "camera_matrix",
                "projection_matrix",
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
