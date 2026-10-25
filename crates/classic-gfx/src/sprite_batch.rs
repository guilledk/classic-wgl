//! Instanced isometric sprites.
//!
//! Every iso sprite becomes one [`IsoSpriteInstance`] (its model matrix,
//! depth corners, packed-atlas region, tint, ghost group, ...) in a per-frame
//! instance buffer, and the normal, ghost and shadow passes draw runs of them
//! with one `drawElementsInstanced` each instead of one draw per sprite.
//!
//! - **Order.**  [`plan_batches`] keeps the result identical to drawing the
//!   sprites one by one in the render-list order: an instance only joins an
//!   earlier batch with its key when its screen rect overlaps nothing drawn
//!   in between, so every overlapping pair keeps its relative order (blended
//!   edges, depth ties and the ghost-id writes all depend on it).  Sprites on
//!   one atlas page (same colour / normal / depth textures) share a key.
//! - **Ghost groups.**  The ghost pass must skip pixels a sprite's own group
//!   (a vehicle's parts) already covers.  The per-draw stencil reference this
//!   used to need split every group into its own draw, so the group test now
//!   runs in the shader: [`Gfx::begin_ghost_ids`] blits the scene depth into
//!   the [`GhostIdTarget`] and the instances draw their group id into it with
//!   the normal pass's exact depth and discard rules (the stencil
//!   `REPLACE` semantics), and the ghost pass discards where the id under it
//!   is its own.  The normal pass still writes stencil (a constant 0) so the
//!   model ghost composite's `NOTEQUAL 255` test is unchanged.  Where the
//!   depth blit is unsupported (a multisampled main framebuffer, or a blit
//!   error) the plan splits batches by group and the passes use the stencil
//!   as before.
//! - **Shadow casters** write depth only and commute, so they batch by
//!   texture alone ([`plan_shadow_batches`]).
//!
//! WebGL 2 has no base instance, so each batch re-points the instance
//! attributes at its first record (as the particle rings do), inside the
//! batcher's own VAO so the divisors never leak into the shared one.

use std::cell::Cell;

use glam::Mat4;
use glow::HasContext;

use crate::{Gfx, IsoSpritePass, RenderSettings, SpriteRegion};

/// Attribute locations of the per-instance `vec4`s (after `vertex_pos` = 0
/// and `tex_coord` = 1), shared by every instanced sprite program.
pub const SPRITE_INSTANCE_ATTRS: [&str; 15] = [
    "vertex_pos",
    "tex_coord",
    "i_m0",
    "i_m1",
    "i_m3",
    "i_corners",
    "i_uv",
    "i_content",
    "i_tint",
    "i_misc",
    "i_outline",
    "i_uv_b",
    "i_content_b",
    "i_interp",
    "i_quad",
];

/// Number of per-instance `vec4` attributes.
const INSTANCE_VEC4S: u32 = 13;
/// First per-instance attribute location.
const FIRST_INSTANCE_LOC: u32 = 2;

/// Bytes per [`IsoSpriteInstance`].
pub const SPRITE_INSTANCE_BYTES: usize = std::mem::size_of::<IsoSpriteInstance>();

/// Texture unit the ghost pass reads the ghost-id target from (clear of every
/// scene binding: sprites 0-2, shadow 3, model composite 4-5, particles 6,
/// bloom 7-8, smoke noise 9, particle composite 10, light grid 11).
pub const GHOST_ID_UNIT: u32 = 12;

/// Texture units of a direction blend's direction-B colour, depth and normal
/// sheets (A uses 0-2; 13-15 are clear of every other binding).
pub const BLEND_UNITS: [u32; 3] = [13, 14, 15];

/// The ghost-pass alpha (`classic-gfx` §14).
pub const GHOST_ALPHA: f32 = 0.4;

/// One iso sprite's per-instance data (13 `vec4`s, std430-like packing).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct IsoSpriteInstance {
    /// Model matrix columns x, y and w (`compute_iso_sprite_model`); the
    /// quad has `z = 0`, so its z column never contributes.
    pub m: [[f32; 4]; 3],
    /// Window-space depth corners `[bottom-left, bottom-right, top-left,
    /// top-right]` (`compute_iso_depth_corners`).
    pub corners: [f32; 4],
    /// Sheet UV rect `[u0, v0, u1, v1]` of the frame's trimmed content.
    pub uv: [f32; 4],
    /// `trim_offset.xy, content_size.xy` in source pixels (with `outline.zw`
    /// = `source_size`: the shader divides them exactly as the per-draw path
    /// does, so both sample the same texels).
    pub content: [f32; 4],
    /// `tint.rgb, depth_scale` (window units per `gray − 0.5` of the depth
    /// map: `1` legacy global, `2R / (near − far)` anchor-relative).
    pub tint: [f32; 4],
    /// `depth_base, ghost_group, selected (0/1), region mode` (0 packed, 1
    /// grid, 2 direction blend).
    pub misc: [f32; 4],
    /// `outline_delta.xy` (sheet-UV silhouette offset), then `source_size.xy`
    /// (packed), or direction A's anchor in its source cell, in pixels (blend).
    pub outline: [f32; 4],
    /// Direction blend only: B's sheet UV rect.
    pub uv_b: [f32; 4],
    /// Direction blend only: B's `trim_offset.xy, content_size.xy`.
    pub content_b: [f32; 4],
    /// Direction blend only: B's anchor in its source cell (px), then the
    /// drawn heading minus A's and minus B's (tile-space radians).
    pub interp: [f32; 4],
    /// Direction blend only: the drawn quad `x0, y0, w, h` in pixels from the
    /// anchor (x right, y down).
    pub quad: [f32; 4],
}

impl IsoSpriteInstance {
    /// Build an instance.  `tex_size` is the colour sheet's pixel size (the
    /// grid region and the outline offset need it).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        model: &Mat4,
        region: &SpriteRegion<'_>,
        tex_size: (u32, u32),
        corners: &[f32; 4],
        depth_base: f32,
        depth_scale: f32,
        ghost_group: u32,
        tint: &[f32; 3],
        selected: bool,
        outline_radius: f32,
    ) -> Self {
        let (uv, content, source, grid) = region_uv(region);
        Self {
            m: {
                let c = model.to_cols_array_2d();
                [c[0], c[1], c[3]]
            },
            corners: *corners,
            uv,
            content,
            tint: [tint[0], tint[1], tint[2], depth_scale],
            misc: [
                depth_base,
                ghost_group as f32,
                if selected { 1.0 } else { 0.0 },
                if grid { 1.0 } else { 0.0 },
            ],
            outline: {
                let d = outline_delta(region, tex_size, outline_radius);
                [d[0], d[1], source[0], source[1]]
            },
            ..Default::default()
        }
    }

    /// Turn this instance into a direction blend (`misc.w = 2`): it draws
    /// `b.quad` and warps the region it was built with (direction A) and
    /// `b`'s direction B to the drawn heading (`sheet.frag#INTERP`).
    pub fn blend(mut self, b: &DirBlendRegion) -> Self {
        self.misc[3] = 2.0;
        self.outline[2] = b.anchor_a_px[0];
        self.outline[3] = b.anchor_a_px[1];
        self.uv_b = b.uv_b;
        self.content_b = [b.trim_b[0], b.trim_b[1], b.content_b[0], b.content_b[1]];
        self.interp = [b.anchor_b_px[0], b.anchor_b_px[1], b.dtheta_a, b.dtheta_b];
        self.quad = b.quad;
        self
    }

    /// Whether this instance is a direction blend.
    pub fn is_blend(&self) -> bool {
        self.misc[3] > 1.5
    }

    /// The instance's ghost group.
    pub fn ghost_group(&self) -> u32 {
        self.misc[1] as u32
    }
}

/// The direction-B half of a direction-blend instance
/// ([`IsoSpriteInstance::blend`]).  Pixel values are in each frame's source
/// cell; angles are tile-space radians (drawn heading minus the frame's).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DirBlendRegion {
    pub uv_b: [f32; 4],
    pub trim_b: [f32; 2],
    pub content_b: [f32; 2],
    pub anchor_a_px: [f32; 2],
    pub anchor_b_px: [f32; 2],
    pub dtheta_a: f32,
    pub dtheta_b: f32,
    /// The drawn quad `x0, y0, w, h` in pixels from the anchor (y down).
    pub quad: [f32; 4],
}

/// A region as instance data: `(uv, content, source, grid)`.
///
/// - packed: the sheet UV rect, `[trim_offset, content_size]` and the source
///   size (the shader divides them exactly as the per-draw path does);
/// - grid (`grid = true`): `uv = [frame, tile_set_size.xy, 0]`, run through
///   the per-draw path's own `tileUv`, so both sample the same texels.
pub fn region_uv(region: &SpriteRegion<'_>) -> ([f32; 4], [f32; 4], [f32; 2], bool) {
    match region {
        SpriteRegion::Grid { frame, tile_set_size } => (
            [*frame, tile_set_size[0], tile_set_size[1], 0.0],
            [0.0, 0.0, 1.0, 1.0],
            [1.0, 1.0],
            true,
        ),
        SpriteRegion::Uv { uv_rect, trim_offset, source_size, content_size } => (
            **uv_rect,
            [trim_offset[0], trim_offset[1], content_size[0], content_size[1]],
            **source_size,
            false,
        ),
    }
}

/// The silhouette outline offset in sheet UV: `outline_radius` content
/// pixels (`bind_iso_sprite`'s formula).
pub fn outline_delta(region: &SpriteRegion<'_>, tex_size: (u32, u32), radius: f32) -> [f32; 2] {
    match region {
        SpriteRegion::Grid { .. } => {
            [radius / tex_size.0.max(1) as f32, radius / tex_size.1.max(1) as f32]
        }
        SpriteRegion::Uv { uv_rect, content_size, .. } => {
            let ext_x = (uv_rect[2] - uv_rect[0]).abs().max(1e-6);
            let ext_y = (uv_rect[3] - uv_rect[1]).abs().max(1e-6);
            [radius * ext_x / content_size[0].max(1.0), radius * ext_y / content_size[1].max(1.0)]
        }
    }
}

/// A screen-space bounding rect `[x0, y0, x1, y1]` (any consistent space).
pub type ScreenRect = [f32; 4];

fn overlaps(a: &ScreenRect, b: &ScreenRect) -> bool {
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}

fn union(a: &ScreenRect, b: &ScreenRect) -> ScreenRect {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}

/// One planned draw: `key` and the instance indices (into the caller's
/// list) it draws, in draw order.
#[derive(Clone, Debug, PartialEq)]
pub struct PlannedBatch {
    pub key: u32,
    pub instances: Vec<u32>,
    bounds: ScreenRect,
}

/// How many batches back [`plan_batches`] looks for a match before opening a
/// new one (bounds the planning cost; a miss only costs an extra draw).
pub const PLAN_LOOKBACK: usize = 64;

/// Group instances (in draw order, with a batch `key` and a screen `rect`
/// each) into batches whose draws reproduce the one-by-one order: instance
/// `i` joins the latest batch with its key unless its rect overlaps any
/// instance drawn in a later batch, in which case it opens a new one.
///
/// Every pair of overlapping instances keeps its relative order, so the
/// blended colour, depth and ghost ids come out exactly as before; only
/// non-overlapping draws are reordered.
pub fn plan_batches(keys: &[u32], rects: &[ScreenRect]) -> Vec<PlannedBatch> {
    debug_assert_eq!(keys.len(), rects.len());
    let mut batches: Vec<PlannedBatch> = Vec::new();
    for (i, (&key, rect)) in keys.iter().zip(rects).enumerate() {
        let mut target = None;
        let lo = batches.len().saturating_sub(PLAN_LOOKBACK);
        for b in (lo..batches.len()).rev() {
            if batches[b].key == key {
                target = Some(b);
                break;
            }
            // Instance `i` would move ahead of batch `b`: only when it
            // overlaps none of `b`'s instances.
            if overlaps(&batches[b].bounds, rect)
                && batches[b].instances.iter().any(|&j| overlaps(&rects[j as usize], rect))
            {
                break;
            }
        }
        match target {
            Some(b) => {
                batches[b].instances.push(i as u32);
                batches[b].bounds = union(&batches[b].bounds, rect);
            }
            None => batches.push(PlannedBatch { key, instances: vec![i as u32], bounds: *rect }),
        }
    }
    batches
}

/// Group shadow casters by key alone (depth-only draws commute), keeping the
/// first-seen key order and the instance order within a key.
pub fn plan_shadow_batches(keys: &[u32]) -> Vec<PlannedBatch> {
    let mut batches: Vec<PlannedBatch> = Vec::new();
    let mut index: std::collections::HashMap<u32, usize> = Default::default();
    for (i, &key) in keys.iter().enumerate() {
        let b = *index.entry(key).or_insert_with(|| {
            batches.push(PlannedBatch { key, instances: Vec::new(), bounds: [0.0; 4] });
            batches.len() - 1
        });
        batches[b].instances.push(i as u32);
    }
    batches
}

/// A batch ready to draw: a contiguous range of the uploaded instance buffer
/// and the textures it binds.
#[derive(Clone, Debug, PartialEq)]
pub struct SpriteBatch {
    pub first: u32,
    pub count: u32,
    pub texture: String,
    pub depth_map: Option<String>,
    pub normal_map: Option<String>,
    /// The batch's stencil ghost group on the stencil fallback (`None` when
    /// the ghost ids carry the groups).
    pub ghost_group: Option<u32>,
    /// Direction-blend batches: direction B's textures (the `#INTERP`
    /// programs bind them on [`BLEND_UNITS`]).
    pub blend: Option<BlendTextures>,
}

/// Direction B's textures of a direction-blend batch.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BlendTextures {
    pub texture: String,
    pub depth_map: Option<String>,
    pub normal_map: Option<String>,
}

/// A GPU instance buffer with its VAO (quad attributes + the instance
/// attributes, divisor 1).
pub(crate) struct SpriteInstanceGpu {
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    capacity: usize,
}

impl SpriteInstanceGpu {
    fn new(gfx_gl: &glow::Context, quad: &crate::QuadBuffers) -> Self {
        let gl = gfx_gl;
        unsafe {
            let vao = gl.create_vertex_array().expect("sprite VAO");
            let vbo = gl.create_buffer().expect("sprite VBO");
            gl.bind_vertex_array(Some(vao));
            quad.verts.bind(gl);
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, 0, 0);
            quad.uv.bind(gl);
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(1, 2, glow::FLOAT, false, 0, 0);
            quad.indices.bind(gl);
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            for k in 0..INSTANCE_VEC4S {
                let loc = FIRST_INSTANCE_LOC + k;
                gl.enable_vertex_attrib_array(loc);
                gl.vertex_attrib_pointer_f32(
                    loc,
                    4,
                    glow::FLOAT,
                    false,
                    SPRITE_INSTANCE_BYTES as i32,
                    (k * 16) as i32,
                );
                gl.vertex_attrib_divisor(loc, 1);
            }
            gl.bind_vertex_array(None);
            Self { vao, vbo, capacity: 0 }
        }
    }

    fn upload(&mut self, gl: &glow::Context, instances: &[IsoSpriteInstance]) {
        let bytes: &[u8] = bytemuck::cast_slice(instances);
        unsafe {
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            // Orphan the store every frame: writing into the one the frames
            // still in flight read would stall on them.  Grow geometrically so
            // a slowly rising count reallocates rarely.
            if instances.len() > self.capacity {
                self.capacity = instances.len().next_power_of_two().max(64);
            }
            gl.buffer_data_size(
                glow::ARRAY_BUFFER,
                (self.capacity * SPRITE_INSTANCE_BYTES) as i32,
                glow::STREAM_DRAW,
            );
            if !bytes.is_empty() {
                gl.buffer_sub_data_u8_slice(glow::ARRAY_BUFFER, 0, bytes);
            }
        }
    }

    /// Bind the VAO and point the instance attributes at record `first`.
    fn bind_range(&self, gl: &glow::Context, first: u32) {
        unsafe {
            gl.bind_vertex_array(Some(self.vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            for k in 0..INSTANCE_VEC4S {
                gl.vertex_attrib_pointer_f32(
                    FIRST_INSTANCE_LOC + k,
                    4,
                    glow::FLOAT,
                    false,
                    SPRITE_INSTANCE_BYTES as i32,
                    (first as usize * SPRITE_INSTANCE_BYTES) as i32 + (k * 16) as i32,
                );
            }
        }
    }
}

/// The ghost-id target: an `R8` id texture (group / 255, `NEAREST`) plus a
/// `DEPTH24_STENCIL8` renderbuffer the scene depth is blitted into.
pub struct GhostIdTarget {
    fbo: glow::Framebuffer,
    ids: glow::Texture,
    depth: glow::Renderbuffer,
    pub width: u32,
    pub height: u32,
}

impl GhostIdTarget {
    fn new(gl: &glow::Context, width: u32, height: u32) -> Self {
        unsafe {
            let fbo = gl.create_framebuffer().expect("ghost id fbo");
            let ids = gl.create_texture().expect("ghost id texture");
            let depth = gl.create_renderbuffer().expect("ghost id depth");
            gl.bind_texture(glow::TEXTURE_2D, Some(ids));
            for (p, v) in [
                (glow::TEXTURE_MIN_FILTER, glow::NEAREST),
                (glow::TEXTURE_MAG_FILTER, glow::NEAREST),
                (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
            ] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, p, v as i32);
            }
            gl.bind_texture(glow::TEXTURE_2D, None);
            let mut t = Self { fbo, ids, depth, width: 0, height: 0 };
            t.resize(gl, width, height);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(ids),
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

    fn resize(&mut self, gl: &glow::Context, width: u32, height: u32) {
        if (width, height) == (self.width, self.height) {
            return;
        }
        self.width = width;
        self.height = height;
        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(self.ids));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::R8 as i32,
                width as i32,
                height as i32,
                0,
                glow::RED,
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

/// Batcher state kept on `Gfx`.
#[derive(Default)]
pub(crate) struct SpriteBatchState {
    sprites: Option<SpriteInstanceGpu>,
    shadows: Option<SpriteInstanceGpu>,
    ids: Option<GhostIdTarget>,
    /// The ghost-id pass cannot run here (sticky).
    ids_unsupported: Cell<bool>,
    /// The main framebuffer's sample count was checked.
    ids_checked: Cell<bool>,
    /// A depth blit succeeded once.  Only the first one checks `glGetError`:
    /// under Mesa's glthread every `glGet*` syncs the driver thread, and a
    /// per-frame check cost a 6 ms stall.
    blit_verified: Cell<bool>,
    /// The ids hold this frame's groups (between `begin_ghost_ids` and the
    /// next frame's).
    ids_valid: Cell<bool>,
}

impl Gfx {
    /// Whether the ghost groups can run in the shader (the ghost-id pass);
    /// when `false`, plan batches split by group and the passes use the
    /// stencil.
    pub fn ghost_ids_supported(&self) -> bool {
        if self.sprite_batch.ids_unsupported.get() {
            return false;
        }
        if self.sprite_batch.ids_checked.get() {
            return true;
        }
        self.sprite_batch.ids_checked.set(true);
        let gl = &self.gl;
        let main_fbo = self.render_target.as_ref().map(|rt| rt.fbo());
        let samples = unsafe {
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, main_fbo);
            let s = gl.get_parameter_i32(glow::SAMPLES);
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, None);
            s
        };
        if samples > 0 {
            log::info!("sprite ghost ids: main framebuffer has {samples} samples; stencil groups");
            self.sprite_batch.ids_unsupported.set(true);
            self.bind_main_target();
            return false;
        }
        self.bind_main_target();
        true
    }

    /// Upload this frame's sprite instances (in batch order) for the normal,
    /// ghost and ghost-id passes.
    pub fn upload_sprite_instances(&mut self, instances: &[IsoSpriteInstance]) {
        let gl = &self.gl;
        let gpu =
            self.sprite_batch.sprites.get_or_insert_with(|| SpriteInstanceGpu::new(gl, &self.quad));
        gpu.upload(gl, instances);
        unsafe { gl.bind_vertex_array(Some(self.vao)) };
        self.sprite_batch.ids_valid.set(false);
    }

    /// Upload this frame's shadow-caster instances (in shadow batch order).
    pub fn upload_shadow_sprite_instances(&mut self, instances: &[IsoSpriteInstance]) {
        let gl = &self.gl;
        let gpu =
            self.sprite_batch.shadows.get_or_insert_with(|| SpriteInstanceGpu::new(gl, &self.quad));
        gpu.upload(gl, instances);
        unsafe { gl.bind_vertex_array(Some(self.vao)) };
    }

    /// Draw shadow-caster batches into the bound shadow map (between
    /// `begin_shadow_pass` and `end_shadow_pass`).  Returns the draw count.
    pub fn draw_shadow_sprite_batches(&self, view_proj: &Mat4, batches: &[SpriteBatch]) -> u32 {
        let Some(gpu) = self.sprite_batch.shadows.as_ref() else { return 0 };
        let gl = &self.gl;
        let s = self.shader("shadowSpriteInst");
        s.bind(gl);
        s.uniform_1i(gl, "tex_sampler", 0);
        s.uniform_mat4(gl, "light_view_proj", view_proj);
        let mut draws = 0;
        for b in batches {
            if b.count == 0 {
                continue;
            }
            self.texture(&b.texture).bind(gl, 0);
            gpu.bind_range(gl, b.first);
            unsafe {
                gl.draw_elements_instanced(
                    glow::TRIANGLES,
                    self.quad.index_count as i32,
                    glow::UNSIGNED_SHORT,
                    0,
                    b.count as i32,
                );
            }
            draws += 1;
        }
        unsafe { gl.bind_vertex_array(Some(self.vao)) };
        draws
    }

    /// Bind a batch's textures on the instanced sheet program `s`.
    fn bind_sprite_batch_textures(&self, s: &crate::Shader, b: &SpriteBatch) {
        let gl = &self.gl;
        self.texture(&b.texture).bind(gl, 0);
        match b.depth_map.as_deref().and_then(|d| self.textures.get(d)) {
            Some(dt) => {
                dt.bind(gl, 1);
                s.uniform_1f(gl, "use_depth_map", 1.0);
            }
            // A named depth map that is not loaded still writes depth in the
            // per-draw path (`depth_mask(depth_map.is_some())`) but samples
            // nothing; keep both behaviours.
            None => {
                s.uniform_1f(gl, "use_depth_map", if b.depth_map.is_some() { 1.0 } else { 0.0 })
            }
        }
        match b.normal_map.as_deref().and_then(|n| self.textures.get(n)) {
            Some(nt) => {
                nt.bind(gl, 2);
                s.uniform_1f(gl, "use_normal_map", 1.0);
            }
            None => s.uniform_1f(gl, "use_normal_map", 0.0),
        }
        if let Some(bt) = &b.blend {
            self.texture(&bt.texture).bind(gl, BLEND_UNITS[0]);
            if let Some(dt) = bt.depth_map.as_deref().and_then(|d| self.textures.get(d)) {
                dt.bind(gl, BLEND_UNITS[1]);
            }
            if let Some(nt) = bt.normal_map.as_deref().and_then(|n| self.textures.get(n)) {
                nt.bind(gl, BLEND_UNITS[2]);
            }
            unsafe { gl.active_texture(glow::TEXTURE0) };
        }
    }

    /// Start the ghost-id pass: blit the scene depth into the id target and
    /// clear its ids to 0.  Returns `false` (main target still bound) when the
    /// blit is unsupported; the caller then plans by group and uses the
    /// stencil.  On `true`, draw [`Gfx::draw_ghost_id_batches`], which
    /// rebinds the main target when done.
    pub fn begin_ghost_ids(&mut self, bounds: Option<ScreenRect>) -> bool {
        if self.sprite_batch.ids_unsupported.get() {
            return false;
        }
        let (mw, mh) = self.main_target_size();
        let [x0, y0, x1, y1] = self.target_rect(bounds, (mw, mh));
        let main_fbo = self.render_target.as_ref().map(|rt| rt.fbo());
        let gl = &self.gl;
        match self.sprite_batch.ids.as_mut() {
            Some(t) => t.resize(gl, mw, mh),
            None => self.sprite_batch.ids = Some(GhostIdTarget::new(gl, mw, mh)),
        }
        let t = self.sprite_batch.ids.as_ref().expect("ghost id target");
        let verify = !self.sprite_batch.blit_verified.get();
        unsafe {
            if verify {
                while gl.get_error() != glow::NO_ERROR {}
            }
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, main_fbo);
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(t.fbo));
            gl.blit_framebuffer(
                x0,
                y0,
                x1,
                y1,
                x0,
                y0,
                x1,
                y1,
                glow::DEPTH_BUFFER_BIT,
                glow::NEAREST,
            );
            if verify {
                let err = gl.get_error();
                if err != glow::NO_ERROR {
                    log::warn!("sprite ghost ids: depth blit failed (0x{err:04X}); stencil groups");
                    self.sprite_batch.ids_unsupported.set(true);
                    self.bind_main_target();
                    return false;
                }
                self.sprite_batch.blit_verified.set(true);
            }
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(t.fbo));
            gl.viewport(0, 0, mw as i32, mh as i32);
            gl.enable(glow::SCISSOR_TEST);
            gl.scissor(x0, y0, x1 - x0, y1 - y0);
            gl.clear_color(0.0, 0.0, 0.0, 0.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            gl.clear_color(0.0, 0.0, 0.0, 1.0);
        }
        true
    }

    /// A pre-projection screen rect (the camera's pixel space, y down) as a
    /// framebuffer rect `[x0, y0, x1, y1]` (y up), clamped to the target and
    /// rounded out; `None` is the whole target.
    fn target_rect(&self, bounds: Option<ScreenRect>, (mw, mh): (u32, u32)) -> [i32; 4] {
        let Some(b) = bounds else { return [0, 0, mw as i32, mh as i32] };
        let sx = mw as f32 / self.viewport_w.max(1.0);
        let sy = mh as f32 / self.viewport_h.max(1.0);
        let clamp = |v: f32, hi: u32| (v.max(0.0) as i32).min(hi as i32);
        let x0 = clamp((b[0] * sx).floor() - 1.0, mw);
        let x1 = clamp((b[2] * sx).ceil() + 1.0, mw);
        let y0 = clamp(((self.viewport_h - b[3]) * sy).floor() - 1.0, mh);
        let y1 = clamp(((self.viewport_h - b[1]) * sy).ceil() + 1.0, mh);
        [x0, y0, x1.max(x0), y1.max(y0)]
    }

    /// Draw every batch's ghost ids (the normal pass's depth state and
    /// discards, no blending), then rebind the main target.
    pub fn draw_ghost_id_batches(
        &self,
        camera: &Mat4,
        world_matrix: &Mat4,
        settings: &RenderSettings,
        batches: &[SpriteBatch],
    ) -> u32 {
        let mut draws = 0;
        if let Some(gpu) = self.sprite_batch.sprites.as_ref() {
            let gl = &self.gl;
            unsafe {
                gl.disable(glow::BLEND);
                gl.enable(glow::DEPTH_TEST);
                gl.depth_func(glow::LEQUAL);
                gl.disable(glow::STENCIL_TEST);
            }
            let mut programs = ProgramPair::new("spriteGhostIds", "spriteGhostIdsInterp");
            for b in batches {
                if b.count == 0 {
                    continue;
                }
                let s = programs.select(self, b.blend.is_some(), |s| {
                    self.bind_instanced_view(s, camera, world_matrix, settings);
                });
                self.bind_sprite_batch_textures(s, b);
                unsafe { gl.depth_mask(b.depth_map.is_some()) };
                gpu.bind_range(gl, b.first);
                unsafe {
                    gl.draw_elements_instanced(
                        glow::TRIANGLES,
                        self.quad.index_count as i32,
                        glow::UNSIGNED_SHORT,
                        0,
                        b.count as i32,
                    );
                }
                draws += 1;
            }
            unsafe {
                gl.depth_mask(true);
                gl.disable(glow::DEPTH_TEST);
                gl.enable(glow::BLEND);
                gl.bind_vertex_array(Some(self.vao));
            }
            self.sprite_batch.ids_valid.set(true);
        }
        unsafe { self.gl.disable(glow::SCISSOR_TEST) };
        self.bind_main_target();
        draws
    }

    /// The uniforms every instanced sheet draw of a pass shares.
    fn bind_instanced_view(
        &self,
        s: &crate::Shader,
        camera: &Mat4,
        world_matrix: &Mat4,
        settings: &RenderSettings,
    ) {
        let gl = &self.gl;
        s.uniform_mat4(gl, "projection_matrix", &self.projection());
        s.uniform_mat4(gl, "camera_matrix", camera);
        s.uniform_mat4(gl, "world_matrix", world_matrix);
        s.uniform_1f(gl, "ppm", settings.ppm);
        s.uniform_vec2(gl, "depth_span", &settings.depth_span);
        s.uniform_1i(gl, "tex_sampler", 0);
        s.uniform_1i(gl, "depth_sampler", 1);
        s.uniform_1i(gl, "normal_sampler", 2);
        s.uniform_1i(gl, "tex_sampler_b", BLEND_UNITS[0] as i32);
        s.uniform_1i(gl, "depth_sampler_b", BLEND_UNITS[1] as i32);
        s.uniform_1i(gl, "normal_sampler_b", BLEND_UNITS[2] as i32);
    }

    /// The pass state of the instanced sheet program `s` (lighting, shadow,
    /// light grid, ghost ids, ghost alpha).
    #[allow(clippy::too_many_arguments)]
    fn bind_sheet_pass(
        &self,
        s: &crate::Shader,
        camera: &Mat4,
        world_matrix: &Mat4,
        settings: &RenderSettings,
        pass: &IsoSpritePass,
        selection_color: &[f32; 3],
        ids: bool,
    ) {
        let gl = &self.gl;
        self.bind_instanced_view(s, camera, world_matrix, settings);
        s.uniform_vec3(gl, "ambient_color", glam::Vec3::from_array(settings.ambient));
        s.uniform_vec3(gl, "light_direction", glam::Vec3::from_array(settings.light_dir));
        s.uniform_vec3(gl, "light_color", glam::Vec3::from_array(settings.light_color));
        s.uniform_1f(gl, "use_lighting", 1.0);
        s.uniform_vec3(gl, "selection_color", glam::Vec3::from_array(*selection_color));
        self.bind_shadow(s, settings);
        self.bind_light_grid(s, Some((camera, world_matrix, settings.ppm)));
        s.uniform_1i(gl, "ghost_ids", GHOST_ID_UNIT as i32);
        s.uniform_1f(gl, "use_ghost_ids", if ids { 1.0 } else { 0.0 });
        s.uniform_1f(
            gl,
            "ghost_alpha",
            match pass {
                IsoSpritePass::Normal => 0.0,
                IsoSpritePass::Ghost => GHOST_ALPHA,
            },
        );
    }

    /// Draw one pass of sprite batches (all normals, or all ghosts).  Returns
    /// the draw count.
    pub fn draw_iso_sprite_batches(
        &self,
        camera: &Mat4,
        world_matrix: &Mat4,
        settings: &RenderSettings,
        batches: &[SpriteBatch],
        pass: IsoSpritePass,
        selection_color: &[f32; 3],
    ) -> u32 {
        let Some(gpu) = self.sprite_batch.sprites.as_ref() else { return 0 };
        let gl = &self.gl;
        let ids = matches!(pass, IsoSpritePass::Ghost)
            && self.sprite_batch.ids_valid.get()
            && self.sprite_batch.ids.is_some();
        if let (true, Some(t)) = (ids, self.sprite_batch.ids.as_ref()) {
            unsafe {
                gl.active_texture(glow::TEXTURE0 + GHOST_ID_UNIT);
                gl.bind_texture(glow::TEXTURE_2D, Some(t.ids));
                gl.active_texture(glow::TEXTURE0);
            }
        }
        unsafe {
            gl.enable(glow::DEPTH_TEST);
            gl.enable(glow::STENCIL_TEST);
            match pass {
                IsoSpritePass::Normal => {
                    gl.depth_func(glow::LEQUAL);
                    gl.stencil_op(glow::KEEP, glow::KEEP, glow::REPLACE);
                    gl.stencil_mask(0xFF);
                }
                IsoSpritePass::Ghost => {
                    gl.depth_func(glow::GREATER);
                    gl.depth_mask(false);
                    gl.stencil_op(glow::KEEP, glow::KEEP, glow::KEEP);
                    gl.stencil_mask(0x00);
                }
            }
        }
        let mut programs = ProgramPair::new("imageSheetInst", "imageSheetInterp");
        let mut draws = 0;
        for b in batches {
            if b.count == 0 {
                continue;
            }
            let s = programs.select(self, b.blend.is_some(), |s| {
                self.bind_sheet_pass(
                    s,
                    camera,
                    world_matrix,
                    settings,
                    &pass,
                    selection_color,
                    ids,
                );
            });
            self.bind_sprite_batch_textures(s, b);
            // The stencil reference: the batch's group on the stencil
            // fallback, else a constant (0: the ids carry the groups, and the
            // model ghost composite only tests `NOTEQUAL 255`).
            let group = b.ghost_group.unwrap_or(0);
            unsafe {
                match pass {
                    IsoSpritePass::Normal => {
                        gl.depth_mask(b.depth_map.is_some());
                        gl.stencil_func(glow::ALWAYS, group as i32, 0xFF);
                    }
                    IsoSpritePass::Ghost => {
                        if group != 0 {
                            gl.stencil_func(glow::NOTEQUAL, group as i32, 0xFF);
                        } else {
                            gl.stencil_func(glow::ALWAYS, 0, 0xFF);
                        }
                    }
                }
            }
            gpu.bind_range(gl, b.first);
            unsafe {
                gl.draw_elements_instanced(
                    glow::TRIANGLES,
                    self.quad.index_count as i32,
                    glow::UNSIGNED_SHORT,
                    0,
                    b.count as i32,
                );
            }
            draws += 1;
        }
        unsafe {
            gl.disable(glow::STENCIL_TEST);
            gl.stencil_mask(0xFF);
            gl.depth_mask(true);
            gl.depth_func(glow::LEQUAL);
            gl.disable(glow::DEPTH_TEST);
            gl.bind_vertex_array(Some(self.vao));
        }
        draws
    }
}

/// The plain and the direction-blend variant of one instanced program: binds
/// the one a batch needs, setting its pass uniforms the first time (GL keeps
/// uniforms per program).
struct ProgramPair {
    names: [&'static str; 2],
    ready: [bool; 2],
    current: Option<usize>,
}

impl ProgramPair {
    fn new(plain: &'static str, blend: &'static str) -> Self {
        Self { names: [plain, blend], ready: [false; 2], current: None }
    }

    fn select<'g>(
        &mut self,
        gfx: &'g Gfx,
        blend: bool,
        prepare: impl FnOnce(&crate::Shader),
    ) -> &'g crate::Shader {
        let i = blend as usize;
        let s = gfx.shader(self.names[i]);
        if self.current != Some(i) {
            s.bind(&gfx.gl);
            self.current = Some(i);
        }
        if !self.ready[i] {
            prepare(s);
            self.ready[i] = true;
        }
        s
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn r(x0: f32, y0: f32, x1: f32, y1: f32) -> ScreenRect {
        [x0, y0, x1, y1]
    }

    fn members(b: &[PlannedBatch]) -> Vec<(u32, Vec<u32>)> {
        b.iter().map(|b| (b.key, b.instances.clone())).collect()
    }

    #[test]
    fn instance_is_thirteen_vec4s() {
        assert_eq!(SPRITE_INSTANCE_BYTES, 208);
        assert_eq!(SPRITE_INSTANCE_ATTRS.len() as u32, FIRST_INSTANCE_LOC + INSTANCE_VEC4S);
    }

    #[test]
    fn one_key_is_one_batch() {
        let rects = [r(0.0, 0.0, 10.0, 10.0), r(5.0, 5.0, 15.0, 15.0), r(50.0, 0.0, 60.0, 9.0)];
        let plan = plan_batches(&[7, 7, 7], &rects);
        assert_eq!(members(&plan), [(7, vec![0, 1, 2])]);
    }

    #[test]
    fn disjoint_instances_merge_across_keys() {
        // A B A B with no overlaps -> two batches.
        let rects = [
            r(0.0, 0.0, 1.0, 1.0),
            r(10.0, 0.0, 11.0, 1.0),
            r(20.0, 0.0, 21.0, 1.0),
            r(30.0, 0.0, 31.0, 1.0),
        ];
        let plan = plan_batches(&[1, 2, 1, 2], &rects);
        assert_eq!(members(&plan), [(1, vec![0, 2]), (2, vec![1, 3])]);
    }

    #[test]
    fn overlap_keeps_relative_order() {
        // 0 (A) under 1 (B) under 2 (A): 2 overlaps 1, so it cannot join 0's
        // batch ahead of 1.
        let rects = [r(0.0, 0.0, 10.0, 10.0), r(5.0, 5.0, 15.0, 15.0), r(8.0, 8.0, 20.0, 20.0)];
        let plan = plan_batches(&[1, 2, 1], &rects);
        assert_eq!(members(&plan), [(1, vec![0]), (2, vec![1]), (1, vec![2])]);
    }

    #[test]
    fn plan_preserves_every_overlapping_pair_order() {
        // Pseudo-random scene: check the invariant directly.
        let mut seed = 12345u32;
        let mut rnd = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 8) as f32 / (1u32 << 24) as f32
        };
        let n = 400;
        let mut rects = Vec::new();
        let mut keys = Vec::new();
        for _ in 0..n {
            let (x, y) = (rnd() * 100.0, rnd() * 100.0);
            rects.push(r(x, y, x + 2.0 + rnd() * 8.0, y + 2.0 + rnd() * 8.0));
            keys.push((rnd() * 4.0) as u32);
        }
        let plan = plan_batches(&keys, &rects);
        let mut pos = vec![0usize; n];
        let mut k = 0;
        for b in &plan {
            for &i in &b.instances {
                assert_eq!(keys[i as usize], b.key);
                pos[i as usize] = k;
                k += 1;
            }
        }
        assert_eq!(k, n, "every instance planned once");
        for i in 0..n {
            for j in i + 1..n {
                if overlaps(&rects[i], &rects[j]) {
                    assert!(pos[i] < pos[j], "overlapping {i} and {j} swapped");
                }
            }
        }
        assert!(plan.len() < n / 2, "{} batches for {n} instances", plan.len());
    }

    #[test]
    fn shadow_plan_groups_by_key() {
        let plan = plan_shadow_batches(&[3, 1, 3, 2, 1]);
        assert_eq!(members(&plan), [(3, vec![0, 2]), (1, vec![1, 4]), (2, vec![3])]);
    }

    #[test]
    fn regions_map_to_instance_data() {
        let (uv, _, _, grid) =
            region_uv(&SpriteRegion::Grid { frame: 5.0, tile_set_size: [4.0, 2.0] });
        assert_eq!((uv, grid), ([5.0, 4.0, 2.0, 0.0], true));
        let (uv, content, source, grid) = region_uv(&SpriteRegion::Uv {
            uv_rect: &[0.1, 0.2, 0.3, 0.4],
            trim_offset: &[10.0, 20.0],
            source_size: &[100.0, 200.0],
            content_size: &[50.0, 100.0],
        });
        assert_eq!(uv, [0.1, 0.2, 0.3, 0.4]);
        assert_eq!((content, source), ([10.0, 20.0, 50.0, 100.0], [100.0, 200.0]));
        assert!(!grid);
    }
}
