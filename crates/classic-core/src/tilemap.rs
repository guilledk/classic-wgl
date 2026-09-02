//! # Skill: `classic-iso`
//!
//! **Read `.agents/skills/classic-iso/SKILL.md` before working on this module.**
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
pub fn build_mesh(size_x: i32, size_y: i32, tiles: &[u32], heights: &[f32]) -> (Vec<f32>, usize) {
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
    // World-space tile lattice: `(tx·TILE_M, −ty·TILE_M, h)`.
    let wx = |t: i32| t as f32 * TILE_M;
    let wy = |t: i32| -t as f32 * TILE_M;

    let vnormals = build_vertex_normals(size_x, size_y, &at);
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

            // Height is already world metres (no `* height_scale`).
            let z_nw = h_nw;
            let z_ne = h_ne;
            let z_sw = h_sw;
            let z_se = h_se;

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
            let steepness = ((z_max - z_min) / TILE_M).min(1.0);
            let face_tid = -steepness;

            push_vert(&mut data, wx(tx), wy(ty), z_nw, mx0, my0, face_tid, n_nw);
            push_vert(&mut data, wx(tx + 1), wy(ty), z_ne, mx1, my0, face_tid, n_ne);
            push_vert(&mut data, wx(tx), wy(ty + 1), z_sw, mx0, my1, face_tid, n_sw);
            push_vert(&mut data, wx(tx + 1), wy(ty), z_ne, mx1, my0, face_tid, n_ne);
            push_vert(&mut data, wx(tx + 1), wy(ty + 1), z_se, mx1, my1, face_tid, n_se);
            push_vert(&mut data, wx(tx), wy(ty + 1), z_sw, mx0, my1, face_tid, n_sw);

            // Wall faces — only at map borders (outer cliff sides).
            let wall_tid = tid.max(1) as f32;
            let h_this = (z_nw + z_ne + z_sw + z_se) / 4.0;

            // East wall (right border): outward +tx → +X.
            if tx + 1 >= size_x && h_this > 0.0 {
                let n = [1.0, 0.0, 0.0];
                push_wall(&mut data, tx as f32 + 1.0, ty as f32, z_ne, z_se, mx1, my0, wall_tid, n);
            }
            // South wall (bottom border): outward +ty → −Y.
            if ty + 1 >= size_y && h_this > 0.0 {
                let n = [0.0, -1.0, 0.0];
                push_wall(&mut data, tx as f32, ty as f32 + 1.0, z_sw, z_se, mx0, my1, wall_tid, n);
            }
            // West wall (left border): outward −tx → −X.
            if tx == 0 && h_this > 0.0 {
                let n = [-1.0, 0.0, 0.0];
                push_wall(&mut data, tx as f32, ty as f32, z_nw, z_sw, mx0, my0, wall_tid, n);
            }
            // North wall (top border): outward −ty → +Y.
            if ty == 0 && h_this > 0.0 {
                let n = [0.0, 1.0, 0.0];
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
fn build_vertex_normals(size_x: i32, size_y: i32, at: &impl Fn(i32, i32) -> f32) -> Vec<[f32; 3]> {
    let stride = size_x as usize + 1;
    let mut acc = vec![[0f32; 3]; stride * (size_y as usize + 1)];

    // Metric world position: `(tx·TILE_M, −ty·TILE_M, h)`.
    let pos =
        |tx: i32, ty: i32| glam::Vec3::new(tx as f32 * TILE_M, -ty as f32 * TILE_M, at(tx, ty));

    for ty in 0..size_y {
        for tx in 0..size_x {
            let nw = pos(tx, ty);
            let ne = pos(tx + 1, ty);
            let sw = pos(tx, ty + 1);
            let se = pos(tx + 1, ty + 1);

            // Two triangles (NW→NE→SW, NE→SE→SW).  The `+ty → −Y` flip makes
            // the up normal `(sw−nw) × (ne−nw)` (and the analogous cross for the
            // second triangle) — both +Z on flat terrain.
            let n1 = (sw - nw).cross(ne - nw).normalize();
            let n2 = (sw - ne).cross(se - ne).normalize();

            let mut add = |vx: i32, vy: i32, n: glam::Vec3| {
                let i = vy as usize * stride + vx as usize;
                acc[i][0] += n.x;
                acc[i][1] += n.y;
                acc[i][2] += n.z;
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
    tx: f32,
    ty: f32,
    z_lo: f32,
    z_hi: f32,
    mxv: f32,
    myv: f32,
    tid: f32,
    normal: [f32; 3],
) {
    let n = normal;
    // Two triangles: lo, mid, hi, lo, hi, mid (twisted quad = 6 verts).  The
    // corner is converted from tile space to world metres.
    let wx = tx * TILE_M;
    let wy = -ty * TILE_M;
    let mid_x = (tx + 0.5) * TILE_M;
    let mid_y = -(ty + 0.5) * TILE_M;
    let mid_z = (z_lo + z_hi) / 2.0;

    push_vert(data, wx, wy, z_lo, mxv, myv, tid, n);
    push_vert(data, mid_x, mid_y, mid_z, mxv, myv, tid, n);
    push_vert(data, wx, wy, z_hi, mxv, myv, tid, n);
    push_vert(data, wx, wy, z_lo, mxv, myv, tid, n);
    push_vert(data, wx, wy, z_hi, mxv, myv, tid, n);
    push_vert(data, mid_x, mid_y, mid_z, mxv, myv, tid, n);
}

/// Bilinear interpolation of height data at an iso-space position (px, py).
///
/// `heights` has shape `(size_x + 1) × (size_y + 1)` (one sample per edge vertex).
///
/// Re-exported from `classic-pathfinder` (the single source of truth shared
/// with the wasm worker module) so callers keep using
/// `classic_core::tilemap::bilinear_height`.
pub use classic_pathfinder::bilinear_height;

/// Pixels per metre: the fixed conversion the raster space uses between
/// world metres and tileset pixels.  `height_data` is authored in **metres**
/// (the exporter's unit); the mesh and sprite positioning convert metres to
/// screen pixels via `* PPM_TARGET`.
pub const PPM_TARGET: f32 = 64.0;

/// Pixel size of a tile edge in the tileset texture.  This is a **raster**
/// dimension only — never a spatial unit.  See [`TILE_M`] for the metre length.
pub const TILE_PX: f32 = 45.0;

/// Metre length of a tile edge: `TILE_PX / PPM_TARGET = 45 / 64 = 0.703125 m`.
///
/// This is the missing constant that makes the tile lattice a proper metric
/// grid.  A one-tile step along `+tx`/`+ty` is `TILE_M` metres at `PPM_TARGET`
/// px/m, so horizontal distance and vertical height (already authored in
/// metres) finally share one unit — resolving the long-standing "32 vs 45"
/// horizontal/height incommensurability.
pub const TILE_M: f32 = TILE_PX / PPM_TARGET;

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

/// Cast a world-space [`Ray`] against the terrain height field and return the
/// first hit point (nearest the ray origin), or `None` if the ray never crosses
/// the surface within `max_dist` metres.
///
/// Terrain is modelled as the solid surface `z = height(x, y)` above the
/// `z = 0` plane, sampled with [`sample_height_mesh`] (the same triangle-linear
/// interpolation the mesh uses).  The ray is marched in fixed [`TILE_M`]-sized
/// world-metre steps; the first step that moves to the far side of the surface
/// brackets the crossing, which is then refined by bisection.  This is the
/// terrain-only form of the general raycast — colliders would add stop
/// conditions inside the same march loop.
///
/// `heights` has shape `(size_x + 1) × (size_y + 1)`, as in [`sample_height_mesh`].
pub fn raycast_terrain(
    ray: crate::math::Ray,
    heights: &[f32],
    size_x: i32,
    size_y: i32,
    max_dist: f32,
) -> Option<glam::Vec3> {
    // World metres → tile coords, mirroring `iso_world_pos`'s inverse
    // (`iso_world_pos(tx, ty, h) = (tx·TILE_M, −ty·TILE_M, h)`).
    let height_at = |world: glam::Vec3| -> f32 {
        sample_height_mesh(heights, size_x, size_y, world.x / TILE_M, -world.y / TILE_M)
    };

    let step = TILE_M;
    let steps = (max_dist / step).ceil().max(1.0) as usize;

    let mut t0 = 0.0;
    let mut d0 = ray.at(t0).z - height_at(ray.at(t0));
    // The origin is already on the surface.
    if d0.abs() < 1e-6 {
        return Some(ray.at(t0));
    }

    for _ in 0..steps {
        let t1 = (t0 + step).min(max_dist);
        let p1 = ray.at(t1);
        let d1 = p1.z - height_at(p1);
        if d1 * d0 <= 0.0 {
            // Crossing between t0 and t1: bisect on the sign of
            // `p.z − height_at(p)`, keeping `lo` below and `hi` above the surface.
            let (mut lo, mut hi) = if d0 < 0.0 { (t0, t1) } else { (t1, t0) };
            for _ in 0..24 {
                let mid = (lo + hi) * 0.5;
                let d = ray.at(mid).z - height_at(ray.at(mid));
                if d < 0.0 {
                    lo = mid;
                } else if d > 0.0 {
                    hi = mid;
                } else {
                    lo = mid;
                    hi = mid;
                    break;
                }
            }
            return Some(ray.at((lo + hi) * 0.5));
        }
        t0 = t1;
        d0 = d1;
        if t1 >= max_dist {
            break;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{iso_camera_px, iso_camera_ray, iso_world_pos, DEPTH_FAR, DEPTH_NEAR};

    fn heights(size_x: i32, size_y: i32, h: fn(f32, f32) -> f32) -> Vec<f32> {
        (0..=size_y).flat_map(|ty| (0..=size_x).map(move |tx| h(tx as f32, ty as f32))).collect()
    }

    #[test]
    fn raycast_terrain_round_trips_through_camera() {
        // A gentle ramp so the ray crosses the surface exactly once.
        let (size_x, size_y) = (16, 16);
        let hd = heights(size_x, size_y, |_tx, ty| 0.5 + 0.1 * ty);

        for (tx, ty) in [(3.0, 4.0), (8.0, 12.0), (14.0, 2.0)] {
            let surface = iso_world_pos(tx, ty, 0.5 + 0.1 * ty);
            let px = iso_camera_px(surface);
            let ray = iso_camera_ray(px.truncate());
            let hit = raycast_terrain(ray, &hd, size_x, size_y, DEPTH_NEAR - DEPTH_FAR)
                .expect("ray must hit the ramp");
            assert!((hit - surface).length() < 1e-3, "hit {hit:?} != surface {surface:?}");
        }
    }

    #[test]
    fn raycast_terrain_flat_is_exact() {
        let (size_x, size_y) = (8, 8);
        let hd = heights(size_x, size_y, |_, _| 1.0);
        let surface = iso_world_pos(4.0, 4.0, 1.0);
        let px = iso_camera_px(surface);
        let ray = iso_camera_ray(px.truncate());
        let hit = raycast_terrain(ray, &hd, size_x, size_y, DEPTH_NEAR - DEPTH_FAR).unwrap();
        assert!((hit - surface).length() < 1e-3, "hit {hit:?} != surface {surface:?}");
    }

    #[test]
    fn raycast_terrain_prefers_front_surface() {
        // A front plateau (small tx, h = 4) with a valley behind it (large tx,
        // h = 0).  The camera ray must stop at the plateau, not the occluded
        // valley floor — i.e. the ray is cast from the camera side, not from
        // the ground plane.
        let (size_x, size_y) = (16, 16);
        let hd = heights(size_x, size_y, |tx, _ty| {
            if tx <= 4.0 {
                4.0
            } else if tx >= 6.0 {
                0.0
            } else {
                4.0 - 2.0 * (tx - 4.0)
            }
        });
        let surface = iso_world_pos(2.0, 8.0, 4.0);
        let px = iso_camera_px(surface);
        let ray = iso_camera_ray(px.truncate());
        let hit = raycast_terrain(ray, &hd, size_x, size_y, DEPTH_NEAR - DEPTH_FAR).unwrap();
        assert!(
            (hit - surface).length() < 1e-2,
            "hit {hit:?} should be the front plateau, not the valley behind it"
        );
    }
}
