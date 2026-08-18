use classic_core::simplex_noise::SimplexNoise;
use classic_core::terrain::fractal::{
    domain_warp, smoothstep, tiling_fbm_2d, tiling_noise_2d, Fbm,
};

#[test]
fn fbm_is_normalised_to_single_octave_range() {
    let n = SimplexNoise::new("fbm");
    for octaves in [1u32, 3, 6, 8] {
        let f = Fbm::standard(octaves, 1.0 / 16.0);
        let mut lo = f64::MAX;
        let mut hi = f64::MIN;
        for y in 0..120 {
            for x in 0..120 {
                let v = f.sample(&n, x as f64, y as f64);
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
        assert!(lo >= -1.0 && hi <= 1.0, "octaves={octaves} range [{lo}, {hi}] escaped [-1, 1]");
        // A degenerate (constant) field would break every downstream layer.
        assert!(hi - lo > 0.2, "octaves={octaves} produced a nearly flat field");
    }
}

#[test]
fn ridged_and_billow_stay_in_unit_range() {
    let n = SimplexNoise::new("ridged");
    let f = Fbm::standard(4, 1.0 / 20.0);
    for y in 0..80 {
        for x in 0..80 {
            let r = f.sample_ridged(&n, x as f64, y as f64);
            let b = f.sample_billow(&n, x as f64, y as f64);
            assert!((0.0..=1.0).contains(&r), "ridged out of range: {r}");
            assert!((0.0..=1.0).contains(&b), "billow out of range: {b}");
        }
    }
}

#[test]
fn fbm_is_deterministic_and_seed_sensitive() {
    let f = Fbm::standard(4, 1.0 / 16.0);
    let a = SimplexNoise::new("seed-a");
    let a2 = SimplexNoise::new("seed-a");
    let b = SimplexNoise::new("seed-b");

    let mut differs = false;
    for i in 0..200 {
        let (x, y) = (i as f64 * 0.7, i as f64 * 1.3);
        assert_eq!(f.sample(&a, x, y), f.sample(&a2, x, y));
        if (f.sample(&a, x, y) - f.sample(&b, x, y)).abs() > 1e-9 {
            differs = true;
        }
    }
    assert!(differs, "different seeds produced identical fields");
}

#[test]
fn smoothstep_matches_glsl_semantics() {
    assert_eq!(smoothstep(0.0, 1.0, -1.0), 0.0);
    assert_eq!(smoothstep(0.0, 1.0, 2.0), 1.0);
    assert!((smoothstep(0.0, 1.0, 0.5) - 0.5).abs() < 1e-12);
    // Degenerate edges must not divide by zero.
    assert_eq!(smoothstep(1.0, 1.0, 0.0), 0.0);
    assert_eq!(smoothstep(1.0, 1.0, 2.0), 1.0);
}

#[test]
fn domain_warp_displaces_but_stays_bounded() {
    let n = SimplexNoise::new("warp");
    let amp = 5.0;
    let mut moved = false;
    for i in 0..100 {
        let (x, y) = (i as f64, i as f64 * 0.5);
        let (wx, wy) = domain_warp(&n, x, y, 1.0 / 30.0, amp);
        assert!((wx - x).abs() <= amp + 1e-9);
        assert!((wy - y).abs() <= amp + 1e-9);
        if (wx - x).abs() > 0.1 {
            moved = true;
        }
    }
    assert!(moved, "domain_warp produced no displacement");
}

/// The tileset relies on this: each cell must abut its own opposite edge
/// without a visible seam.
#[test]
fn tiling_noise_is_exactly_periodic() {
    let n = SimplexNoise::new("tile");
    let period = 32.0;
    for i in 0..32 {
        let v = i as f64;
        let a = tiling_noise_2d(&n, 0.0, v, period, 0.8);
        let b = tiling_noise_2d(&n, period, v, period, 0.8);
        assert!((a - b).abs() < 1e-12, "u seam at v={v}: {a} vs {b}");

        let c = tiling_noise_2d(&n, v, 0.0, period, 0.8);
        let d = tiling_noise_2d(&n, v, period, period, 0.8);
        assert!((c - d).abs() < 1e-12, "v seam at u={v}: {c} vs {d}");
    }
}

#[test]
fn tiling_fbm_is_exactly_periodic_across_octaves() {
    let n = SimplexNoise::new("tile-fbm");
    let period = 32.0;
    for i in 0..32 {
        let v = i as f64;
        let a = tiling_fbm_2d(&n, 0.0, v, period, 3, 0.5);
        let b = tiling_fbm_2d(&n, period, v, period, 3, 0.5);
        assert!((a - b).abs() < 1e-12, "fbm u seam at v={v}: {a} vs {b}");
    }
}

#[test]
fn tiling_noise_actually_varies() {
    let n = SimplexNoise::new("tile-var");
    let mut lo = f64::MAX;
    let mut hi = f64::MIN;
    for y in 0..32 {
        for x in 0..32 {
            let v = tiling_fbm_2d(&n, x as f64, y as f64, 32.0, 3, 0.5);
            lo = lo.min(v);
            hi = hi.max(v);
        }
    }
    assert!(hi - lo > 0.2, "periodic noise is nearly constant: [{lo}, {hi}]");
}
