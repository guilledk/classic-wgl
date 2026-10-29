//! The inspector's sprite preview: the selected entity's sprite drawn into
//! the detail panel's preview box ([`InspectorState::preview_rect`]), with
//! pose steppers when its sheet has pose axes (sheet v2: directions ×
//! optional pitch × roll).
//!
//! The frames are the engine's own: `Engine::posed_parts` frames a scratch
//! copy of the entity through the same selection `Engine::sprite_pose` and
//! the vehicle layer framing run, so the preview never hand-computes a frame
//! index.  A vehicle's steppers drive its body pose and every visible layer
//! is stacked as the game places it (painter-sorted by anchor depth; the
//! game resolves layer overlap per pixel through the depth maps).  A v1
//! vehicle shows its body only.  The pose starts as the one the entity draws
//! when selected.
//!
//! Steppers are buttons only: the keys free while the inspector is open
//! cannot be told apart from a guest's bindings, so none are taken.

use classic_core::components::{IsoVehicle, Model};
use classic_core::vehicle_def::SpriteAxes;
use classic_engine::{Engine, Pose, PosePart};
use classic_gfx::RenderSettings;
use glam::{Mat4, Vec2, Vec3};

use super::tree::{DIM, GROUP, WHITE};
use super::{Rect, Row, PAD, ROW_H};
use crate::state::DemoStateRef;

/// Width of the stepper column at the box's left edge.
pub const STEP_W: f32 = 132.0;
/// Width of a stepper's `[-]` / `[+]` hit zone.
pub const BUTTON_W: f32 = 26.0;
/// Upscale cap: a small sprite is magnified at most this much.
const MAX_SCALE: f32 = 3.0;

/// One pose axis a stepper drives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PoseAxis {
    Dir,
    Pitch,
    Roll,
}

impl PoseAxis {
    pub fn label(self) -> &'static str {
        match self {
            Self::Dir => "dir",
            Self::Pitch => "pitch",
            Self::Roll => "roll",
        }
    }
}

/// The steps along `axis` on `axes` (direction count or tilt levels).
pub fn axis_len(axes: &SpriteAxes, axis: PoseAxis) -> u32 {
    match axis {
        PoseAxis::Dir => axes.directions.max(1),
        PoseAxis::Pitch => axes.pitch.map_or(1, |a| a.count()),
        PoseAxis::Roll => axes.roll.map_or(1, |a| a.count()),
    }
}

/// The axes a sheet steps along: direction always, pitch / roll when the
/// sheet bakes more than one level of them.
pub fn stepper_axes(axes: &SpriteAxes) -> Vec<PoseAxis> {
    [PoseAxis::Dir, PoseAxis::Pitch, PoseAxis::Roll]
        .into_iter()
        .filter(|&a| a == PoseAxis::Dir || axis_len(axes, a) > 1)
        .collect()
}

/// Step `pose` by `delta` along `axis`: the direction wraps round the
/// sheet's directions, the tilt levels clamp to the sheet's range.
pub fn step(axes: &SpriteAxes, pose: Pose, axis: PoseAxis, delta: i32) -> Pose {
    let n = axis_len(axes, axis) as i64;
    let mut p = pose;
    match axis {
        PoseAxis::Dir => p.dir = (p.dir as i64 + delta as i64).rem_euclid(n) as u32,
        PoseAxis::Pitch => p.pitch = (p.pitch as i64 + delta as i64).clamp(0, n - 1) as u32,
        PoseAxis::Roll => p.roll = (p.roll as i64 + delta as i64).clamp(0, n - 1) as u32,
    }
    p
}

fn pose_value(pose: Pose, axis: PoseAxis) -> u32 {
    match axis {
        PoseAxis::Dir => pose.dir,
        PoseAxis::Pitch => pose.pitch,
        PoseAxis::Roll => pose.roll,
    }
}

/// The preview's per-selection state, held in `InspectorState`.
#[derive(Debug, Default)]
pub struct PreviewState {
    entity: Option<String>,
    axes: Option<SpriteAxes>,
    pose: Option<Pose>,
}

impl PreviewState {
    /// Follow the selection: a newly selected entity starts at the pose it
    /// draws now.  `true` when the selection changed.
    pub fn follow(&mut self, engine: &Engine, selected: Option<&str>) -> bool {
        if self.entity.as_deref() == selected {
            return false;
        }
        self.entity = selected.map(str::to_string);
        self.axes = selected.and_then(|n| engine.pose_axes(n));
        self.pose = self.axes.and(selected.and_then(|n| engine.current_pose(n)));
        true
    }

    /// The axes being stepped (`None`: the entity draws a fixed frame).
    pub fn axes(&self) -> Option<SpriteAxes> {
        self.axes
    }

    pub fn pose(&self) -> Option<Pose> {
        self.pose
    }

    /// Step the pose along `axis`; `false` without axes.
    pub fn step(&mut self, axis: PoseAxis, delta: i32) -> bool {
        let (Some(axes), Some(pose)) = (self.axes, self.pose) else { return false };
        self.pose = Some(step(&axes, pose, axis, delta));
        true
    }

    /// The stepper rows' text, in [`stepper_axes`] order.
    pub fn stepper_text(&self) -> Vec<(PoseAxis, String)> {
        let (Some(axes), Some(pose)) = (self.axes, self.pose) else { return Vec::new() };
        stepper_axes(&axes)
            .into_iter()
            .map(|a| {
                (a, format!("{:<5} {}/{}", a.label(), pose_value(pose, a), axis_len(&axes, a)))
            })
            .collect()
    }
}

/// Where the preview box's pieces sit.
#[derive(Clone, Debug, PartialEq)]
pub struct BoxLayout {
    /// The frame-name line along the top.
    pub label: Vec2,
    /// Per stepper: its axis, `[-]` zone, value text origin and `[+]` zone.
    pub steppers: Vec<(PoseAxis, Rect, Vec2, Rect)>,
    /// The area the sprite is fitted into.
    pub sprite: Rect,
}

/// Lay the preview box `r` out for `axes` steppers (none: the sprite takes
/// the whole box under the label).
pub fn box_layout(r: Rect, axes: &[PoseAxis]) -> BoxLayout {
    let label = Vec2::new(r.x + PAD, r.y + PAD);
    let top = r.y + PAD + ROW_H;
    let steppers = axes
        .iter()
        .enumerate()
        .map(|(i, &a)| {
            let y = top + PAD + i as f32 * (ROW_H + PAD);
            let minus = Rect { x: r.x + PAD, y, w: BUTTON_W, h: ROW_H };
            let plus = Rect { x: r.x + PAD + STEP_W - BUTTON_W, y, w: BUTTON_W, h: ROW_H };
            (a, minus, Vec2::new(minus.x + BUTTON_W + 4.0, y), plus)
        })
        .collect();
    let left = if axes.is_empty() { r.x + PAD } else { r.x + 2.0 * PAD + STEP_W };
    let sprite = Rect {
        x: left,
        y: top,
        w: (r.x + r.w - PAD - left).max(0.0),
        h: (r.y + r.h - PAD - top).max(0.0),
    };
    BoxLayout { label, steppers, sprite }
}

/// The stepper click at `p` in `layout`: its axis and `-1` / `+1`.
pub fn hit(layout: &BoxLayout, p: Vec2) -> Option<(PoseAxis, i32)> {
    layout.steppers.iter().find_map(|&(a, minus, _, plus)| {
        if minus.contains(p) {
            Some((a, -1))
        } else if plus.contains(p) {
            Some((a, 1))
        } else {
            None
        }
    })
}

/// Fit `parts` (body-anchor-relative, zoom-1 pixels) into `area`: the
/// scale and the screen position of the body's anchor, centring the union
/// of the parts' trimmed content.  `None` for nothing to draw.
pub fn fit(parts: &[PosePart], area: Rect) -> Option<(f32, Vec2)> {
    let mut lo = Vec2::splat(f32::MAX);
    let mut hi = Vec2::splat(f32::MIN);
    for p in parts {
        let min = p.offset - p.frame.anchor_px + p.frame.content_min;
        lo = lo.min(min);
        hi = hi.max(min + p.frame.content_size);
    }
    let size = hi - lo;
    if parts.is_empty() || size.x <= 0.0 || size.y <= 0.0 || area.w <= 0.0 || area.h <= 0.0 {
        return None;
    }
    let scale = (area.w / size.x).min(area.h / size.y).min(MAX_SCALE);
    let centre = Vec2::new(area.x + area.w * 0.5, area.y + area.h * 0.5);
    Some((scale, centre - (lo + hi) * 0.5 * scale))
}

/// The parts the selected entity draws at the preview pose, back to front.
fn parts(engine: &Engine, name: &str, st: &PreviewState) -> Vec<PosePart> {
    let mut parts = engine.posed_parts(name, st.pose);
    parts.sort_by(|a, b| a.depth.total_cmp(&b.depth));
    parts
}

/// The top line of the box: the body's frame, or why nothing draws.
pub fn label(engine: &Engine, name: &str, st: &PreviewState) -> String {
    let parts = engine.posed_parts(name, st.pose);
    let Some(body) = parts.iter().find(|p| p.entity == name).or(parts.first()) else {
        return "(no frame to draw)".into();
    };
    let frame =
        body.frame.frame_name.clone().unwrap_or_else(|| format!("grid #{}", body.frame.frame));
    let resident = parts.iter().all(|p| engine.texture_size(&p.frame.texture).is_some());
    let more =
        if parts.len() > 1 { format!("  +{} layers", parts.len() - 1) } else { String::new() };
    format!("{frame}{more}{}", if resident { "" } else { "  (sheet not resident)" })
}

/// The detail rows under the preview box: the pose and each part's frame,
/// or a note for a `Model` (no 3D preview yet).
pub fn rows(engine: &Engine, name: &str, st: &PreviewState) -> Vec<Row> {
    let row = |text: String, color| Row { text, color, action: None };
    let Some(&e) = engine.names.get(name) else { return Vec::new() };
    if engine.world.get::<&Model>(e).is_ok() {
        return vec![
            row("Preview".into(), GROUP),
            row("  3D preview deferred (glTF Model turntable)".into(), DIM),
        ];
    }
    let parts = engine.posed_parts(name, st.pose);
    if parts.is_empty() {
        return Vec::new();
    }
    let mut out = vec![row("Preview".into(), GROUP)];
    match (st.axes, st.pose) {
        (Some(axes), Some(p)) => out.push(row(
            format!(
                "pose         dir {}/{}  pitch {}/{}  roll {}/{}",
                p.dir,
                axes.directions.max(1),
                p.pitch,
                axis_len(&axes, PoseAxis::Pitch),
                p.roll,
                axis_len(&axes, PoseAxis::Roll)
            ),
            WHITE,
        )),
        _ => out.push(row("pose         (fixed frame: no pose axes)".into(), DIM)),
    }
    for p in &parts {
        let frame =
            p.frame.frame_name.clone().unwrap_or_else(|| format!("grid #{}", p.frame.frame));
        out.push(row(format!("  {:<18} {frame}  on {}", p.entity, p.frame.texture), WHITE));
    }
    let v1 = engine.world.get::<&IsoVehicle>(e).is_ok_and(|v| v.layers.is_empty());
    if v1 {
        out.push(row("  v1 vehicle: body only (wheels and tires not composed)".into(), DIM));
    }
    out
}

/// Overlay hook: draw the selected entity's sprite into the preview box.
pub fn draw(engine: &mut Engine, state: &DemoStateRef) {
    let (layout, parts) = {
        let s = state.borrow();
        let st = &s.inspector;
        let (Some(r), Some(name)) = (st.preview_rect(), st.selected_entity()) else { return };
        let axes = st.preview.axes().map(|a| stepper_axes(&a)).unwrap_or_default();
        (box_layout(r, &axes), parts(engine, name, &st.preview))
    };
    let Some((scale, origin)) = fit(&parts, layout.sprite) else { return };
    let Some(gfx) = engine.gfx.as_mut() else { return };
    let settings = RenderSettings {
        ambient: [1.0; 3],
        light_dir: [0.0, 0.0, 1.0],
        light_color: [0.0; 3],
        depth_span: [0.0, 1.0],
        ppm: classic_core::tilemap::PPM_TARGET,
        shadow: None,
    };
    for p in &parts {
        let tl = origin + (p.offset - p.frame.anchor_px) * scale;
        let size = p.frame.cell * scale;
        let model = Mat4::from_translation(Vec3::new(tl.x, tl.y, 0.0))
            * Mat4::from_scale(Vec3::new(size.x, size.y, 1.0));
        let region = p.frame.region.as_region();
        gfx.draw_sprite(&model, &Mat4::IDENTITY, &p.frame.texture, region, true, 1.0, &settings);
    }
}
