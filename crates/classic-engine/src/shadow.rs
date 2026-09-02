//! Directional (sun) shadow-mapping math: fit an orthographic light-space box
//! around the tilemap AABB and build the `view * proj` matrix that maps
//! **light space** (the lit shaders' `vLightPos`) to light clip space.
//!
//! # The two spaces
//!
//! The renderer carries terrain positions in two different spaces, and mixing
//! them silently produces a shadow map that compiles, runs, and casts nothing:
//!
//! | Space | Transform | Up axis | Metric? | Used by |
//! |---|---|---|---|---|
//! | **light** | `iso_world_light_matrix * world` = `S(scale) · Rz(-45°) · D⁻¹` | `+Z` | **yes** | `light_dir`, `vNormal`, `vLightPos`, `Light::position`, this module |
//! | **screen** | `model · world_matrix · vertex`, then `y -= ppm·z` | `(0,-1,1)/√2` | no | rasterised geometry only |
//!
//! Two distortions separate them, and *both* have shipped as bugs.
//!
//! The `y -= vertex.z` shear is what makes height read as height in an
//! isometric view, but it leaves the result carrying height in *both* y and z.
//! Projecting that along a `light_dir` authored with +Z up presented the sun at
//! ~2.7° instead of 30° — a near-degenerate grazing angle that cast nothing.
//!
//! The `diag(1, 0.5, 1)` inside `iso_matrix` is the isometric 2:1
//! foreshortening.  It makes the space **non-metric**: one tile spans 45 px
//! along x but 22.5 px along y, so `length`, `normalize` and `dot` — every
//! operation lighting is built from — silently mean something else along y.
//! Light space drops it, which is why `light_matrix` is `S(scale) · Rz(-45°)`
//! and not `S(scale) · iso_to_cartesian_4()`.
//!
//! `light_dir` is authored with +Z up (`classic-demo/src/lighting.rs` sets
//! `d.z = sin(elevation)`) and `normal_matrix` is `(mat3(light_matrix))⁻ᵀ`, so
//! both live here.  The shadow map must too.

use glam::{Mat4, Vec3, Vec4};

/// World-space margin added around the tilemap box when fitting the light ortho
/// (protects map edges and the near/far planes).
pub const SHADOW_PADDING: f32 = 64.0;

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

/// The directional light's view/projection matrices in light space.
pub struct LightMatrix {
    pub view: Mat4,
    pub proj: Mat4,
    /// `proj * view` — maps light space to light clip space.
    pub view_proj: Mat4,
    /// Width of one shadow-map texel, in world units.  Normal-offset bias
    /// scales with this: the receiver is nudged along its normal by roughly a
    /// texel, which is exactly the distance over which the stored depth is
    /// ambiguous.
    pub world_texel: f32,
}

/// Compute the **light-space** position of a world-metre point:
/// `light_matrix * vertex`, with +Z up.
///
/// This is deliberately *not* the position the terrain rasterises at — see the
/// module header for the two distortions that separate them.  `light_dir`,
/// `vNormal` and this function must all agree on +Z up and on a metric frame.
/// See `shadow_space_sees_the_sun_at_its_authored_elevation` and
/// `light_space_is_isotropic`.
fn world_corner(light_matrix: &Mat4, p: Vec3) -> Vec3 {
    let v = *light_matrix * Vec4::new(p.x, p.y, p.z, 1.0);
    Vec3::new(v.x, v.y, v.z)
}

/// Fit an orthographic light-space box around the tilemap AABB plus the shadow
/// casters (sprite world quads) standing on it.
///
/// * `light_matrix` — the world→light transform, exactly as passed to the
///   shaders (`classic_core::math::iso_world_light_matrix`).
/// * `size_x` / `size_y` — tile dimensions; `z_max` — max terrain height in
///   **metres** (`max(height_data)`).
/// * `light_dir` — the toward-light direction (same space as the shader's
///   `light_direction`); normalized internally.
/// * `padding` — world-space margin added around the box (protects the map
///   edges and reduces clamp artifacts).
/// * `casters` — world-metre sprite model matrices.  The quads stand up out of
///   the terrain plane and must fit inside the box or their shadows clip at the
///   near plane.
/// * `shadow_map_size` — resolution of the depth target, used to report
///   [`LightMatrix::world_texel`].
#[allow(clippy::too_many_arguments)]
pub fn fit_directional_light_matrix(
    light_matrix: &Mat4,
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
    // `h ∈ [0, z_max]`.
    let wx = size_x * classic_core::tilemap::TILE_M;
    let wy = -size_y * classic_core::tilemap::TILE_M;
    for x in [0.0f32, wx] {
        for y in [wy, 0.0f32] {
            for z in [0.0f32, z_max] {
                corners.push(world_corner(light_matrix, Vec3::new(x, y, z)));
            }
        }
    }
    for model in casters {
        for (u, v) in [(0.0f32, 0.0f32), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
            let w = *model * Vec4::new(u, v, 0.0, 1.0);
            corners.push(world_corner(light_matrix, Vec3::new(w.x, w.y, w.z)));
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
    use classic_core::tilemap::{PPM_TARGET, TILE_M};
    use glam::{Vec3Swizzles, Vec4Swizzles};
    use std::f32::consts::FRAC_1_SQRT_2;

    /// The world→light matrix the tilemap + sprite shadow passes use.
    fn world_light(scale: [f32; 3]) -> Mat4 {
        classic_core::math::iso_world_light_matrix(scale.into())
    }

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

    /// Light-space corners of a world-metre sprite model.
    fn sprite_light_corners(lm: &Mat4, model: &Mat4) -> [Vec3; 4] {
        let mut out = [Vec3::ZERO; 4];
        for (i, (u, v)) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)].iter().enumerate() {
            let w = *model * Vec4::new(*u, *v, 0.0, 1.0);
            out[i] = world_corner(lm, Vec3::new(w.x, w.y, w.z));
        }
        out
    }

    fn assert_corners_inside(m: &LightMatrix, lm: &Mat4, size: f32, zmax: f32) {
        let wx = size * TILE_M;
        let wy = -size * TILE_M;
        for x in [0.0, wx] {
            for y in [wy, 0.0] {
                for z in [0.0, zmax] {
                    let w = world_corner(lm, Vec3::new(x, y, z));
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
        let lm = world_light([45.0, 45.0, 1.0]);
        let m = fit_directional_light_matrix(
            &lm,
            200.0,
            200.0,
            0.0,
            Vec3::new(0.45, -0.35, 0.82),
            10.0,
            &[],
            SHADOW_MAP_SIZE_F,
        );
        assert_corners_inside(&m, &lm, 200.0, 0.0);
    }

    #[test]
    fn relief_tilemap_corners_land_in_ndc() {
        let lm = world_light([45.0, 45.0, 1.0]);
        let zmax = 20.0; // 20 m of relief
        let m = fit_directional_light_matrix(
            &lm,
            400.0,
            400.0,
            zmax,
            Vec3::new(0.45, -0.35, 0.82),
            50.0,
            &[],
            SHADOW_MAP_SIZE_F,
        );
        assert_corners_inside(&m, &lm, 400.0, zmax);
    }

    #[test]
    fn vertical_light_uses_alternate_up() {
        let lm = world_light([45.0, 45.0, 1.0]);
        let m = fit_directional_light_matrix(
            &lm,
            100.0,
            100.0,
            500.0,
            Vec3::Z,
            10.0,
            &[],
            SHADOW_MAP_SIZE_F,
        );
        assert_corners_inside(&m, &lm, 100.0, 500.0);
    }

    #[test]
    fn caster_world_quad_fits_inside_box() {
        // A sprite world quad standing up out of the ground plane must land
        // inside the fitted box — otherwise its shadow clips at the near plane.
        let lm = world_light([45.0, 45.0, 1.0]);
        let caster = sprite_model(Vec3::ZERO, 100.0 / PPM_TARGET, 400.0 / PPM_TARGET);
        let m = fit_directional_light_matrix(
            &lm,
            200.0,
            200.0,
            0.0,
            Vec3::new(0.45, -0.35, 0.82),
            64.0,
            &[caster],
            SHADOW_MAP_SIZE_F,
        );
        for c in sprite_light_corners(&lm, &caster) {
            let clip = m.view_proj * Vec4::new(c.x, c.y, c.z, 1.0);
            let ndc = clip.xyz() / clip.w;
            assert!(ndc.x >= -1.0 && ndc.x <= 1.0, "caster ndc.x={}", ndc.x);
            assert!(ndc.y >= -1.0 && ndc.y <= 1.0, "caster ndc.y={}", ndc.y);
            assert!(ndc.z >= -1.0 && ndc.z <= 1.0, "caster ndc.z={}", ndc.z);
        }
    }

    /// A sprite quad must be authored as *standing* geometry: its height extent
    /// lives in world Z, and it does not recede into the scene (its light-space
    /// depth is constant), or it would shadow tiles behind it.
    #[test]
    fn sprite_world_quad_stands_up_out_of_the_ground() {
        let lm = world_light([45.0, 45.0, 1.0]);
        let model = sprite_model(Vec3::ZERO, 100.0 / PPM_TARGET, 400.0 / PPM_TARGET);
        let corners = sprite_light_corners(&lm, &model);

        let z_min = corners.iter().map(|c| c.z).fold(f32::MAX, f32::min);
        let z_max = corners.iter().map(|c| c.z).fold(f32::MIN, f32::max);
        assert!(
            (z_max - z_min - 400.0).abs() < 1e-2,
            "sprite spans {:.1} in light-space height, expected 400",
            z_max - z_min
        );

        let y_min = corners.iter().map(|c| c.y).fold(f32::MAX, f32::min);
        let y_max = corners.iter().map(|c| c.y).fold(f32::MIN, f32::max);
        assert!((y_max - y_min).abs() < 1e-2, "sprite recedes in light-space depth");
    }

    /// The basetest sun: azimuth 120°, elevation 30°, already unit length.
    const SUN: Vec3 = Vec3::new(0.75, 0.433_012_7, 0.5);

    /// The real depth-target resolution, so the tests cannot drift from it.
    const SHADOW_MAP_SIZE_F: f32 = classic_gfx::SHADOW_MAP_SIZE as f32;

    /// **Space contract.**  `light_dir` is authored in light space (+Z up;
    /// `classic-demo/src/lighting.rs` sets `d.z = sin(elevation)`), and
    /// `vNormal` is transformed into that same space.  The shadow map must
    /// therefore project positions in *that* space too.
    ///
    /// This asserts the shadow map actually sees the sun at its authored
    /// elevation.  Projecting sheared ("hybrid") positions, where a height
    /// increase moves a vertex along `(0,-1,1)`, instead presents the sun at
    /// ~2.7° — a near-degenerate grazing angle that casts no usable shadow.
    #[test]
    fn shadow_space_sees_the_sun_at_its_authored_elevation() {
        let lm = world_light([45.0, 45.0, 1.0]);

        let ground = world_corner(&lm, Vec3::new(0.0, 0.0, 0.0));
        let raised = world_corner(&lm, Vec3::new(0.0, 0.0, 1.0));
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
    ///
    /// Both endpoints go through `world_corner`, so this exercises the actual
    /// vertex transform.  (Constructing the receiver as `caster - light_dir*t`
    /// instead would be vacuous: an orthographic projection along `light_dir`
    /// maps any such pair to one texel by construction, whatever the shear.)
    #[test]
    fn caster_and_the_ground_it_shadows_share_a_shadow_texel() {
        let lm = world_light([45.0, 45.0, 1.0]);
        let m =
            fit_directional_light_matrix(&lm, 200.0, 200.0, 0.0, SUN, 64.0, &[], SHADOW_MAP_SIZE_F);

        // A terrain vertex at world (50·TILE_M, −50·TILE_M) standing `h` metres
        // proud of the map.
        let base = Vec3::new(50.0 * TILE_M, -50.0 * TILE_M, 0.0);
        let h = 256.0 / PPM_TARGET; // metres
        let caster = world_corner(&lm, base + Vec3::new(0.0, 0.0, h));

        // Where the sun ray through it meets z = 0, in light space (SUN is
        // authored there).  Round-trip the receiver through world space so both
        // endpoints still go through `world_corner`.
        let drop = Vec3::new(
            -SUN.x * h * PPM_TARGET / SUN.z,
            -SUN.y * h * PPM_TARGET / SUN.z,
            -h * PPM_TARGET,
        );
        let receiver_light = caster + drop;
        let receiver_world = lm.inverse().transform_point3(receiver_light);
        let receiver = world_corner(&lm, receiver_world);

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

    /// The light-space transform must carry height in **z alone**.
    ///
    /// This test previously asserted the opposite — that raising a tile also
    /// shifted world y down, replicating the vertex shader's isometric shear.
    /// That shear belongs to the rasterised screen space only; applying it here
    /// is what made the sun read as 2.7° elevation and the shadow map useless.
    #[test]
    fn light_space_carries_height_in_z_only() {
        let lm = world_light([45.0, 45.0, 1.0]);
        let base = Vec3::new(3.0 * TILE_M, -4.0 * TILE_M, 0.0);

        let flat = world_corner(&lm, base);
        let raised = world_corner(&lm, base + Vec3::new(0.0, 0.0, 500.0 / PPM_TARGET));

        // Raising the tile by 500 px moves it 500 px along +Z and nowhere else.
        assert!((raised.x - flat.x).abs() < 1e-2, "height leaked into x");
        assert!((raised.y - flat.y).abs() < 1e-2, "height leaked into y (the iso shear)");
        assert!((raised.z - flat.z - 500.0).abs() < 1e-2, "height did not land in z");
    }

    /// **Space contract.**  Light space must be *metric*: a one-tile step along
    /// `+tx`, a one-tile step along `+ty`, and `TILE_M` metres of height must
    /// all be the same light-space distance (`TILE_PX`).
    ///
    /// This is the invariant the isometric `diag(1, 0.5, 1)` squash breaks.
    /// Under it a `+ty` step measured 22.5 px against a `+tx` step's 45 px, so
    /// every `length()` / `normalize()` / `dot()` in the lighting path was
    /// evaluating a world compressed 2× along one axis: point-light pools came
    /// out as screen-space circles instead of ground-plane ellipses, and no
    /// choice of sprite normal frame could agree with the terrain's.
    #[test]
    fn light_space_is_isotropic() {
        let lm = world_light([45.0, 45.0, 1.0]);

        let o = world_corner(&lm, Vec3::ZERO);
        let dx = world_corner(&lm, Vec3::new(TILE_M, 0.0, 0.0)) - o;
        let dy = world_corner(&lm, Vec3::new(0.0, -TILE_M, 0.0)) - o;
        let dz = world_corner(&lm, Vec3::new(0.0, 0.0, TILE_M)) - o;

        for (name, d) in [("+tx", dx), ("+ty", dy), ("+z", dz)] {
            assert!(
                (d.length() - 45.0).abs() < 1e-2,
                "a one-tile step along {name} spans {:.2} px, expected 45 — \
                 light space is not metric",
                d.length()
            );
        }
        // ...and the two ground axes must stay perpendicular.
        assert!(
            dx.normalize().dot(dy.normalize()).abs() < 1e-3,
            "tile axes are not orthogonal in light space"
        );
    }

    /// **Space contract.**  A flying sprite's altitude must land in light-space
    /// **z**, not y.  Lifting the whole quad along world +Z raises its
    /// light-space height by the same amount and leaves its depth alone.
    #[test]
    fn sprite_altitude_lands_in_height_not_depth() {
        let lm = world_light([45.0, 45.0, 1.0]);
        let width_m = 100.0 / PPM_TARGET;
        let height_m = 400.0 / PPM_TARGET;

        let grounded = sprite_light_corners(&lm, &sprite_model(Vec3::ZERO, width_m, height_m));
        let flying = sprite_light_corners(
            &lm,
            &sprite_model(Vec3::new(0.0, 0.0, 320.0 / PPM_TARGET), width_m, height_m),
        );

        for (g, f) in grounded.iter().zip(flying.iter()) {
            assert!((f.y - g.y).abs() < 1e-2, "altitude leaked into light-space y");
            assert!(
                (f.z - g.z - 320.0).abs() < 1e-2,
                "altitude did not land in light-space z ({:.1} vs {:.1})",
                f.z,
                g.z
            );
        }
    }
}
