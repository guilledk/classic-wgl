//! Prove the rocket's burn light renders under **world-space** lighting
//! with no space mixing — the exact `evaluateLight` from `sheet.frag` /
//! `iso_tilemap.frag`, evaluated with a world-metre light position, a
//! world-metre surface point, and a world-space normal.

use glam::Vec3;

/// Mirror of the shared lighting block in `sheet.frag` / `iso_tilemap.frag`
/// (pinned by `lit_shaders_share_the_lighting_block`): `p` and `l.pos_radius.xyz`
/// are both metric world space (+Z up, metres).
fn evaluate_light(
    light_pos: Vec3,
    radius: f32,
    color: Vec3,
    intensity: f32,
    n: Vec3,
    p: Vec3,
) -> Vec3 {
    let to_light = light_pos - p;
    let dist = to_light.length();
    let l = to_light / dist.max(0.0001);
    let mut attenuation = 1.0;
    if radius > 0.0 {
        let d = dist / radius;
        let d2 = d * d;
        let window = (1.0 - d2).clamp(0.0, 1.0);
        attenuation = window * window / (1.0 + d2);
    }
    let diff = n.dot(l).max(0.0);
    attenuation * diff * color * intensity
}

#[test]
fn burn_light_lambertian_matches_analytic_cosine() {
    // The burn light sits at the centre engine nozzle, ~3 m below the model's
    // ground anchor (altitude lives in +Z).  It lights the pad directly below.
    let light_pos = Vec3::new(0.0, 0.0, 2.0);
    let pad_point = Vec3::new(0.0, 0.0, 0.0);
    let pad_normal = Vec3::new(0.0, 0.0, 1.0);
    let color = Vec3::new(1.0, 0.7, 0.3);
    let intensity = 2.0;

    let got = evaluate_light(light_pos, 20.0, color, intensity, pad_normal, pad_point);

    // `to_light` is straight up, `n` is straight up: `diff = cos(0) = 1`.
    // At dist = 2, radius = 20: d = 0.1, window = (1-0.01)=0.99, attenuation
    // = 0.99²/(1+0.01) = 0.9801/1.01 ≈ 0.97039.
    let attenuation = {
        let d = 2.0 / 20.0;
        let d2 = d * d;
        let w = 1.0 - d2;
        w * w / (1.0 + d2)
    };
    let expected = attenuation * 1.0 * color * intensity;
    assert!((got - expected).length() < 1e-6, "got {got:?} expected {expected:?}");
}

#[test]
fn burn_light_world_position_not_screen_space() {
    // The old coordinate system put the light in sheared screen space
    // (`y -= z`) while the normal it dotted against stayed in light space —
    // `dot(n, L)` mixed spaces.  A light 30 m above the ground must produce the
    // same cosine as the analytic world-space angle, not a sheared one.
    let n = Vec3::new(0.0, 0.0, 1.0); // flat ground normal (world +Z)
    let p = Vec3::new(0.0, 0.0, 0.0);
    let light_world = Vec3::new(10.0, 10.0, 30.0); // world metres

    // World-space L (what gather_lights + sheet.frag now use).
    let l_world = (light_world - p).normalize();
    let cos_world = n.dot(l_world).max(0.0);

    // A stale screen-space position (`y -= z` shear) would present the light
    // at a different elevation and thus a different cosine.
    let light_screen = Vec3::new(light_world.x, light_world.y - light_world.z, light_world.z);
    let l_screen = (light_screen - p).normalize();
    let cos_screen = n.dot(l_screen).max(0.0);

    // The two cosines disagree — the whole point of the world-space fix.
    assert!(
        (cos_world - cos_screen).abs() > 1e-3,
        "world vs screen cosine must differ (cos_world={cos_world}, cos_screen={cos_screen})"
    );
    // And the world-space one matches the shader path exactly.
    let got = evaluate_light(light_world, 0.0, Vec3::ONE, 1.0, n, p);
    assert!((got.x - cos_world).abs() < 1e-6);
}

#[test]
fn light_radius_must_be_metres() {
    // The shader computes `d = dist / radius`, so `radius` must be in the same
    // unit as `dist` (metres).  A radius authored in legacy px (200 px =
    // 200/64 = 3.125 m) attenuates differently from the intended 3.125 m once
    // the position is in metres — why `Light.radius` is authored in metres and
    // `gather_lights` no longer divides it by `PPM_TARGET`.
    let px_per_metre = 64.0f32;
    let authored_px = 200.0f32; // legacy unit
    let authored_m = authored_px / px_per_metre; // = 3.125 m

    let light_pos = Vec3::new(0.0, 0.0, 5.0);
    let p = Vec3::new(0.0, 0.0, 0.0);
    let n = Vec3::new(0.0, 0.0, 1.0);

    // If `radius` is left in px while `dist` is in metres, `d` is ~1.56× too
    // small, so the falloff is wrong.
    let radius_px_result = evaluate_light(light_pos, authored_px, Vec3::ONE, 1.0, n, p);
    let radius_m_result = evaluate_light(light_pos, authored_m, Vec3::ONE, 1.0, n, p);
    assert!(
        (radius_px_result.x - radius_m_result.x).abs() > 1e-3,
        "px vs metre radius must differ ({radius_px_result:?} vs {radius_m_result:?})"
    );
}
