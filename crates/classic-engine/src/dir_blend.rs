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
        let mut ra = content_rect(fa, anchor_a_px);
        let mut rb = content_rect(&fb, anchor_b_px);
        // A tilt residual moves texels well outside their own content rect
        // (a pure yaw keeps them within the union + margin): bound the quad by
        // each neighbour's rect, over its depth range, through its full
        // rotation.  Untilted blends keep the plain union.
        if blend.tilt != [0.0; 3] {
            ra = warped_rect(ra, -blend.dtheta_a, blend.tilt, fa.depth_scale);
            rb = warped_rect(rb, -blend.dtheta_b, blend.tilt, fb.depth_scale);
        }
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

/// The pixel bounds (from the anchor, y down) of `rect`'s texels after the
/// warp `R = Q · Rz(phi)` (`sheet.frag` `residual()`): every texel lies in
/// `rect × [−range, range]` metres of anchor depth, so its 8 corners,
/// unprojected, rotated and reprojected, bound it.  `depth_scale` is the
/// sheet's window units per `gray − 0.5` (the shader's `rangeM` source).
pub(crate) fn warped_rect(rect: [f32; 4], phi: f32, tilt: [f32; 3], depth_scale: f32) -> [f32; 4] {
    use classic_core::math::{iso_basis, DEPTH_FAR, DEPTH_NEAR};
    use classic_core::tilemap::PPM_TARGET;
    let (r, u, b) = iso_basis();
    let range = depth_scale * (DEPTH_NEAR - DEPTH_FAR) * 0.5;
    let t = glam::Vec3::from(tilt);
    let w = (1.0 - t.length_squared()).max(0.0).sqrt();
    let rot = glam::Quat::from_xyzw(t.x, t.y, t.z, w) * glam::Quat::from_rotation_z(phi);
    let mut out = rect;
    for x in [rect[0], rect[2]] {
        for y in [rect[1], rect[3]] {
            for d in [-range, range] {
                let p = rot * ((r * x - u * y) / PPM_TARGET - b * d);
                let (qx, qy) = (p.dot(r) * PPM_TARGET, -p.dot(u) * PPM_TARGET);
                out = [out[0].min(qx), out[1].min(qy), out[2].max(qx), out[3].max(qy)];
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::warped_rect;
    use classic_core::math::{iso_basis, DEPTH_FAR, DEPTH_NEAR};
    use classic_core::tilemap::PPM_TARGET;

    #[test]
    fn warped_rect_bounds_every_rotated_texel() {
        let rect = [-120.0, -260.0, 110.0, 40.0];
        let range = 3.2f32;
        let scale = 2.0 * range / (DEPTH_NEAR - DEPTH_FAR);
        let tilt = classic_core::vehicle_def::tilt_residual(0.4, 0.17, -0.12, 0.35, 0.0);
        let phi = -0.2f32;
        let out = warped_rect(rect, phi, tilt, scale);
        assert!(out[0] <= rect[0] && out[1] <= rect[1] && out[2] >= rect[2] && out[3] >= rect[3]);
        // Sample texels inside rect × depth range: each lands inside `out`.
        let (r, u, b) = iso_basis();
        let t = glam::Vec3::from(tilt);
        let q = glam::Quat::from_xyzw(t.x, t.y, t.z, (1.0 - t.length_squared()).sqrt())
            * glam::Quat::from_rotation_z(phi);
        for i in 0..=10 {
            for j in 0..=10 {
                for d in [-range, 0.0, range] {
                    let x = rect[0] + (rect[2] - rect[0]) * i as f32 / 10.0;
                    let y = rect[1] + (rect[3] - rect[1]) * j as f32 / 10.0;
                    let p = q * ((r * x - u * y) / PPM_TARGET - b * d);
                    let (px, py) = (p.dot(r) * PPM_TARGET, -p.dot(u) * PPM_TARGET);
                    assert!(px >= out[0] - 1e-3 && px <= out[2] + 1e-3, "{px} {out:?}");
                    assert!(py >= out[1] - 1e-3 && py <= out[3] + 1e-3, "{py} {out:?}");
                }
            }
        }
        // A tilt widens the bound beyond the source rect.
        assert!(out[1] < rect[1] - 1.0 || out[3] > rect[3] + 1.0);
    }
}
