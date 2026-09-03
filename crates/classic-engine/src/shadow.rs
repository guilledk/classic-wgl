//! Directional (sun) shadow-mapping math: fit an orthographic box around the
//! tilemap AABB and build the `view * proj` matrix that maps **world space**
//! (the same space the lit shaders evaluate `vLightPos`/`vNormal` in) to light
//! clip space.
//!
//! # The one space
//!
//! After the coordinate-system unification the renderer lives in a single
//! Blender-canonical world-metre space (+Z up).  Lighting — `light_dir`,
//! `vNormal`, `vLightPos`, `Light::position`, and this module's shadow map —
//! all use that space directly; the isometric screen image is a separate
//! raster-only projection (`iso_camera_px`).  There is no separate "light
//! space" anymore: world space is already metric, so `length`, `normalize` and
//! `dot` mean the same thing in every direction without any extra transform.

use glam::{Mat4, Vec3, Vec4};

/// World-space margin added around the tilemap box when fitting the light ortho
/// (protects map edges and the near/far planes).
pub const SHADOW_PADDING: f32 = 1.0;

/// Constant depth bias (light NDC units) added to the stored depth before the
/// manual shadow compare.  Deliberately small — it only mops up depth
/// quantisation; [`SHADOW_NORMAL_OFFSET`] does the real acne suppression.
///
/// A large depth bias cannot fix acne without also detaching shadows from their
/// casters ("peter-panning"), because the depth error it must cover scales with
/// the surface's slope relative to the light and is unbounded at grazing
/// angles.  Offsetting along the normal is bounded and geometry-relative.
pub const SHADOW_BIAS: f32 = 0.00025;

/// Normal-offset bias, in shadow-map texels.  Before sampling, the receiver is
/// pushed along its surface normal by this many texel widths, so a surface
/// never samples the texel it itself wrote.
pub const SHADOW_NORMAL_OFFSET: f32 = 1.5;

/// Diffuse fraction a fully-shadowed pixel keeps (`0..=1`).  Lit pixels keep
/// `1.0`.
///
/// Tuned against `basetest`: shadowed terrain lands at luma 48 against 73 lit —
/// dark enough to read as occlusion, light enough that terrain relief stays
/// visible inside the shadow.
///
/// **When changing the shadow geometry, set this to `0.0` first.**  A partial
/// value makes "the shadow map is broken" and "the shadow is subtle"
/// indistinguishable both by eye and by pixel diff, which is precisely how a
/// completely non-functional shadow map survived a full session of
/// verification.  `CLASSIC_SHADOW_DEBUG=1` exists for the same reason.
pub const SHADOW_STRENGTH: f32 = 0.4;

/// The directional light's view/projection matrices in world space.
pub struct LightMatrix {
    pub view: Mat4,
    pub proj: Mat4,
    /// `proj * view` — maps world space to light clip space.
    pub view_proj: Mat4,
    /// Width of one shadow-map texel, in world metres.  Normal-offset bias
    /// scales with this: the receiver is nudged along its normal by roughly a
    /// texel, which is exactly the distance over which the stored depth is
    /// ambiguous.
    pub world_texel: f32,
}

/// Fit an orthographic box around the tilemap AABB plus the shadow casters
/// (sprite world quads) standing on it.
///
/// * `origin` — the tilemap's world-metre position (its `Transform.position`);
///   the terrain AABB is offset by it, matching the sprite casters' world space.
/// * `size_x` / `size_y` — tile dimensions; `z_max` — max terrain height in
///   **metres** (`max(height_data)`).
/// * `light_dir` — the toward-light direction in world space (+Z up, same space
///   as the shader's `light_direction`); normalized internally.
/// * `padding` — world-metre margin added around the box (protects the map
///   edges and reduces clamp artifacts).
/// * `casters` — world-metre sprite model matrices.  The quads stand up out of
///   the terrain plane and must fit inside the box or their shadows clip at the
///   near plane.
/// * `shadow_map_size` — resolution of the depth target, used to report
///   [`LightMatrix::world_texel`].
#[allow(clippy::too_many_arguments)]
pub fn fit_directional_light_matrix(
    origin: Vec3,
    size_x: f32,
    size_y: f32,
    z_max: f32,
    light_dir: Vec3,
    padding: f32,
    casters: &[Mat4],
    shadow_map_size: f32,
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
    // Terrain AABB in world metres: `tx ∈ [0, size_x]`, `ty ∈ [0, size_y]`,
    // `h ∈ [0, z_max]`, offset by the tilemap's world position.
    let wx = size_x * classic_core::tilemap::TILE_M;
    let wy = -size_y * classic_core::tilemap::TILE_M;
    for x in [0.0f32, wx] {
        for y in [wy, 0.0f32] {
            for z in [0.0f32, z_max] {
                corners.push(origin + Vec3::new(x, y, z));
            }
        }
    }
    for model in casters {
        for (u, v) in [(0.0f32, 0.0f32), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
            let w = *model * Vec4::new(u, v, 0.0, 1.0);
            corners.push(Vec3::new(w.x, w.y, w.z));
        }
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
    let world_texel = (right - left).max(top - bottom) / shadow_map_size.max(1.0);

    LightMatrix { view, proj, view_proj, world_texel }
}

#[cfg(test)]
mod tests {
    use super::*;
    use classic_core::tilemap::PPM_TARGET;
    use classic_core::tilemap::TILE_M;
    use glam::{Vec3Swizzles, Vec4Swizzles};
    use std::f32::consts::FRAC_1_SQRT_2;

    /// A world-metre sprite quad: width along the isometric right direction,
    /// height down world −Z, bottom-centre anchored at `pos`.
    fn sprite_model(pos: Vec3, width_m: f32, height_m: f32) -> Mat4 {
        let r = Mat4::from_cols(
            Vec4::new(FRAC_1_SQRT_2, -FRAC_1_SQRT_2, 0.0, 0.0),
            Vec4::new(0.0, 0.0, -1.0, 0.0),
            Vec4::new(FRAC_1_SQRT_2, FRAC_1_SQRT_2, 0.0, 0.0),
            Vec4::W,
        );
        Mat4::from_translation(pos)
            * r
            * Mat4::from_scale(Vec3::new(width_m, height_m, 1.0))
            * Mat4::from_translation(Vec3::new(-0.5, -1.0, 0.0))
    }

    /// World-space corners of a world-metre sprite model.
    fn sprite_corners(model: &Mat4) -> [Vec3; 4] {
        let mut out = [Vec3::ZERO; 4];
        for (i, (u, v)) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)].iter().enumerate() {
            let w = *model * Vec4::new(*u, *v, 0.0, 1.0);
            out[i] = Vec3::new(w.x, w.y, w.z);
        }
        out
    }

    fn assert_corners_inside(m: &LightMatrix, size: f32, zmax: f32) {
        let wx = size * TILE_M;
        let wy = -size * TILE_M;
        for x in [0.0, wx] {
            for y in [wy, 0.0] {
                for z in [0.0, zmax] {
                    let clip = m.view_proj * Vec4::new(x, y, z, 1.0);
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
        let m = fit_directional_light_matrix(
            Vec3::ZERO,
            200.0,
            200.0,
            0.0,
            Vec3::new(0.566_422, -0.070_803, 0.821_068),
            10.0,
            &[],
            SHADOW_MAP_SIZE_F,
        );
        assert_corners_inside(&m, 200.0, 0.0);
    }

    #[test]
    fn relief_tilemap_corners_land_in_ndc() {
        let zmax = 20.0; // 20 m of relief
        let m = fit_directional_light_matrix(
            Vec3::ZERO,
            400.0,
            400.0,
            zmax,
            Vec3::new(0.566_422, -0.070_803, 0.821_068),
            50.0,
            &[],
            SHADOW_MAP_SIZE_F,
        );
        assert_corners_inside(&m, 400.0, zmax);
    }

    #[test]
    fn vertical_light_uses_alternate_up() {
        let m = fit_directional_light_matrix(
            Vec3::ZERO,
            100.0,
            100.0,
            500.0,
            Vec3::Z,
            10.0,
            &[],
            SHADOW_MAP_SIZE_F,
        );
        assert_corners_inside(&m, 100.0, 500.0);
    }

    #[test]
    fn caster_world_quad_fits_inside_box() {
        // A sprite world quad standing up out of the ground plane must land
        // inside the fitted box — otherwise its shadow clips at the near plane.
        let caster = sprite_model(Vec3::ZERO, 100.0 / PPM_TARGET, 400.0 / PPM_TARGET);
        let m = fit_directional_light_matrix(
            Vec3::ZERO,
            200.0,
            200.0,
            0.0,
            Vec3::new(0.566_422, -0.070_803, 0.821_068),
            64.0,
            &[caster],
            SHADOW_MAP_SIZE_F,
        );
        for c in sprite_corners(&caster) {
            let clip = m.view_proj * Vec4::new(c.x, c.y, c.z, 1.0);
            let ndc = clip.xyz() / clip.w;
            assert!(ndc.x >= -1.0 && ndc.x <= 1.0, "caster ndc.x={}", ndc.x);
            assert!(ndc.y >= -1.0 && ndc.y <= 1.0, "caster ndc.y={}", ndc.y);
            assert!(ndc.z >= -1.0 && ndc.z <= 1.0, "caster ndc.z={}", ndc.z);
        }
    }

    /// A sprite quad must be authored as *standing* geometry: its height extent
    /// lives in world Z, and it does not recede into the scene (its world-space
    /// depth along the light direction is constant), or it would shadow tiles
    /// behind it.
    #[test]
    fn sprite_world_quad_stands_up_out_of_the_ground() {
        let model = sprite_model(Vec3::ZERO, 100.0 / PPM_TARGET, 400.0 / PPM_TARGET);
        let corners = sprite_corners(&model);

        let z_min = corners.iter().map(|c| c.z).fold(f32::MAX, f32::min);
        let z_max = corners.iter().map(|c| c.z).fold(f32::MIN, f32::max);
        assert!(
            (z_max - z_min - 400.0 / PPM_TARGET).abs() < 1e-2,
            "sprite spans {:.3} in world-space height, expected {}",
            z_max - z_min,
            400.0 / PPM_TARGET
        );

        // The sprite stands straight up: its width runs along the horizontal
        // (1, −1) direction, so it never recedes along the perpendicular (1, 1)
        // depth axis — `x + y` is constant across its corners.
        let recede_min = corners.iter().map(|c| c.x + c.y).fold(f32::MAX, f32::min);
        let recede_max = corners.iter().map(|c| c.x + c.y).fold(f32::MIN, f32::max);
        assert!((recede_max - recede_min).abs() < 1e-2, "sprite recedes along the depth axis");
    }

    /// The basetest sun in world space (azimuth 120° light-space, elevation 30°,
    /// already unit length).
    const SUN: Vec3 = Vec3::new(0.224_144, -0.836_516, 0.5);

    /// The real depth-target resolution, so the tests cannot drift from it.
    const SHADOW_MAP_SIZE_F: f32 = classic_gfx::SHADOW_MAP_SIZE as f32;

    /// **Space contract.**  `light_dir` is authored in world space (+Z up;
    /// `classic-demo/src/lighting.rs` sets `d.z = sin(elevation)`), and the
    /// shadow map projects world-space positions in that same space.  This
    /// asserts the shadow map actually sees the sun at its authored elevation:
    /// raising terrain must move a vertex along world +Z, not along a sheared
    /// `(0,-1,1)`.
    #[test]
    fn shadow_space_sees_the_sun_at_its_authored_elevation() {
        let ground = Vec3::new(0.0, 0.0, 0.0);
        let raised = Vec3::new(0.0, 0.0, 1.0);
        let up = (raised - ground).normalize();

        let elevation_deg = up.dot(SUN).asin().to_degrees();
        assert!(
            (elevation_deg - 30.0).abs() < 1.0,
            "shadow space sees the sun at {elevation_deg:.1}°, expected 30° \
             (raising terrain must move a vertex along +Z, not along (0,-1,1))"
        );
    }

    /// **Space contract.**  A raised terrain vertex and the *flat terrain
    /// vertex its shadow lands on* must resolve to the same shadow-map texel —
    /// that is the entire premise of a shadow map.  The occluder must also be
    /// nearer the light in depth, or the compare picks the wrong surface.
    #[test]
    fn caster_and_the_ground_it_shadows_share_a_shadow_texel() {
        let m = fit_directional_light_matrix(
            Vec3::ZERO,
            200.0,
            200.0,
            0.0,
            SUN,
            64.0,
            &[],
            SHADOW_MAP_SIZE_F,
        );

        // A terrain vertex at world (50·TILE_M, −50·TILE_M) standing `h` metres
        // proud of the map.
        let base = Vec3::new(50.0 * TILE_M, -50.0 * TILE_M, 0.0);
        let h = 256.0 / PPM_TARGET; // metres
        let caster = base + Vec3::new(0.0, 0.0, h);

        // Where the sun ray through it meets z = 0, in world space (SUN is
        // authored there).
        let t = h / SUN.z;
        let receiver = base + Vec3::new(-SUN.x * t, -SUN.y * t, 0.0);

        let project = |p: Vec3| {
            let clip = m.view_proj * p.extend(1.0);
            let ndc = clip.xyz() / clip.w;
            (ndc.xy() * 0.5 + 0.5, ndc.z * 0.5 + 0.5)
        };
        let (caster_uv, caster_depth) = project(caster);
        let (receiver_uv, receiver_depth) = project(receiver);

        let slip = (caster_uv - receiver_uv).length() * SHADOW_MAP_SIZE_F;
        assert!(
            slip < 1.0,
            "caster and the ground it shadows land {slip:.1} texels apart in the \
             shadow map: it is projecting a different space than the one \
             `light_dir` is authored in"
        );
        assert!(
            caster_depth < receiver_depth,
            "caster depth {caster_depth:.5} is not nearer the light than the \
             ground it shadows ({receiver_depth:.5})"
        );
    }
}
