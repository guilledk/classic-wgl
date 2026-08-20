use classic_core::tilemap::{bilinear_height, sample_height_mesh};

#[test]
fn bilinear_height_flat_uniform() {
    let size_x: i32 = 4;
    let size_y: i32 = 4;
    let h = vec![1.0_f32; ((size_x + 1) * (size_y + 1)) as usize];

    assert!((bilinear_height(&h, size_x, size_y, 1.0, 1.0) - 1.0).abs() < 0.001);
    assert!((bilinear_height(&h, size_x, size_y, 1.5, 1.5) - 1.0).abs() < 0.001);
}

#[test]
fn bilinear_height_slope() {
    let sx: i32 = 4;
    let sy: i32 = 4;
    let mut h = vec![0.0_f32; ((sx + 1) * (sy + 1)) as usize];
    for ty in 0..=sy {
        for tx in 0..=sx {
            h[(ty * (sx + 1) + tx) as usize] = tx as f32;
        }
    }

    assert_eq!(bilinear_height(&h, sx, sy, 0.0, 0.0), 0.0);
    assert_eq!(bilinear_height(&h, sx, sy, 1.0, 0.0), 1.0);
    assert_eq!(bilinear_height(&h, sx, sy, 2.0, 0.0), 2.0);
    assert_eq!(bilinear_height(&h, sx, sy, 0.0, 1.0), 0.0);

    assert!((bilinear_height(&h, sx, sy, 0.5, 0.0) - 0.5).abs() < 0.001);
    assert!((bilinear_height(&h, sx, sy, 1.5, 0.0) - 1.5).abs() < 0.001);
}

#[test]
fn bilinear_height_clamps_to_boundary() {
    let sx: i32 = 4;
    let sy: i32 = 4;
    let h = vec![1.0_f32; ((sx + 1) * (sy + 1)) as usize];

    // Should not panic
    let _ = bilinear_height(&h, sx, sy, -5.0, -5.0);
    let _ = bilinear_height(&h, sx, sy, 100.0, 100.0);
}

#[test]
fn bilinear_height_empty_or_short_is_flat() {
    // A generated map has no height grid until its guest commits it; sampling
    // before then must return flat (0.0) rather than panic.
    let sx: i32 = 4;
    let sy: i32 = 4;
    assert_eq!(bilinear_height(&[], sx, sy, 2.0, 2.0), 0.0);
    let short = vec![0.0_f32; 3];
    assert_eq!(bilinear_height(&short, sx, sy, 2.0, 2.0), 0.0);
}

#[test]
fn bilinear_height_interpolates_diagonally() {
    let sx: i32 = 2;
    let sy: i32 = 2;
    // height vertex grid (sx+1)×(sy+1) = 3×3 = 9 values
    // Row-major: y*(sx+1)+x
    // [0,1] = 0,0,0  (y=0)
    // [3,5] = 0,1,1  (y=1)
    // [6,8] = 0,1,1  (y=2)
    let h = vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0];

    assert_eq!(bilinear_height(&h, sx, sy, 0.0, 0.0), 0.0);
    assert_eq!(bilinear_height(&h, sx, sy, 2.0, 2.0), 1.0);
    let mid = bilinear_height(&h, sx, sy, 1.5, 1.5);
    assert!((mid - 1.0).abs() < 0.001, "mid={}", mid);
    let low = bilinear_height(&h, sx, sy, 0.5, 0.5);
    assert!(low > 0.0 && low < 0.5, "low={}", low);
}

#[test]
fn sample_height_mesh_flat_uniform() {
    let size_x: i32 = 4;
    let size_y: i32 = 4;
    let h = vec![1.0_f32; ((size_x + 1) * (size_y + 1)) as usize];

    assert!((sample_height_mesh(&h, size_x, size_y, 1.0, 1.0) - 1.0).abs() < 0.001);
    assert!((sample_height_mesh(&h, size_x, size_y, 1.5, 1.5) - 1.0).abs() < 0.001);
}

#[test]
fn sample_height_mesh_matches_vertices_and_edges() {
    // A plane tilted along x only: h = tx.  Triangle-linear interpolation is
    // exact for a planar field, so it must reproduce the vertex values and the
    // linear ramp everywhere (identical to bilinear in this case).
    let sx: i32 = 4;
    let sy: i32 = 4;
    let mut h = vec![0.0_f32; ((sx + 1) * (sy + 1)) as usize];
    for ty in 0..=sy {
        for tx in 0..=sx {
            h[(ty * (sx + 1) + tx) as usize] = tx as f32;
        }
    }

    assert_eq!(sample_height_mesh(&h, sx, sy, 0.0, 0.0), 0.0);
    assert_eq!(sample_height_mesh(&h, sx, sy, 1.0, 0.0), 1.0);
    assert_eq!(sample_height_mesh(&h, sx, sy, 2.0, 0.0), 2.0);
    assert!((sample_height_mesh(&h, sx, sy, 0.5, 0.0) - 0.5).abs() < 0.001);
    assert!((sample_height_mesh(&h, sx, sy, 1.5, 0.0) - 1.5).abs() < 0.001);
}

#[test]
fn sample_height_mesh_splits_on_diagonal() {
    // A cell whose NE→SW diagonal is a ridge: NW=0, NE=1, SW=1, SE=0.
    // The triangle sampler is linear within each triangle and continuous on
    // the shared diagonal (bilinear would bow this saddle to 0.5 at centre).
    let sx: i32 = 1;
    let sy: i32 = 1;
    // vertex grid 2x2: [NW, NE, SW, SE] row-major
    let h = vec![0.0, 1.0, 1.0, 0.0];

    // Lower triangle (NW→NE→SW): center (0.25, 0.25) → 0·0.5 + 1·0.25 + 1·0.25
    let lower = sample_height_mesh(&h, sx, sy, 0.25, 0.25);
    assert!((lower - 0.5).abs() < 1e-6, "lower={lower}");

    // Upper triangle (NE→SE→SW): center (0.75, 0.75) → 1·0.25 + 0·0.5 + 1·0.25
    let upper = sample_height_mesh(&h, sx, sy, 0.75, 0.75);
    assert!((upper - 0.5).abs() < 1e-6, "upper={upper}");

    // The NE→SW ridge (fx + fy == 1) is at height 1 everywhere, and both
    // triangles agree on it.
    for (px, py) in [(0.5, 0.5), (0.75, 0.25), (0.25, 0.75)] {
        let v = sample_height_mesh(&h, sx, sy, px, py);
        assert!((v - 1.0).abs() < 1e-6, "ridge ({px},{py})={v}");
    }
}

#[test]
fn sample_height_mesh_clamps_to_boundary() {
    let sx: i32 = 4;
    let sy: i32 = 4;
    let h = vec![1.0_f32; ((sx + 1) * (sy + 1)) as usize];

    let _ = sample_height_mesh(&h, sx, sy, -5.0, -5.0);
    let _ = sample_height_mesh(&h, sx, sy, 100.0, 100.0);
}

#[test]
fn sample_height_mesh_empty_or_short_is_flat() {
    let sx: i32 = 4;
    let sy: i32 = 4;
    assert_eq!(sample_height_mesh(&[], sx, sy, 2.0, 2.0), 0.0);
    let short = vec![0.0_f32; 3];
    assert_eq!(sample_height_mesh(&short, sx, sy, 2.0, 2.0), 0.0);
}
