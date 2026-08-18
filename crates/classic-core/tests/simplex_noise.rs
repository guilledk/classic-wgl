use classic_core::simplex_noise::SimplexNoise;

#[test]
fn deterministic_for_same_string_seed() {
    let a = SimplexNoise::new("42");
    let b = SimplexNoise::new("42");

    for i in 0..20 {
        let x = i as f64 * 0.37;
        let y = i as f64 * 0.71;
        assert_eq!(a.noise_2d(x, y), b.noise_2d(x, y));
    }
}

#[test]
fn deterministic_for_3d_with_same_seed() {
    let a = SimplexNoise::new("classic-wgl");
    let b = SimplexNoise::new("classic-wgl");
    assert_eq!(a.noise_3d(1.0, 2.0, 3.0), b.noise_3d(1.0, 2.0, 3.0));
}

#[test]
fn different_seeds_produce_different_outputs() {
    let a = SimplexNoise::new("classic-wgl-seed-a");
    let b = SimplexNoise::new("classic-wgl-seed-b");

    let any_different = (0..50).any(|i| {
        let v = i as f64 * 0.37;
        a.noise_2d(v, v * 0.5) != b.noise_2d(v, v * 0.5)
    });
    assert!(any_different);
}

#[test]
fn noise_2d_stays_in_range() {
    let n = SimplexNoise::new("7");
    let mut x = -5.0;
    while x <= 5.0 {
        let mut y = -5.0;
        while y <= 5.0 {
            let v = n.noise_2d(x, y);
            assert!(v >= -1.0, "noise_2d({x},{y}) = {v} < -1");
            assert!(v <= 1.0, "noise_2d({x},{y}) = {v} > 1");
            y += 0.5;
        }
        x += 0.5;
    }
}

#[test]
fn noise_3d_stays_in_range() {
    let n = SimplexNoise::new("7");
    for x in -3..=3 {
        for y in -3..=3 {
            for z in -3..=3 {
                let v = n.noise_3d(x as f64, y as f64, z as f64);
                assert!(v >= -1.0, "noise_3d({x},{y},{z}) = {v} < -1");
                assert!(v <= 1.0, "noise_3d({x},{y},{z}) = {v} > 1");
            }
        }
    }
}

#[test]
fn noise_4d_stays_in_range() {
    let n = SimplexNoise::new("7");
    for x in -2..=2 {
        for y in -2..=2 {
            for z in -2..=2 {
                for w in -2..=2 {
                    let v = n.noise_4d(x as f64, y as f64, z as f64, w as f64);
                    assert!(v >= -1.0, "noise_4d({x},{y},{z},{w}) = {v} < -1");
                    assert!(v <= 1.0, "noise_4d({x},{y},{z},{w}) = {v} > 1");
                }
            }
        }
    }
}
