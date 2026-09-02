/// The mesh capacity bound must be exact: too small and every large map pays
/// for reallocation mid-build, too large and a 400x400 map reserves hundreds
/// of megabytes it never uses.
#[test]
fn mesh_capacity_bound_is_tight_and_sufficient() {
    for (sx, sy) in [(1, 1), (8, 8), (16, 4), (64, 96)] {
        let tiles = vec![1u32; (sx * sy) as usize];
        let heights = vec![1.0f32; ((sx + 1) * (sy + 1)) as usize];
        let (data, vcount) = classic_core::tilemap::build_mesh(sx, sy, &tiles, &heights);

        // Every tile is non-empty here, so this is the true worst case.
        let bound = (sx as usize * sy as usize * 6) + 2 * (sx as usize + sy as usize) * 6;
        assert!(vcount <= bound, "{sx}x{sy}: emitted {vcount} vertices, bound was {bound}");
        assert_eq!(
            data.capacity(),
            bound * 9,
            "{sx}x{sy}: capacity should equal the bound exactly (no realloc, no waste)"
        );
    }
}

/// Smooth vertex normals must be a no-op on level terrain, so enabling them
/// cannot disturb the existing flat demo scene or its golden baseline.
#[test]
fn flat_terrain_normals_are_all_up() {
    let sx = 8;
    let sy = 8;
    let tiles = vec![1u32; (sx * sy) as usize];
    let heights = vec![1.0f32; ((sx + 1) * (sy + 1)) as usize];
    let (data, vcount) = classic_core::tilemap::build_mesh(sx, sy, &tiles, &heights);
    assert_eq!(data.len(), vcount * 9);
    for v in 0..vcount {
        let n = &data[v * 9 + 6..v * 9 + 9];
        // Border walls carry their own axis-aligned normals; top faces must
        // all be exactly +Z.
        let is_wall = n[0] != 0.0 || n[1] != 0.0;
        if !is_wall {
            assert_eq!(n, [0.0, 0.0, 1.0], "vertex {v} top-face normal is not +Z");
        }
    }
}
