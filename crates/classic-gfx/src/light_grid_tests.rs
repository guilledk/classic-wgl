use super::*;
use classic_core::components::{Light, LightKind};
use classic_core::math::iso_camera_matrix;

const PPM: f32 = 64.0;

/// The engine's frame transform for a `w`x`h` main target at `zoom`, panned
/// so world point `focus` sits at the screen centre.
fn view_for(w: u32, h: u32, zoom: f32, focus: Vec3) -> (Mat4, LightGridView) {
    let projection = Mat4::orthographic_rh(0.0, w as f32, h as f32, 0.0, -10000.0, 10000.0);
    let world_matrix = iso_camera_matrix();
    let to_px = clip_from_world(&Mat4::IDENTITY, &Mat4::IDENTITY, &world_matrix, PPM);
    let f = to_px.transform_point3(focus);
    let camera = Mat4::from_translation(Vec3::new(w as f32 * 0.5, h as f32 * 0.5, 0.0))
        * Mat4::from_scale(Vec3::new(zoom, zoom, 1.0))
        * Mat4::from_translation(Vec3::new(-f.x, -f.y, 0.0));
    let clip = clip_from_world(&projection, &camera, &world_matrix, PPM);
    let view = LightGridView::new(&clip, w, h).expect("orthographic");
    (clip, view)
}

fn point(pos: Vec3, radius: f32) -> Light {
    Light { kind: LightKind::Point, position: pos, radius, intensity: 1.0, ..Default::default() }
}

/// Window pixel of a world point, through the full clip matrix (independent
/// of `LightGridView`'s folded rows).
fn window_px(clip: &Mat4, p: Vec3, w: u32, h: u32) -> [f32; 2] {
    let c = *clip * p.extend(1.0);
    [(c.x / c.w * 0.5 + 0.5) * w as f32, (c.y / c.w * 0.5 + 0.5) * h as f32]
}

/// Deterministic generator (SplitMix64) for the property tests.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) >> 40) as f32 / (1u64 << 24) as f32
    }
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next()
    }
    fn unit(&mut self) -> Vec3 {
        loop {
            let v = Vec3::new(self.range(-1.0, 1.0), self.range(-1.0, 1.0), self.range(-1.0, 1.0));
            if v.length_squared() > 1e-4 && v.length_squared() <= 1.0 {
                return v.normalize();
            }
        }
    }
}

/// Every world point inside a light's sphere rasterises in a tile whose list
/// holds that light (or that runs the full loop) -- across zooms, pans,
/// lights straddling tile edges and lights partly or wholly off screen.
#[test]
fn binning_is_conservative() {
    let mut rng = Rng(7);
    for &(w, h) in &[(1280u32, 720u32), (1920, 1080), (3840, 2160), (1001, 333)] {
        for &zoom in &[0.15f32, 0.4, 1.0, 2.0, 3.0] {
            let focus = Vec3::new(rng.range(-50.0, 50.0), rng.range(-50.0, 50.0), 0.0);
            let (clip, view) = view_for(w, h, zoom, focus);
            // Scatter lights well beyond the visible area.
            let span = (w.max(h) as f32 / (PPM * zoom)) * 1.2;
            let lights: Vec<Light> = (0..120)
                .map(|_| {
                    let off = Vec3::new(rng.range(-span, span), rng.range(-span, span), 0.0);
                    point(focus + off + Vec3::Z * rng.range(0.0, 8.0), rng.range(0.2, 14.0))
                })
                .collect();
            let grid = LightGrid::build(&lights, &view, usize::MAX);
            let mut checked = 0;
            for (i, l) in lights.iter().enumerate() {
                for k in 0..200 {
                    // Bias half the samples onto the sphere's surface.
                    let r = if k % 2 == 0 { 0.9999 } else { rng.next().cbrt() * 0.9999 };
                    let p = l.position + rng.unit() * l.radius * r;
                    let [x, y] = window_px(&clip, p, w, h);
                    if !(x >= 0.0 && y >= 0.0 && x < w as f32 && y < h as f32) {
                        continue;
                    }
                    // The shader's lookup: pixel centre -> tile.
                    let (tx, ty) =
                        ((x / LIGHT_TILE_PX as f32) as u32, (y / LIGHT_TILE_PX as f32) as u32);
                    if let Some(list) = grid.tile_lights(tx, ty) {
                        assert!(
                            list.contains(&i),
                            "light {i} missing from tile ({tx},{ty}) @ zoom {zoom}"
                        );
                    }
                    checked += 1;
                }
            }
            assert!(checked > 1000, "too few on-screen samples ({checked})");
        }
    }
}

/// Lists keep UBO order (so the shader adds the same terms in the same order
/// as the full loop) and never repeat a light.
#[test]
fn lists_are_ascending_and_unique() {
    let (_, view) = view_for(1280, 720, 1.0, Vec3::ZERO);
    let lights: Vec<Light> =
        (0..40).map(|i| point(Vec3::new(i as f32 * 0.3 - 6.0, 0.0, 1.0), 3.0)).collect();
    let grid = LightGrid::build(&lights, &view, usize::MAX);
    let (tx, ty) = view.tiles();
    let mut seen = 0;
    for y in 0..ty {
        for x in 0..tx {
            let list = grid.tile_lights(x, y).expect("under the cap");
            assert!(list.windows(2).all(|p| p[0] < p[1]), "tile ({x},{y}): {list:?}");
            seen += list.len();
        }
    }
    assert!(seen > 0);
}

/// A light centred on a tile corner lands in all four tiles around it.
#[test]
fn light_on_a_tile_corner_lands_in_every_neighbour() {
    let (w, h) = (1280, 720);
    let (_, view) = view_for(w, h, 1.0, Vec3::ZERO);
    // The focus is at the screen centre (640, 360): a tile corner for 32 px
    // tiles horizontally (640 = 20 * 32), mid-tile vertically (360 = 11.25 * 32).
    let tiny = point(Vec3::ZERO, 0.01);
    let r = view.tile_rect(&tiny).unwrap();
    assert_eq!([r[0], r[2]], [19, 20], "straddles the vertical tile edge");
    assert_eq!([r[1], r[3]], [11, 11]);
    let grid = LightGrid::build(&[tiny], &view, usize::MAX);
    for x in [19, 20] {
        assert_eq!(grid.tile_lights(x, 11), Some(vec![0]));
    }
    assert_eq!(grid.tile_lights(21, 11), Some(vec![]));
    assert_eq!(grid.tile_lights(19, 12), Some(vec![]));
}

/// Off-screen lights cost nothing; a light straddling the screen edge is
/// clipped to the grid; an unbounded light reaches every tile.
#[test]
fn off_screen_partial_and_unbounded_lights() {
    let (w, h) = (1280, 720);
    let (_, view) = view_for(w, h, 1.0, Vec3::ZERO);
    let (tx, ty) = view.tiles();
    assert_eq!((tx, ty), (40, 23));
    // Far away: no tile.
    assert_eq!(view.tile_rect(&point(Vec3::new(500.0, 500.0, 0.0), 5.0)), None);
    // Find a world point on the left screen edge and hang a light there.
    let edge = (0..4000)
        .map(|i| Vec3::new(-(i as f32) * 0.01, i as f32 * 0.01, 0.0))
        .find(|p| view.pixel(*p)[0] <= 0.0)
        .unwrap();
    let r = view.tile_rect(&point(edge, 1.0)).unwrap();
    assert_eq!(r[0], 0, "clipped at the left edge");
    assert!(r[2] < tx / 2);
    // Unbounded (radius <= 0) and non-finite radii reach everywhere.
    for radius in [0.0, -1.0, f32::INFINITY, f32::NAN] {
        assert_eq!(view.tile_rect(&point(edge, radius)), Some([0, 0, tx - 1, ty - 1]));
    }
    let grid = LightGrid::build(&[point(edge, 0.0)], &view, usize::MAX);
    assert_eq!(grid.tile_lights(tx - 1, ty - 1), Some(vec![0]));
}

/// Zooming in grows a light's footprint by the zoom factor.
#[test]
fn zoom_scales_the_footprint() {
    let light = point(Vec3::new(0.0, 0.0, 1.0), 2.0);
    let width = |zoom| {
        let (_, view) = view_for(3840, 2160, zoom, Vec3::ZERO);
        let r = view.tile_rect(&light).unwrap();
        r[2] - r[0] + 1
    };
    // radius 2 m * 64 px/m * zoom, both sides, over 32 px tiles.
    assert!(width(0.15) <= 2);
    assert!((7..=10).contains(&width(1.0)), "{}", width(1.0));
    assert!((23..=26).contains(&width(3.0)), "{}", width(3.0));
}

/// A tile with more than `LIGHT_TILE_CAP` lights, or whose list does not fit
/// the texture, falls back to the full loop; it never drops a light.
#[test]
fn overflow_falls_back_to_the_full_loop() {
    let (_, view) = view_for(1280, 720, 1.0, Vec3::ZERO);
    let mut lights: Vec<Light> = (0..LIGHT_TILE_CAP).map(|_| point(Vec3::ZERO, 0.2)).collect();
    let grid = LightGrid::build(&lights, &view, usize::MAX);
    assert_eq!(grid.tile_lights(19, 11).map(|l| l.len()), Some(LIGHT_TILE_CAP), "at the cap");
    lights.push(point(Vec3::ZERO, 0.2));
    let grid = LightGrid::build(&lights, &view, usize::MAX);
    assert_eq!(grid.tile_lights(19, 11), None, "over the cap");
    assert_eq!(grid.tile_lights(0, 0), Some(vec![]), "other tiles keep their lists");

    // Storage: room for the headers plus one word of lists.
    let (tx, ty) = view.tiles();
    let headers = (tx * ty) as usize;
    let lights = [point(Vec3::ZERO, 0.2), point(Vec3::new(30.0, -30.0, 0.0), 0.2)];
    let full = LightGrid::build(&lights, &view, usize::MAX);
    let lit: Vec<(u32, u32)> = (0..ty)
        .flat_map(|y| (0..tx).map(move |x| (x, y)))
        .filter(|&(x, y)| full.tile_lights(x, y).is_some_and(|l| !l.is_empty()))
        .collect();
    assert!(lit.len() >= 2);
    let tight = LightGrid::build(&lights, &view, headers + 1);
    assert!(tight.words.len() <= headers + 1);
    let (kept, spilled): (Vec<_>, Vec<_>) =
        lit.iter().partition(|&&(x, y)| tight.tile_lights(x, y).is_some());
    assert_eq!(kept.len(), 1);
    for &(x, y) in &kept {
        assert_eq!(tight.tile_lights(x, y), full.tile_lights(x, y));
    }
    assert!(!spilled.is_empty());
}

/// Only the lights `pack_lights` uploads are binned (indices stay bytes).
#[test]
fn only_uploaded_lights_are_binned() {
    let (_, view) = view_for(1280, 720, 1.0, Vec3::ZERO);
    let mut lights = vec![point(Vec3::new(500.0, 500.0, 0.0), 1.0); MAX_LIGHTS];
    lights.push(point(Vec3::ZERO, 1.0));
    let grid = LightGrid::build(&lights, &view, usize::MAX);
    assert_eq!(grid.tile_lights(19, 11), Some(vec![]));
}

/// A perspective camera cannot use the affine bound: the grid stays off.
#[test]
fn non_orthographic_view_disables_the_grid() {
    let p = Mat4::perspective_rh_gl(1.0, 1.6, 0.1, 100.0);
    assert!(LightGridView::new(&p, 1280, 720).is_none());
    assert!(LightGridView::new(&Mat4::IDENTITY, 0, 720).is_none());
}

/// The model pass renders at `viewport x min(1, 1/zoom)`; its fragments map
/// back to the main-target tile they cover.
#[test]
fn target_map_scales_fragments_back_to_main_pixels() {
    let map = grid_map((1280, 720), (427, 240), (40, 23));
    // Last model-target pixel centre maps inside the last main tile.
    let x = 426.5 * map[0];
    let y = 239.5 * map[1];
    assert_eq!((x as u32, y as u32), (39, 22));
    assert_eq!(map[2..], [40.0, 23.0]);
    let same = grid_map((1280, 720), (1280, 720), (40, 23));
    assert_eq!(same[..2], [1.0 / 32.0, 1.0 / 32.0]);
}

/// `clip_from_world` must keep mirroring the vertex shaders' raster formula.
#[test]
fn clip_from_world_mirrors_the_vertex_shaders() {
    for src in [
        crate::shaders::ISO_TILEMAP_VERT,
        crate::shaders::MESH_VERT,
        crate::shaders::DIRECT_TEX_VERT,
    ] {
        assert!(src.contains("vec4(view.x * ppm, -view.y * ppm, 0.0, 1.0)"));
        assert!(src.contains("projection_matrix * camera_matrix * screenPos"));
        assert!(src.contains("vLightPos = world;"));
    }
}

fn grid_block(src: &str) -> &str {
    let begin = src.find("// --- BEGIN LIGHT GRID").expect("BEGIN LIGHT GRID marker");
    let end = src.find("// --- END LIGHT GRID ---").expect("END LIGHT GRID marker");
    &src[begin..end]
}

/// The lit fragment shaders read the grid with the same code, and use it.
#[test]
fn lit_fragment_shaders_share_the_light_grid() {
    use crate::shaders::{ISO_TILEMAP_FRAG, MESH_FRAG, SHEET_FRAG};
    let sheet = grid_block(SHEET_FRAG);
    assert_eq!(sheet, grid_block(ISO_TILEMAP_FRAG), "iso_tilemap.frag drifted");
    assert_eq!(sheet, grid_block(MESH_FRAG), "mesh.frag drifted");
    for src in [SHEET_FRAG, ISO_TILEMAP_FRAG, MESH_FRAG] {
        assert!(src.contains("evaluateLightsTiled(n, vLightPos)"));
        assert!(!src.contains("evaluateLights(n, vLightPos)"));
    }
}

/// The shader block's constants are the layout declared in this module.
#[test]
fn shader_constants_match_the_layout() {
    let block = grid_block(crate::shaders::SHEET_FRAG);
    for line in [
        format!("#define LIGHT_GRID_ROW {LIGHT_GRID_ROW}\n"),
        format!("#define LIGHT_GRID_ROW_SHIFT {LIGHT_GRID_ROW_SHIFT}\n"),
        format!("#define LIGHT_GRID_COUNT_BITS {LIGHT_GRID_COUNT_BITS}u\n"),
        format!("#define LIGHT_GRID_OVERFLOW {LIGHT_GRID_OVERFLOW}u\n"),
    ] {
        assert!(block.contains(&line), "missing {line:?}");
    }
    assert!(crate::shaders::SHEET_FRAG.contains(&format!("#define MAX_LIGHTS {MAX_LIGHTS}\n")));
}
