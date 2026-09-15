//! Render-side helpers: UI label measurement, nav/tilemap GPU rebuilds, and
//! iso sprite frame/model/depth resolution.

use std::collections::HashMap;

use classic_core::components::{IsoSprite, NavMesh, SdfTextRender, Tilemap, UiNode};
use classic_core::math::{iso_view_depth, iso_world_pos, DEPTH_FAR, DEPTH_NEAR};
use classic_core::sdf_builder::build_sdf_glyph_buffer;
use classic_core::tilemap::{build_mesh, build_tile_texture, sample_height_mesh, PPM_TARGET};
use classic_core::types::FrameTable;
use classic_core::{RoleKind, Transform};
use classic_gfx::GlBuffer;
use glam::{Mat4, Vec2, Vec3, Vec4};
use glow::HasContext;

use crate::{Engine, ResolvedFrame, TilemapGpu};

impl Engine {
    /// Pre-measure all UI-managed SDF text labels so their UiNode.size
    /// is correct from the first frame. Call once after all UI init is complete.
    /// Without this, text inside button containers appears at wrong positions for
    /// one frame because spawn_sdf_text creates entities with
    /// UiNode.size = (max_width, 0) — the anchor math in position_children_of
    /// uses these stale dimensions until the render pass updates them.
    pub fn measure_all_ui_labels(&mut self) {
        let mut to_measure: Vec<(hecs::Entity, SdfTextRender, f32)> = Vec::new();
        for (e, (tf, sdf)) in self.world.query::<(&Transform, &SdfTextRender)>().iter() {
            if self.world.get::<&UiNode>(e).map(|n| n.parent.is_some()).unwrap_or(false) {
                to_measure.push((e, sdf.clone(), tf.scale.x));
            }
        }

        let mut changed = false;
        for (e, sdf, scale) in &to_measure {
            let Some(font) = self.sdf_fonts.get(&sdf.atlas_name) else { continue };
            let buf = build_sdf_glyph_buffer(font, &sdf.text, *scale, sdf.justify, 0.0);
            if let Ok(mut node) = self.world.get::<&mut UiNode>(*e) {
                if (node.size.x - buf.text_width).abs() > 0.1
                    || (node.size.y - buf.text_height).abs() > 0.1
                {
                    node.size.x = buf.text_width;
                    node.size.y = buf.text_height;
                    changed = true;
                }
            }
        }

        if changed {
            if let Some(ref mut ui) = self.ui {
                ui.refresh_layout(&mut self.world);
                ui.sync_colliders(&self.world, &mut self.physics);
            }
        }
    }

    /// Build and upload GPU resources for the nav mesh overlay.
    pub fn init_nav_mesh_render(&mut self) {
        let Some(nav_entity) = self.entity_by_role(RoleKind::NavMesh) else {
            return;
        };
        let (size_x, size_y, nav_data) = {
            let nav = match self.world.get::<&NavMesh>(nav_entity) {
                Ok(n) => n,
                Err(_) => return,
            };
            (nav.size_x, nav.size_y, nav.data.clone())
        };

        // A generated map has no nav grid until its guest uploads one (deferred
        // to `commit_terrain` → `rebuild_nav_gpu`).  A grid of the wrong size is
        // equally not ready.  Skip building the overlay until then; the nav mesh
        // overlay is rebuilt once the guest commits.
        if nav_data.len() != (size_x * size_y) as usize {
            return;
        }

        // Use parent tilemap's actual height data so nav tiles sit on terrain surface.
        let heights = self
            .entity_by_role(RoleKind::Tilemap)
            .and_then(|e| self.world.get::<&Tilemap>(e).ok())
            .map(|tm| tm.height_data.clone())
            .filter(|h| h.len() == (size_x as usize + 1) * (size_y as usize + 1))
            .unwrap_or_else(|| vec![1.0f32; (size_x as usize + 1) * (size_y as usize + 1)]);

        let Some(gfx) = self.gfx.as_mut() else { return };

        let (mesh_data, vcount) = build_mesh(size_x, size_y, &nav_data, &heights);
        let mesh_buf =
            GlBuffer::from_slice(&gfx.gl, glow::ARRAY_BUFFER, &mesh_data, glow::DYNAMIC_DRAW);

        let (tile_pixels, tw, th) = build_tile_texture(size_x, size_y, &nav_data);
        let tile_tex = Engine::upload_data_texture(&gfx.gl, &tile_pixels, tw, th);

        self.nav_gpu = Some(TilemapGpu { mesh_buf, vertex_count: vcount, tile_tex });

        // Add Transform to nav entity so render query (&Transform, &NavMesh) matches.
        // Borrow position + scale from parent tilemap.
        {
            let (pos, scl) = self
                .entity_by_role(RoleKind::Tilemap)
                .and_then(|e| self.world.get::<&Transform>(e).ok())
                .map(|tf| (tf.position, tf.scale))
                .unwrap_or((glam::Vec3::ZERO, glam::Vec3::ONE));
            let _ = self.world.insert_one(nav_entity, Transform::new(pos, scl));
        }
    }

    /// After height edits, recalculate nav mesh walkability and rebuild GPU resources.
    pub fn sync_nav_heights(&mut self) {
        let Some(nav_entity) = self.entity_by_role(RoleKind::NavMesh) else {
            return;
        };
        let Some(tm_entity) = self.entity_by_role(RoleKind::Tilemap) else {
            return;
        };
        let (sx, sy) = {
            let Ok(nav) = self.world.get::<&NavMesh>(nav_entity) else {
                return;
            };
            (nav.size_x, nav.size_y)
        };
        let hd = {
            let Ok(tm) = self.world.get::<&Tilemap>(tm_entity) else {
                return;
            };
            tm.height_data.clone()
        };
        let threshold = self.nav_slope_threshold;
        let stride = sx as usize + 1;
        let at = |tx: i32, ty: i32| -> f32 {
            hd.get(ty as usize * stride + tx as usize).copied().unwrap_or(0.0)
        };

        let mut changed = false;
        if let Ok(mut nav) = self.world.get::<&mut NavMesh>(nav_entity) {
            for ty in 0..sy {
                for tx in 0..sx {
                    let idx = (ty * sx + tx) as usize;
                    let h = at(tx, ty);
                    let mut walkable: u32 = 1;
                    // The four orthogonal neighbours.  The `ty + 1` bound
                    // previously compared against `sx`, so on any non-square
                    // map the southern edge was tested against the wrong
                    // dimension — invisible while the demo map was both
                    // square and perfectly flat.
                    let neighbours = [
                        (tx > 0).then(|| at(tx - 1, ty)),
                        (tx + 1 < sx).then(|| at(tx + 1, ty)),
                        (ty > 0).then(|| at(tx, ty - 1)),
                        (ty + 1 < sy).then(|| at(tx, ty + 1)),
                    ];
                    for n in neighbours.into_iter().flatten() {
                        if (h - n).abs() > threshold {
                            walkable = 0;
                        }
                    }
                    if nav.data.len() > idx {
                        if nav.data[idx] != walkable {
                            changed = true;
                        }
                        nav.data[idx] = walkable;
                    }
                }
            }
        }
        if changed {
            self.rebuild_nav_gpu();
        }
        self.refresh_nav_snapshot();
    }

    /// Upload RGBA `pixels` as a `NEAREST`-filtered `CLAMP_TO_EDGE` 2D texture.
    /// Used for tilemap data and nav-mesh data textures.
    pub(crate) fn upload_data_texture(
        gl: &std::rc::Rc<glow::Context>,
        pixels: &[u8],
        width: u32,
        height: u32,
    ) -> glow::Texture {
        let tex = unsafe { gl.create_texture() }.expect("create data texture");
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
                glow::PixelUnpackData::Slice(Some(pixels)),
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
        }
        tex
    }

    /// Rebuild nav mesh GPU buffers from current NavMesh component data.
    pub fn rebuild_nav_gpu(&mut self) {
        let Some(nav_entity) = self.entity_by_role(RoleKind::NavMesh) else {
            return;
        };
        let (sx, sy, data) = {
            let nav = match self.world.get::<&NavMesh>(nav_entity) {
                Ok(n) => n,
                Err(_) => return,
            };
            (nav.size_x, nav.size_y, nav.data.clone())
        };
        // Take the terrain's own heights, exactly as `init_nav_mesh_render`
        // does.  Rebuilding the overlay on a flat grid instead left it
        // detached from the surface after any nav edit — unnoticeable on the
        // flat demo map, glaring over a crater field.
        let heights = self
            .entity_by_role(RoleKind::Tilemap)
            .and_then(|e| self.world.get::<&Tilemap>(e).ok())
            .map(|tm| tm.height_data.clone())
            .filter(|h| h.len() == (sx as usize + 1) * (sy as usize + 1))
            .unwrap_or_else(|| vec![1.0f32; (sx as usize + 1) * (sy as usize + 1)]);
        let Some(gfx) = self.gfx.as_mut() else { return };

        let (mesh_data, vcount) = build_mesh(sx, sy, &data, &heights);
        let mesh_buf =
            GlBuffer::from_slice(&gfx.gl, glow::ARRAY_BUFFER, &mesh_data, glow::DYNAMIC_DRAW);
        let (tile_pixels, tw, th) = build_tile_texture(sx, sy, &data);
        let tile_tex = Engine::upload_data_texture(&gfx.gl, &tile_pixels, tw, th);
        self.nav_gpu = Some(TilemapGpu { mesh_buf, vertex_count: vcount, tile_tex });
    }

    /// Rebuild the tilemap mesh from current data + heights and re-upload to GPU.
    pub fn rebuild_tilemap_mesh(&mut self) {
        let Some(tm_entity) = self.entity_by_role(RoleKind::Tilemap) else {
            classic_core::cl_warn!(
                classic_core::instrument::Chan::Editor,
                "rebuild_tilemap_mesh: no Tilemap-role entity"
            );
            return;
        };
        let (size_x, size_y, tiles, heights) = {
            let tm = match self.world.get::<&Tilemap>(tm_entity) {
                Ok(t) => t,
                Err(_) => {
                    classic_core::cl_warn!(
                        classic_core::instrument::Chan::Editor,
                        "rebuild_tilemap_mesh: no Tilemap on the Tilemap-role entity"
                    );
                    return;
                }
            };
            (tm.size_x, tm.size_y, tm.data.clone(), tm.height_data.clone())
        };

        let gfx = match self.gfx.as_mut() {
            Some(g) => g,
            None => {
                classic_core::cl_warn!(
                    classic_core::instrument::Chan::Editor,
                    "rebuild_tilemap_mesh: gfx not initialized"
                );
                return;
            }
        };

        let (mesh_data, vcount) = build_mesh(size_x, size_y, &tiles, &heights);
        let mesh_buf =
            GlBuffer::from_slice(&gfx.gl, glow::ARRAY_BUFFER, &mesh_data, glow::DYNAMIC_DRAW);

        let (tile_pixels, tw, th) = build_tile_texture(size_x, size_y, &tiles);
        let tile_tex = Engine::upload_data_texture(&gfx.gl, &tile_pixels, tw, th);

        let entity_name = self.debug_name(tm_entity);
        if let Some(gpu) = self.tilemap_gpu.get_mut(&entity_name) {
            gpu.mesh_buf = mesh_buf;
            gpu.vertex_count = vcount;
            gpu.tile_tex = tile_tex;
            classic_core::cl_info!(
                classic_core::instrument::Chan::Editor,
                "rebuild_tilemap_mesh: {vcount} vertices uploaded for '{entity_name}'"
            );
        }
    }

    /// Resolve a `frame_name` for a texture through its packed-atlas frame
    /// table, returning the bound sheet texture name, the normalized UV rect,
    /// the frame's content pixel size, its trim/anchor metadata, and any
    /// per-sheet normal/depth companion GL texture names.  Returns `None` if
    /// the texture has no frame table or the name is unknown.
    pub(crate) fn resolve_frame(
        tables: &HashMap<String, FrameTable>,
        texture: &str,
        frame_name: &str,
    ) -> Option<ResolvedFrame> {
        let table = tables.get(texture)?;
        let frame = table.frames.get(frame_name)?;
        let sheet = table.sheets.get(frame.sheet as usize)?;
        let uv = table.uv_rect(frame)?;
        let (normal_tex, depth_name) =
            table.companions.get(frame.sheet as usize).cloned().unwrap_or((None, None));
        let depth_tex = depth_name;
        Some(ResolvedFrame {
            sheet_name: sheet.name.clone(),
            uv_rect: uv,
            size: [frame.rect[2] as f32, frame.rect[3] as f32],
            source_size: frame.source_size,
            trim_offset: frame.trim_offset,
            anchor: frame.anchor,
            normal_tex,
            depth_tex,
        })
    }

    /// Compute the anchor to use when drawing a (possibly trimmed) packed
    /// frame.  A packer-provided anchor wins; otherwise the component's anchor
    /// (a `[0..1]` ratio of the original source cell) is translated into
    /// trimmed-frame space using `source_size` + `trim_offset`, so the
    /// ground-contact point stays put when empty space is trimmed away.
    pub(crate) fn effective_anchor(component_anchor: Vec2, frame: &ResolvedFrame) -> Vec2 {
        if let Some(a) = frame.anchor {
            return Vec2::new(a[0], a[1]);
        }
        if frame.source_size[0] == 0 || frame.source_size[1] == 0 {
            return component_anchor;
        }
        let cw = frame.source_size[0] as f32;
        let ch = frame.source_size[1] as f32;
        let bx0 = frame.trim_offset[0] as f32;
        let by0 = frame.trim_offset[1] as f32;
        let fw = frame.size[0].max(1.0);
        let fh = frame.size[1].max(1.0);
        Vec2::new((component_anchor.x * cw - bx0) / fw, (component_anchor.y * ch - by0) / fh)
    }

    /// Compute the world-metre model matrix for an IsoSprite.
    ///
    /// The quad is authored in **Blender-world metres**: its width runs along
    /// the isometric "right" direction `(1/√2, −1/√2, 0)` and its height runs
    /// down world −Z, so under the orthographic camera it rasterises to the
    /// same screen-aligned rectangle the old billboard drew.  Lighting and
    /// shadowing operate on true standing geometry (no `sprite_anchor`
    /// unproject).
    ///
    /// `tex_dim` is the source-cell quad size in pixels; `anchor_px` is the
    /// ground-contact point in that same pixel space (the quad is shifted so it
    /// lands on the sprite's position).
    pub(crate) fn compute_iso_sprite_model(
        iso_sprite: &IsoSprite,
        sprite_tf: &Transform,
        tilemap_tf: &Transform,
        tilemap: &Tilemap,
        tex_dim: (f32, f32),
        anchor_px: Vec2,
    ) -> Mat4 {
        let h = sample_height_mesh(
            &tilemap.height_data,
            tilemap.size_x,
            tilemap.size_y,
            sprite_tf.position.x,
            sprite_tf.position.y,
        );
        // `frame_offset` is Blender-world metres: horizontal drift in x/y, the
        // altitude in z (see `load_animation_offsets`).
        let altitude = iso_sprite.frame_offset.z;
        let drift = Vec3::new(iso_sprite.frame_offset.x, iso_sprite.frame_offset.y, 0.0);

        // Ground anchor in Blender-world metres (terrain height + altitude).
        let world_pos = iso_world_pos(sprite_tf.position.x, sprite_tf.position.y, h + altitude)
            + drift
            + tilemap_tf.position;

        // Quad dimensions in metres: source-cell pixels map to metres at
        // `PPM_TARGET` px/m along both screen axes.
        let w = tex_dim.0 * sprite_tf.scale.x / PPM_TARGET;
        let hh = tex_dim.1 * sprite_tf.scale.y / PPM_TARGET;

        // Anchor in normalized `[0,1]` quad space.
        let ua = anchor_px.x / tex_dim.0.max(f32::EPSILON);
        let wa = anchor_px.y / tex_dim.1.max(f32::EPSILON);

        // Billboard basis: width along `(1/√2, −1/√2, 0)`, height down world
        // −Z (the sheet's v=0 top lands at higher z, v=1 feet at the anchor).
        let billboard = Mat4::from_cols(
            Vec4::new(std::f32::consts::FRAC_1_SQRT_2, -std::f32::consts::FRAC_1_SQRT_2, 0.0, 0.0),
            Vec4::new(0.0, 0.0, -1.0, 0.0),
            Vec4::new(std::f32::consts::FRAC_1_SQRT_2, std::f32::consts::FRAC_1_SQRT_2, 0.0, 0.0),
            Vec4::W,
        );

        Mat4::from_translation(world_pos)
            * billboard
            * Mat4::from_scale(Vec3::new(w, hh, 1.0))
            * Mat4::from_translation(Vec3::new(-ua, -wa, 0.0))
    }

    /// Camera view depth of a world point, normalised to **window space**
    /// `[0, 1]` (`0` = nearest, `1` = farthest).
    pub(crate) fn world_depth(world: Vec3) -> f32 {
        (DEPTH_NEAR - iso_view_depth(world)) / (DEPTH_NEAR - DEPTH_FAR)
    }

    /// Compute iso depth corners for the footprint, in **window space** `[0, 1]`.
    /// `pos` is the sprite's tile position (x/y in tiles, z in metres).
    pub(crate) fn compute_iso_depth_corners(pos: Vec3, footprint: &[glam::Vec2]) -> [f32; 4] {
        let base_depth = Self::world_depth(iso_world_pos(pos.x, pos.y, pos.z));
        let default_footprint = [
            glam::Vec2::new(0.5, -0.5),
            glam::Vec2::new(0.5, 0.5),
            glam::Vec2::new(-0.5, 0.5),
            glam::Vec2::new(-0.5, -0.5),
        ];
        let footprint = if footprint.len() >= 4 { &footprint[..4] } else { &default_footprint };

        let mut raw_depths = [0.0f32; 4];
        for i in 0..4 {
            let pt = &footprint[i];
            let world = iso_world_pos(pos.x + pt.x, pos.y + pt.y, pos.z);
            raw_depths[i] = Self::world_depth(world).min(base_depth);
        }

        let min_fp = raw_depths.iter().cloned().fold(f32::MAX, f32::min);

        // shader layout: x=SW, y=SE, z=NW, w=NE
        // footprint order: [NE, SE, SW, NW]
        [min_fp, min_fp, raw_depths[3], raw_depths[0]]
    }
}
