//! Per-frame planning of the instanced iso-sprite batches (`classic_gfx::
//! sprite_batch`): one instance per [`IsoDraw`], keyed by atlas page, planned
//! so the batched draws reproduce the one-by-one render-list order.

use std::collections::HashMap;

use classic_core::tilemap::PPM_TARGET;
use classic_gfx::{
    plan_batches, plan_shadow_batches, BlendTextures, Gfx, IsoSpriteInstance, SpriteBatch,
    SpriteRegion,
};
use glam::{Mat4, Vec3};

use crate::{IsoDraw, OUTLINE_RADIUS_PX};

/// The frame's sprite batches: the instances in batch order (uploaded once
/// for the normal, ghost and ghost-id passes) and the shadow casters in
/// shadow batch order.
#[derive(Default)]
pub(crate) struct SpriteFrame {
    pub instances: Vec<IsoSpriteInstance>,
    pub batches: Vec<SpriteBatch>,
    pub shadow_instances: Vec<IsoSpriteInstance>,
    pub shadow_batches: Vec<SpriteBatch>,
    /// Whether any instance has a nonzero ghost group (else the ghost-id pass
    /// is skipped: nothing would read it).
    pub any_ghost_group: bool,
    /// Union of the grouped instances' screen rects: the ghost-id pass only
    /// blits and clears this much of the target.
    pub group_bounds: Option<[f32; 4]>,
    /// The batches carry stencil groups (the ghost-id pass is unsupported).
    pub stencil_groups: bool,
}

/// Screen rect `[x0, y0, x1, y1]` (pre-projection pixels) of an iso sprite
/// quad: its four corners through `direct_tex.vert`'s raster transform
/// (`world_matrix`, `ppm`, `camera`), padded a pixel.
pub(crate) fn iso_screen_rect(model: &Mat4, world_matrix: &Mat4, camera: &Mat4) -> [f32; 4] {
    let mut r = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for (x, y) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
        let world = model.transform_point3(Vec3::new(x, y, 0.0));
        let view = world_matrix.transform_point3(world);
        let s = camera.transform_point3(Vec3::new(view.x * PPM_TARGET, -view.y * PPM_TARGET, 0.0));
        r = [r[0].min(s.x), r[1].min(s.y), r[2].max(s.x), r[3].max(s.y)];
    }
    [r[0] - 1.0, r[1] - 1.0, r[2] + 1.0, r[3] + 1.0]
}

type Key = (String, Option<String>, Option<String>, u32, Option<BlendTextures>);

/// Plan this frame's sprite batches.  `stencil_groups` splits batches by
/// ghost group (the stencil fallback when the ghost-id pass cannot run).
pub(crate) fn plan_sprite_frame(
    draws: &[IsoDraw],
    gfx: &Gfx,
    camera: &Mat4,
    stencil_groups: bool,
) -> SpriteFrame {
    let mut keys: Vec<u32> = Vec::with_capacity(draws.len());
    let mut key_table: Vec<Key> = Vec::new();
    let mut key_index: HashMap<Key, u32> = HashMap::new();
    let mut rects = Vec::with_capacity(draws.len());
    let mut all: Vec<IsoSpriteInstance> = Vec::with_capacity(draws.len());
    let mut shadow_all: Vec<IsoSpriteInstance> = Vec::with_capacity(draws.len());
    let mut shadow_keys: Vec<u32> = Vec::with_capacity(draws.len());
    let mut shadow_key_table: Vec<String> = Vec::new();
    let mut shadow_key_index: HashMap<String, u32> = HashMap::new();
    let mut any_ghost_group = false;
    let mut group_bounds: Option<[f32; 4]> = None;
    for d in draws {
        let tex_size = gfx.textures.get(&d.texture).map_or((1, 1), |t| t.size);
        let region = d.region();
        let instance = |model: &Mat4, region: &SpriteRegion<'_>, tex_size| {
            IsoSpriteInstance::new(
                model,
                region,
                tex_size,
                &d.depth_corners,
                d.depth_base,
                d.depth_scale,
                d.ghost_group,
                &[d.color[0], d.color[1], d.color[2]],
                d.selected,
                OUTLINE_RADIUS_PX,
            )
        };
        let base = instance(&d.model, &region, tex_size);
        // The shadow caster: one baked direction (a blend casts its nearer).
        let (shadow_instance, shadow_texture) = match &d.shadow_caster {
            Some((m, tex, (uv_rect, trim_offset, source_size, content_size))) => {
                let r = SpriteRegion::Uv { uv_rect, trim_offset, source_size, content_size };
                let ts = gfx.textures.get(tex).map_or((1, 1), |t| t.size);
                (instance(m, &r, ts), tex.clone())
            }
            None => (base, d.texture.clone()),
        };
        all.push(match &d.blend {
            Some((b, _)) => base.blend(b),
            None => base,
        });
        shadow_all.push(shadow_instance);
        any_ghost_group |= d.ghost_group != 0;
        let rect = iso_screen_rect(&d.model, &d.world_matrix, camera);
        if d.ghost_group != 0 {
            group_bounds = Some(match group_bounds {
                None => rect,
                Some(b) => {
                    [b[0].min(rect[0]), b[1].min(rect[1]), b[2].max(rect[2]), b[3].max(rect[3])]
                }
            });
        }
        let key: Key = (
            d.texture.clone(),
            d.depth_map.clone(),
            d.normal_map.clone(),
            if stencil_groups { d.ghost_group } else { 0 },
            d.blend.as_ref().map(|(_, t)| t.clone()),
        );
        let next = key_table.len() as u32;
        let k = *key_index.entry(key.clone()).or_insert_with(|| {
            key_table.push(key);
            next
        });
        keys.push(k);
        rects.push(rect);
        let next = shadow_key_table.len() as u32;
        let sk = *shadow_key_index.entry(shadow_texture.clone()).or_insert_with(|| {
            shadow_key_table.push(shadow_texture);
            next
        });
        shadow_keys.push(sk);
    }

    let mut frame =
        SpriteFrame { any_ghost_group, group_bounds, stencil_groups, ..Default::default() };
    for p in plan_batches(&keys, &rects) {
        let (texture, depth_map, normal_map, group, blend) = key_table[p.key as usize].clone();
        frame.batches.push(SpriteBatch {
            first: frame.instances.len() as u32,
            count: p.instances.len() as u32,
            texture,
            depth_map,
            normal_map,
            ghost_group: stencil_groups.then_some(group),
            blend,
        });
        frame.instances.extend(p.instances.iter().map(|&i| all[i as usize]));
    }
    for p in plan_shadow_batches(&shadow_keys) {
        frame.shadow_batches.push(SpriteBatch {
            first: frame.shadow_instances.len() as u32,
            count: p.instances.len() as u32,
            texture: shadow_key_table[p.key as usize].clone(),
            depth_map: None,
            normal_map: None,
            ghost_group: None,
            blend: None,
        });
        frame.shadow_instances.extend(p.instances.iter().map(|&i| shadow_all[i as usize]));
    }
    frame
}
