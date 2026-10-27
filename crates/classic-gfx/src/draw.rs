//! `Gfx` draw calls.

use glam::{Mat4, Vec3};
use glow::HasContext;

use crate::buffer::vertex_attrib_ptr_f32;
use crate::{
    EmissiveRect, Gfx, GlBuffer, IsoSpritePass, ModelMeshGpu, RenderSettings, Shader, SpriteRegion,
    TeamTint, TileBlend, BLEND_LUT_UNIT, MODEL_VERTEX_STRIDE, SHADOW_MAP_UNIT,
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

    /// Set the screen-tile light grid uniforms on a lit fragment shader
    /// (`light_grid_on`, `light_grid_map`).  `view` is the draw's `(camera,
    /// world_matrix, ppm)`; the grid is used only when it rasterises with the
    /// transform the grid was binned for, else (and for `None`: screen-space
    /// draws) the shader runs the full light loop.
    pub(crate) fn bind_light_grid(&self, s: &Shader, view: Option<(&Mat4, &Mat4, f32)>) {
        let gl = &self.gl;
        let map = view.and_then(|(camera, world_matrix, ppm)| {
            let clip = crate::clip_from_world(&self.projection(), camera, world_matrix, ppm);
            self.light_grid.map_for(&clip, self.pass_target.get())
        });
        match map {
            Some(map) => {
                self.light_grid.bind(gl);
                s.uniform_vec4(gl, "light_grid_map", &map);
                s.uniform_1f(gl, "light_grid_on", 1.0);
            }
            None => s.uniform_1f(gl, "light_grid_on", 0.0),
        }
    }

    /// Bind the directional shadow map and set the sampling uniforms on a lit
    /// shader (`isoTilemap` / `imageSheet`).  When `settings.shadow` is `None`,
    /// `use_shadow` is cleared to 0 (the shaders skip the term).
    pub(crate) fn bind_shadow(&self, s: &Shader, settings: &RenderSettings) {
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
            None => self.bind_no_shadow(s),
        }
    }

    /// Switch a lit shader's sun shadow off.  Its `sampler2DShadow` must
    /// still read a compare-mode depth texture on its own unit (WebGL 2
    /// rejects the draw otherwise, and unit 0 holds the albedo `sampler2D`),
    /// so the 1x1 placeholder is bound there.
    fn bind_no_shadow(&self, s: &Shader) {
        let gl = &self.gl;
        unsafe {
            gl.active_texture(glow::TEXTURE0 + SHADOW_MAP_UNIT);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.shadow_placeholder));
            gl.active_texture(glow::TEXTURE0);
        }
        s.uniform_1i(gl, "shadow_map", SHADOW_MAP_UNIT as i32);
        s.uniform_1f(gl, "use_shadow", 0.0);
        s.uniform_1f(gl, "shadow_debug", 0.0);
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
        self.bind_no_shadow(s);
        self.bind_light_grid(s, None);

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
        s.uniform_vec2(gl, "depth_span", &settings.depth_span);

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
        // The one-sprite path draws legacy (global-window) depth maps only.
        s.uniform_1f(gl, "depth_scale", 1.0);
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
        self.bind_light_grid(s, Some((camera, world_matrix, settings.ppm)));

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
        blend: Option<&TileBlend>,
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
        self.bind_light_grid(s, Some((camera, world_matrix, settings.ppm)));

        // The sampler always points at its own unit so it never aliases the
        // tile data/tileset units, whether or not a table is bound.
        s.uniform_1i(gl, "blend_lut", BLEND_LUT_UNIT as i32);
        match blend {
            Some(b) => {
                unsafe {
                    gl.active_texture(glow::TEXTURE0 + BLEND_LUT_UNIT);
                    gl.bind_texture(glow::TEXTURE_2D, Some(b.lut));
                }
                s.uniform_1i(gl, "blend_enabled", 1);
                s.uniform_1f(gl, "blend_width", b.width);
                s.uniform_1f(gl, "blend_max_lod", b.max_lod);
                s.uniform_1f(gl, "blend_noise_scale", b.noise_scale);
                s.uniform_1f(gl, "blend_noise_strength", b.noise_strength);
                s.uniform_1f(gl, "blend_height_strength", b.height_strength);
            }
            None => s.uniform_1i(gl, "blend_enabled", 0),
        }

        unsafe {
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            gl.depth_mask(true);
            gl.draw_arrays(glow::TRIANGLES, 0, vertex_count);
            gl.disable(glow::DEPTH_TEST);
        }
    }

    /// Start a run of 3D model draws ([`Gfx::draw_model_instance`]), all with
    /// the same camera, `world_matrix` (`iso_camera_matrix`) and lighting.
    ///
    /// The per-pass state — projection/camera/world matrices, depth span,
    /// sun, shadow map and light grid — is set once here on both model
    /// programs (`mesh` and `meshAlphaTest`; GL keeps uniforms per program),
    /// and the scoped depth test (`LEQUAL`, depth writes) is switched on
    /// until [`Gfx::end_model_draws`].  Each instance then sets only what is
    /// its own.  The pixels are those of drawing each instance with every
    /// uniform set.
    pub fn begin_model_draws(
        &self,
        camera: &Mat4,
        world_matrix: &Mat4,
        settings: &RenderSettings,
    ) -> ModelDraws<'_> {
        let gl = &self.gl;
        let programs = [self.shader("mesh"), self.shader("meshAlphaTest")];
        // The grid lookup is the same for every instance: compute it once.
        let clip = crate::clip_from_world(&self.projection(), camera, world_matrix, settings.ppm);
        let grid = self.light_grid.map_for(&clip, self.pass_target.get());
        for s in programs {
            s.bind(gl);
            self.bind_view(s, camera, &Mat4::IDENTITY, false);
            s.uniform_mat4(gl, "world_matrix", world_matrix);
            s.uniform_vec2(gl, "depth_span", &settings.depth_span);
            s.uniform_1f(gl, "ppm", settings.ppm);
            s.uniform_1i(gl, "tex_sampler", 0);
            s.uniform_vec3(gl, "ambient_color", Vec3::from_array(settings.ambient));
            s.uniform_vec3(gl, "light_direction", Vec3::from_array(settings.light_dir));
            s.uniform_vec3(gl, "light_color", Vec3::from_array(settings.light_color));
            self.bind_shadow(s, settings);
            match grid {
                Some(map) => {
                    self.light_grid.bind(gl);
                    s.uniform_vec4(gl, "light_grid_map", &map);
                    s.uniform_1f(gl, "light_grid_on", 1.0);
                }
                None => s.uniform_1f(gl, "light_grid_on", 0.0),
            }
        }
        unsafe {
            gl.active_texture(glow::TEXTURE0);
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            gl.depth_mask(true);
        }
        ModelDraws { programs, bound: std::cell::Cell::new(Some(1)) }
    }

    /// Draw one mesh instance of a 3D glTF model inside a
    /// [`Gfx::begin_model_draws`] run.
    ///
    /// `model` is the node's full world transform in metres (placement ·
    /// `gltf_to_world` · node world).  The vertex shader writes the canonical
    /// camera view depth, so the model occludes against terrain and sprites
    /// through the shared depth buffer.  Lighting is the shared world-space
    /// block (sun + shadow + point lights).
    ///
    /// `team` tints the texels of the mesh's team region (only when it is
    /// textured); `None` draws exactly as an untinted mesh.  `emissive` are
    /// the mesh's self-lit rects (strengths already scaled by the model's
    /// multiplier, at most [`crate::MAX_EMISSIVE`] used); empty = no glow.
    /// `alpha_test` selects the `meshAlphaTest` program (cuts texels with
    /// alpha below 0.01); without it nothing is discarded and the draw keeps
    /// early depth testing.  Programs switch only when `alpha_test` does.
    /// `ghost: false` marks its pixels in the model target's ghost mask, so
    /// the ghost composite never draws them over what occludes them.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_model_instance(
        &self,
        run: &ModelDraws<'_>,
        model: &Mat4,
        mesh: &ModelMeshGpu,
        texture_name: Option<&str>,
        base_color: &[f32; 4],
        team: Option<&TeamTint>,
        emissive: &[EmissiveRect],
        alpha_test: bool,
        ghost: bool,
    ) {
        let gl = &self.gl;
        let which = alpha_test as usize;
        let s = run.programs[which];
        if run.bound.get() != Some(which) {
            s.bind(gl);
            run.bound.set(Some(which));
        }
        s.uniform_mat4(gl, "model_matrix", model);
        match texture_name.and_then(|tn| self.textures.get(tn)) {
            Some(t) => {
                t.bind(gl, 0);
                s.uniform_1f(gl, "use_texture", 1.0);
            }
            None => s.uniform_1f(gl, "use_texture", 0.0),
        }
        s.uniform_vec4(gl, "base_color", base_color);
        s.uniform_1f(gl, "no_ghost", if ghost { 0.0 } else { 1.0 });
        match team {
            Some(t) => {
                s.uniform_1f(gl, "use_team", 1.0);
                s.uniform_vec4(gl, "team_rect", &t.rect);
                s.uniform_vec3(gl, "team_tint", Vec3::from_array(t.tint));
            }
            None => s.uniform_1f(gl, "use_team", 0.0),
        }
        let emissive = &emissive[..emissive.len().min(crate::MAX_EMISSIVE)];
        s.uniform_1f(gl, "emissive_count", emissive.len() as f32);
        if !emissive.is_empty() {
            let mut rects = [[0.0f32; 4]; crate::MAX_EMISSIVE];
            let mut strengths = [0.0f32; crate::MAX_EMISSIVE];
            for (i, e) in emissive.iter().enumerate() {
                rects[i] = e.rect;
                strengths[i] = e.strength;
            }
            s.uniform_vec4_array(gl, "emissive_rect", &rects[..emissive.len()]);
            s.uniform_1f_array(gl, "emissive_strength", &strengths[..emissive.len()]);
        }

        vertex_attrib_ptr_f32(gl, &mesh.vbo, s.attr("vertex_pos"), 3, MODEL_VERTEX_STRIDE, 0);
        vertex_attrib_ptr_f32(gl, &mesh.vbo, s.attr("normal"), 3, MODEL_VERTEX_STRIDE, 12);
        vertex_attrib_ptr_f32(gl, &mesh.vbo, s.attr("tex_coord"), 2, MODEL_VERTEX_STRIDE, 24);
        mesh.indices.bind(gl);
        crate::perf::count("model_draw_calls", 1);
        unsafe {
            gl.draw_elements(glow::TRIANGLES, mesh.index_count as i32, glow::UNSIGNED_INT, 0);
        }
    }

    /// End a [`Gfx::begin_model_draws`] run (switches the depth test back
    /// off).
    pub fn end_model_draws(&self, _run: ModelDraws<'_>) {
        unsafe { self.gl.disable(glow::DEPTH_TEST) };
    }
}

/// A run of model draws ([`Gfx::begin_model_draws`]): the two model programs
/// with their pass state set, and which one is bound.
pub struct ModelDraws<'a> {
    programs: [&'a Shader; 2],
    bound: std::cell::Cell<Option<usize>>,
}
