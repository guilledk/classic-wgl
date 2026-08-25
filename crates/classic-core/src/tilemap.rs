//! # Skill: `classic-iso`
//!
//! **Read `.claude/skills/classic-iso/SKILL.md` before working on this module.**
//!
//! Isometric tilemap mesh generation.

/// One vertex in the interleaved tilemap buffer.
/// Layout: vertexPos(3), mapCoord(2), tileId(1), normal(3) = 9 floats, 36 bytes.
const VERT_FLOATS: usize = 9;

#[derive(Clone, Copy, Debug, Default)]
pub struct TileVertex {
    pub pos: [f32; 3],
    pub map_coord: [f32; 2],
    pub tile_id: f32,
    pub normal: [f32; 3],
}

/// Build the full tilemap mesh.
///
/// Returns the interleaved vertex data as a flat `[f32]` and the vertex count.
/// Each vertex is 9 floats.  Drawn as non-indexed `TRIANGLES`.
pub fn build_mesh(
    size_x: i32,
    size_y: i32,
    tiles: &[u32],
    heights: &[f32],
    height_scale: f32,
) -> (Vec<f32>, usize) {
    assert_eq!(heights.len(), ((size_x + 1) * (size_y + 1)) as usize); // +1 for edge samples
    assert!(tiles.len() >= (size_x * size_y) as usize);

    // Precompute normalized map coords for every integer tile coordinate.
    let mx: Vec<f32> = (0..=size_x).map(|i| i as f32 / size_x as f32).collect();
    let my: Vec<f32> = (0..=size_y).map(|i| i as f32 / size_y as f32).collect();

    // Exact upper bound: 6 vertices for every tile's two top-face triangles,
    // plus 6 per wall.  Walls are only ever emitted on the map perimeter —
    // one per exterior edge, so `2 * (size_x + size_y)` of them.
    //
    // Budgeting 30 vertices for *every* tile (as if all four walls could
    // appear anywhere) over-reserves by 5x: 173 MB rather than 35 MB on a
    // 400x400 map, all of it touched by the allocator and immediately wasted.
    let top_verts = (size_x as usize) * (size_y as usize) * 6;
    let wall_verts = 2 * (size_x as usize + size_y as usize) * 6;
    let mut data: Vec<f32> = Vec::with_capacity((top_verts + wall_verts) * VERT_FLOATS);

    let at = |tx: i32, ty: i32| -> f32 {
        let tx = tx.clamp(0, size_x) as usize;
        let ty = ty.clamp(0, size_y) as usize;
        heights[ty * (size_x as usize + 1) + tx]
    };

    let tile_at = |tx: i32, ty: i32| -> u32 {
        if tx < 0 || ty < 0 || tx >= size_x || ty >= size_y {
            0
        } else {
            tiles[(ty * size_x + tx) as usize]
        }
    };

    // Smooth per-vertex normals, accumulated from the two triangles of every
    // adjacent tile.  Flat (per-face) normals make the triangulation of a
    // continuous height field plainly visible as a herringbone of facets.
    //
    // On a level map every face normal is already +Z, so the averaged result
    // is bit-identical to the per-face value and existing flat scenes are
    // unaffected.
    let vnormals = build_vertex_normals(size_x, size_y, height_scale, &at);
    let vn = |tx: i32, ty: i32| -> [f32; 3] {
        vnormals
            [ty.clamp(0, size_y) as usize * (size_x as usize + 1) + tx.clamp(0, size_x) as usize]
    };

    for ty in 0..size_y {
        for tx in 0..size_x {
            let tid = tile_at(tx, ty);
            let h_nw = at(tx, ty);
            let h_ne = at(tx + 1, ty);
            let h_sw = at(tx, ty + 1);
            let h_se = at(tx + 1, ty + 1);

            // Skip empty tiles.
            if tid == 0 && h_nw == 0.0 && h_ne == 0.0 && h_sw == 0.0 && h_se == 0.0 {
                continue;
            }

            let z_nw = h_nw * height_scale;
            let z_ne = h_ne * height_scale;
            let z_sw = h_sw * height_scale;
            let z_se = h_se * height_scale;

            let mx0 = mx[tx as usize];
            let mx1 = mx[tx as usize + 1];
            let my0 = my[ty as usize];
            let my1 = my[ty as usize + 1];

            // Top face: two triangles NW→NE→SW, NE→SE→SW.
            let n_nw = vn(tx, ty);
            let n_ne = vn(tx + 1, ty);
            let n_sw = vn(tx, ty + 1);
            let n_se = vn(tx + 1, ty + 1);

            // Face tileId = -steepness (always ≤ 0.5 for fragment shader to route to tileset).
            let z_max = z_nw.max(z_ne).max(z_sw).max(z_se);
            let z_min = z_nw.min(z_ne).min(z_sw).min(z_se);
            let steepness = ((z_max - z_min) / height_scale.max(0.001)).min(1.0);
            let face_tid = -steepness;

            push_vert(&mut data, tx as f32, ty as f32, z_nw, mx0, my0, face_tid, n_nw);
            push_vert(&mut data, tx as f32 + 1.0, ty as f32, z_ne, mx1, my0, face_tid, n_ne);
            push_vert(&mut data, tx as f32, ty as f32 + 1.0, z_sw, mx0, my1, face_tid, n_sw);
            push_vert(&mut data, tx as f32 + 1.0, ty as f32, z_ne, mx1, my0, face_tid, n_ne);
            push_vert(&mut data, tx as f32 + 1.0, ty as f32 + 1.0, z_se, mx1, my1, face_tid, n_se);
            push_vert(&mut data, tx as f32, ty as f32 + 1.0, z_sw, mx0, my1, face_tid, n_sw);

            // Wall faces — only at map borders (outer cliff sides).
            let wall_tid = tid.max(1) as f32;
            let h_this = (z_nw + z_ne + z_sw + z_se) / (4.0 * height_scale.max(0.001));

            // East wall (right border)
            if tx + 1 >= size_x && h_this > 0.0 {
                let n = [1.0, 0.0, 0.0];
                push_wall(&mut data, tx as f32 + 1.0, ty as f32, z_ne, z_se, mx1, my0, wall_tid, n);
            }
            // South wall (bottom border)
            if ty + 1 >= size_y && h_this > 0.0 {
                let n = [0.0, 1.0, 0.0];
                push_wall(&mut data, tx as f32, ty as f32 + 1.0, z_sw, z_se, mx0, my1, wall_tid, n);
            }
            // West wall (left border)
            if tx == 0 && h_this > 0.0 {
                let n = [-1.0, 0.0, 0.0];
                push_wall(&mut data, tx as f32, ty as f32, z_nw, z_sw, mx0, my0, wall_tid, n);
            }
            // North wall (top border)
            if ty == 0 && h_this > 0.0 {
                let n = [0.0, -1.0, 0.0];
                push_wall(&mut data, tx as f32, ty as f32, z_nw, z_ne, mx0, my0, wall_tid, n);
            }
        }
    }

    let vcount = data.len() / VERT_FLOATS;
    (data, vcount)
}

/// Accumulate the two triangle normals of every tile onto its four corner
/// vertices, then normalise.  Produces the smooth shading normals used by the
/// top faces.
fn build_vertex_normals(
    size_x: i32,
    size_y: i32,
    height_scale: f32,
    at: &impl Fn(i32, i32) -> f32,
) -> Vec<[f32; 3]> {
    let stride = size_x as usize + 1;
    let mut acc = vec![[0f32; 3]; stride * (size_y as usize + 1)];

    for ty in 0..size_y {
        for tx in 0..size_x {
            let z_nw = at(tx, ty) * height_scale;
            let z_ne = at(tx + 1, ty) * height_scale;
            let z_sw = at(tx, ty + 1) * height_scale;
            let z_se = at(tx + 1, ty + 1) * height_scale;

            let n1 = tri_normal((1.0, 0.0, z_ne - z_nw), (0.0, 1.0, z_sw - z_nw));
            let n2 = tri_normal((0.0, 1.0, z_se - z_ne), (-1.0, 1.0, z_sw - z_ne));

            let mut add = |vx: i32, vy: i32, n: [f32; 3]| {
                let i = vy as usize * stride + vx as usize;
                acc[i][0] += n[0];
                acc[i][1] += n[1];
                acc[i][2] += n[2];
            };
            // Triangle 1 touches NW, NE, SW; triangle 2 touches NE, SE, SW.
            add(tx, ty, n1);
            add(tx + 1, ty, n1);
            add(tx, ty + 1, n1);
            add(tx + 1, ty, n2);
            add(tx + 1, ty + 1, n2);
            add(tx, ty + 1, n2);
        }
    }

    for n in acc.iter_mut() {
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if len > 1e-6 {
            n[0] /= len;
            n[1] /= len;
            n[2] /= len;
        } else {
            *n = [0.0, 0.0, 1.0];
        }
    }
    acc
}

#[allow(clippy::too_many_arguments)]
fn push_vert(
    data: &mut Vec<f32>,
    x: f32,
    y: f32,
    z: f32,
    mxv: f32,
    myv: f32,
    tid: f32,
    normal: [f32; 3],
) {
    data.extend_from_slice(&[x, y, z, mxv, myv, tid, normal[0], normal[1], normal[2]]);
}

#[allow(clippy::too_many_arguments)]
fn push_wall(
    data: &mut Vec<f32>,
    x: f32,
    y: f32,
    z_lo: f32,
    z_hi: f32,
    mxv: f32,
    myv: f32,
    tid: f32,
    normal: [f32; 3],
) {
    let n = normal;
    // Two triangles: lo, mid, hi, lo, hi, mid (twisted quad = 6 verts)
    let mid_x = x + 0.5;
    let mid_y = y + 0.5;
    let mid_z = (z_lo + z_hi) / 2.0;

    push_vert(data, x, y, z_lo, mxv, myv, tid, n);
    push_vert(data, mid_x, mid_y, mid_z, mxv, myv, tid, n);
    push_vert(data, x, y, z_hi, mxv, myv, tid, n);
    push_vert(data, x, y, z_lo, mxv, myv, tid, n);
    push_vert(data, x, y, z_hi, mxv, myv, tid, n);
    push_vert(data, mid_x, mid_y, mid_z, mxv, myv, tid, n);
}

fn tri_normal(d1: (f32, f32, f32), d2: (f32, f32, f32)) -> [f32; 3] {
    let a = glam::Vec3::new(d1.0, d1.1, d1.2);
    let b = glam::Vec3::new(d2.0, d2.1, d2.2);
    let n = a.cross(b).normalize();
    [n.x, n.y, n.z]
}

/// Bilinear interpolation of height data at an iso-space position (px, py).
///
/// `heights` has shape `(size_x + 1) × (size_y + 1)` (one sample per edge vertex).
///
/// Re-exported from `classic-pathfinder` (the single source of truth shared
/// with the wasm worker module) so callers keep using
/// `classic_core::tilemap::bilinear_height`.
pub use classic_pathfinder::bilinear_height;

/// Horizontal depth divisor in the canonical iso-depth formula
/// `iso_depth(tx, ty, z) = (tx - ty) / HORIZONTAL_DEPTH_SCALE + 0.5 + z / D`.
/// One unit of `tx - ty` spans this many iso-depth steps.
///
/// Legacy fixed value (== [`horizontal_depth_scale`] for a 200×200 map).
/// Prefer [`horizontal_depth_scale`] so larger maps are not clipped at the
/// NE/SW corners.
pub const HORIZONTAL_DEPTH_SCALE: f32 = 400.0;

/// Horizontal depth divisor for a tilemap of `size_x × size_y`, in the
/// canonical iso-depth formula
/// `iso_depth(tx, ty, z) = (tx - ty) / scale + 0.5 + z / D`.
///
/// `tx - ty` spans `[-size_y, size_x]`, so `scale = 2 · max(size_x, size_y)`
/// keeps the horizontal term within `[-0.5, +0.5]` (window depth `[0, 1]`)
/// for every tile.  A fixed scale smaller than this clips the NE (`tx - ty` at
/// its maximum) and SW (`tx - ty` at its minimum) corners, since window depth
/// outside `[0, 1]` maps to clip-z outside `[-1, 1]`.
pub fn horizontal_depth_scale(size_x: i32, size_y: i32) -> f32 {
    2.0 * size_x.max(size_y).max(1) as f32
}

/// Pixels per metre: the fixed conversion the render/depth space uses between
/// world metres and tileset pixels.  `height_data` is authored in **metres**
/// (the exporter's unit); the mesh and sprite positioning convert metres to
/// screen pixels via `* PPM_TARGET`.
pub const PPM_TARGET: f32 = 64.0;

/// Height depth divisor in the canonical iso-depth formula, for `z` in
/// **metres** (`height_data`, after re-expression from tileset pixels).
///
/// Derived from the exporter's 30°-elevation view axis (see
/// `classic-assets` / `make_lrv_spritesheet.py`): the camera basis is
/// `back = right × up = (−√(3/8), −√(3/8), +1/2)`, so one metre of height
/// contributes `back.z = 0.5` of view depth while one tile of `tx - ty`
/// contributes `√(3/8) · (TILE_PX / PPM_TARGET)`.  The height term is
/// **positive** (`+ z / D`): `back.z = +0.5` means taller terrain is farther,
/// i.e. larger depth.
///
/// The mesh/sprite `z` is carried in tileset pixels; the depth formula converts
/// it back to metres via `z_m = z_px / PPM_TARGET`, so the pixel-space divisor
/// is `HEIGHT_DEPTH_SCALE_M · PPM_TARGET ≈ 22045.4`:
///
/// `z_m / 344.46 = (z_px / 64) / 344.46 = z_px / 22045.4`
pub const HEIGHT_DEPTH_SCALE_M: f32 = 344.46;

/// Sample terrain height at iso-space position `(px, py)` using the same
/// triangle-linear interpolation as [`build_mesh`] (top faces split into
/// `NW→NE→SW` and `NE→SE→SW`), so sprite positioning and per-pixel depth
/// match the terrain mesh exactly rather than via [`bilinear_height`].
///
/// `heights` has shape `(size_x + 1) × (size_y + 1)` (one sample per vertex).
/// Barycentric weights:
///   - lower triangle (`fx + fy ≤ 1`): `h_nw·(1-fx-fy) + h_ne·fx + h_sw·fy`
///   - upper triangle (otherwise):   `h_ne·(1-fy) + h_se·(fx+fy-1) + h_sw·(1-fx)`
pub fn sample_height_mesh(heights: &[f32], size_x: i32, size_y: i32, px: f32, py: f32) -> f32 {
    // Same empty/not-yet-committed grid tolerance as `bilinear_height`.
    if heights.len() != (size_x as usize + 1) * (size_y as usize + 1) {
        return 0.0;
    }

    let ftx = px.floor() as i32;
    let fty = py.floor() as i32;
    let fx = px - ftx as f32;
    let fy = py - fty as f32;

    let at = |tx: i32, ty: i32| -> f32 {
        let tx = tx.clamp(0, size_x) as usize;
        let ty = ty.clamp(0, size_y) as usize;
        heights[ty * (size_x as usize + 1) + tx]
    };

    let h_nw = at(ftx, fty);
    let h_ne = at(ftx + 1, fty);
    let h_sw = at(ftx, fty + 1);
    let h_se = at(ftx + 1, fty + 1);

    if fx + fy <= 1.0 {
        h_nw * (1.0 - fx - fy) + h_ne * fx + h_sw * fy
    } else {
        h_ne * (1.0 - fy) + h_se * (fx + fy - 1.0) + h_sw * (1.0 - fx)
    }
}

/// Build the tile-data texture as RGBA `u8` pixels.
///
/// Each tile's value (0..255) is stored in R, G, B channels with A=255.
/// Returns `(pixels, width, height)` where `pixels.len() == width * height * 4`.
pub fn build_tile_texture(size_x: i32, size_y: i32, tiles: &[u32]) -> (Vec<u8>, u32, u32) {
    let w = size_x as u32;
    let h = size_y as u32;
    let mut pixels = vec![0u8; (w * h * 4) as usize];
    for ty in 0..size_y {
        for tx in 0..size_x {
            let idx = (ty * size_x + tx) as usize;
            let v = tiles.get(idx).copied().unwrap_or(0).min(255) as u8;
            let p = (ty as u32 * w + tx as u32) as usize * 4;
            pixels[p] = v;
            pixels[p + 1] = v;
            pixels[p + 2] = v;
            pixels[p + 3] = 255;
        }
    }
    (pixels, w, h)
}
