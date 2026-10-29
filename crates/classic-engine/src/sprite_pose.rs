//! Standalone posed sprites: a plain `IsoSprite` whose frame table carries
//! pose axes (`FrameTable::axes`, sheet v2: directions × optional pitch ×
//! roll) is driven by a continuous heading plus a tilt instead of a raw
//! frame index, so a grounded container gets the same direction warp
//! (`dir_interp`) and anchor depth as a vehicle layer.
//!
//! - [`Engine::sprite_pose`] picks the nearest pitch / roll level and frames
//!   the direction like a vehicle layer (`Engine::set_layer_frame`): the
//!   nearest direction, or both neighbours *within that tilt frame* plus a
//!   [`DirBlend`](classic_core::components::DirBlend) on a `dir_interp`
//!   sheet.  The warp is a yaw about the anchor's vertical; a tilt frame
//!   baked yaw ∘ tilt about the ground anchor differs from its neighbour by
//!   exactly that yaw, so the blend needs no tilt correction.
//! - [`Engine::sprite_fit_terrain`] fits the tilt to the terrain under a
//!   square footprint along the heading ([`Engine::terrain_tilt`]) and lifts
//!   the sprite (`frame_offset.z`) so no corner of the drawn (quantized)
//!   tilt buries — the engine port of the lunar guest's `container_pose_dir`
//!   + `rest_z`, with a continuous heading.

use classic_core::components::{IsoSprite, Transform};
use classic_core::tilemap::TILE_M;
use classic_core::vehicle_def::{pick_tilt, SpriteAxes, TiltPick};

use crate::vehicle::layers::LayerTilt;
use crate::Engine;

/// The tile-space unit vectors `(forward, left)` of tile heading `heading`
/// (radians, `atan2(dy, dx)`: `0` = East, `+π/2` = South): left is forward
/// turned a quarter anticlockwise on screen (heading East: North, tile −y),
/// as `vehicle::layers::chassis_to_tile` turns the chassis `y` axis.
fn heading_axes(heading: f32) -> ([f32; 2], [f32; 2]) {
    let (s, c) = heading.sin_cos();
    ([c, s], [s, -c])
}

impl Engine {
    /// Pose a named `IsoSprite` by tile heading (radians) and tilt (`pitch`
    /// nose-up, `roll` left-up, radians), through its texture's pose axes
    /// (`FrameTable::sprite_axes`).  The tilt picks the nearest baked level;
    /// on a `tilt_interp` sheet the warp then rotates it to the exact tilt
    /// (clamped to the baked range), else it snaps.  The direction is the
    /// nearest one, or the two neighbours in that tilt frame blended on a
    /// `dir_interp` sheet.  The sprite's anchor is
    /// kept for both directions.  `false` without an `IsoSprite` or a frame
    /// table.
    pub fn sprite_pose(&mut self, name: &str, heading: f32, pitch: f32, roll: f32) -> bool {
        let Some(&e) = self.names.get(name) else { return false };
        let Ok(mut s) = self.world.get::<&mut IsoSprite>(e) else { return false };
        Self::pose_sprite(&self.frame_tables, &mut s, heading, pitch, roll)
    }

    /// [`Engine::sprite_pose`] on a sprite value: frame `s` for tile heading
    /// `heading` and tilt `(pitch, roll)` through its texture's pose axes.
    /// `false` (and `s` untouched) without a frame table.
    pub(crate) fn pose_sprite(
        tables: &std::collections::HashMap<String, classic_core::types::FrameTable>,
        s: &mut IsoSprite,
        heading: f32,
        pitch: f32,
        roll: f32,
    ) -> bool {
        let Some(table) = tables.get(&s.texture) else { return false };
        let (axes, pick) = Self::sprite_tilt(table, heading, pitch, roll);
        let dirs = axes.directions.max(1);
        let anchors = vec![s.anchor.to_array(); dirs as usize];
        let frame = |d, levels: Option<(u32, u32)>| {
            let (p, r) = levels.unwrap_or((pick.pitch, pick.roll));
            axes.frame_index(d, p, r)
        };
        let interp = table.tilt_interp && table.dir_interp;
        let tilt = LayerTilt { pick, pitch: axes.pitch, roll: axes.roll, interp };
        Self::set_layer_frame(tables, s, heading, dirs, &anchors, frame, Some(tilt));
        true
    }

    /// A posed sheet's axes and the frames / warp drawing tilt `(pitch,
    /// roll)` on it.
    fn sprite_tilt(
        table: &classic_core::types::FrameTable,
        heading: f32,
        pitch: f32,
        roll: f32,
    ) -> (SpriteAxes, TiltPick) {
        let axes = table.sprite_axes();
        let interp = table.tilt_interp && table.dir_interp;
        (axes, pick_tilt(axes.pitch, axes.roll, heading, pitch, roll, interp))
    }

    /// The terrain tilt `(pitch, roll)` (radians, nose-up / left-up
    /// positive) under a `span_m` square centred on tile point `(x, y)` and
    /// facing tile heading `heading`: the height differences between the
    /// front / rear and left / right edge midpoints over the span.
    pub fn terrain_tilt(&self, x: f32, y: f32, heading: f32, span_m: f32) -> (f32, f32) {
        let span = span_m.max(1e-3);
        let h = 0.5 * span / TILE_M;
        let (f, l) = heading_axes(heading);
        let at = |a: [f32; 2], k: f32| self.ground_at(x + a[0] * h * k, y + a[1] * h * k);
        let pitch = ((at(f, 1.0) - at(f, -1.0)) / span).atan();
        let roll = ((at(l, 1.0) - at(l, -1.0)) / span).atan();
        (pitch, roll)
    }

    /// Fit a named posed sprite to the terrain under it: pose it facing tile
    /// heading `heading` (radians) with the [`Engine::terrain_tilt`] of a
    /// `footprint_m` square at its position, and set its altitude
    /// (`frame_offset.z`) so the drawn tilt's lowest footprint corner just
    /// touches the terrain.  Returns the fitted (unquantized) tilt, or
    /// `None` when [`Engine::sprite_pose`] fails.
    pub fn sprite_fit_terrain(
        &mut self,
        name: &str,
        heading: f32,
        footprint_m: f32,
    ) -> Option<(f32, f32)> {
        let &e = self.names.get(name)?;
        let p = self.world.get::<&Transform>(e).ok()?.position;
        let (pitch, roll) = self.terrain_tilt(p.x, p.y, heading, footprint_m);
        if !self.sprite_pose(name, heading, pitch, roll) {
            return None;
        }
        let texture = self.world.get::<&IsoSprite>(e).ok()?.texture.clone();
        let table = self.frame_tables.get(&texture)?;
        // The tilt drawn: the baked level, or the exact one on a `tilt_interp`
        // sheet.
        let (dp, dr) = Self::sprite_tilt(table, heading, pitch, roll).1.drawn;
        let half = 0.5 * footprint_m.max(1e-3);
        let (f, l) = heading_axes(heading);
        let ground = self.height_at(p.x, p.y);
        let mut rest = f32::MIN;
        for (sf, sl) in [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
            let cx = p.x + (sf * f[0] + sl * l[0]) * half / TILE_M;
            let cy = p.y + (sf * f[1] + sl * l[1]) * half / TILE_M;
            // The corner's height above the anchor on the drawn tilt plane.
            let lift = sf * half * dp.tan() + sl * half * dr.tan();
            rest = rest.max(self.ground_at(cx, cy) - lift);
        }
        if let Ok(mut s) = self.world.get::<&mut IsoSprite>(e) {
            s.frame_offset.z = rest - ground;
        }
        Some((pitch, roll))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use classic_core::components::{Role, Tilemap};
    use classic_core::types::FrameTable;
    use classic_core::vehicle_def::AngleAxis;
    use classic_core::RoleKind;
    use glam::{Vec2, Vec3};

    const DEG: f32 = std::f32::consts::PI / 180.0;

    /// A `size`² tilemap whose vertex heights rise `rise` metres per tile
    /// toward +x tile (East).
    fn slope_tilemap(e: &mut Engine, size: i32, rise: f32) {
        let mut heights = Vec::with_capacity(((size + 1) * (size + 1)) as usize);
        for _vy in 0..=size {
            for vx in 0..=size {
                heights.push(vx as f32 * rise);
            }
        }
        let tm = Tilemap {
            position: Vec3::ZERO,
            scale: Vec3::ONE,
            size_x: size,
            size_y: size,
            tile_set: "tileset".into(),
            tile_pixel_size: [32, 32],
            max_tile: 16,
            tiles_grid: None,
            heights_grid: None,
            data: vec![0; (size * size) as usize],
            height_data: heights,
            height_scale: 64.0,
            tile_set_pixel_size: [0, 0],
            tiles_per_row: 0,
            mouse_iso_pos: Vec3::ZERO,
            selection_iso_begin: Vec3::splat(-1.0),
            selection_iso_end: Vec3::splat(-1.0),
            blend: None,
        };
        let ent = e.world.spawn((tm, Role::new(RoleKind::Tilemap)));
        e.names.insert("tilemap".into(), ent);
    }

    /// A 16-direction × 5-pitch × 3-roll (±20°) container table, frames
    /// named `box_{i}`.
    fn box_table(interp: bool, axes: Option<SpriteAxes>) -> FrameTable {
        let axes = axes.unwrap_or(SpriteAxes {
            directions: 16,
            pitch: Some(AngleAxis::symmetric(5, 20.0)),
            roll: Some(AngleAxis::symmetric(3, 20.0)),
        });
        let frames: serde_json::Map<String, serde_json::Value> = (0..axes.frame_count())
            .map(|i| (format!("box_{i}"), serde_json::json!({"sheet": 0, "rect": [0, 0, 8, 8]})))
            .collect();
        let mut t: FrameTable = serde_json::from_value(serde_json::json!({
            "version": 2, "dir_interp": interp, "axes": axes,
            "sheets": [{"name": "s", "src": "/res/s.png", "size": [64, 64]}],
            "frames": frames,
        }))
        .unwrap();
        t.precompute_companions();
        t
    }

    fn spawn_box(e: &mut Engine, interp: bool, at: Vec3) {
        e.frame_tables.insert("box".into(), box_table(interp, None));
        let sprite = IsoSprite {
            position: at,
            scale: Vec3::ONE,
            texture: "box".into(),
            tilemap: "tilemap".into(),
            frame: 0.0,
            frame_name: None,
            tile_set_size: Vec2::ONE,
            anchor: Vec2::new(0.5, 0.75),
            frame_offset: Vec3::ZERO,
            footprint: vec![],
            ghost_group: 0,
            color: [1.0; 4],
            dir_blend: None,
            no_ghost: false,
            team_tint: None,
            emissive: None,
        };
        let ent = e.world.spawn((sprite, Transform::new(at, Vec3::ONE)));
        e.names.insert("box".into(), ent);
    }

    fn sprite(e: &Engine) -> IsoSprite {
        IsoSprite::clone(&e.world.get::<&IsoSprite>(e.names["box"]).unwrap())
    }

    #[test]
    fn sprite_pose_frames_heading_and_tilt() {
        let mut e = Engine::new_for_test();
        spawn_box(&mut e, false, Vec3::new(4.0, 4.0, 0.0));
        // Heading 100° -> nearest of 16 is 4 (90°); pitch 10° -> level 3 of
        // 5 (-20, -10, 0, 10, 20); roll -20° -> level 0 of 3.
        assert!(e.sprite_pose("box", 100.0 * DEG, 10.0 * DEG, -20.0 * DEG));
        let s = sprite(&e);
        // (pitch 3 · 3 rolls + roll 0) · 16 dirs + dir 4.
        assert_eq!(s.frame, (9 * 16 + 4) as f32);
        assert_eq!(s.frame_name.as_deref(), Some("box_148"));
        assert!(s.dir_blend.is_none(), "no dir_interp: nearest direction");
        assert_eq!(s.anchor, Vec2::new(0.5, 0.75), "the sprite's own anchor");
        // Out-of-range tilt clamps to the end levels; heading wraps.
        assert!(e.sprite_pose("box", -22.5 * DEG, 45.0 * DEG, 45.0 * DEG));
        assert_eq!(sprite(&e).frame, ((4 * 3 + 2) * 16 + 15) as f32);
        // Level: the middle levels.
        assert!(e.sprite_pose("box", 0.0, 0.0, 0.0));
        assert_eq!(sprite(&e).frame, ((2 * 3 + 1) * 16) as f32);
        assert!(!e.sprite_pose("ghost", 0.0, 0.0, 0.0));
    }

    #[test]
    fn sprite_pose_blends_neighbours_within_one_tilt_frame() {
        let mut e = Engine::new_for_test();
        spawn_box(&mut e, true, Vec3::new(4.0, 4.0, 0.0));
        let step = 22.5 * DEG;
        // 30° = direction 1 + a third of a step, nose-down 20°, roll level.
        assert!(e.sprite_pose("box", 30.0 * DEG, -20.0 * DEG, 0.0));
        let s = sprite(&e);
        // (pitch 0 · 3 rolls + roll 1) · 16 dirs.
        let tilt = 16;
        assert_eq!(s.frame, (tilt + 1) as f32);
        let b = s.dir_blend.expect("dir_interp blends");
        assert_eq!(b.frame_b, (tilt + 2) as f32, "B shares A's tilt frame");
        assert!((b.dtheta_a - (30.0 * DEG - step)).abs() < 1e-5);
        assert!((b.dtheta_b - (30.0 * DEG - 2.0 * step)).abs() < 1e-5);
        assert_eq!(b.anchor_b, s.anchor);
        // Across the wrap: 350° blends direction 15 into 0, same tilt frame.
        assert!(e.sprite_pose("box", 350.0 * DEG, -20.0 * DEG, 0.0));
        let s = sprite(&e);
        assert_eq!(s.frame, (tilt + 15) as f32);
        assert_eq!(s.dir_blend.unwrap().frame_b, tilt as f32);
    }

    #[test]
    fn direction_only_tables_pose_by_frame_count() {
        let mut e = Engine::new_for_test();
        spawn_box(&mut e, false, Vec3::new(4.0, 4.0, 0.0));
        let mut t = box_table(false, Some(SpriteAxes::directions(16)));
        t.axes = None;
        assert_eq!(t.sprite_axes(), SpriteAxes::directions(16));
        e.frame_tables.insert("box".into(), t);
        // Tilt is ignored on a sheet without tilt axes.
        assert!(e.sprite_pose("box", 180.0 * DEG, 15.0 * DEG, 15.0 * DEG));
        assert_eq!(sprite(&e).frame, 8.0);
    }

    #[test]
    fn terrain_fit_tilts_to_a_slope_at_every_heading() {
        let mut e = Engine::new_for_test();
        // 0.5 m per tile toward East.
        let rise = 0.5;
        slope_tilemap(&mut e, 16, rise);
        spawn_box(&mut e, false, Vec3::new(8.0, 8.0, 0.0));
        let slope = (rise / TILE_M).atan();
        let span = 2.44;
        let cases = [
            // heading, pitch, roll
            (0.0, slope, 0.0),    // facing uphill: nose up
            (180.0, -slope, 0.0), // facing downhill
            (90.0, 0.0, slope),   // facing South: East (uphill) is left
            (270.0, 0.0, -slope), // facing North: East is right
            (
                45.0,
                (slope.tan() * 45f32.to_radians().cos()).atan(),
                (slope.tan() * 45f32.to_radians().sin()).atan(),
            ),
        ];
        for (hdg, want_p, want_r) in cases {
            let (p, r) = e.terrain_tilt(8.0, 8.0, hdg * DEG, span);
            assert!((p - want_p).abs() < 1e-4 && (r - want_r).abs() < 1e-4, "{hdg}: {p} {r}");
            let fit = e.sprite_fit_terrain("box", hdg * DEG, span).unwrap();
            assert_eq!(fit, (p, r));
            let axes = e.frame_tables["box"].sprite_axes();
            let (pi, ri) = axes.tilt_levels(p, r);
            let dir = classic_core::vehicle_def::heading_to_dir_n(hdg * DEG, 16);
            assert_eq!(sprite(&e).frame, axes.frame_index(dir, pi, ri) as f32, "{hdg}");
        }
        // The drawn tilt is quantized to the nearest 10° level; the altitude
        // keeps every footprint corner of that drawn tilt out of the ground.
        e.sprite_fit_terrain("box", 0.0, span).unwrap();
        let s = sprite(&e);
        let axes = e.frame_tables["box"].sprite_axes();
        let (pi, _) = axes.tilt_levels(slope, 0.0);
        let drawn = axes.tilt_angles(pi, 1).0;
        let half = span * 0.5;
        let ground = e.height_at(8.0, 8.0);
        for (sf, sl) in [(1.0f32, 1.0f32), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
            let corner = ground + s.frame_offset.z + sf * half * drawn.tan();
            let terrain = e.height_at(8.0 + sf * half / TILE_M, 8.0 - sl * half / TILE_M);
            assert!(corner >= terrain - 1e-4, "corner {sf},{sl} buried: {corner} < {terrain}");
        }
        assert!(s.frame_offset.z >= 0.0);
        // Flat ground: level frame, no lift.
        let mut flat = Engine::new_for_test();
        slope_tilemap(&mut flat, 16, 0.0);
        spawn_box(&mut flat, false, Vec3::new(8.0, 8.0, 0.0));
        assert_eq!(flat.sprite_fit_terrain("box", 0.3, span), Some((0.0, 0.0)));
        assert_eq!(sprite(&flat).frame_offset.z, 0.0);
    }

    #[test]
    fn tilt_interp_warps_the_nearest_level_to_the_exact_tilt() {
        let mut e = Engine::new_for_test();
        spawn_box(&mut e, true, Vec3::new(4.0, 4.0, 0.0));
        // 3 x 3 levels at +-20 (the prototype's pick).
        let three = Some(AngleAxis::symmetric(3, 20.0));
        let axes = SpriteAxes { directions: 16, pitch: three, roll: three };
        let mut t = box_table(true, Some(axes));
        t.tilt_interp = true;
        e.frame_tables.insert("box".into(), t);
        let (h, p, r) = (30.0 * DEG, 8.0 * DEG, -12.0 * DEG);
        assert!(e.sprite_pose("box", h, p, r));
        let s = sprite(&e);
        // Nearest levels: pitch 1 (0), roll 0 (-20); directions 1 and 2.
        assert_eq!(s.frame, (3 * 16 + 1) as f32);
        let b = s.dir_blend.unwrap();
        assert_eq!(b.frame_b, (3 * 16 + 2) as f32);
        let want = classic_core::vehicle_def::tilt_residual(h, p, r, 0.0, -20.0 * DEG);
        assert_eq!(b.tilt, want);
        assert!(b.tilt.iter().any(|v| v.abs() > 1e-3));
        // Both residuals (8°) pass a quarter step (5°): the diagonal level
        // (pitch 2 at 20°, roll 1 at 0°) is the third source, on the nearer
        // yaw neighbour (A: direction 1, a third of a step away).
        let th = b.third.expect("large residual: a third source");
        assert_eq!(th.frame, ((2 * 3 + 1) * 16 + 1) as f32);
        assert!(!th.from_b);
        assert_eq!(th.anchor, s.anchor);
        let want = classic_core::vehicle_def::tilt_residual(h, p, r, 20.0 * DEG, 0.0);
        assert_eq!(th.tilt, want);
        // Near a level: no third source.
        assert!(e.sprite_pose("box", 40.0 * DEG, 3.0 * DEG, -18.0 * DEG));
        let b = sprite(&e).dir_blend.unwrap();
        assert!(b.third.is_none());
        // Past the midpoint the third rides B's direction.
        assert!(e.sprite_pose("box", 40.0 * DEG, 8.0 * DEG, -18.0 * DEG));
        let th = sprite(&e).dir_blend.unwrap().third.unwrap();
        assert_eq!(th.frame, ((2 * 3) * 16 + 2) as f32);
        assert!(th.from_b);
        // Without the opt-in the same pose snaps (no residual, no third).
        e.frame_tables.get_mut("box").unwrap().tilt_interp = false;
        assert!(e.sprite_pose("box", h, p, r));
        let b = sprite(&e).dir_blend.unwrap();
        assert_eq!(b.tilt, [0.0; 3]);
        assert!(b.third.is_none());
    }

    #[test]
    fn terrain_fit_rests_on_a_raised_floor() {
        let mut e = Engine::new_for_test();
        slope_tilemap(&mut e, 16, 0.0);
        spawn_box(&mut e, false, Vec3::new(8.0, 8.0, 0.0));
        // A 0.5 m deck under the whole footprint: level, lifted 0.5 m over the
        // terrain (the sprite's altitude reference).
        assert!(e.set_floor_rect(7, 6.0, 6.0, 10.0, 10.0, 0.5));
        assert_eq!(e.sprite_fit_terrain("box", 0.0, 2.44), Some((0.0, 0.0)));
        assert!((sprite(&e).frame_offset.z - 0.5).abs() < 1e-5);
        // The deck edge under its front half: it tilts nose-up onto it and
        // rests with its lowest corner on the ground.
        assert!(e.set_floor_rect(7, 8.0, 6.0, 10.0, 10.0, 0.2));
        let (p, r) = e.sprite_fit_terrain("box", 0.0, 2.44).unwrap();
        assert!(p > 0.05 && r.abs() < 1e-5, "{p} {r}");
        // Cleared: back on the terrain.
        assert!(e.clear_floor_rect(7));
        assert_eq!(e.sprite_fit_terrain("box", 0.0, 2.44), Some((0.0, 0.0)));
        assert_eq!(sprite(&e).frame_offset.z, 0.0);
    }

    #[test]
    fn terrain_fit_rests_on_the_exact_tilt_when_warping() {
        let mut e = Engine::new_for_test();
        let rise = 0.1;
        slope_tilemap(&mut e, 16, rise);
        spawn_box(&mut e, true, Vec3::new(8.0, 8.0, 0.0));
        e.frame_tables.get_mut("box").unwrap().tilt_interp = true;
        let span = 2.6;
        let (p, r) = e.sprite_fit_terrain("box", 0.0, span).unwrap();
        assert!((p - (rise / TILE_M).atan()).abs() < 1e-4 && r.abs() < 1e-4);
        // A planar slope drawn at its exact tilt needs no lift.
        assert!(sprite(&e).frame_offset.z.abs() < 1e-4, "{}", sprite(&e).frame_offset.z);
        assert!(sprite(&e).dir_blend.unwrap().tilt.iter().any(|v| v.abs() > 1e-3));
    }
}
