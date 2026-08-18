//! Verification of the lunar terrain generator.
//!
//! These tests are the primary regression net for the lunar scene: they are
//! pure CPU, need no GL, and check the *gameplay guarantees* (pathability,
//! buildable area, reachable spawns) rather than pixel output.

use classic_core::pathfinder::find_path;
use classic_core::terrain::lunar::{generate_lunar, LandingZone, LunarParams, LunarTerrain};
use classic_core::terrain::material::{material_for_tile_id, tile_count};
use classic_core::terrain::tileset::{build_default_lunar_tileset, build_lunar_tileset};
use classic_core::tilemap::{bilinear_height, build_mesh, build_tile_texture};

/// Smaller than the shipping 200x200 so the suite stays fast; every invariant
/// under test is scale-independent.
fn params() -> LunarParams {
    LunarParams { seed: String::from("test-apollo"), size_x: 96, size_y: 96, ..Default::default() }
}

fn gen() -> LunarTerrain {
    generate_lunar(&params())
}

/// Per-tile gradient magnitude, in height units per tile — the same formula
/// the generator uses to derive walkability.
fn tile_slope(t: &LunarTerrain, tx: i32, ty: i32) -> f32 {
    let vw = t.size_x as usize + 1;
    let i = ty as usize * vw + tx as usize;
    let nw = t.heights[i];
    let ne = t.heights[i + 1];
    let sw = t.heights[i + vw];
    let se = t.heights[i + vw + 1];
    let dzdx = ((ne + se) - (nw + sw)) * 0.5;
    let dzdy = ((sw + se) - (nw + ne)) * 0.5;
    (dzdx * dzdx + dzdy * dzdy).sqrt()
}

// ---------------------------------------------------------------------------
// determinism
// ---------------------------------------------------------------------------

#[test]
fn generation_is_deterministic() {
    let a = gen();
    let b = gen();
    assert_eq!(a.heights, b.heights, "heights differ between identical runs");
    assert_eq!(a.tiles, b.tiles, "tiles differ between identical runs");
    assert_eq!(a.nav, b.nav, "nav differs between identical runs");
    assert_eq!(a.spawn_points, b.spawn_points);
    assert_eq!(a.stats, b.stats);
}

#[test]
fn different_seeds_produce_different_maps() {
    let a = gen();
    let b = generate_lunar(&LunarParams { seed: String::from("other-seed"), ..params() });
    assert_ne!(a.heights, b.heights, "distinct seeds produced identical terrain");
}

// ---------------------------------------------------------------------------
// engine contract: array shapes and value ranges
// ---------------------------------------------------------------------------

/// `build_mesh` asserts these exact lengths; getting the vertex/tile grid
/// distinction wrong is the single easiest way to break the tilemap.
#[test]
fn grid_lengths_match_the_tilemap_contract() {
    let t = gen();
    let vertices = (t.size_x as usize + 1) * (t.size_y as usize + 1);
    let tiles = (t.size_x as usize) * (t.size_y as usize);
    assert_eq!(t.heights.len(), vertices, "heights must be a vertex grid");
    assert_eq!(t.tiles.len(), tiles, "tiles must be a tile grid");
    assert_eq!(t.nav.len(), tiles, "nav must be a tile grid");
}

/// A tile with id 0 and four zero corners is silently dropped by `build_mesh`,
/// leaving a hole in the map.  Neither condition may ever occur.
#[test]
fn no_tile_is_dropped_by_the_mesh_builder() {
    let t = gen();
    assert!(t.tiles.iter().all(|id| *id >= 1), "tile id 0 is reserved and must never be emitted");
    assert!(
        t.heights.iter().all(|h| *h > 0.0),
        "every vertex must sit above zero so no tile can be culled"
    );
}

/// Tile ids travel through a single 8-bit texture channel, decoded in the
/// fragment shader as `floor(r * 256.0)`.
#[test]
fn tile_ids_survive_the_data_texture_round_trip() {
    let t = gen();
    let max_id = *t.tiles.iter().max().unwrap();
    assert!(max_id <= tile_count(), "tile id {max_id} exceeds the material table");
    assert!(max_id < 255, "tile id {max_id} cannot be encoded in the data texture");

    for id in t.tiles.iter() {
        assert!(material_for_tile_id(*id).is_some(), "tile id {id} maps to no material");
    }

    let (pixels, w, h) = build_tile_texture(t.size_x, t.size_y, &t.tiles);
    assert_eq!((w, h), (t.size_x as u32, t.size_y as u32));
    for (i, id) in t.tiles.iter().enumerate() {
        let stored = pixels[i * 4] as u32;
        // The shader computes floor(stored / 255 * 256), which is exact for
        // every value below 255.
        let decoded = ((stored as f32 / 255.0) * 256.0).floor() as u32;
        assert_eq!(decoded, *id, "tile {i} decoded as {decoded}, expected {id}");
    }
}

#[test]
fn output_feeds_build_mesh_without_panicking() {
    let t = gen();
    let (mesh, vcount) = build_mesh(t.size_x, t.size_y, &t.tiles, &t.heights, 32.0);
    assert!(vcount > 0, "mesh is empty");
    assert_eq!(mesh.len(), vcount * 9, "mesh stride must be 9 floats per vertex");
    assert!(mesh.iter().all(|f| f.is_finite()), "mesh contains NaN or infinity");
}

#[test]
fn heights_are_finite_and_within_the_configured_range() {
    let p = params();
    let t = generate_lunar(&p);
    let mut min = f32::MAX;
    let mut max = f32::MIN;
    for h in &t.heights {
        assert!(h.is_finite(), "non-finite height");
        min = min.min(*h);
        max = max.max(*h);
    }
    assert!((min - p.floor_height).abs() < 1e-3, "lowest point should sit at the floor: {min}");
    assert!(max <= p.max_height + 1e-3, "height {max} exceeded the ceiling");
    assert!(max - min > 1.0, "terrain is suspiciously flat overall: range {}", max - min);

    // Bilinear sampling is used for mouse picking and agent placement.
    for (px, py) in [(0.0, 0.0), (12.5, 7.25), (t.size_x as f32, t.size_y as f32)] {
        let h = bilinear_height(&t.heights, t.size_x, t.size_y, px, py);
        assert!(h.is_finite() && h > 0.0, "bilinear_height({px}, {py}) = {h}");
    }
}

// ---------------------------------------------------------------------------
// terrain character
// ---------------------------------------------------------------------------

/// Relaxation models talus mass wasting.  It is what keeps interior slopes
/// bounded, which matters because `build_mesh` emits no wall geometry for
/// interior height discontinuities.
#[test]
fn slope_relaxation_bounds_every_interior_slope() {
    let p = params();
    let t = generate_lunar(&p);
    assert!(
        t.stats.relax_iterations_used <= p.relax_iterations,
        "relaxation exceeded its iteration budget"
    );
    // Landing-zone skirts are stamped after relaxation, so allow a small
    // margin over the talus cap.
    assert!(
        t.stats.max_slope_actual <= p.max_slope * 1.25,
        "worst slope {} far exceeds the angle of repose {}",
        t.stats.max_slope_actual,
        p.max_slope
    );
}

/// Two thresholds must produce two regimes: if everything were walkable the
/// crater rims would provide no chokepoints at all.
#[test]
fn impassable_terrain_exists_but_does_not_dominate() {
    let t = gen();
    let w = t.stats.walkable_fraction;
    assert!(w > 0.55, "only {:.1}% of the map is walkable", w * 100.0);
    assert!(w < 0.995, "{:.1}% walkable — no chokepoints were generated", w * 100.0);
}

/// The RTS requirement: enough genuinely flat ground to build on.
#[test]
fn map_has_substantial_buildable_area() {
    let t = gen();
    assert!(
        t.stats.buildable_fraction > 0.25,
        "only {:.1}% of the map is buildable",
        t.stats.buildable_fraction * 100.0
    );
}

#[test]
fn maria_and_highlands_both_exist() {
    let t = gen();
    let m = t.stats.mare_fraction;
    assert!(
        (0.15..0.85).contains(&m),
        "mare fraction {:.2} means the mask degenerated to a single terrain type",
        m
    );
}

#[test]
fn craters_are_actually_stamped() {
    let p = params();
    let flat = generate_lunar(&LunarParams { crater_density: 0.0, ..p.clone() });
    let cratered = generate_lunar(&p);
    assert!(cratered.stats.craters > 0, "no craters survived site rejection");
    assert_ne!(flat.heights, cratered.heights, "crater stamping had no effect");

    // Compare detrended fields: the final normalisation shifts everything to
    // a common floor, so absolute heights say nothing about excavation.
    let detrend = |t: &LunarTerrain| {
        let mean: f32 = t.heights.iter().sum::<f32>() / t.heights.len() as f32;
        t.heights.iter().map(|h| h - mean).collect::<Vec<f32>>()
    };
    let a = detrend(&flat);
    let b = detrend(&cratered);
    let deepest = a.iter().zip(&b).map(|(f, c)| c - f).fold(f32::MAX, f32::min);
    let highest = a.iter().zip(&b).map(|(f, c)| c - f).fold(f32::MIN, f32::max);
    assert!(deepest < -0.5, "craters excavated almost nothing (deepest cut {deepest})");
    assert!(highest > 0.1, "craters deposited no rim or ejecta (highest pile {highest})");
}

/// Craters are allowed to clip a pad's skirt (the pad is stamped flat
/// afterwards), but nothing may leave a pit in the core.  A tile only counts
/// as inside the core when all four of its corners are, hence the one-tile
/// inset.
#[test]
fn landing_zone_cores_stay_flat_despite_nearby_craters() {
    let t = gen();
    for z in &t.landing_zones {
        let core = z.radius - 1.0;
        let r = core.ceil() as i32;
        let mut checked = 0;
        for dy in -r..=r {
            for dx in -r..=r {
                let tx = z.x.floor() as i32 + dx;
                let ty = z.y.floor() as i32 + dy;
                if tx < 0 || ty < 0 || tx >= t.size_x || ty >= t.size_y {
                    continue;
                }
                let cx = tx as f32 + 0.5 - z.x;
                let cy = ty as f32 + 0.5 - z.y;
                if (cx * cx + cy * cy).sqrt() > core {
                    continue;
                }
                assert!(
                    tile_slope(&t, tx, ty) < 0.02,
                    "landing zone at ({}, {}) is not flat at ({tx}, {ty}): slope {}",
                    z.x,
                    z.y,
                    tile_slope(&t, tx, ty)
                );
                checked += 1;
            }
        }
        assert!(checked > 100, "only {checked} tiles checked in a radius-{} pad", z.radius);
    }
}

// ---------------------------------------------------------------------------
// gameplay guarantees
// ---------------------------------------------------------------------------

#[test]
fn landing_zones_are_placed_and_flat() {
    let p = params();
    let t = generate_lunar(&p);
    assert_eq!(t.landing_zones.len(), p.auto_landing_zones as usize);
    assert_eq!(t.spawn_points.len(), t.landing_zones.len());

    for z in &t.landing_zones {
        let vw = t.size_x as usize + 1;
        let r = z.radius.ceil() as i32;
        let mut heights = Vec::new();
        for dy in -r..=r {
            for dx in -r..=r {
                if (dx * dx + dy * dy) as f32 > z.radius * z.radius {
                    continue;
                }
                let x = (z.x as i32 + dx).clamp(0, t.size_x) as usize;
                let y = (z.y as i32 + dy).clamp(0, t.size_y) as usize;
                heights.push(t.heights[y * vw + x]);
            }
        }
        let lo = heights.iter().cloned().fold(f32::MAX, f32::min);
        let hi = heights.iter().cloned().fold(f32::MIN, f32::max);
        assert!(hi - lo < 0.15, "landing zone at ({}, {}) varies by {}", z.x, z.y, hi - lo);
    }
}

#[test]
fn spawn_points_are_in_bounds_and_walkable() {
    let t = gen();
    for (x, y) in &t.spawn_points {
        assert!(
            *x >= 0 && *y >= 0 && *x < t.size_x && *y < t.size_y,
            "spawn ({x}, {y}) out of bounds"
        );
        let i = *y as usize * t.size_x as usize + *x as usize;
        assert_eq!(t.nav[i], 1, "spawn ({x}, {y}) is on blocked terrain");
    }
}

/// The headline gameplay guarantee, checked with the engine's own A*: every
/// start must be able to reach every other start.
#[test]
fn every_spawn_pair_is_mutually_reachable() {
    let t = gen();
    let nav: Vec<i32> = t.nav.iter().map(|v| *v as i32).collect();
    assert!(t.spawn_points.len() >= 2, "need at least two spawns to test reachability");
    for (i, a) in t.spawn_points.iter().enumerate() {
        for b in t.spawn_points.iter().skip(i + 1) {
            let path = find_path(&nav, t.size_x, t.size_y, *a, *b);
            assert!(path.is_some(), "no path from {a:?} to {b:?}");
        }
    }
}

/// Reachability must hold across seeds, not just the one the defaults were
/// tuned against — otherwise the corridor carving is not doing its job.
#[test]
fn reachability_holds_across_many_seeds() {
    for seed in ["a", "b", "c", "d", "e", "f", "tranquillity", "serenitatis"] {
        let t = generate_lunar(&LunarParams { seed: seed.to_string(), ..params() });
        let nav: Vec<i32> = t.nav.iter().map(|v| *v as i32).collect();
        for (i, a) in t.spawn_points.iter().enumerate() {
            for b in t.spawn_points.iter().skip(i + 1) {
                assert!(
                    find_path(&nav, t.size_x, t.size_y, *a, *b).is_some(),
                    "seed '{seed}': no path from {a:?} to {b:?} \
                     (corridors carved: {})",
                    t.stats.corridors_carved
                );
            }
        }
        assert!(t.stats.buildable_fraction > 0.15, "seed '{seed}': too little buildable ground");
    }
}

/// Walkable ground that cannot be reached is worse than no ground at all —
/// the pathfinder would happily route an agent into an unreachable pocket.
#[test]
fn all_walkable_tiles_belong_to_one_component() {
    let t = gen();
    let nav: Vec<i32> = t.nav.iter().map(|v| *v as i32).collect();
    let origin = t.spawn_points[0];
    // Sample rather than test all 9216 cells: A* over the full grid for every
    // walkable tile would dominate the suite runtime.
    let mut checked = 0;
    for (i, v) in t.nav.iter().enumerate() {
        if *v != 1 || i % 37 != 0 {
            continue;
        }
        let cell = ((i % t.size_x as usize) as i32, (i / t.size_x as usize) as i32);
        assert!(
            find_path(&nav, t.size_x, t.size_y, origin, cell).is_some(),
            "walkable tile {cell:?} is unreachable from spawn {origin:?}"
        );
        checked += 1;
    }
    assert!(checked > 50, "sampling stride selected too few tiles ({checked})");
}

#[test]
fn explicit_landing_zones_are_honoured() {
    let zone = LandingZone { x: 30.0, y: 40.0, radius: 6.0, skirt: 5.0 };
    let t = generate_lunar(&LunarParams { landing_zones: vec![zone], ..params() });
    assert_eq!(t.landing_zones, vec![zone]);
    assert_eq!(t.spawn_points.len(), 1);
}

// ---------------------------------------------------------------------------
// performance
// ---------------------------------------------------------------------------

/// The generator runs at startup and behind an interactive "Regenerate"
/// button, so the shipping map size has to stay responsive even in a debug
/// build (release is roughly 10x faster).
#[test]
fn full_size_generation_is_fast_enough() {
    let p = LunarParams::default();
    let start = std::time::Instant::now();
    let t = generate_lunar(&p);
    let elapsed = start.elapsed();
    assert_eq!((t.size_x, t.size_y), (p.size_x, p.size_y));
    assert!(
        elapsed.as_secs_f32() < 40.0,
        "{}x{} generation took {elapsed:?} in a debug build",
        p.size_x,
        p.size_y
    );
}

/// The defaults are tuned at one size but the scale-free parameters should
/// hold across a wide range: doubling the map must not change the *character*
/// of the terrain, only how much of it there is.
#[test]
fn terrain_character_is_stable_across_map_sizes() {
    let mut prev: Option<(f32, f32)> = None;
    for size in [200, 400, 600] {
        let t = generate_lunar(&LunarParams { size_x: size, size_y: size, ..params() });

        // Crater population must track area, since density is per-1000-tiles.
        // The surviving rate sits below `crater_density` because of the mare
        // and landing-zone rejections, and creeps up slightly with size as the
        // fixed-radius zone exclusions cover proportionally less of the map.
        let per_1000 = t.stats.craters as f32 / (size as f32 * size as f32 / 1000.0);
        assert!(
            (9.0..15.0).contains(&per_1000),
            "{size}x{size}: {per_1000:.1} craters per 1000 tiles is off-trend"
        );

        assert!(
            t.stats.walkable_fraction > 0.6,
            "{size}x{size}: only {:.0}% walkable",
            t.stats.walkable_fraction * 100.0
        );
        assert!(
            t.stats.buildable_fraction > 0.25,
            "{size}x{size}: only {:.0}% buildable",
            t.stats.buildable_fraction * 100.0
        );
        assert!(
            t.stats.max_slope_actual <= LunarParams::default().max_slope * 1.25,
            "{size}x{size}: relaxation did not converge (worst slope {})",
            t.stats.max_slope_actual
        );
        // Relief is set by the crater depth law and the highland amplitude,
        // both absolute, so it must not drift with map size.
        let relief = t.stats.max_height - t.stats.min_height;
        assert!((4.0..12.0).contains(&relief), "{size}x{size}: relief {relief:.1} out of band");

        if let Some((pw, pb)) = prev {
            assert!(
                (t.stats.walkable_fraction - pw).abs() < 0.15
                    && (t.stats.buildable_fraction - pb).abs() < 0.15,
                "{size}x{size}: terrain character shifted vs the previous size"
            );
        }
        prev = Some((t.stats.walkable_fraction, t.stats.buildable_fraction));
    }
}

// ---------------------------------------------------------------------------
// tileset
// ---------------------------------------------------------------------------

#[test]
fn tileset_has_the_expected_geometry_and_is_deterministic() {
    let (a, w, h) = build_default_lunar_tileset("moon");
    assert_eq!((w, h), (256, 256));
    assert_eq!(a.len(), (w * h * 4) as usize);
    let (b, _, _) = build_default_lunar_tileset("moon");
    assert_eq!(a, b, "tileset generation is not deterministic");
    let (c, _, _) = build_default_lunar_tileset("mars");
    assert_ne!(a, c, "tileset ignores its seed");
    assert!(a.chunks(4).all(|p| p[3] == 255), "tileset must be fully opaque");
}

/// Every cell must wrap: the shader samples cells with `fract()`, so a
/// non-periodic cell would draw a seam wherever two same-id tiles meet.
#[test]
fn tileset_cells_tile_without_seams() {
    let tile_px = 32u32;
    let (rgba, w, _h) = build_lunar_tileset("moon", tile_px, 8, 8);
    let px = |x: u32, y: u32, c: usize| rgba[((y * w + x) * 4) as usize + c] as i32;

    for row in 0..8u32 {
        for col in 0..8u32 {
            let x0 = col * tile_px;
            let y0 = row * tile_px;
            for k in 0..tile_px {
                for c in 0..3 {
                    // Wrapping in x: column 0 continues from column 31.
                    let left = px(x0, y0 + k, c);
                    let right = px(x0 + tile_px - 1, y0 + k, c);
                    assert!(
                        (left - right).abs() <= 40,
                        "cell ({col}, {row}) has an x seam at row {k}: {left} vs {right}"
                    );
                    let top = px(x0 + k, y0, c);
                    let bottom = px(x0 + k, y0 + tile_px - 1, c);
                    assert!(
                        (top - bottom).abs() <= 40,
                        "cell ({col}, {row}) has a y seam at column {k}: {top} vs {bottom}"
                    );
                }
            }
        }
    }
}

/// The generator must never emit an id that lands on an unpainted cell.
#[test]
fn every_generated_tile_id_maps_into_the_tileset() {
    let t = gen();
    let cells = 8 * 8;
    for id in t.tiles.iter() {
        assert!(*id < cells, "tile id {id} falls outside the 8x8 tileset");
    }
}

#[test]
fn material_classes_are_visually_distinct() {
    let (rgba, w, _h) = build_lunar_tileset("moon", 32, 8, 8);
    // Mean luminance of a cell.
    let mean = |id: u32| {
        let (col, row) = (id % 8, id / 8);
        let mut sum = 0u64;
        for y in 0..32 {
            for x in 0..32 {
                let o = (((row * 32 + y) * w + col * 32 + x) * 4) as usize;
                sum += rgba[o] as u64 + rgba[o + 1] as u64 + rgba[o + 2] as u64;
            }
        }
        (sum as f32) / (32.0 * 32.0 * 3.0)
    };
    use classic_core::terrain::material::{tile_id, LunarMaterial};
    let mare = mean(tile_id(LunarMaterial::MareSmooth, 0));
    let regolith = mean(tile_id(LunarMaterial::Regolith, 0));
    let ray = mean(tile_id(LunarMaterial::Ray, 0));
    assert!(mare < regolith, "mare ({mare}) should be darker than regolith ({regolith})");
    assert!(regolith < ray, "rays ({ray}) should be the brightest material");
}
