//! Vehicle definition v2 and sprite-sheet v2 contracts.
//!
//! This module is the wire contract between the content pipeline
//! (classic-kitbash / classic-assets / classic-roms) and the engine for
//! multi-layer sprite units.  It is pure data plus the pure functions that pin
//! its meaning (frame indexing, angle quantization, depth and team-mask
//! decoding); nothing here touches the ECS or GL.
//!
//! # Vehicle definition v2
//!
//! A [`VehicleDef`](crate::types::VehicleDef) with `version: 2` (or any
//! non-empty `layers`) describes the unit as a list of [`VehicleLayerDef`]s.
//! A v1 def (8 directions, exactly 5 `parts` plus optional steering `tires`)
//! keeps loading unchanged, and [`legacy_layers`] maps it onto the same layer
//! model so later code has one shape to consume.
//!
//! ## Directions
//!
//! `directions` is 8, 16 or 24 ([`SUPPORTED_DIRECTIONS`]).  Direction `k` is
//! the tile-space heading `k · 2π / N` (`atan2(dy, dx)`): `0` = East (+x
//! tile), increasing toward +y tile (South), exactly the v1 8-way order.  A
//! layer may carry its own `directions` (a turret can be baked at 32 while
//! the hull is at 16); it defaults to the vehicle's.
//!
//! ## Frame index
//!
//! Every layer's frames are named `{texture}_{index}` in its frame table (the
//! v1 naming), and `index` is a mixed-radix number over the layer's axes,
//! minor to major ([`FrameAxis::ORDER`]):
//!
//! `dir`, `spin`, `fire`, `roll`, `steer`, `pitch`, `clip`
//!
//! An axis the layer does not have has count 1 and drops out.  This keeps
//! the v1 layouts bit-for-bit: the v1 body is `(pitch · roll_levels + roll) ·
//! dirs + dir` and a v1 steering tire is `steer · dirs + dir`.  See
//! [`VehicleLayerDef::frame_index`].
//!
//! ## Angle axes
//!
//! `pitch`, `roll` and `steer` are [`AngleAxis`]es: `levels` frames spanning
//! `[min_deg, max_deg]` (`min_deg` defaults to `-max_deg`), index 0 at
//! `min_deg`.  Signs follow v1: pitch nose-up positive, roll left-up
//! positive, steer turn-left positive.  A turret's `pitch` is its gun
//! elevation.
//!
//! # Sprite sheet v2 (`frames.json`)
//!
//! Per sheet ([`SpriteSheetEntry`](crate::types::SpriteSheetEntry)):
//!
//! - `depth_mode: "anchor"` + `depth_range_m: R` — the depth companion is
//!   **anchor-relative**: gray `g` is a signed view-depth offset of
//!   `(g − 0.5) · 2R` metres from the frame's ground anchor, positive =
//!   farther from the camera (the global window's sign).  The legacy
//!   `"global"` mode (the default) keeps today's meaning: `g − 0.5` is a
//!   window-space offset over the engine's whole `DEPTH_NEAR..DEPTH_FAR`
//!   window (~1.7 m per 8-bit step).  [`depth_scale`] turns either into the
//!   window units the shader adds to the anchor depth.
//! - `team_mask: true` — the normal companion's **alpha** carries the team
//!   mask: weight `w = clamp((1 − a) · 2, 0, 1)` ([`team_mask_weight`]), so
//!   `a = 1` is no team colour and `a <= 0.5` is full team colour.  Never 0,
//!   so encoders that zero RGB under zero alpha cannot eat the normal.
//! - Emissive stays the `(0.5, 0.5, 0.5)` marker normal; its strength is a
//!   per-instance engine value, not sheet data.
//!
//! Per frame table: `dir_interp: true` opts the texture into the engine's
//! direction interpolation (two-neighbour backward warp).  It needs an
//! anchor-relative depth companion, and the bake must yaw the unit about its
//! ground anchor's vertical axis.  Its depth companion keeps gray 0 for transparent
//! texels (real depth uses 1..255): the warp reads it as background.

use std::collections::BTreeMap;

use crate::math::{DEPTH_FAR, DEPTH_NEAR};

/// The current vehicle definition version.
pub const VEHICLE_DEF_VERSION: u32 = 2;

/// Direction counts a vehicle or layer may be baked at.
pub const SUPPORTED_DIRECTIONS: [u32; 3] = [8, 16, 24];

/// Recommended anchor-relative depth range (metres, `±R`): 8-bit over ±3 m is
/// a 2.4 cm step, which the interpolation prototype measured at 28.6 dB.
pub const RECOMMENDED_DEPTH_RANGE_M: f32 = 3.0;

/// Default clip playback rate (frames per second).
pub const DEFAULT_CLIP_FPS: f32 = 12.0;

/// What a layer is, which fixes the frame axes it may carry and how the
/// engine drives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerKind {
    /// The chassis: `dir × pitch × roll`.  Exactly one per vehicle.
    Hull,
    /// A wheel or tire: `dir × spin`, optionally `× steer`.
    Wheel,
    /// One side's running gear (a track or a leg bank): `dir × spin`, with a
    /// `side`.
    GearSide,
    /// A turret with its own slewed yaw: own `dir × pitch (gun) × fire`.
    Turret,
    /// A tool (crane arm, drill, ...): `dir × clip`.
    Tool,
    /// Walker legs: `dir × clip`.
    Legs,
    /// Carried cargo: `dir × clip`.
    Cargo,
    /// A lift (fork, platform, ...): `dir × clip`.
    Lift,
}

impl LayerKind {
    /// Whether this kind may carry `axis`.  `dir` is always allowed.
    pub fn allows(self, axis: FrameAxis) -> bool {
        use FrameAxis::*;
        match self {
            LayerKind::Hull => matches!(axis, Dir | Pitch | Roll),
            LayerKind::Wheel => matches!(axis, Dir | Spin | Steer),
            LayerKind::GearSide => matches!(axis, Dir | Spin),
            LayerKind::Turret => matches!(axis, Dir | Pitch | Fire),
            LayerKind::Tool | LayerKind::Legs | LayerKind::Cargo | LayerKind::Lift => {
                matches!(axis, Dir | Clip)
            }
        }
    }
}

/// Which side a [`LayerKind::GearSide`] layer runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GearSide {
    Left,
    Right,
}

/// How a spawned vehicle's layers take part in the ghost pass.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GhostGroupMode {
    /// One ghost group per spawned vehicle, shared by all its layers: the
    /// parts never ghost through each other, but still ghost through terrain
    /// and other entities (the v1 behaviour).
    #[default]
    Instance,
    /// Group 0: every layer ghosts wherever it is occluded, even by its own
    /// vehicle's other layers.
    None,
}

/// One frame axis of a layer.  The declaration order is the mixed-radix
/// order of the frame index, minor to major.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FrameAxis {
    Dir,
    Spin,
    Fire,
    Roll,
    Steer,
    Pitch,
    Clip,
}

impl FrameAxis {
    /// Minor-to-major order of the frame index.
    pub const ORDER: [FrameAxis; 7] = [
        FrameAxis::Dir,
        FrameAxis::Spin,
        FrameAxis::Fire,
        FrameAxis::Roll,
        FrameAxis::Steer,
        FrameAxis::Pitch,
        FrameAxis::Clip,
    ];

    /// The axis name used in docs and validation messages.
    pub fn name(self) -> &'static str {
        match self {
            FrameAxis::Dir => "dir",
            FrameAxis::Spin => "spin",
            FrameAxis::Fire => "fire",
            FrameAxis::Roll => "roll",
            FrameAxis::Steer => "steer",
            FrameAxis::Pitch => "pitch",
            FrameAxis::Clip => "clip",
        }
    }
}

/// A quantized angle axis: `levels` frames spanning `[min_deg, max_deg]`,
/// index 0 at `min_deg`.
#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct AngleAxis {
    pub levels: u32,
    pub max_deg: f32,
    /// Lower bound; `None` = `-max_deg` (symmetric).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_deg: Option<f32>,
}

impl AngleAxis {
    /// A symmetric `±max_deg` axis.
    pub fn symmetric(levels: u32, max_deg: f32) -> Self {
        Self { levels, max_deg, min_deg: None }
    }

    /// The lower bound in degrees.
    pub fn min(&self) -> f32 {
        self.min_deg.unwrap_or(-self.max_deg)
    }

    /// Frame count along this axis (at least 1).
    pub fn count(&self) -> u32 {
        self.levels.max(1)
    }

    /// The nearest frame index for `angle_rad`, clamped to the axis.
    pub fn index(&self, angle_rad: f32) -> u32 {
        let levels = self.count();
        if levels <= 1 {
            return 0;
        }
        let lo = self.min().to_radians();
        let hi = self.max_deg.to_radians();
        let span = hi - lo;
        if span.abs() <= 1e-6 || !span.is_finite() || !angle_rad.is_finite() {
            return (levels - 1) / 2;
        }
        let t = ((angle_rad - lo) / span).clamp(0.0, 1.0);
        ((t * (levels - 1) as f32).round() as u32).min(levels - 1)
    }

    /// The angle (radians) frame `index` was baked at.
    pub fn angle(&self, index: u32) -> f32 {
        let levels = self.count();
        if levels <= 1 {
            return 0.0;
        }
        let t = index.min(levels - 1) as f32 / (levels - 1) as f32;
        (self.min() + t * (self.max_deg - self.min())).to_radians()
    }
}

/// A named clip in a layer's clip axis: frames `start .. start + frames`.
#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct ClipDef {
    pub start: u32,
    pub frames: u32,
    #[serde(default = "default_clip_fps")]
    pub fps: f32,
}

fn default_clip_fps() -> f32 {
    DEFAULT_CLIP_FPS
}

/// A named attachment point on a layer.
///
/// `offset` is metres in the layer's local frame: `x` forward (along the
/// layer's heading), `y` left, `z` up, origin at the layer's ground anchor.
/// A turret socket turns with the turret's yaw, then with its gun pitch about
/// the turret's local `y` axis through its `pitch_pivot` (default the
/// origin; offsets are authored at zero elevation); a hull socket turns with
/// the heading, pitch and roll.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct SocketDef {
    pub layer: String,
    #[serde(default)]
    pub offset: [f32; 3],
}

fn default_true() -> bool {
    true
}

fn is_true(v: &bool) -> bool {
    *v
}

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}

fn is_zero_f32(v: &f32) -> bool {
    *v == 0.0
}

/// One sprite layer of a v2 vehicle.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct VehicleLayerDef {
    /// Unique within the vehicle; keys the anchors artifact and sockets.
    pub name: String,
    pub kind: LayerKind,
    /// Texture whose frame table holds `{texture}_{index}` frames.
    pub texture: String,
    /// Own direction count; `None` = the vehicle's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directions: Option<u32>,
    /// The layer this one is placed relative to; `None` = the hull (or, for
    /// the hull, the vehicle's ground point).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Metres in the parent's frame (`x` forward, `y` left, `z` up).  `None`
    /// derives the placement from the anchors artifact, as v1 does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<[f32; 3]>,
    /// Required for [`LayerKind::GearSide`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side: Option<GearSide>,
    /// Hull body pitch, or a turret's gun pitch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pitch: Option<AngleAxis>,
    /// Hull body roll.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roll: Option<AngleAxis>,
    /// Wheel steering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steer: Option<AngleAxis>,
    /// Spin frames over one full revolution (or one track-tread cycle);
    /// `0`/`1` = no spin.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub spin_frames: u32,
    /// Rolling radius (metres): spin advances `distance / (2π · radius)`
    /// revolutions.  For a track, the tread cycle length is `2π · radius`.
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub radius_m: f32,
    /// Turret fire frames (0/1 = none); frame 0 is the idle pose.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub fire_frames: u32,
    /// Turret slew rate (degrees per second); `0` = instant.
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub yaw_rate_deg_per_sec: f32,
    /// Turret gun-pitch pivot (trunnion), metres in the turret frame (`x`
    /// forward, `y` left, `z` up); `None` = the layer origin.  Socket offsets
    /// on the turret are authored at zero elevation and pitch about it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pitch_pivot: Option<[f32; 3]>,
    /// Named clips over the clip axis (tool / legs / cargo / lift).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub clips: BTreeMap<String, ClipDef>,
    /// Whether the layer draws a ghost pass.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub ghost: bool,
    /// Initial visibility.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub visible: bool,
    /// Anchor-relative depth range override (metres); the sheet's own
    /// `depth_range_m` wins when it sets one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth_range_m: Option<f32>,
}

/// One frame selection: an index per axis (out-of-range values wrap for
/// `dir`/`spin`/`fire` and clamp for the rest).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameSelect {
    pub dir: u32,
    pub spin: u32,
    pub fire: u32,
    pub roll: u32,
    pub steer: u32,
    pub pitch: u32,
    /// Absolute frame on the clip axis (`clip.start + local frame`).
    pub clip: u32,
}

impl VehicleLayerDef {
    /// A bare layer with no axes but `dir`.
    pub fn new(name: &str, kind: LayerKind, texture: &str) -> Self {
        Self {
            name: name.to_string(),
            kind,
            texture: texture.to_string(),
            directions: None,
            parent: None,
            offset: None,
            side: None,
            pitch: None,
            roll: None,
            steer: None,
            spin_frames: 0,
            radius_m: 0.0,
            fire_frames: 0,
            yaw_rate_deg_per_sec: 0.0,
            pitch_pivot: None,
            clips: BTreeMap::new(),
            ghost: true,
            visible: true,
            depth_range_m: None,
        }
    }

    /// This layer's direction count, given the vehicle's.
    pub fn dirs(&self, vehicle_dirs: u32) -> u32 {
        self.directions.unwrap_or(vehicle_dirs).max(1)
    }

    /// Length of the clip axis: one past the last frame any clip uses.
    pub fn clip_frames(&self) -> u32 {
        self.clips.values().map(|c| c.start + c.frames).max().unwrap_or(0)
    }

    /// Frame count along `axis` (1 when absent).
    pub fn axis_count(&self, axis: FrameAxis, vehicle_dirs: u32) -> u32 {
        match axis {
            FrameAxis::Dir => self.dirs(vehicle_dirs),
            FrameAxis::Spin => self.spin_frames.max(1),
            FrameAxis::Fire => self.fire_frames.max(1),
            FrameAxis::Roll => self.roll.map_or(1, |a| a.count()),
            FrameAxis::Steer => self.steer.map_or(1, |a| a.count()),
            FrameAxis::Pitch => self.pitch.map_or(1, |a| a.count()),
            FrameAxis::Clip => self.clip_frames().max(1),
        }
    }

    /// Total frames the layer's sheet must hold.
    pub fn frame_count(&self, vehicle_dirs: u32) -> u32 {
        FrameAxis::ORDER.iter().map(|&a| self.axis_count(a, vehicle_dirs)).product()
    }

    /// The `{texture}_{index}` index of `sel` (mixed radix over
    /// [`FrameAxis::ORDER`]).
    pub fn frame_index(&self, vehicle_dirs: u32, sel: &FrameSelect) -> u32 {
        let mut index = 0u32;
        let mut stride = 1u32;
        for axis in FrameAxis::ORDER {
            let n = self.axis_count(axis, vehicle_dirs);
            let v = match axis {
                FrameAxis::Dir => sel.dir % n,
                FrameAxis::Spin => sel.spin % n,
                FrameAxis::Fire => sel.fire % n,
                FrameAxis::Roll => sel.roll.min(n - 1),
                FrameAxis::Steer => sel.steer.min(n - 1),
                FrameAxis::Pitch => sel.pitch.min(n - 1),
                FrameAxis::Clip => sel.clip.min(n - 1),
            };
            index += v * stride;
            stride *= n;
        }
        index
    }

    /// The axes present (count > 1, plus `dir`).
    pub fn axes(&self, vehicle_dirs: u32) -> Vec<FrameAxis> {
        FrameAxis::ORDER
            .iter()
            .copied()
            .filter(|&a| a == FrameAxis::Dir || self.axis_count(a, vehicle_dirs) > 1)
            .collect()
    }
}

/// Quantize a continuous tile-space heading (radians) to the nearest of `n`
/// directions.
pub fn heading_to_dir_n(heading: f32, n: u32) -> u32 {
    let n = n.max(1);
    if !heading.is_finite() {
        return 0;
    }
    let step = std::f32::consts::TAU / n as f32;
    (heading.rem_euclid(std::f32::consts::TAU) / step).round() as u32 % n
}

/// The two directions straddling `heading` and the blend weight toward the
/// second: `(a, b, t)` with `heading ≈ lerp(dir(a), dir(b), t)`, `b = a + 1
/// mod n`, `t ∈ [0, 1)`.
pub fn heading_neighbours(heading: f32, n: u32) -> (u32, u32, f32) {
    let n = n.max(1);
    if !heading.is_finite() {
        return (0, 1 % n, 0.0);
    }
    let f = heading.rem_euclid(std::f32::consts::TAU) / (std::f32::consts::TAU / n as f32);
    let a = f.floor();
    let t = f - a;
    let a = a as u32 % n;
    (a, (a + 1) % n, t)
}

/// Window-space depth units per unit of `(gray − 0.5)` for a depth sheet.
///
/// - `anchor_range_m: None` — the legacy global sheet: `1.0`, i.e. the gray
///   already spans the engine's whole depth window.
/// - `Some(R)` — an anchor-relative sheet over `±R` metres: `2R / (DEPTH_NEAR
///   − DEPTH_FAR)`.
///
/// The fragment depth is `depth_base + (gray − 0.5) · depth_scale`.
pub fn depth_scale(anchor_range_m: Option<f32>) -> f32 {
    match anchor_range_m {
        None => 1.0,
        Some(r) => 2.0 * r / (DEPTH_NEAR - DEPTH_FAR),
    }
}

/// Signed view-depth offset (metres, positive = farther) a gray value
/// encodes in an anchor-relative sheet over `±range_m`.
pub fn anchor_depth_offset_m(gray: f32, range_m: f32) -> f32 {
    (gray - 0.5) * 2.0 * range_m
}

/// The gray value that encodes a signed view-depth offset (metres, positive
/// = farther) in an anchor-relative sheet over `±range_m` (clamped to
/// `[0, 1]`).  The inverse of [`anchor_depth_offset_m`]; what the bake
/// writes.
pub fn anchor_depth_gray(offset_m: f32, range_m: f32) -> f32 {
    (offset_m / (2.0 * range_m.max(1e-6)) + 0.5).clamp(0.0, 1.0)
}

/// Team-colour weight encoded in a normal sheet's alpha: `clamp((1 − a) · 2,
/// 0, 1)`.  `a = 1` is no team colour, `a <= 0.5` full team colour.
pub fn team_mask_weight(alpha: f32) -> f32 {
    ((1.0 - alpha) * 2.0).clamp(0.0, 1.0)
}

/// The alpha a bake writes for team weight `w ∈ [0, 1]` (the inverse of
/// [`team_mask_weight`] on its non-saturated range).
pub fn team_mask_alpha(weight: f32) -> f32 {
    1.0 - 0.5 * weight.clamp(0.0, 1.0)
}

/// Synthesize v2 layers from a v1 def: the hull from `parts[0]` (with the
/// def's pitch/roll), the four wheels from `parts[1..5]`, and the steering
/// tires from `tires`, each parented to its wheel.
pub fn legacy_layers(def: &crate::types::VehicleDef) -> Vec<VehicleLayerDef> {
    let mut layers = Vec::with_capacity(def.parts.len() + def.tires.len());
    for (i, part) in def.parts.iter().enumerate() {
        if i == 0 {
            let mut hull = VehicleLayerDef::new(&part.name, LayerKind::Hull, &part.texture);
            if def.pitch_levels > 1 {
                hull.pitch = Some(AngleAxis::symmetric(def.pitch_levels, def.pitch_max_deg));
            }
            if def.roll_levels > 1 {
                hull.roll = Some(AngleAxis::symmetric(def.roll_levels, def.roll_max_deg));
            }
            layers.push(hull);
        } else {
            layers.push(VehicleLayerDef::new(&part.name, LayerKind::Wheel, &part.texture));
        }
    }
    for (i, tire) in def.tires.iter().enumerate() {
        let mut layer = VehicleLayerDef::new(&tire.name, LayerKind::Wheel, &tire.texture);
        if def.steer_levels > 1 {
            layer.steer = Some(AngleAxis::symmetric(def.steer_levels, def.steer_max_deg));
        }
        layer.parent = def.parts.get(i + 1).map(|p| p.name.clone());
        layer.offset = Some([0.0; 3]);
        layers.push(layer);
    }
    layers
}

/// Check a def against the v1 or v2 contract, collecting every problem.
pub fn validate(def: &crate::types::VehicleDef) -> Result<(), Vec<String>> {
    let mut errs = Vec::new();
    if !SUPPORTED_DIRECTIONS.contains(&def.directions) {
        errs.push(format!("directions {} not in {SUPPORTED_DIRECTIONS:?}", def.directions));
    }
    if def.version > VEHICLE_DEF_VERSION {
        errs.push(format!(
            "version {} is newer than this engine supports ({VEHICLE_DEF_VERSION})",
            def.version
        ));
    }
    if def.px_per_m < 0.0 || !def.px_per_m.is_finite() {
        errs.push(format!("px_per_m {} must be a finite value >= 0", def.px_per_m));
    }
    if let Some(r) = def.depth_range_m {
        if !(r > 0.0 && r.is_finite()) {
            errs.push(format!("depth_range_m {r} must be > 0"));
        }
    }
    if !def.is_v2() {
        if def.parts.len() != 5 {
            errs.push(format!("v1 def needs exactly 5 parts, has {}", def.parts.len()));
        }
        if def.directions != 8 {
            errs.push("v1 def must use 8 directions (set version 2 for 16/24)".into());
        }
        return if errs.is_empty() { Ok(()) } else { Err(errs) };
    }
    if !def.parts.is_empty() || !def.tires.is_empty() {
        errs.push("v2 def must use `layers`, not `parts`/`tires`".into());
    }
    let mut names = std::collections::HashSet::new();
    let hulls = def.layers.iter().filter(|l| l.kind == LayerKind::Hull).count();
    if hulls != 1 {
        errs.push(format!("v2 def needs exactly one hull layer, has {hulls}"));
    }
    for l in &def.layers {
        let who = format!("layer '{}'", l.name);
        if l.name.is_empty() {
            errs.push("a layer has an empty name".into());
        }
        if !names.insert(l.name.as_str()) {
            errs.push(format!("{who}: duplicate name"));
        }
        if l.texture.is_empty() {
            errs.push(format!("{who}: empty texture"));
        }
        if let Some(n) = l.directions {
            if !SUPPORTED_DIRECTIONS.contains(&n) {
                errs.push(format!("{who}: directions {n} not supported"));
            }
        }
        for axis in FrameAxis::ORDER {
            if axis != FrameAxis::Dir
                && l.axis_count(axis, def.directions) > 1
                && !l.kind.allows(axis)
            {
                errs.push(format!("{who}: kind {:?} cannot carry a {} axis", l.kind, axis.name()));
            }
        }
        for (name, ax) in [("pitch", l.pitch), ("roll", l.roll), ("steer", l.steer)] {
            if let Some(ax) = ax {
                if !(ax.max_deg.is_finite() && ax.min().is_finite() && ax.max_deg > ax.min()) {
                    errs.push(format!("{who}: {name} needs min_deg < max_deg"));
                }
            }
        }
        if l.kind == LayerKind::GearSide && l.side.is_none() {
            errs.push(format!("{who}: gear_side needs a side"));
        }
        if l.spin_frames > 1 && (l.radius_m.is_nan() || l.radius_m <= 0.0) {
            errs.push(format!("{who}: spin_frames needs radius_m > 0"));
        }
        for (cname, c) in &l.clips {
            if c.frames == 0 {
                errs.push(format!("{who}: clip '{cname}' has no frames"));
            }
            if !(c.fps > 0.0 && c.fps.is_finite()) {
                errs.push(format!("{who}: clip '{cname}' fps must be > 0"));
            }
        }
        if let Some(r) = l.depth_range_m {
            if !(r > 0.0 && r.is_finite()) {
                errs.push(format!("{who}: depth_range_m {r} must be > 0"));
            }
        }
        if let Some(o) = l.offset {
            if o.iter().any(|v| !v.is_finite()) {
                errs.push(format!("{who}: offset must be finite"));
            }
        }
    }
    for l in &def.layers {
        if let Some(p) = &l.parent {
            if p == &l.name || !names.contains(p.as_str()) {
                errs.push(format!("layer '{}': unknown parent '{p}'", l.name));
            }
        }
    }
    // Parent chains must terminate (no cycles).
    for l in &def.layers {
        let mut cur = l.parent.as_deref();
        let mut hops = 0;
        while let Some(p) = cur {
            hops += 1;
            if hops > def.layers.len() {
                errs.push(format!("layer '{}': parent cycle", l.name));
                break;
            }
            cur = def.layers.iter().find(|x| x.name == p).and_then(|x| x.parent.as_deref());
        }
    }
    for (sname, s) in &def.sockets {
        if !names.contains(s.layer.as_str()) {
            errs.push(format!("socket '{sname}': unknown layer '{}'", s.layer));
        }
        if s.offset.iter().any(|v| !v.is_finite()) {
            errs.push(format!("socket '{sname}': offset must be finite"));
        }
    }
    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs)
    }
}
