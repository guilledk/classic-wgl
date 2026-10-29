//! Read-only sprite framing for tools: what an entity's sprite sheet draws,
//! and what it would draw at another pose, without touching the entity.
//! classic-demo's inspector draws its sprite preview from these.
//!
//! - [`Engine::sprite_frame`] is the public face of the renderer's packed
//!   frame lookup (`resolve_frame` + `packed_geometry`): the sheet texture
//!   to bind (a KTX2 / Basis sheet binds under the same name), the
//!   [`SpriteRegion`] to draw it with, and the source-cell quad, trimmed
//!   content and anchor in sheet pixels.  [`Engine::iso_sprite_frame`] adds
//!   the uniform-grid fallback the iso pass takes without a frame table.
//! - [`Engine::pose_axes`] / [`Engine::current_pose`] / [`Engine::posed_parts`]
//!   step an entity through its sheet's pose axes (sheet v2: directions ×
//!   optional pitch × roll).  Frames come from the engine's own selection —
//!   [`Engine::sprite_pose`]'s framing for a posed sprite, the per-layer
//!   framing of `apply_layers` for every layer of a v2 vehicle (stacked as
//!   the game places them), the simulation's body frame for a v1 vehicle —
//!   on scratch copies of the live sprites.

use classic_core::components::{IsoSprite, IsoVehicle, SpriteRender};
use classic_core::math::{iso_camera_matrix, iso_view_depth, iso_world_pos};
use classic_core::tilemap::PPM_TARGET;
use classic_core::vehicle_def::{heading_to_dir_n, pick_tilt, AngleAxis, LayerKind, SpriteAxes};
use classic_gfx::SpriteRegion;
use glam::{Vec2, Vec3};

use crate::dir_blend::packed_geometry;
use crate::Engine;

/// How far past a direction's centre a stepped pose's heading sits, in
/// direction steps: enough that the blend's neighbour search lands on the
/// stepped direction (not the one before it) under float rounding, far too
/// small to change the frame.
const DIR_BIAS: f32 = 1e-3;

/// An owned [`SpriteRegion`]: where a frame sits in its texture.
#[derive(Clone, Debug, PartialEq)]
pub enum FrameRegion {
    /// A uniform-grid cell (no frame table).
    Grid { frame: f32, tile_set_size: [f32; 2] },
    /// A packed-atlas frame: UV rect plus trim metadata (sheet pixels).
    Uv { uv_rect: [f32; 4], trim_offset: [f32; 2], source_size: [f32; 2], content_size: [f32; 2] },
}

impl FrameRegion {
    /// The region as `Gfx::draw_sprite` takes it.
    pub fn as_region(&self) -> SpriteRegion<'_> {
        match self {
            Self::Grid { frame, tile_set_size } => {
                SpriteRegion::Grid { frame: *frame, tile_set_size: *tile_set_size }
            }
            Self::Uv { uv_rect, trim_offset, source_size, content_size } => {
                SpriteRegion::Uv { uv_rect, trim_offset, source_size, content_size }
            }
        }
    }
}

/// One drawable sprite frame.  The quad `Gfx::draw_sprite` draws is the
/// source cell (`cell`, sheet pixels); the trimmed content sits at
/// `content_min` in it, and the ground anchor at `anchor_px`.
#[derive(Clone, Debug, PartialEq)]
pub struct SpriteFrame {
    /// The GL texture to bind: the frame's sheet, or the grid texture.
    pub texture: String,
    /// The packed frame's name (`None` on the grid path).
    pub frame_name: Option<String>,
    /// The flat frame index the sprite carries.
    pub frame: f32,
    pub region: FrameRegion,
    pub cell: Vec2,
    pub content_min: Vec2,
    pub content_size: Vec2,
    pub anchor_px: Vec2,
}

/// A pose on a sheet's [`SpriteAxes`]: direction and tilt level indices.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Pose {
    pub dir: u32,
    pub pitch: u32,
    pub roll: u32,
}

impl Pose {
    /// This pose on `axes` as the continuous drivers the engine frames from:
    /// tile heading (radians, just past the direction's centre) and the
    /// levels' baked `(pitch, roll)` angles.
    pub fn angles(self, axes: &SpriteAxes) -> (f32, f32, f32) {
        let dirs = axes.directions.max(1);
        let step = std::f32::consts::TAU / dirs as f32;
        let heading = ((self.dir % dirs) as f32 + DIR_BIAS) * step;
        let (pitch, roll) = axes.tilt_angles(self.pitch, self.roll);
        (heading, pitch, roll)
    }
}

/// One sprite of a posed entity: its entity name, frame, the screen offset
/// of its anchor from the body's anchor (pixels at camera zoom 1, `y` down)
/// and its view depth (larger is nearer the camera: draw ascending).
#[derive(Clone, Debug, PartialEq)]
pub struct PosePart {
    pub entity: String,
    pub frame: SpriteFrame,
    pub offset: Vec2,
    pub depth: f32,
}

/// A body-relative offset (tile `x`, `y`; metres `z`) as screen pixels at
/// zoom 1 and view depth.
fn project_offset(off: Vec3) -> (Vec2, f32) {
    let world = iso_world_pos(off.x, off.y, off.z);
    let v = iso_camera_matrix().transform_vector3(world);
    (Vec2::new(v.x * PPM_TARGET, -v.y * PPM_TARGET), iso_view_depth(world))
}

/// A v1 vehicle's body axes: its directions × symmetric pitch × roll levels.
fn v1_axes(v: &IsoVehicle) -> SpriteAxes {
    let axis = |levels: u32, max: f32| {
        (levels > 1).then(|| AngleAxis::symmetric(levels, max.to_degrees()))
    };
    SpriteAxes {
        directions: v.directions.max(1),
        pitch: axis(v.pitch_levels, v.pitch_max),
        roll: axis(v.roll_levels, v.roll_max),
    }
}

impl Engine {
    /// The packed frame `frame_name` of `texture` (through its frame table)
    /// with the sprite anchor `anchor` (a `[0, 1]` ratio of the source cell;
    /// a packer anchor wins), as the iso pass draws it.  `None` without a
    /// frame table or for an unknown name.
    pub fn sprite_frame(
        &self,
        texture: &str,
        frame_name: &str,
        anchor: Vec2,
    ) -> Option<SpriteFrame> {
        let fr = Self::resolve_frame(&self.frame_tables, texture, frame_name)?;
        let ((sw, sh), anchor_px, (uv_rect, trim_offset, source_size, content_size)) =
            packed_geometry(&fr, anchor);
        Some(SpriteFrame {
            texture: fr.sheet_name.clone(),
            frame_name: Some(frame_name.to_string()),
            frame: 0.0,
            region: FrameRegion::Uv { uv_rect, trim_offset, source_size, content_size },
            cell: Vec2::new(sw, sh),
            content_min: Vec2::from(trim_offset),
            content_size: Vec2::from(content_size),
            anchor_px,
        })
    }

    /// A uniform-grid cell of a loaded texture (`tile_set_size` cells across
    /// × down), anchored at the ratio `anchor`.  `None` until the texture is
    /// uploaded.
    fn grid_frame(
        &self,
        texture: &str,
        frame: f32,
        tile_set_size: Vec2,
        anchor: Vec2,
    ) -> Option<SpriteFrame> {
        let (w, h) = self.texture_size(texture)?;
        let ts = tile_set_size.max(Vec2::splat(0.001));
        let cell = Vec2::new(w as f32 / ts.x, h as f32 / ts.y);
        Some(SpriteFrame {
            texture: texture.to_string(),
            frame_name: None,
            frame,
            region: FrameRegion::Grid { frame, tile_set_size: ts.to_array() },
            cell,
            content_min: Vec2::ZERO,
            content_size: cell,
            anchor_px: cell * anchor,
        })
    }

    /// The frame an `IsoSprite` value draws (direction A of a blend): its
    /// packed `frame_name`, else its grid cell.
    pub fn iso_sprite_frame(&self, s: &IsoSprite) -> Option<SpriteFrame> {
        let packed =
            s.frame_name.as_deref().and_then(|n| self.sprite_frame(&s.texture, n, s.anchor));
        match packed {
            Some(f) => Some(SpriteFrame { frame: s.frame, ..f }),
            None => self.grid_frame(&s.texture, s.frame, s.tile_set_size, s.anchor),
        }
    }

    /// The frame a screen-space `SpriteRender` draws, anchored at its centre.
    pub fn sprite_render_frame(&self, s: &SpriteRender) -> Option<SpriteFrame> {
        let centre = Vec2::splat(0.5);
        let packed = s.frame_name.as_deref().and_then(|n| self.sprite_frame(&s.texture, n, centre));
        match packed {
            Some(f) => Some(SpriteFrame { frame: s.frame, ..f }),
            None => self.grid_frame(&s.texture, s.frame, s.tile_set_size, centre),
        }
    }

    /// The pose axes entity `name` steps through: a v2 vehicle's hull
    /// (directions × hull pitch × roll), a v1 vehicle's body sheet, or a
    /// posed `IsoSprite`'s declared `FrameTable::axes`.  `None` for anything
    /// else (it draws one fixed frame).
    pub fn pose_axes(&self, name: &str) -> Option<SpriteAxes> {
        let &e = self.names.get(name)?;
        if let Ok(v) = self.world.get::<&IsoVehicle>(e) {
            if v.layers.is_empty() {
                return Some(v1_axes(&v));
            }
            let def = self.vehicles.get(&v.def_name)?;
            let hull = def.layers.iter().find(|l| l.kind == LayerKind::Hull)?;
            let directions = hull.dirs(def.directions);
            return Some(SpriteAxes { directions, pitch: hull.pitch, roll: hull.roll });
        }
        let s = self.world.get::<&IsoSprite>(e).ok()?;
        self.frame_tables.get(&s.texture)?.axes
    }

    /// The pose entity `name` draws now on its [`Engine::pose_axes`]: a
    /// vehicle's quantized heading and the tilt levels its body picks, a
    /// posed sprite's current frame read back through its axes.
    pub fn current_pose(&self, name: &str) -> Option<Pose> {
        let axes = self.pose_axes(name)?;
        let &e = self.names.get(name)?;
        if let Ok(v) = self.world.get::<&IsoVehicle>(e) {
            if v.layers.is_empty() {
                return Some(Pose { dir: v.direction, pitch: v.pitch_index, roll: v.roll_index });
            }
            let def = self.vehicles.get(&v.def_name)?;
            let hull = def.layers.iter().find(|l| l.kind == LayerKind::Hull)?;
            let interp = self.frame_tables.get(&hull.texture).is_some_and(|t| t.tilt_interp);
            let pick = pick_tilt(hull.pitch, hull.roll, v.heading, v.pitch, v.roll, interp);
            let dir = heading_to_dir_n(v.heading, axes.directions);
            return Some(Pose { dir, pitch: pick.pitch, roll: pick.roll });
        }
        let s = self.world.get::<&IsoSprite>(e).ok()?;
        let (dir, pitch, roll) = axes.pose_of(s.frame.max(0.0) as u32);
        Some(Pose { dir, pitch, roll })
    }

    /// The sprites entity `name` draws at `pose` (`None`: as drawn now),
    /// framed by the engine's own selection on scratch copies (the entity
    /// is untouched): every visible layer of a v2 vehicle placed as the game
    /// places it, a v1 vehicle's body (its wheels are not composed), a posed
    /// sprite, or the one frame of an unposed sprite / `SpriteRender`.
    /// Ordered as stored; sort by [`PosePart::depth`] to draw.
    pub fn posed_parts(&self, name: &str, pose: Option<Pose>) -> Vec<PosePart> {
        let Some(&e) = self.names.get(name) else { return Vec::new() };
        let one = |frame: Option<SpriteFrame>| {
            frame
                .map(|frame| PosePart {
                    entity: name.to_string(),
                    frame,
                    offset: Vec2::ZERO,
                    depth: 0.0,
                })
                .into_iter()
                .collect::<Vec<_>>()
        };
        let posed = pose.and_then(|p| Some((p, self.pose_axes(name)?)));
        let Some((pose, axes)) = posed else {
            if let Ok(s) = self.world.get::<&IsoSprite>(e) {
                return one(self.iso_sprite_frame(&s));
            }
            if let Ok(s) = self.world.get::<&SpriteRender>(e) {
                return one(self.sprite_render_frame(&s));
            }
            return Vec::new();
        };
        let (heading, pitch, roll) = pose.angles(&axes);
        let layered = self.world.get::<&IsoVehicle>(e).is_ok_and(|v| !v.layers.is_empty());
        if layered {
            return self
                .layered_pose(e, heading, pitch, roll)
                .into_iter()
                .filter_map(|p| {
                    let frame = self.iso_sprite_frame(&p.sprite)?;
                    let (offset, depth) = project_offset(p.offset);
                    Some(PosePart { entity: p.entity, frame, offset, depth })
                })
                .collect();
        }
        if let Some(s) = self.v1_body_pose(e, pose.dir, pose.pitch, pose.roll) {
            return one(self.iso_sprite_frame(&s));
        }
        let Ok(live) = self.world.get::<&IsoSprite>(e) else { return Vec::new() };
        let mut s = IsoSprite::clone(&live);
        if !Self::pose_sprite(&self.frame_tables, &mut s, heading, pitch, roll) {
            return Vec::new();
        }
        one(self.iso_sprite_frame(&s))
    }
}

#[cfg(test)]
#[path = "sprite_preview_tests.rs"]
mod tests;
