//! Directional (sun) shadow-mapping math: fit an orthographic light-space box
//! around the tilemap world AABB and build the `view * proj` matrix that maps
//! world space (the lit shaders' `worldPos`) to light clip space.
//!
//! The box is derived from the exact world transform used by the terrain vertex
//! shader: `worldPos = model * iso_matrix * vertex; worldPos.y -= vertex.z`.
//! `model` is the tilemap translation and `iso_matrix` is `S(scale) * iso`, the
//! same matrices `draw_tilemap` feeds the shader.

use glam::{Mat4, Vec3, Vec4};

/// World-space margin added around the tilemap box when fitting the light ortho
/// (protects map edges and the near/far planes).
pub const SHADOW_PADDING: f32 = 64.0;

/// Depth bias (light NDC units) added to the stored depth before the manual
/// shadow compare in the shaders.  Kept small: the shadow pass's polygon offset
/// now carries the acne suppression.
pub const SHADOW_BIAS: f32 = 0.0002;

/// Diffuse fraction a fully-shadowed pixel keeps (`0..=1`).  Lit pixels keep
/// `1.0`; `0.65` reads as a subtle slope shade rather than an opaque black,
/// matching the old baked terrain-darkening look.
pub const SHADOW_STRENGTH: f32 = 0.65;

/// The directional light's view/projection matrices in world space.
pub struct LightMatrix {
    pub view: Mat4,
    pub proj: Mat4,
    /// `proj * view` — maps world space to light clip space.
    pub view_proj: Mat4,
}

/// Compute the world-space position of a tile-grid point `(x, y, z)` (z in px),
/// replicating the terrain vertex shader's world transform.
fn world_corner(model: &Mat4, iso_matrix: &Mat4, p: Vec3) -> Vec3 {
    let v = *model * (*iso_matrix * Vec4::new(p.x, p.y, p.z, 1.0));
    let mut out = Vec3::new(v.x, v.y, v.z);
    out.y -= p.z;
    out
}

/// The four world-space corners of a sprite billboard (a unit quad in the
/// cartesian x-y plane, transformed by the sprite's model matrix).
fn sprite_billboard_corners(model: &Mat4) -> [Vec3; 4] {
    let pts = [
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::new(1.0, 1.0, 0.0),
    ];
    let mut out = [Vec3::ZERO; 4];
    for (i, p) in pts.iter().enumerate() {
        let v = *model * p.extend(1.0);
        out[i] = Vec3::new(v.x, v.y, v.z);
    }
    out
}

/// Fit an orthographic light-space box around the tilemap AABB plus the shadow
/// casters (sprite billboards) standing on it.
///
/// * `model` / `iso_matrix` — the tilemap's world transform (translation and
///   `S(scale) * iso`), exactly as passed to `draw_tilemap`.
/// * `size_x` / `size_y` — tile dimensions; `z_max` — max terrain height in px
///   (`max(height_data) * height_scale`).
/// * `light_dir` — the toward-light direction (same space as the shader's
///   `light_direction`); normalized internally.
/// * `padding` — world-space margin added around the box (protects the map
///   edges and reduces clamp artifacts).
/// * `casters` — world-space model matrices of the sprite shadow casters, whose
///   billboards extend "up" out of the terrain plane and must fit inside the
///   box or their shadows clip at the near plane.
#[allow(clippy::too_many_arguments)]
pub fn fit_directional_light_matrix(
    model: &Mat4,
    iso_matrix: &Mat4,
    size_x: f32,
    size_y: f32,
    z_max: f32,
    light_dir: Vec3,
    padding: f32,
    casters: &[Mat4],
) -> LightMatrix {
    let light_dir = {
        let d = light_dir.normalize_or_zero();
        if d == Vec3::ZERO {
            Vec3::Z
        } else {
            d
        }
    };

    let mut corners: Vec<Vec3> = Vec::with_capacity(8 + casters.len() * 4);
    for x in [0.0f32, size_x] {
        for y in [0.0f32, size_y] {
            for z in [0.0f32, z_max] {
                corners.push(world_corner(model, iso_matrix, Vec3::new(x, y, z)));
            }
        }
    }
    for c in casters {
        corners.extend_from_slice(&sprite_billboard_corners(c));
    }

    let mut center = Vec3::ZERO;
    for c in &corners {
        center += *c;
    }
    center /= corners.len() as f32;

    // Distance from the centroid to the farthest corner — the eye is pushed out
    // along +light_dir so every corner lies in front of the near plane.
    let radius = corners.iter().map(|c| c.distance(center)).fold(0.0f32, f32::max) + padding;

    // A robust up vector: `(0,0,1)` collapses when the light is nearly vertical.
    let up = if light_dir.z.abs() > 0.99 { Vec3::Y } else { Vec3::Z };

    let eye = center + light_dir * radius;
    let view = Mat4::look_at_rh(eye, center, up);

    // Transform the corners into light view space and fit an ortho box.  The
    // right-handed look-at points the view down `-z`, so view-space z is
    // negative for points in front of the eye; near/far are positive distances
    // derived from the inverted z-extent.
    let mut mn = Vec3::splat(f32::MAX);
    let mut mx = Vec3::splat(f32::MIN);
    for c in &corners {
        let v = view.transform_point3(*c);
        mn = mn.min(v);
        mx = mx.max(v);
    }

    let left = mn.x - padding;
    let right = mx.x + padding;
    let bottom = mn.y - padding;
    let top = mx.y + padding;
    // mx.z is the least-negative z (nearest point) → near = -(mx.z) is its
    // distance; mn.z is the most-negative (farthest) → far = -(mn.z).
    let near = (-mx.z - padding).max(0.001);
    let far = -mn.z + padding;

    let proj = Mat4::orthographic_rh(left, right, bottom, top, near, far);
    let view_proj = proj * view;

    LightMatrix { view, proj, view_proj }
}

#[cfg(test)]
mod tests {
    use super::*;
    use classic_core::math::cartesian_to_iso_4;
    use glam::Vec4Swizzles;

    fn tilemap_mats(scale: [f32; 3], pos: Vec3) -> (Mat4, Mat4) {
        let iso = cartesian_to_iso_4().inverse();
        let iso_matrix = Mat4::from_scale(scale.into()) * iso;
        let model = Mat4::from_translation(pos);
        (model, iso_matrix)
    }

    fn assert_corners_inside(m: &LightMatrix, model: &Mat4, iso: &Mat4, size: f32, zmax: f32) {
        for x in [0.0, size] {
            for y in [0.0, size] {
                for z in [0.0, zmax] {
                    let w = world_corner(model, iso, Vec3::new(x, y, z));
                    let clip = m.view_proj * Vec4::new(w.x, w.y, w.z, 1.0);
                    let ndc = clip.xyz() / clip.w;
                    assert!(ndc.x >= -1.0 && ndc.x <= 1.0, "x={x} y={y} z={z}: ndc.x={}", ndc.x);
                    assert!(ndc.y >= -1.0 && ndc.y <= 1.0, "x={x} y={y} z={z}: ndc.y={}", ndc.y);
                    assert!(ndc.z >= -1.0 && ndc.z <= 1.0, "x={x} y={y} z={z}: ndc.z={}", ndc.z);
                }
            }
        }
    }

    #[test]
    fn flat_tilemap_corners_land_in_ndc() {
        let (model, iso) = tilemap_mats([45.0, 45.0, 1.0], Vec3::ZERO);
        let m = fit_directional_light_matrix(
            &model,
            &iso,
            200.0,
            200.0,
            0.0,
            Vec3::new(0.45, -0.35, 0.82),
            10.0,
            &[],
        );
        assert_corners_inside(&m, &model, &iso, 200.0, 0.0);
    }

    #[test]
    fn relief_tilemap_corners_land_in_ndc() {
        let (model, iso) = tilemap_mats([45.0, 45.0, 1.0], Vec3::new(12.5, -8.0, 0.0));
        let zmax = 20.0 * 64.0; // 20 m of relief at 64 px/m
        let m = fit_directional_light_matrix(
            &model,
            &iso,
            400.0,
            400.0,
            zmax,
            Vec3::new(0.45, -0.35, 0.82),
            50.0,
            &[],
        );
        assert_corners_inside(&m, &model, &iso, 400.0, zmax);
    }

    #[test]
    fn vertical_light_uses_alternate_up() {
        let (model, iso) = tilemap_mats([45.0, 45.0, 1.0], Vec3::ZERO);
        let m = fit_directional_light_matrix(&model, &iso, 100.0, 100.0, 500.0, Vec3::Z, 10.0, &[]);
        assert_corners_inside(&m, &model, &iso, 100.0, 500.0);
    }

    #[test]
    fn caster_billboard_fits_inside_box() {
        // A sprite billboard standing "up" (in -y, screen up) above the ground
        // plane must land inside the fitted box — otherwise its shadow clips at
        // the near plane.
        let (model, iso) = tilemap_mats([45.0, 45.0, 1.0], Vec3::ZERO);
        // A billboard at the map centre, 400 px tall (screen up = -y), anchored
        // on the ground: model = translate(centre) * scale(100, 400, 1).
        let caster = Mat4::from_translation(Vec3::new(0.0, -200.0, 0.0))
            * Mat4::from_scale(Vec3::new(100.0, 400.0, 1.0));
        let m = fit_directional_light_matrix(
            &model,
            &iso,
            200.0,
            200.0,
            0.0,
            Vec3::new(0.45, -0.35, 0.82),
            64.0,
            &[caster],
        );
        for c in sprite_billboard_corners(&caster) {
            let clip = m.view_proj * Vec4::new(c.x, c.y, c.z, 1.0);
            let ndc = clip.xyz() / clip.w;
            assert!(ndc.x >= -1.0 && ndc.x <= 1.0, "caster ndc.x={}", ndc.x);
            assert!(ndc.y >= -1.0 && ndc.y <= 1.0, "caster ndc.y={}", ndc.y);
            assert!(ndc.z >= -1.0 && ndc.z <= 1.0, "caster ndc.z={}", ndc.z);
        }
    }

    #[test]
    fn corner_world_transform_matches_shader_convention() {
        // The world transform must reproduce the shader's `y -= z` height lift.
        let iso = cartesian_to_iso_4().inverse();
        let iso_matrix = Mat4::from_scale(Vec3::new(45.0, 45.0, 1.0)) * iso;
        let model = Mat4::from_translation(Vec3::new(10.0, 20.0, 0.0));

        let flat = world_corner(&model, &iso_matrix, Vec3::new(3.0, 4.0, 0.0));
        let raised = world_corner(&model, &iso_matrix, Vec3::new(3.0, 4.0, 500.0));

        // Raising the tile by 500 px shifts world y down by 500 and z up by 500.
        assert!((raised.y - flat.y + 500.0).abs() < 1e-3);
        assert!((raised.x - flat.x).abs() < 1e-3);
        assert!((raised.z - flat.z - 500.0).abs() < 1e-3);
    }
}
