use classic_core::tilemap::bilinear_height;

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
