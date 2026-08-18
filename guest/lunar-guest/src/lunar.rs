//! Procedural lunar surface generator.
//!
//! Produces a height field, a tile-material grid and a navigation grid for the
//! isometric tilemap, from layered simplex noise plus a meteorite impact
//! crater field.
//!
//! # Design
//!
//! The generator is built around one idea: the features that make a lunar
//! surface *look* right are the same features that make it *play* right for an
//! RTS, so they should not be fought against each other.
//!
//! | Layer | Lunar analogue | Gameplay effect |
//! |---|---|---|
//! | Low-frequency mare mask | Maria are basaltic flood plains — genuinely flat | Large organic buildable regions |
//! | Roughness attenuated by the mask | Highlands are crater-saturated, maria are not | Roughness only where it is harmless |
//! | Age-ordered crater field | Young craters overprint old ones | Natural chokepoints and cover |
//! | Wrinkle ridges, mare-weighted | A real mare-only tectonic feature | Low-amplitude visual interest |
//! | Slope relaxation (talus) | Regolith mass wasting at the angle of repose | Bounds every slope on the map |
//! | Landing-zone stamps with a wide skirt | Reads as a dust-filled basin floor | Guaranteed player-start pads |
//! | Bright ejecta rays | Copernicus / Tycho ray systems | Pure albedo, zero gameplay cost |
//!
//! # Two slope thresholds
//!
//! [`LunarParams::max_slope`] is the *physical* cap enforced by relaxation —
//! the angle of repose.  [`LunarParams::nav_max_slope`] is the lower
//! *walkability* cap.  Terrain between the two exists, is steep, and is
//! impassable: that band is what forms crater-rim chokepoints.  Without two
//! separate thresholds you get either absurd cliffs or a map with no
//! impassable terrain at all.
//!
//! # Grid layouts
//!
//! Heights live on a **vertex** grid of `(size_x + 1) * (size_y + 1)` indexed
//! `y * (size_x + 1) + x`; tiles and nav live on a **tile** grid of
//! `size_x * size_y` indexed `y * size_x + x`.  `build_mesh` asserts both.

use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use classic_terrain::fractal::{domain_warp, smoothstep, Fbm};
use crate::material::{tile_id, LunarMaterial};
use classic_terrain::simplex_noise::{Random, SimplexNoise};

/// A guaranteed-flat circular region for an RTS start position or base site.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LandingZone {
    /// Centre in tile coordinates.
    pub x: f32,
    pub y: f32,
    /// Radius of the perfectly flat core, in tiles.
    pub radius: f32,
    /// Width of the smooth blend ring outside the core, in tiles.  A wide
    /// skirt is what stops the zone reading as a cookie-cutter disc.
    pub skirt: f32,
}

/// Tuning parameters for [`generate_lunar`].
#[derive(Clone, Debug, PartialEq)]
pub struct LunarParams {
    /// Seed string.  Every noise layer derives its own generator by
    /// suffixing this, so changing it re-rolls the whole map coherently.
    pub seed: String,
    pub size_x: i32,
    pub size_y: i32,

    // -- macro relief ------------------------------------------------------
    /// Base frequency of the mare/highland mask, in cycles per tile.
    pub mare_frequency: f64,
    /// Mask noise value at the mare/highland boundary.  Lower values mean
    /// more mare (more flat ground).
    pub mare_threshold: f64,
    /// Half-width of the smoothstep at the boundary — the softness of basin
    /// margins.
    pub mare_edge: f64,
    /// Height difference between mare floor and highland crest.
    pub highland_amplitude: f32,
    /// Amplitude of the broad undulation added on top of the mask.
    pub macro_amplitude: f32,

    // -- regolith roughness ------------------------------------------------
    pub regolith: Fbm,
    pub regolith_amplitude: f32,
    /// Fraction of the roughness that survives inside maria.  This single
    /// number is what makes basins flat without making them look synthetic.
    pub mare_roughness: f32,

    // -- wrinkle ridges ----------------------------------------------------
    pub ridge_amplitude: f32,
    pub ridge_frequency: f64,

    // -- crater field ------------------------------------------------------
    /// Impact sites per 1000 tiles.  Expressed as a density rather than an
    /// absolute count so that the surface looks the same at any map size —
    /// an absolute count silently over-saturates small maps.
    pub crater_density: f32,
    pub crater_radius_min: f32,
    pub crater_radius_max: f32,
    /// Power-law exponent for the radius distribution.  Values above 1 bias
    /// hard towards small craters, matching the real size-frequency curve.
    pub crater_size_exponent: f32,
    /// Depth as a fraction of radius for simple craters (real lunar craters
    /// sit near 0.2 of *diameter*, i.e. 0.4 of radius; a little shallower
    /// reads better at this tile scale).
    pub crater_depth_ratio: f32,
    /// Rim height as a fraction of crater depth.
    pub crater_rim_ratio: f32,
    /// Radius above which craters become "complex": flat floor, central peak,
    /// and a much slower depth-with-size growth.
    pub crater_complex_radius: f32,
    /// Extent of the ejecta blanket, in crater radii.
    pub ejecta_extent: f32,
    /// Relative crater density inside maria.  Mare surfaces are far younger
    /// than the highlands, so they have accumulated far fewer impacts — which
    /// is the physical reason the basins stay flat enough to build on.
    pub mare_crater_factor: f32,
    /// The N largest craters also get bright ejecta ray systems.
    pub ray_crater_count: u32,
    /// Extent of the ray system, in crater radii.
    pub ray_extent: f32,

    // -- gameplay ----------------------------------------------------------
    /// Explicit landing zones.  If empty, `auto_landing_zones` are placed.
    pub landing_zones: Vec<LandingZone>,
    pub auto_landing_zones: u32,
    pub landing_zone_radius: f32,
    pub landing_zone_skirt: f32,
    /// Angle-of-repose cap enforced by relaxation, in height units per tile.
    pub max_slope: f32,
    /// Walkability cap, in height units per tile.  Must be below `max_slope`
    /// for impassable terrain to exist at all.
    pub nav_max_slope: f32,
    /// Slope below which terrain counts as buildable (reported in stats).
    pub build_max_slope: f32,
    /// Iteration budget for slope relaxation.  Relaxation stops early once
    /// the worst over-slope falls below `relax_tolerance`.
    pub relax_iterations: u32,
    pub relax_tolerance: f32,

    // -- range -------------------------------------------------------------
    /// Height the lowest point on the map is lifted to.  Must be > 0: a tile
    /// whose four corners are all exactly zero is skipped by `build_mesh`.
    pub floor_height: f32,
    /// Hard ceiling applied after everything else.
    pub max_height: f32,
}

impl Default for LunarParams {
    /// Defaults tuned for a 400x400 map at `LUNAR_HEIGHT_SCALE`, tile scale 45.
    ///
    /// Most parameters are in scale-free units — frequencies in cycles per
    /// tile, crater population as a density per 1000 tiles, amplitudes in
    /// height units — so they hold at any map size.  Only the genuinely
    /// absolute quantities (`size_*`, `auto_landing_zones`, `ray_crater_count`)
    /// need revisiting when the map grows.
    fn default() -> Self {
        Self {
            seed: String::from("apollo"),
            size_x: 400,
            size_y: 400,

            mare_frequency: 1.0 / 70.0,
            mare_threshold: -0.02,
            mare_edge: 0.28,
            highland_amplitude: 2.6,
            macro_amplitude: 0.5,

            regolith: Fbm::standard(5, 1.0 / 22.0),
            regolith_amplitude: 0.8,
            mare_roughness: 0.14,

            ridge_amplitude: 0.4,
            ridge_frequency: 1.0 / 55.0,

            crater_density: 14.0,
            crater_radius_min: 1.5,
            crater_radius_max: 18.0,
            crater_size_exponent: 3.2,
            crater_depth_ratio: 0.26,
            crater_rim_ratio: 0.28,
            crater_complex_radius: 11.0,
            ejecta_extent: 2.4,
            mare_crater_factor: 0.22,
            // A ray system reaches `ray_extent` radii, so it covers a fixed
            // *absolute* area — which is a quarter of the relative footprint
            // it had on a 200x200 map.  Scale the count with the area to keep
            // the same visual density of young, bright craters.
            ray_crater_count: 4,
            ray_extent: 4.0,

            landing_zones: Vec::new(),
            // Six starts rather than four: the placement ring grows with the
            // map, so four pads on a 400x400 map sit ~180 tiles apart.
            auto_landing_zones: 6,
            landing_zone_radius: 9.0,
            landing_zone_skirt: 7.0,
            max_slope: 1.15,
            nav_max_slope: 0.62,
            build_max_slope: 0.22,
            relax_iterations: 96,
            relax_tolerance: 0.02,

            floor_height: 0.25,
            max_height: 14.0,
        }
    }
}

/// Diagnostics reported alongside the generated terrain.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LunarStats {
    pub craters: u32,
    pub min_height: f32,
    pub max_height: f32,
    /// Worst adjacent-vertex height difference remaining after relaxation.
    pub max_slope_actual: f32,
    pub relax_iterations_used: u32,
    /// Fraction of tiles that are walkable and in the main component.
    pub walkable_fraction: f32,
    /// Fraction of tiles flat enough to build on.
    pub buildable_fraction: f32,
    /// Fraction of the map classified as mare.
    pub mare_fraction: f32,
    /// Number of landing zones that needed a corridor carved to reach the
    /// main walkable component.
    pub corridors_carved: u32,
}

/// The generated terrain.
#[derive(Clone, Debug)]
pub struct LunarTerrain {
    pub size_x: i32,
    pub size_y: i32,
    /// Vertex grid, `(size_x + 1) * (size_y + 1)`.
    pub heights: Vec<f32>,
    /// Tile grid, `size_x * size_y`.  Material ids, always >= 1.
    pub tiles: Vec<u32>,
    /// Tile grid, `size_x * size_y`.  1 = walkable, 0 = blocked.
    pub nav: Vec<u32>,
    pub landing_zones: Vec<LandingZone>,
    /// One start cell per landing zone, guaranteed walkable and mutually
    /// reachable.
    pub spawn_points: Vec<(i32, i32)>,
    pub stats: LunarStats,
}

impl LunarTerrain {
    #[inline]
    pub fn height_at(&self, x: i32, y: i32) -> f32 {
        let x = x.clamp(0, self.size_x) as usize;
        let y = y.clamp(0, self.size_y) as usize;
        self.heights[y * (self.size_x as usize + 1) + x]
    }
}

/// One impact site.
#[derive(Clone, Copy, Debug)]
struct Crater {
    x: f32,
    y: f32,
    radius: f32,
    depth: f32,
    rim: f32,
    complex: bool,
    /// Per-crater decorrelation offset for the rim/ejecta noise lookups.
    phase: f64,
}

#[inline]
fn smoothstep_f32(edge0: f32, edge1: f32, x: f32) -> f32 {
    if (edge1 - edge0).abs() < f32::EPSILON {
        return if x < edge0 { 0.0 } else { 1.0 };
    }
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Stable integer hash for per-tile variant selection.  Avoids float noise so
/// variants never shimmer between regenerations of the same seed.
#[inline]
fn tile_hash(x: i32, y: i32, salt: u32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x27d4_eb2d) ^ (y as u32).wrapping_mul(0x1656_67b1);
    h ^= salt.wrapping_mul(0x9e37_79b9);
    h ^= h >> 15;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h
}

/// Generate a lunar surface.
///
/// Deterministic: the same [`LunarParams`] always yields byte-identical
/// output, on every platform, including `wasm32`.
pub fn generate_lunar(p: &LunarParams) -> LunarTerrain {
    let sx = p.size_x.max(1);
    let sy = p.size_y.max(1);
    let vw = sx as usize + 1;
    let vh = sy as usize + 1;
    let tw = sx as usize;
    let th = sy as usize;

    let n_mare = SimplexNoise::new(&format!("{}:mare", p.seed));
    let n_macro = SimplexNoise::new(&format!("{}:macro", p.seed));
    let n_reg = SimplexNoise::new(&format!("{}:regolith", p.seed));
    let n_ridge = SimplexNoise::new(&format!("{}:ridge", p.seed));
    let n_warp = SimplexNoise::new(&format!("{}:warp", p.seed));
    let n_crater = SimplexNoise::new(&format!("{}:crater", p.seed));
    let n_ray = SimplexNoise::new(&format!("{}:ray", p.seed));
    let n_tile = SimplexNoise::new(&format!("{}:tile", p.seed));
    let mut rng = Random::from_seed_str(&format!("{}:sites", p.seed));

    // -- stage 1: mare mask ------------------------------------------------
    // 0 = mare basin (flat lava plain), 1 = highland (rough, crater-saturated).
    let mare_fbm = Fbm::new(2, p.mare_frequency, 2.0, 0.5);
    let mut mare = vec![0f32; vw * vh];
    for y in 0..vh {
        for x in 0..vw {
            let n = mare_fbm.sample(&n_mare, x as f64, y as f64);
            mare[y * vw + x] =
                smoothstep(p.mare_threshold - p.mare_edge, p.mare_threshold + p.mare_edge, n)
                    as f32;
        }
    }

    // -- stage 2: macro relief ---------------------------------------------
    let macro_fbm = Fbm::new(2, p.mare_frequency * 1.9, 2.0, 0.5);
    let mut heights = vec![0f32; vw * vh];
    for y in 0..vh {
        for x in 0..vw {
            let i = y * vw + x;
            let m = mare[i];
            let macro_n = macro_fbm.sample(&n_macro, x as f64, y as f64) as f32;
            heights[i] = p.highland_amplitude * m + p.macro_amplitude * macro_n * (0.3 + 0.7 * m);
        }
    }

    // -- stage 3: landing zone placement -----------------------------------
    // Chosen from the mask and macro relief only, *before* craters exist, so
    // crater sites can be rejected against them.  Scoring by mare-ness plus
    // local height variance naturally lands the zones in basin floors.
    let zones = if p.landing_zones.is_empty() {
        place_landing_zones(p, sx, sy, &mare, &heights, vw)
    } else {
        p.landing_zones.clone()
    };

    // -- stage 4: regolith roughness ---------------------------------------
    for y in 0..vh {
        for x in 0..vw {
            let i = y * vw + x;
            let m = mare[i];
            let r = p.regolith.sample(&n_reg, x as f64, y as f64) as f32;
            let atten = p.mare_roughness + (1.0 - p.mare_roughness) * m;
            heights[i] += p.regolith_amplitude * r * atten;
        }
    }

    // -- stage 5: wrinkle ridges -------------------------------------------
    // Real wrinkle ridges are a mare feature, so weight by (1 - m).  Only the
    // upper part of the ridged field is kept, which yields thin sinuous
    // crests rather than a general lumpiness.
    if p.ridge_amplitude > 0.0 {
        let ridge_fbm = Fbm::new(3, p.ridge_frequency, 2.0, 0.5);
        for y in 0..vh {
            for x in 0..vw {
                let i = y * vw + x;
                let mare_w = 1.0 - mare[i];
                if mare_w <= 0.01 {
                    continue;
                }
                let (wx, wy) =
                    domain_warp(&n_warp, x as f64, y as f64, p.ridge_frequency * 2.0, 14.0);
                let r = ridge_fbm.sample_ridged(&n_ridge, wx, wy) as f32;
                let crest = ((r - 0.62) / 0.38).clamp(0.0, 1.0);
                heights[i] += p.ridge_amplitude * crest * crest * mare_w;
            }
        }
    }

    // -- stage 6: crater field ---------------------------------------------
    let craters = sample_craters(p, sx, sy, &zones, &mare, vw, &mut rng);
    // Tile-grid feature masks fed to the classifier in stage 9.
    let mut floor_mask = vec![0f32; tw * th];
    let mut rim_mask = vec![0f32; tw * th];
    let mut ray_boost = vec![0f32; tw * th];
    // Running record of how much rim/ejecta material each vertex already
    // carries.  Deposits combine with `max`, not `+`: ejecta is excavated
    // material redistributed, so a saturated crater field must not inflate
    // the mean elevation.  Summing instead sends the whole map into the
    // height ceiling once the blankets start overlapping.
    let mut deposit = vec![0f32; vw * vh];
    // Snapshot of the pre-impact surface.  Craters nest — a young crater
    // carves down from the floor of the old one it sits in — and without a
    // limit that compounding runs away over a saturated field, dragging the
    // global minimum down and (via the normalisation in stage 9) shoving the
    // rest of the map into the ceiling.  Bottoming out instead produces a
    // flat floor, which is what deep lunar craters actually have: they fill
    // with impact melt and slumped regolith.
    let pre_impact = heights.clone();
    let max_excavation = crater_depth(p, p.crater_radius_max) * 1.25;

    for (idx, c) in craters.iter().enumerate() {
        stamp_crater(
            c,
            p,
            sx,
            sy,
            vw,
            &mut heights,
            &mut deposit,
            &pre_impact,
            max_excavation,
            &mut floor_mask,
            &mut rim_mask,
            &n_crater,
        );
        // Only the N largest craters (they sort first) are young enough to
        // still have a ray system.
        if (idx as u32) < p.ray_crater_count {
            stamp_rays(c, p, sx, sy, &mut ray_boost, &n_ray);
        }
    }

    // -- stages 7 & 8: slope relaxation and landing zone flattening --------
    let (mut relax_used, _) = relax_slopes(
        &mut heights,
        vw,
        vh,
        p.max_slope,
        p.relax_iterations,
        p.relax_tolerance,
        None,
    );

    for z in &zones {
        flatten_zone(z, sx, sy, vw, &mut heights);
    }

    // A pad dropped next to a highland leaves its skirt over the angle of
    // repose, so relax once more — with the pad cores pinned, so the second
    // pass settles the skirt without undoing the flattening it exists to
    // protect.
    let pinned = pad_core_mask(&zones, sx, sy, vw, vh);
    let (used, _) = relax_slopes(
        &mut heights,
        vw,
        vh,
        p.max_slope,
        p.relax_iterations,
        p.relax_tolerance,
        Some(&pinned),
    );
    relax_used += used;

    // -- stage 9: normalise the range --------------------------------------
    // Shift rather than clamp: clamping the low end would produce flat
    // "lakes" wherever the terrain dipped below the floor.
    let mut min_h = f32::MAX;
    for h in heights.iter() {
        if *h < min_h {
            min_h = *h;
        }
    }
    let shift = p.floor_height - min_h;
    let mut max_h = f32::MIN;
    for h in heights.iter_mut() {
        *h = (*h + shift).min(p.max_height);
        if *h > max_h {
            max_h = *h;
        }
    }

    // -- stage 10: tile classification -------------------------------------
    let mut tiles = vec![0u32; tw * th];
    let mut slopes = vec![0f32; tw * th];
    let mut mare_tiles = 0u32;
    let mut buildable = 0u32;
    for ty in 0..th {
        for tx in 0..tw {
            let i = ty * tw + tx;
            let nw = heights[ty * vw + tx];
            let ne = heights[ty * vw + tx + 1];
            let sw = heights[(ty + 1) * vw + tx];
            let se = heights[(ty + 1) * vw + tx + 1];
            // Per-tile gradient magnitude, in height units per tile.
            let dzdx = ((ne + se) - (nw + sw)) * 0.5;
            let dzdy = ((sw + se) - (nw + ne)) * 0.5;
            slopes[i] = libm::sqrtf(dzdx * dzdx + dzdy * dzdy);
        }
    }

    // Walkability keys off the raw per-tile slope, but *materials* key off a
    // smoothed copy.  Regolith slope varies enough tile-to-tile that hard
    // thresholds on the raw value flip classes back and forth across a
    // boundary, and the large albedo gaps between classes turn that into a
    // visible checkerboard.  Averaging over the 3x3 neighbourhood also matches
    // how material actually distributes itself: by local context, not by one
    // 45px patch of ground.
    let shade_slopes = box_blur(&slopes, sx, sy);

    for ty in 0..th {
        for tx in 0..tw {
            let i = ty * tw + tx;
            let slope = slopes[i];
            if slope <= p.build_max_slope {
                buildable += 1;
            }

            let m = (mare[ty * vw + tx]
                + mare[ty * vw + tx + 1]
                + mare[(ty + 1) * vw + tx]
                + mare[(ty + 1) * vw + tx + 1])
                * 0.25;
            if m < 0.5 {
                mare_tiles += 1;
            }

            let in_zone = zones.iter().any(|z| {
                let dx = tx as f32 + 0.5 - z.x;
                let dy = ty as f32 + 0.5 - z.y;
                dx * dx + dy * dy <= z.radius * z.radius
            });

            let material = classify(
                in_zone,
                ray_boost[i],
                rim_mask[i],
                floor_mask[i],
                shade_slopes[i],
                m,
                p,
                &n_tile,
                tx as f64,
                ty as f64,
            );
            let variant = tile_hash(tx as i32, ty as i32, 0x5eed) % 64;
            tiles[i] = tile_id(material, variant);
        }
    }

    // -- stage 11: navigation ----------------------------------------------
    let mut nav: Vec<u32> =
        slopes.iter().map(|s| if *s <= p.nav_max_slope { 1 } else { 0 }).collect();

    let mut spawn_points: Vec<(i32, i32)> = zones
        .iter()
        .map(|z| (libm::roundf(z.x) as i32, libm::roundf(z.y) as i32))
        .map(|(x, y)| (x.clamp(0, sx - 1), y.clamp(0, sy - 1)))
        .collect();

    let corridors =
        connect_spawns(&mut nav, &mut heights, &mut spawn_points, sx, sy, vw, p.nav_max_slope);

    // Prune every walkable pocket that is not part of the main component:
    // unreachable ground is worse than no ground for pathfinding.
    prune_to_main_component(&mut nav, sx, sy);

    let walkable = nav.iter().filter(|v| **v == 1).count() as f32;
    let total = (tw * th) as f32;
    // Measured last: landing-zone skirts and carved corridors both edit the
    // height field after relaxation, so anything earlier would under-report.
    let max_slope_actual = measure_max_slope(&heights, vw, vh);

    let stats = LunarStats {
        craters: craters.len() as u32,
        min_height: p.floor_height,
        max_height: max_h,
        max_slope_actual,
        relax_iterations_used: relax_used,
        walkable_fraction: walkable / total,
        buildable_fraction: buildable as f32 / total,
        mare_fraction: mare_tiles as f32 / total,
        corridors_carved: corridors,
    };

    LunarTerrain {
        size_x: sx,
        size_y: sy,
        heights,
        tiles,
        nav,
        landing_zones: zones,
        spawn_points,
        stats,
    }
}

// ---------------------------------------------------------------------------
// stage helpers
// ---------------------------------------------------------------------------

/// Place `auto_landing_zones` zones on a ring around the map centre, then snap
/// each to the flattest, most mare-like candidate inside a local search
/// window.  Ring placement keeps starts symmetric (fair for an RTS); the snap
/// keeps them from looking mechanically placed.
fn place_landing_zones(
    p: &LunarParams,
    sx: i32,
    sy: i32,
    mare: &[f32],
    heights: &[f32],
    vw: usize,
) -> Vec<LandingZone> {
    let n = p.auto_landing_zones;
    if n == 0 {
        return Vec::new();
    }
    let cx = sx as f32 * 0.5;
    let cy = sy as f32 * 0.5;
    let ring = sx.min(sy) as f32 * 0.32;
    let search = sx.min(sy) as f32 * 0.10;
    let margin = p.landing_zone_radius + p.landing_zone_skirt + 2.0;

    let mut zones = Vec::with_capacity(n as usize);
    for i in 0..n {
        let a = core::f32::consts::TAU * (i as f32 / n as f32) + core::f32::consts::FRAC_PI_4;
        let ideal_x = cx + ring * libm::cosf(a);
        let ideal_y = cy + ring * libm::sinf(a);

        let mut best = (ideal_x, ideal_y);
        let mut best_score = f32::MAX;
        // Coarse 7x7 sweep of the search window — fine enough at this scale
        // and cheap enough to stay in the startup budget.
        for gy in 0..7 {
            for gx in 0..7 {
                let px = ideal_x + (gx as f32 / 6.0 - 0.5) * 2.0 * search;
                let py = ideal_y + (gy as f32 / 6.0 - 0.5) * 2.0 * search;
                let px = px.clamp(margin, sx as f32 - margin);
                let py = py.clamp(margin, sy as f32 - margin);
                let score = zone_score(px, py, p.landing_zone_radius, sx, sy, mare, heights, vw);
                if score < best_score {
                    best_score = score;
                    best = (px, py);
                }
            }
        }
        zones.push(LandingZone {
            x: best.0,
            y: best.1,
            radius: p.landing_zone_radius,
            skirt: p.landing_zone_skirt,
        });
    }
    zones
}

/// Lower is better: penalises highland-ness and local height variance.
#[allow(clippy::too_many_arguments)]
fn zone_score(
    px: f32,
    py: f32,
    radius: f32,
    sx: i32,
    sy: i32,
    mare: &[f32],
    heights: &[f32],
    vw: usize,
) -> f32 {
    let r = libm::ceilf(radius) as i32;
    let mut n = 0f32;
    let mut sum = 0f32;
    let mut sum_sq = 0f32;
    let mut mare_sum = 0f32;
    for dy in -r..=r {
        for dx in -r..=r {
            if (dx * dx + dy * dy) as f32 > radius * radius {
                continue;
            }
            let x = (px as i32 + dx).clamp(0, sx);
            let y = (py as i32 + dy).clamp(0, sy);
            let i = y as usize * vw + x as usize;
            let h = heights[i];
            sum += h;
            sum_sq += h * h;
            mare_sum += mare[i];
            n += 1.0;
        }
    }
    if n == 0.0 {
        return f32::MAX;
    }
    let mean = sum / n;
    let variance = (sum_sq / n - mean * mean).max(0.0);
    variance * 3.0 + mare_sum / n
}

/// Sample impact sites with a power-law size distribution, rejecting anything
/// that would intrude on a landing zone.  Sorted largest-first so that later
/// (smaller, younger) craters overprint earlier ones — that ordering is the
/// single biggest contributor to a field reading as genuinely lunar.
#[allow(clippy::too_many_arguments)]
fn sample_craters(
    p: &LunarParams,
    sx: i32,
    sy: i32,
    zones: &[LandingZone],
    mare: &[f32],
    vw: usize,
    rng: &mut Random,
) -> Vec<Crater> {
    // Density is per 1000 tiles; the sampling area is inflated to match the
    // 20% overscan below so that off-map centres do not thin out the interior.
    let area = (sx as f32 * 1.2) * (sy as f32 * 1.2);
    let count = libm::roundf((p.crater_density.max(0.0) * area) / 1000.0) as u32;
    let mut craters = Vec::with_capacity(count as usize);
    let r_min = p.crater_radius_min.max(0.5);
    let r_max = p.crater_radius_max.max(r_min + 0.1);
    let ratio = (r_max / r_min) as f64;

    for _ in 0..count {
        // Allow centres slightly off-map so the borders get partial craters
        // instead of a suspiciously clean edge.
        let x = (rng.next_f64() as f32) * (sx as f32 * 1.2) - sx as f32 * 0.1;
        let y = (rng.next_f64() as f32) * (sy as f32 * 1.2) - sy as f32 * 0.1;
        let u = rng.next_f64();
        let radius = r_min * libm::pow(ratio, libm::pow(u, p.crater_size_exponent as f64)) as f32;

        // Keep the *bowl* off the pad core.  Excluding the skirt as well —
        // and scaling the exclusion by the full crater radius — carves an
        // enormous dead zone around every pad on smaller maps, which is what
        // starves them of craters entirely.  The pads are flattened after the
        // crater pass anyway, so partial overlap is harmless.
        let clear = zones.iter().all(|z| {
            let dx = x - z.x;
            let dy = y - z.y;
            let keep = z.radius + radius * 0.55;
            dx * dx + dy * dy > keep * keep
        });
        if !clear {
            continue;
        }

        // Thin the crater population over the maria.  Sampling the mask here
        // rather than filtering afterwards keeps the size distribution intact.
        let mx = (libm::roundf(x) as i32).clamp(0, sx) as usize;
        let my = (libm::roundf(y) as i32).clamp(0, sy) as usize;
        let highland = mare[my * vw + mx];
        let keep_p = p.mare_crater_factor + (1.0 - p.mare_crater_factor) * highland;
        if (rng.next_f64() as f32) > keep_p {
            continue;
        }

        let depth = crater_depth(p, radius);

        craters.push(Crater {
            x,
            y,
            radius,
            depth,
            rim: depth * p.crater_rim_ratio,
            complex: radius > p.crater_complex_radius,
            phase: rng.next_f64() * 100.0,
        });
    }

    craters.sort_by(|a, b| b.radius.total_cmp(&a.radius));
    craters
}

/// Excavation depth for a crater of the given radius.
///
/// Real crater depth grows roughly linearly with size only up to the
/// simple/complex transition; past it, wall slumping means depth grows far
/// more slowly than diameter.
fn crater_depth(p: &LunarParams, radius: f32) -> f32 {
    if radius <= p.crater_complex_radius {
        p.crater_depth_ratio * radius
    } else {
        p.crater_depth_ratio
            * p.crater_complex_radius
            * libm::powf(radius / p.crater_complex_radius, 0.35)
    }
}

/// Carve one crater into the height field and record its feature masks.
#[allow(clippy::too_many_arguments)]
fn stamp_crater(
    c: &Crater,
    p: &LunarParams,
    sx: i32,
    sy: i32,
    vw: usize,
    heights: &mut [f32],
    deposit: &mut [f32],
    pre_impact: &[f32],
    max_excavation: f32,
    floor_mask: &mut [f32],
    rim_mask: &mut [f32],
    noise: &SimplexNoise,
) {
    let reach = c.radius * p.ejecta_extent + 2.0;
    let x0 = (libm::floorf(c.x - reach) as i32).max(0);
    let x1 = (libm::ceilf(c.x + reach) as i32).min(sx);
    let y0 = (libm::floorf(c.y - reach) as i32).max(0);
    let y1 = (libm::ceilf(c.y + reach) as i32).min(sy);
    if x0 > x1 || y0 > y1 {
        return;
    }

    // Reference elevation at the crater centre, taken from the pre-impact
    // surface so a crater's shape does not depend on which other craters
    // happened to be stamped before it.
    let cix = (libm::roundf(c.x) as i32).clamp(0, sx) as usize;
    let ciy = (libm::roundf(c.y) as i32).clamp(0, sy) as usize;
    let h0 = pre_impact[ciy * vw + cix];

    let tw = sx as usize;
    let ejecta_amp = c.rim * 0.55;

    for vy in y0..=y1 {
        for vx in x0..=x1 {
            let dx = vx as f32 - c.x;
            let dy = vy as f32 - c.y;
            let d = libm::sqrtf(dx * dx + dy * dy);
            if d > reach {
                continue;
            }
            let inv = if d > 1e-4 { 1.0 / d } else { 0.0 };
            let ux = dx * inv;
            let uy = dy * inv;

            // Perturb the effective radius by direction so rims are lobed
            // rather than perfect circles.
            let wobble =
                noise.noise_2d(ux as f64 * 2.6 + c.phase, uy as f64 * 2.6 - c.phase) as f32;
            let rp = (c.radius * (1.0 + 0.14 * wobble)).max(0.35);
            let t = d / rp;

            let i = vy as usize * vw + vx as usize;

            if t < 1.0 {
                // Blend the reference elevation from the crater centre out to
                // the local pre-impact surface at the rim.  Anchoring purely
                // to the centre leaves a hard step wherever the rim crosses
                // terrain of a different elevation — which happens constantly
                // for craters whose centres fall off the map edge.
                let w = t * t;
                let anchor = h0 + (pre_impact[i] - h0) * w;
                let bowl = if c.complex {
                    // Complex crater: flat floor out to 55% of the radius,
                    // then a slumped wall, plus a central peak.
                    let wall = smoothstep_f32(0.55, 1.0, t);
                    let peak = libm::powf(1.0 - (t / 0.22).min(1.0), 2.0) * c.depth * 0.45;
                    anchor - c.depth * (1.0 - wall) + peak
                } else {
                    // Simple crater: parabolic bowl.
                    anchor - c.depth * (1.0 - w)
                };
                // Bottom out against the pre-impact surface so nesting cannot
                // compound without limit.
                let bowl = bowl.max(pre_impact[i] - max_excavation);
                // `min` rather than assignment: a small young crater inside an
                // older, deeper one must not fill it back in.
                if bowl < heights[i] {
                    heights[i] = bowl;
                }
            }

            if t > 0.80 {
                let s = (t - 1.0) / 0.15;
                let rim_falloff = libm::expf(-(s * s));
                let rim_noise = 0.75
                    + 0.25
                        * noise.noise_2d(ux as f64 * 5.0 + c.phase, uy as f64 * 5.0 + c.phase)
                            as f32;
                let mut want = c.rim * rim_falloff * rim_noise;

                if t > 1.0 && t < p.ejecta_extent {
                    let f = ((p.ejecta_extent - t) / (p.ejecta_extent - 1.0)).clamp(0.0, 1.0);
                    let hummock = 0.55
                        + 0.45
                            * noise.noise_2d(vx as f64 * 0.35 + c.phase, vy as f64 * 0.35) as f32;
                    want += ejecta_amp * f * f * hummock;
                }

                // Deposit only the amount by which this crater's blanket
                // exceeds what is already here, so overlapping blankets
                // combine as `max` rather than accumulating without bound.
                // Applied incrementally (not as a post-pass) so that a later,
                // younger crater still carves cleanly through the deposit.
                if want > deposit[i] {
                    heights[i] += want - deposit[i];
                    deposit[i] = want;
                }
            }

            // Feature masks live on the tile grid, and carry a strength that
            // scales with crater size.  A saturated surface is covered in
            // small old craters; if every one of them painted a bright rim the
            // map would read as uniform noise.  Only large (and, by the
            // largest-first stamping order, relatively recent) impacts get a
            // visually distinct rim and floor.
            if vx < sx && vy < sy {
                let ti = vy as usize * tw + vx as usize;
                let strength = (c.radius / p.crater_radius_max.max(0.1)).min(1.0);
                if t < 0.85 && strength > floor_mask[ti] {
                    floor_mask[ti] = strength;
                }
                if (0.85..1.25).contains(&t) && strength > rim_mask[ti] {
                    rim_mask[ti] = strength;
                }
            }
        }
    }
}

/// Record the bright ejecta ray streaks of a young crater.  Rays are albedo
/// only — they never touch the height field, so they cost nothing in
/// pathfinding or buildability while doing most of the visual work.
fn stamp_rays(
    c: &Crater,
    p: &LunarParams,
    sx: i32,
    sy: i32,
    ray_boost: &mut [f32],
    noise: &SimplexNoise,
) {
    let reach = c.radius * p.ray_extent;
    let x0 = (libm::floorf(c.x - reach) as i32).max(0);
    let x1 = (libm::ceilf(c.x + reach) as i32).min(sx - 1);
    let y0 = (libm::floorf(c.y - reach) as i32).max(0);
    let y1 = (libm::ceilf(c.y + reach) as i32).min(sy - 1);
    let tw = sx as usize;

    for ty in y0..=y1 {
        for tx in x0..=x1 {
            let dx = tx as f32 + 0.5 - c.x;
            let dy = ty as f32 + 0.5 - c.y;
            let d = libm::sqrtf(dx * dx + dy * dy);
            if d < c.radius || d > reach {
                continue;
            }
            let t = d / c.radius;
            let inv = 1.0 / d;
            // Purely angular noise: constant along a radius, so the pattern
            // reads as straight streaks radiating from the impact.
            let a = noise.noise_2d(
                dx as f64 * inv as f64 * 3.2 + c.phase,
                dy as f64 * inv as f64 * 3.2 - c.phase,
            ) as f32;
            if a <= 0.32 {
                continue;
            }
            let angular = (a - 0.32) / 0.68;
            let radial = ((p.ray_extent - t) / (p.ray_extent - 1.0)).clamp(0.0, 1.0);
            let v = angular * radial * radial;
            let i = ty as usize * tw + tx as usize;
            if v > ray_boost[i] {
                ray_boost[i] = v;
            }
        }
    }
}

/// Thermal-erosion / talus relaxation: repeatedly move material from any
/// vertex that overhangs a 4-neighbour by more than `max_slope`.
///
/// This is the same mass-wasting process that caps real regolith slopes at the
/// angle of repose, and it doubles as the engine's safety net — `build_mesh`
/// emits no wall geometry for interior height discontinuities, so unbounded
/// slopes would render as stretched, badly-lit top faces.
///
/// Vertices flagged in `pinned` are held fixed: they still pull on their
/// neighbours but never move themselves.
///
/// Returns `(iterations_used, worst_remaining_over_slope_edge)`.
#[allow(clippy::too_many_arguments)]
fn relax_slopes(
    heights: &mut [f32],
    vw: usize,
    vh: usize,
    max_slope: f32,
    max_iterations: u32,
    tolerance: f32,
    pinned: Option<&[bool]>,
) -> (u32, f32) {
    if max_slope <= 0.0 || max_iterations == 0 {
        return (0, measure_max_slope(heights, vw, vh));
    }
    // 4 neighbours at 0.18 each moves at most 0.72 of the excess per pass,
    // which converges quickly without oscillating.
    const RATE: f32 = 0.18;
    let mut scratch = heights.to_vec();
    let mut used = 0;

    for _ in 0..max_iterations {
        scratch.copy_from_slice(heights);
        let mut worst = 0f32;
        for y in 0..vh {
            for x in 0..vw {
                let i = y * vw + x;
                if pinned.is_some_and(|m| m[i]) {
                    continue;
                }
                let hi = scratch[i];
                let mut delta = 0f32;
                let mut neigh = [usize::MAX; 4];
                if x > 0 {
                    neigh[0] = i - 1;
                }
                if x + 1 < vw {
                    neigh[1] = i + 1;
                }
                if y > 0 {
                    neigh[2] = i - vw;
                }
                if y + 1 < vh {
                    neigh[3] = i + vw;
                }
                for j in neigh {
                    if j == usize::MAX {
                        continue;
                    }
                    let d = hi - scratch[j];
                    if d > max_slope {
                        let excess = d - max_slope;
                        if excess > worst {
                            worst = excess;
                        }
                        delta -= RATE * excess;
                    } else if d < -max_slope {
                        let excess = -d - max_slope;
                        if excess > worst {
                            worst = excess;
                        }
                        delta += RATE * excess;
                    }
                }
                heights[i] = hi + delta;
            }
        }
        used += 1;
        if worst <= tolerance {
            break;
        }
    }

    (used, measure_max_slope(heights, vw, vh))
}

fn measure_max_slope(heights: &[f32], vw: usize, vh: usize) -> f32 {
    let mut worst = 0f32;
    for y in 0..vh {
        for x in 0..vw {
            let i = y * vw + x;
            if x + 1 < vw {
                let d = (heights[i] - heights[i + 1]).abs();
                if d > worst {
                    worst = d;
                }
            }
            if y + 1 < vh {
                let d = (heights[i] - heights[i + vw]).abs();
                if d > worst {
                    worst = d;
                }
            }
        }
    }
    worst
}

/// Mark every vertex inside a landing pad core, for pinning during the
/// post-flatten relaxation pass.
fn pad_core_mask(zones: &[LandingZone], sx: i32, sy: i32, vw: usize, vh: usize) -> Vec<bool> {
    let mut mask = vec![false; vw * vh];
    for z in zones {
        let x0 = (libm::floorf(z.x - z.radius) as i32).max(0);
        let x1 = (libm::ceilf(z.x + z.radius) as i32).min(sx);
        let y0 = (libm::floorf(z.y - z.radius) as i32).max(0);
        let y1 = (libm::ceilf(z.y + z.radius) as i32).min(sy);
        for vy in y0..=y1 {
            for vx in x0..=x1 {
                let dx = vx as f32 - z.x;
                let dy = vy as f32 - z.y;
                if dx * dx + dy * dy <= z.radius * z.radius {
                    mask[vy as usize * vw + vx as usize] = true;
                }
            }
        }
    }
    mask
}

/// Blend a landing zone flat.  Inside `radius` the height becomes exactly the
/// disc mean; across `skirt` it eases back to the surrounding terrain.
fn flatten_zone(z: &LandingZone, sx: i32, sy: i32, vw: usize, heights: &mut [f32]) {
    let outer = z.radius + z.skirt;
    let x0 = (libm::floorf(z.x - outer) as i32).max(0);
    let x1 = (libm::ceilf(z.x + outer) as i32).min(sx);
    let y0 = (libm::floorf(z.y - outer) as i32).max(0);
    let y1 = (libm::ceilf(z.y + outer) as i32).min(sy);

    let mut sum = 0f32;
    let mut n = 0f32;
    for vy in y0..=y1 {
        for vx in x0..=x1 {
            let dx = vx as f32 - z.x;
            let dy = vy as f32 - z.y;
            if dx * dx + dy * dy <= z.radius * z.radius {
                sum += heights[vy as usize * vw + vx as usize];
                n += 1.0;
            }
        }
    }
    if n == 0.0 {
        return;
    }
    let target = sum / n;

    for vy in y0..=y1 {
        for vx in x0..=x1 {
            let dx = vx as f32 - z.x;
            let dy = vy as f32 - z.y;
            let d = libm::sqrtf(dx * dx + dy * dy);
            if d > outer {
                continue;
            }
            let w = 1.0 - smoothstep_f32(z.radius, outer, d);
            let i = vy as usize * vw + vx as usize;
            heights[i] += (target - heights[i]) * w;
        }
    }
}

/// 3x3 box blur over a tile grid, clamping at the edges.
fn box_blur(src: &[f32], sx: i32, sy: i32) -> Vec<f32> {
    let tw = sx as usize;
    let mut out = vec![0f32; src.len()];
    for ty in 0..sy {
        for tx in 0..sx {
            let mut sum = 0f32;
            let mut n = 0f32;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let nx = (tx + dx).clamp(0, sx - 1);
                    let ny = (ty + dy).clamp(0, sy - 1);
                    sum += src[ny as usize * tw + nx as usize];
                    n += 1.0;
                }
            }
            out[ty as usize * tw + tx as usize] = sum / n;
        }
    }
    out
}

/// Decide the material class for one tile.  Order matters: earlier rules win.
#[allow(clippy::too_many_arguments)]
fn classify(
    in_zone: bool,
    ray: f32,
    rim: f32,
    floor: f32,
    slope: f32,
    mare: f32,
    p: &LunarParams,
    noise: &SimplexNoise,
    tx: f64,
    ty: f64,
) -> LunarMaterial {
    if in_zone {
        return LunarMaterial::LandingPad;
    }
    // Steep ground reads as rock regardless of how it got there — checking
    // slope before provenance keeps impassable terrain visually legible,
    // which matters more for an RTS than geological accuracy.
    if slope > p.nav_max_slope {
        return LunarMaterial::Rocky;
    }
    if rim > 0.55 && slope > p.build_max_slope {
        return LunarMaterial::RimBright;
    }
    if ray > 0.45 {
        return LunarMaterial::Ray;
    }
    if floor > 0.35 && slope <= p.build_max_slope {
        return LunarMaterial::CraterFloor;
    }
    if slope > p.build_max_slope * 1.6 {
        return LunarMaterial::RegolithCoarse;
    }
    // Dither the mare/highland contact.  The mask is a smooth field, so a
    // fixed 0.5 cut traces a clean contour, and with a ~2:1 albedo gap either
    // side that contour reads as a hard stair-stepped shoreline.  Jittering
    // the cut per tile interleaves the two materials across a few tiles
    // instead — which is also what a real gradational contact looks like.
    let contact_jitter = noise.noise_2d(tx / 2.3, ty / 2.3) as f32 * 0.18;
    if mare < 0.5 + contact_jitter {
        // Mottle the maria so the flat regions are not one flat colour.
        let n = noise.noise_2d(tx / 17.0, ty / 17.0);
        if n < -0.15 {
            return LunarMaterial::MareDark;
        }
        return LunarMaterial::MareSmooth;
    }
    LunarMaterial::Regolith
}

// ---------------------------------------------------------------------------
// connectivity
// ---------------------------------------------------------------------------

/// Label 8-connected walkable components.  Returns `(labels, largest_label)`
/// where blocked cells are labelled `0`.
fn label_components(nav: &[u32], sx: i32, sy: i32) -> (Vec<u32>, u32) {
    let tw = sx as usize;
    let th = sy as usize;
    let mut labels = vec![0u32; tw * th];
    let mut next = 0u32;
    let mut best = 0u32;
    let mut best_size = 0usize;
    let mut stack: Vec<usize> = Vec::new();

    for start in 0..tw * th {
        if nav[start] != 1 || labels[start] != 0 {
            continue;
        }
        next += 1;
        let mut size = 0usize;
        stack.push(start);
        labels[start] = next;
        while let Some(i) = stack.pop() {
            size += 1;
            let x = (i % tw) as i32;
            let y = (i / tw) as i32;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    let nx = x + dx;
                    let ny = y + dy;
                    if nx < 0 || ny < 0 || nx >= sx || ny >= sy {
                        continue;
                    }
                    let j = ny as usize * tw + nx as usize;
                    if nav[j] == 1 && labels[j] == 0 {
                        labels[j] = next;
                        stack.push(j);
                    }
                }
            }
        }
        if size > best_size {
            best_size = size;
            best = next;
        }
    }

    (labels, best)
}

/// Guarantee that every spawn point is walkable and mutually reachable.
///
/// Isolated starts are joined to the main component by carving a corridor:
/// a breach is cut through the blocking rim and the heights along it are
/// relaxed until the corridor is genuinely traversable, not merely flagged as
/// such.  Degraded lunar craters really do have breached rims, so the result
/// reads as a natural saddle rather than a bulldozed trench.
///
/// Returns the number of corridors carved.
#[allow(clippy::too_many_arguments)]
fn connect_spawns(
    nav: &mut [u32],
    heights: &mut [f32],
    spawns: &mut [(i32, i32)],
    sx: i32,
    sy: i32,
    vw: usize,
    nav_max_slope: f32,
) -> u32 {
    let tw = sx as usize;
    let th = sy as usize;
    if tw * th == 0 || spawns.is_empty() {
        return 0;
    }

    // Every spawn should start on walkable ground; landing zones are flat by
    // construction, but snap anyway in case of explicit zone overrides.
    let (mut labels, mut main) = label_components(nav, sx, sy);
    for s in spawns.iter_mut() {
        let i = s.1 as usize * tw + s.0 as usize;
        if nav[i] != 1 {
            if let Some((nx, ny)) = nearest_walkable(nav, sx, sy, *s) {
                *s = (nx, ny);
            }
        }
    }

    let mut carved = 0u32;
    let targets: Vec<(i32, i32)> = spawns.to_vec();
    for (sxp, syp) in targets {
        let i = syp as usize * tw + sxp as usize;
        if main != 0 && labels[i] == main {
            continue;
        }
        // BFS across *all* cells (ignoring walkability) to the nearest cell of
        // the main component, then open that route up.
        let Some(path) = route_to_component(&labels, main, sx, sy, (sxp, syp)) else {
            continue;
        };
        for (cx, cy) in &path {
            carve_corridor_cell(nav, heights, sx, sy, vw, *cx, *cy, nav_max_slope);
        }
        carved += 1;
        let relabeled = label_components(nav, sx, sy);
        labels = relabeled.0;
        main = relabeled.1;
    }

    carved
}

fn nearest_walkable(nav: &[u32], sx: i32, sy: i32, from: (i32, i32)) -> Option<(i32, i32)> {
    let tw = sx as usize;
    let th = sy as usize;
    let mut seen = vec![false; tw * th];
    let mut queue = VecDeque::new();
    let start = from.1 as usize * tw + from.0 as usize;
    seen[start] = true;
    queue.push_back(from);
    while let Some((x, y)) = queue.pop_front() {
        let i = y as usize * tw + x as usize;
        if nav[i] == 1 {
            return Some((x, y));
        }
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let nx = x + dx;
            let ny = y + dy;
            if nx < 0 || ny < 0 || nx >= sx || ny >= sy {
                continue;
            }
            let j = ny as usize * tw + nx as usize;
            if !seen[j] {
                seen[j] = true;
                queue.push_back((nx, ny));
            }
        }
    }
    None
}

/// Breadth-first route from `from` to the nearest cell belonging to `target`,
/// ignoring walkability.  Returns the cells to open up, `from` first.
fn route_to_component(
    labels: &[u32],
    target: u32,
    sx: i32,
    sy: i32,
    from: (i32, i32),
) -> Option<Vec<(i32, i32)>> {
    if target == 0 {
        return None;
    }
    let tw = sx as usize;
    let th = sy as usize;
    let mut prev = vec![usize::MAX; tw * th];
    let mut seen = vec![false; tw * th];
    let start = from.1 as usize * tw + from.0 as usize;
    seen[start] = true;
    let mut queue = alloc::collections::VecDeque::new();
    queue.push_back(start);

    let mut goal = usize::MAX;
    while let Some(i) = queue.pop_front() {
        if labels[i] == target {
            goal = i;
            break;
        }
        let x = (i % tw) as i32;
        let y = (i / tw) as i32;
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let nx = x + dx;
            let ny = y + dy;
            if nx < 0 || ny < 0 || nx >= sx || ny >= sy {
                continue;
            }
            let j = ny as usize * tw + nx as usize;
            if !seen[j] {
                seen[j] = true;
                prev[j] = i;
                queue.push_back(j);
            }
        }
    }
    if goal == usize::MAX {
        return None;
    }

    let mut path = Vec::new();
    let mut cur = goal;
    while cur != usize::MAX {
        path.push(((cur % tw) as i32, (cur / tw) as i32));
        if cur == start {
            break;
        }
        cur = prev[cur];
    }
    path.reverse();
    Some(path)
}

/// Open one corridor cell (and its immediate neighbours) by flattening the
/// local height field until the slope drops under the walkability threshold.
#[allow(clippy::too_many_arguments)]
fn carve_corridor_cell(
    nav: &mut [u32],
    heights: &mut [f32],
    sx: i32,
    sy: i32,
    vw: usize,
    cx: i32,
    cy: i32,
    nav_max_slope: f32,
) {
    let tw = sx as usize;
    // Widen by one tile so the corridor is not a single-cell thread.
    for dy in -1..=1 {
        for dx in -1..=1 {
            let tx = cx + dx;
            let ty = cy + dy;
            if tx < 0 || ty < 0 || tx >= sx || ty >= sy {
                continue;
            }
            // Average the four corners towards their mean until the tile is
            // shallow enough to walk.
            for _ in 0..6 {
                let i00 = ty as usize * vw + tx as usize;
                let i10 = i00 + 1;
                let i01 = i00 + vw;
                let i11 = i01 + 1;
                let mean = (heights[i00] + heights[i10] + heights[i01] + heights[i11]) * 0.25;
                let dzdx = ((heights[i10] + heights[i11]) - (heights[i00] + heights[i01])) * 0.5;
                let dzdy = ((heights[i01] + heights[i11]) - (heights[i00] + heights[i10])) * 0.5;
                if libm::sqrtf(dzdx * dzdx + dzdy * dzdy) <= nav_max_slope * 0.7 {
                    break;
                }
                for j in [i00, i10, i01, i11] {
                    heights[j] += (mean - heights[j]) * 0.5;
                }
            }
            nav[ty as usize * tw + tx as usize] = 1;
        }
    }
}

/// Blank every walkable cell outside the largest connected component.
fn prune_to_main_component(nav: &mut [u32], sx: i32, sy: i32) {
    let (labels, main) = label_components(nav, sx, sy);
    if main == 0 {
        return;
    }
    for (i, v) in nav.iter_mut().enumerate() {
        if *v == 1 && labels[i] != main {
            *v = 0;
        }
    }
}
