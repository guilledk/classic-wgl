//! # Skill: `classic-iso`
//!
//! **Read `.claude/skills/classic-iso/SKILL.md` before working on this module.**
//!
//! Isometric tilemap mesh generation.
//!
//! Port of `buildMesh()`, `uploadToGPU()` and helpers from `src/classic/isometric.ts`.

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

    // Worst-case allocation: 2 faces + 4 walls = 30 vertices × 9 floats per tile.
    let max_verts = (size_x * size_y * 30) as usize * VERT_FLOATS;
    let mut data: Vec<f32> = Vec::with_capacity(max_verts);

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
            let n_top = tri_normal((1.0, 0.0, z_ne - z_nw), (0.0, 1.0, z_sw - z_nw));
            let n_top2 = tri_normal((0.0, 1.0, z_se - z_ne), (-1.0, 1.0, z_sw - z_ne));

            // Face tileId = -steepness (always ≤ 0.5 for fragment shader to route to tileset).
            let z_max = z_nw.max(z_ne).max(z_sw).max(z_se);
            let z_min = z_nw.min(z_ne).min(z_sw).min(z_se);
            let steepness = ((z_max - z_min) / height_scale.max(0.001)).min(1.0);
            let face_tid = -steepness;

            push_vert(&mut data, tx as f32, ty as f32, z_nw, mx0, my0, face_tid, n_top);
            push_vert(&mut data, tx as f32 + 1.0, ty as f32, z_ne, mx1, my0, face_tid, n_top);
            push_vert(&mut data, tx as f32, ty as f32 + 1.0, z_sw, mx0, my1, face_tid, n_top);
            push_vert(&mut data, tx as f32 + 1.0, ty as f32, z_ne, mx1, my0, face_tid, n_top2);
            push_vert(
                &mut data,
                tx as f32 + 1.0,
                ty as f32 + 1.0,
                z_se,
                mx1,
                my1,
                face_tid,
                n_top2,
            );
            push_vert(&mut data, tx as f32, ty as f32 + 1.0, z_sw, mx0, my1, face_tid, n_top2);

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
pub fn bilinear_height(heights: &[f32], size_x: i32, size_y: i32, px: f32, py: f32) -> f32 {
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

    h_nw + (h_ne - h_nw) * fx + (h_sw - h_nw) * fy + (h_nw - h_ne - h_sw + h_se) * fx * fy
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
