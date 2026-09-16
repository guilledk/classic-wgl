//! `Gfx` draw calls.

use glam::{Mat4, Vec3};
use glow::HasContext;

use crate::buffer::vertex_attrib_ptr_f32;
use crate::{
    Gfx, GlBuffer, IsoSpritePass, ModelMeshGpu, RenderSettings, Shader, SpriteRegion,
    MODEL_VERTEX_STRIDE, SHADOW_MAP_UNIT,
};

impl Gfx {
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
        texture_name: &str,
        region: SpriteRegion<'_>,
        iso_depth_corners: &[f32; 4],
        depth_base: f32,
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
        s.uniform_1f(gl, "depth_base", depth_base);
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
        texture_name: &str,
        region: SpriteRegion<'_>,
        iso_depth_corners: &[f32; 4],
        depth_base: f32,
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
            texture_name,
            region,
            iso_depth_corners,
            depth_base,
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

    /// Draw one mesh instance of a 3D glTF model (`mesh` shader).
    ///
    /// `model` is the node's full world transform in metres (placement ·
    /// `gltf_to_world` · node world); `world_matrix` is `iso_camera_matrix`.
    /// The vertex shader writes the canonical camera view depth, so the model
    /// occludes against terrain and sprites through the shared depth buffer.
    /// Lighting is the shared world-space block (sun + shadow + point lights).
    /// Scoped depth test like `draw_tilemap`: enabled, drawn, disabled.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_model(
        &self,
        model: &Mat4,
        camera: &Mat4,
        world_matrix: &Mat4,
        mesh: &ModelMeshGpu,
        texture_name: Option<&str>,
        base_color: &[f32; 4],
        settings: &RenderSettings,
    ) {
        let gl = &self.gl;
        let s = self.shader("mesh");

        s.bind(gl);
        self.bind_view(s, camera, model, false);
        s.uniform_mat4(gl, "world_matrix", world_matrix);
        s.uniform_vec2(gl, "depth_span", &settings.depth_span);
        s.uniform_1f(gl, "ppm", settings.ppm);

        match texture_name.and_then(|tn| self.textures.get(tn)) {
            Some(t) => {
                t.bind(gl, 0);
                s.uniform_1i(gl, "tex_sampler", 0);
                s.uniform_1f(gl, "use_texture", 1.0);
            }
            None => s.uniform_1f(gl, "use_texture", 0.0),
        }
        s.uniform_vec4(gl, "base_color", base_color);
        s.uniform_vec3(gl, "ambient_color", Vec3::from_array(settings.ambient));
        s.uniform_vec3(gl, "light_direction", Vec3::from_array(settings.light_dir));
        s.uniform_vec3(gl, "light_color", Vec3::from_array(settings.light_color));
        self.bind_shadow(s, settings);

        vertex_attrib_ptr_f32(gl, &mesh.vbo, s.attr("vertex_pos"), 3, MODEL_VERTEX_STRIDE, 0);
        vertex_attrib_ptr_f32(gl, &mesh.vbo, s.attr("normal"), 3, MODEL_VERTEX_STRIDE, 12);
        vertex_attrib_ptr_f32(gl, &mesh.vbo, s.attr("tex_coord"), 2, MODEL_VERTEX_STRIDE, 24);
        mesh.indices.bind(gl);

        unsafe {
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            gl.depth_mask(true);
            gl.draw_elements(glow::TRIANGLES, mesh.index_count as i32, glow::UNSIGNED_INT, 0);
            gl.disable(glow::DEPTH_TEST);
        }
    }
}
