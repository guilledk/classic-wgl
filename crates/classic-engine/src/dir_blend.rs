//! Direction interpolation draw planning: picks, per iso sprite carrying an
//! `IsoSprite::dir_blend`, between the warped blend of its two baked
//! directions (`classic_gfx` `sheet.frag#INTERP`) and the nearer of them.

use classic_core::components::IsoSprite;
use classic_core::types::FrameTable;
use classic_gfx::{BlendTextures, DirBlendRegion};
use glam::Vec2;
use std::collections::HashMap;

use crate::{Engine, IsoUv, ResolvedFrame};

/// Pixels of slack around the union of the two directions' content rects:
/// the warped silhouette can poke a little past both.
pub(crate) const BLEND_QUAD_MARGIN_PX: f32 = 4.0;

/// How one direction-interpolated sprite draws this frame.
pub(crate) enum DirChoice {
    /// No `dir_blend` (or nothing resolvable): draw `frame` as usual.
    Plain,
    /// Draw direction B alone (it is the nearer one and the blend is off).
    UseB { frame: ResolvedFrame, anchor: Vec2 },
    /// Draw the warped blend.
    Blend(BlendGeom),
}

/// A direction blend's drawn quad and its direction-B half.
pub(crate) struct BlendGeom {
    /// The union quad's size (px) and the anchor's position in it.
    pub tex_dim: (f32, f32),
    pub anchor_px: Vec2,
    pub region: DirBlendRegion,
    pub textures: BlendTextures,
    /// The nearer direction, for the shadow caster: `None` = A.
    pub nearest_b: Option<(ResolvedFrame, Vec2)>,
}

/// A packed frame's draw geometry: source-cell size, the anchor in source-cell
/// pixels, and the instance region `(uv_rect, trim, source, content)`.
pub(crate) fn packed_geometry(fr: &ResolvedFrame, anchor: Vec2) -> ((f32, f32), Vec2, IsoUv) {
    let sw = if fr.source_size[0] > 0 { fr.source_size[0] as f32 } else { fr.size[0] };
    let sh = if fr.source_size[1] > 0 { fr.source_size[1] as f32 } else { fr.size[1] };
    let (cw, ch) = (fr.size[0], fr.size[1]);
    let (bx, by) = (fr.trim_offset[0] as f32, fr.trim_offset[1] as f32);
    let a = Engine::effective_anchor(anchor, fr);
    let anchor_px = Vec2::new(a.x * cw + bx, a.y * ch + by);
    ((sw, sh), anchor_px, (fr.uv_rect, [bx, by], [sw, sh], [cw, ch]))
}

/// A frame's trimmed content rect `[x0, y0, x1, y1]` in pixels from its
/// anchor.
fn content_rect(fr: &ResolvedFrame, anchor_px: Vec2) -> [f32; 4] {
    let (bx, by) = (fr.trim_offset[0] as f32, fr.trim_offset[1] as f32);
    [
        bx - anchor_px.x,
        by - anchor_px.y,
        bx + fr.size[0] - anchor_px.x,
        by + fr.size[1] - anchor_px.y,
    ]
}

impl Engine {
    /// Choose how `sprite` draws given its resolved direction-A frame.
    /// `selected` sprites never blend (the silhouette needs one frame).
    pub(crate) fn dir_choice(
        tables: &HashMap<String, FrameTable>,
        sprite: &IsoSprite,
        frame_a: Option<&ResolvedFrame>,
        selected: bool,
    ) -> DirChoice {
        let Some(blend) = sprite.dir_blend else { return DirChoice::Plain };
        let Some(fa) = frame_a else { return DirChoice::Plain };
        let name_b = format!("{}_{}", sprite.texture, blend.frame_b as u32);
        let Some(fb) = Self::resolve_frame(tables, &sprite.texture, &name_b) else {
            return DirChoice::Plain;
        };
        let b_nearer = -blend.dtheta_b < blend.dtheta_a;
        let blendable = fa.dir_interp
            && fa.anchor_depth
            && fb.anchor_depth
            && fa.depth_tex.is_some()
            && fb.depth_tex.is_some()
            && !selected;
        if !blendable {
            return if b_nearer {
                DirChoice::UseB { frame: fb, anchor: blend.anchor_b }
            } else {
                DirChoice::Plain
            };
        }
        let (_, anchor_a_px, _) = packed_geometry(fa, sprite.anchor);
        let (_, anchor_b_px, _) = packed_geometry(&fb, blend.anchor_b);
        let ra = content_rect(fa, anchor_a_px);
        let rb = content_rect(&fb, anchor_b_px);
        let m = BLEND_QUAD_MARGIN_PX;
        let x0 = (ra[0].min(rb[0]) - m).floor();
        let y0 = (ra[1].min(rb[1]) - m).floor();
        let x1 = (ra[2].max(rb[2]) + m).ceil();
        let y1 = (ra[3].max(rb[3]) + m).ceil();
        DirChoice::Blend(BlendGeom {
            tex_dim: (x1 - x0, y1 - y0),
            anchor_px: Vec2::new(-x0, -y0),
            region: DirBlendRegion {
                uv_b: fb.uv_rect,
                trim_b: [fb.trim_offset[0] as f32, fb.trim_offset[1] as f32],
                content_b: fb.size,
                anchor_a_px: anchor_a_px.to_array(),
                anchor_b_px: anchor_b_px.to_array(),
                dtheta_a: blend.dtheta_a,
                dtheta_b: blend.dtheta_b,
                tilt: blend.tilt,
                quad: [x0, y0, x1 - x0, y1 - y0],
            },
            textures: BlendTextures {
                texture: fb.sheet_name.clone(),
                depth_map: fb.depth_tex.clone(),
                normal_map: fb.normal_tex.clone(),
            },
            nearest_b: b_nearer.then_some((fb, blend.anchor_b)),
        })
    }
}
