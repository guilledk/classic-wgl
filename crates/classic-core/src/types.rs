/// Axis-aligned rectangle. Used by quadtree, colliders, and layout.
#[derive(Clone, Copy, Debug, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self { x, y, width, height }
    }

    pub fn intersects(&self, other: &Rect) -> bool {
        self.x <= other.x + other.width
            && self.x + self.width >= other.x
            && self.y <= other.y + other.height
            && self.y + self.height >= other.y
    }

    /// Whether the point `(px, py)` lies inside (or on) this rect.
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px <= self.x + self.width && py >= self.y && py <= self.y + self.height
    }
}

/// Collider event kinds dispatched by the physics layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColliderEvent {
    Enter,
    Exit,
    Click,
    Selection,
    SelectionTemp,
}

/// A manifest entry for a shader.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct ShaderInfo {
    pub name: String,
    pub vertex: String,
    pub fragment: String,
    pub attr: Vec<String>,
    pub unif: Vec<String>,
}

/// A manifest entry for a texture.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct TextureManifestEntry {
    pub name: String,
    pub src: String,
    /// Optional per-pixel depth map (a grayscale texture with the same tile
    /// layout as this texture; window `[0, 1]`, 0 = nearest, 1 = farthest).
    /// When present, the sprite writes `gl_FragDepth` and is occluded
    /// per-pixel instead of by draw order.
    #[serde(default)]
    pub depth: Option<String>,
    /// Optional per-pixel normal map (RGB = world-space normal remapped
    /// `[-1,1] → [0,1]`, same tile layout as this texture).  When present,
    /// the sprite shades with a runtime Lambertian term (`ambient_color +
    /// max(dot(n, light_dir), 0) * light_color`) instead of baked lighting.
    #[serde(default)]
    pub normal: Option<String>,
    /// Optional `frames.json` sidecar describing a packed atlas over this
    /// texture (and any companion sheets).  When present, frames are
    /// referenced by name through the [`FrameTable`] instead of the flat
    /// uniform-grid `tile_set_size` index.
    #[serde(default)]
    pub frames: Option<String>,
    /// The GPU transcode target when the texture's `src` bytes are a
    /// GPU-compressed (Basis Universal `.basis`) payload (`"BC7_RGBA"`,
    /// `"BC4_R"`, `"RGBA8"`, …).  Absent for uncompressed PNG textures.
    #[serde(default)]
    pub format: Option<String>,
}

/// One sheet (texture) referenced by a [`FrameTable`].
///
/// Cross-language frame-table contract: `SpriteSheetEntry`/`AtlasFrame`/
/// `FrameTable` (here, serde) and `Sheet`/`Frame`/`FrameTable` in classic-assets
/// (`render/schema.py`, msgspec) must stay field-compatible — the `frames.json`
/// sidecar is the wire contract between the classic-assets packer and this
/// loader.  The field set/order is the single invariant; the drift net is the
/// Rust test `pack_scene_bundles_per_sheet_normal_and_depth` (classic-roms
/// xtask) plus the byte-identity gate (a cold build must reproduce
/// byte-identical `.rom`s).
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct SpriteSheetEntry {
    /// Texture name (resolved to an asset handle at load time).
    pub name: String,
    /// Archive/loader path of the sheet PNG.
    pub src: String,
    /// Pixel dimensions of the sheet, used to normalize frame rects.
    pub size: [u32; 2],
    /// Optional per-sheet normal-map PNG packed in the same rect layout as
    /// this sheet (RGB world-space normals `[-1,1] → [0,1]`).  When present,
    /// frames on this sheet shade with the runtime Lambertian term.
    #[serde(default)]
    pub normal: Option<String>,
    /// Optional per-sheet depth-map PNG packed in the same rect layout as this
    /// sheet (grayscale `gl_FragDepth` mask; window `[0, 1]`, 0 = nearest,
    /// 1 = farthest).
    #[serde(default)]
    pub depth: Option<String>,
    /// How the depth companion's gray values are scaled (sheet v2).  `Global`
    /// (the default, and every v1 sheet) spans the engine's whole depth
    /// window; `Anchor` spans `±depth_range_m` metres around each frame's
    /// ground anchor.  See [`crate::vehicle_def`].
    #[serde(default, skip_serializing_if = "DepthMode::is_global")]
    pub depth_mode: DepthMode,
    /// The `±R` metres an `Anchor` depth companion spans (`0` = unset: the
    /// layer's / vehicle's `depth_range_m` applies).
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub depth_range_m: f32,
    /// The normal companion's alpha carries the team mask (sheet v2; weight
    /// [`crate::vehicle_def::team_mask_weight`]).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub team_mask: bool,
}

fn is_zero_f32(v: &f32) -> bool {
    *v == 0.0
}

/// How a depth companion sheet's gray values are scaled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DepthMode {
    /// Legacy: `gray − 0.5` is a window-space offset over the engine's whole
    /// `DEPTH_NEAR..DEPTH_FAR` window (~1.7 m per 8-bit step).
    #[default]
    Global,
    /// `gray − 0.5` times `2 · depth_range_m` is a view-depth offset in metres
    /// from the frame's ground anchor (positive = farther).
    Anchor,
}

impl DepthMode {
    pub fn is_global(&self) -> bool {
        *self == DepthMode::Global
    }
}

impl SpriteSheetEntry {
    /// The anchor-relative range this sheet's depth companion spans, if it is
    /// anchor-relative: its own `depth_range_m`, else `fallback_m` (the
    /// layer's or vehicle's).  `None` for a legacy global sheet, or an anchor
    /// sheet with no range anywhere.
    pub fn anchor_depth_range(&self, fallback_m: Option<f32>) -> Option<f32> {
        match self.depth_mode {
            DepthMode::Global => None,
            DepthMode::Anchor if self.depth_range_m > 0.0 => Some(self.depth_range_m),
            DepthMode::Anchor => fallback_m.filter(|r| *r > 0.0),
        }
    }

    /// Window-space units per `(gray − 0.5)` of this sheet's depth companion
    /// ([`crate::vehicle_def::depth_scale`]).
    pub fn depth_scale(&self, fallback_m: Option<f32>) -> f32 {
        crate::vehicle_def::depth_scale(self.anchor_depth_range(fallback_m))
    }
}

/// A single frame inside a packed sprite atlas.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct AtlasFrame {
    /// Index into [`FrameTable::sheets`] (0 = the owning texture).
    pub sheet: u32,
    /// Pixel rectangle `[x, y, width, height]` within the sheet.
    pub rect: [u32; 4],
    /// Untrimmed source size of the frame, in pixels (0 = unknown).
    #[serde(default)]
    pub source_size: [u32; 2],
    /// Offset of `rect` within `source_size` (trim margin), in pixels.
    #[serde(default)]
    pub trim_offset: [i32; 2],
    /// Optional anchor point in `[0..1]` range (x from left, y from top).
    #[serde(default)]
    pub anchor: Option<[f32; 2]>,
}

/// A packed-atlas frame table: the `frames.json` sidecar for a texture.
///
/// Frames are keyed by name and may span several sheets, so one table can
/// describe a multi-sheet sprite (issue #45).  Sheet 0 is the owning texture;
/// companion sheets are bundled as additional textures by the ROM loader.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct FrameTable {
    #[serde(default = "default_frame_table_version")]
    pub version: u32,
    /// Sheets referenced by frames, in index order.
    pub sheets: Vec<SpriteSheetEntry>,
    /// Name-keyed frames.
    pub frames: std::collections::BTreeMap<String, AtlasFrame>,
    /// Opt this texture into direction interpolation (sheet v2): the engine
    /// warps the two baked directions straddling a continuous heading instead
    /// of snapping to the nearest.  Needs an `Anchor` depth companion.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dir_interp: bool,
    /// Opt this texture into tilt interpolation (sheet v2, needs
    /// `dir_interp`): the direction warp also rotates each neighbour from its
    /// baked pitch / roll level to the drawn tilt (a world-frame residual
    /// rotation about the ground anchor), so tilt is continuous instead of
    /// snapping to the nearest level.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub tilt_interp: bool,
    /// A standalone posed sprite's axes (sheet v2): directions × optional
    /// pitch × roll, driven by heading + tilt (`sprite_pose`).  `None` = a
    /// direction-only sheet of `frames.len()` directions (see
    /// [`FrameTable::sprite_axes`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub axes: Option<crate::vehicle_def::SpriteAxes>,
    /// Precomputed per-sheet companion GL texture names, indexed by sheet.
    /// Each entry is `(normal_tex_name, depth_tex_name)`; `None` when the
    /// sheet has no companion.  Built once at load via
    /// [`FrameTable::precompute_companions`] so the draw path clones instead
    /// of re-formatting `"{sheet}-normal"` / `"{sheet}-depth"` every frame.
    #[serde(skip)]
    pub companions: Vec<(Option<String>, Option<String>)>,
}

fn default_frame_table_version() -> u32 {
    1
}

impl FrameTable {
    /// Normalize a frame's pixel rect to a UV rect `[u0, v0, u1, v1]` in the
    /// sheet's `[0..1]²` space.  Returns `None` if the frame's sheet index is
    /// out of range.
    pub fn uv_rect(&self, frame: &AtlasFrame) -> Option<[f32; 4]> {
        let sheet = self.sheets.get(frame.sheet as usize)?;
        let w = sheet.size[0].max(1) as f32;
        let h = sheet.size[1].max(1) as f32;
        let [x, y, fw, fh] = frame.rect;
        let (x, y, fw, fh) = (x as f32, y as f32, fw as f32, fh as f32);
        Some([x / w, y / h, (x + fw) / w, (y + fh) / h])
    }

    /// Precompute the per-sheet companion texture names (`companions`).  Called
    /// once after deserialization; a no-op if already populated.
    pub fn precompute_companions(&mut self) {
        self.companions = self
            .sheets
            .iter()
            .map(|s| {
                let normal = s.normal.as_ref().map(|_| format!("{}-normal", s.name));
                let depth = s.depth.as_ref().map(|_| format!("{}-depth", s.name));
                (normal, depth)
            })
            .collect();
    }

    /// The axes a posed sprite of this texture is framed by: the declared
    /// `axes`, else a direction-only sheet of `frames.len()` directions.
    pub fn sprite_axes(&self) -> crate::vehicle_def::SpriteAxes {
        self.axes.unwrap_or_else(|| {
            crate::vehicle_def::SpriteAxes::directions(self.frames.len().max(1) as u32)
        })
    }

    /// Check the sheet-v2 rules: an anchor-relative sheet has a range (its own
    /// or `fallback_m`), and a `dir_interp` table has an anchor-relative depth
    /// companion on every sheet.
    pub fn validate(&self, fallback_m: Option<f32>) -> Result<(), Vec<String>> {
        let mut errs = Vec::new();
        if self.version > FRAME_TABLE_VERSION {
            errs.push(format!(
                "frames.json version {} is newer than this engine supports ({FRAME_TABLE_VERSION})",
                self.version
            ));
        }
        for s in &self.sheets {
            if s.depth_mode == DepthMode::Anchor && s.anchor_depth_range(fallback_m).is_none() {
                errs.push(format!("sheet '{}': anchor depth needs depth_range_m > 0", s.name));
            }
            if !(s.depth_range_m >= 0.0 && s.depth_range_m.is_finite()) {
                errs.push(format!("sheet '{}': depth_range_m must be >= 0", s.name));
            }
            if self.dir_interp && (s.depth.is_none() || s.depth_mode != DepthMode::Anchor) {
                errs.push(format!(
                    "sheet '{}': dir_interp needs an anchor-relative depth companion",
                    s.name
                ));
            }
        }
        if self.tilt_interp && !self.dir_interp {
            errs.push("tilt_interp needs dir_interp".into());
        }
        if let Some(axes) = &self.axes {
            if !crate::vehicle_def::SUPPORTED_DIRECTIONS.contains(&axes.directions) {
                errs.push(format!("axes: directions {} not supported", axes.directions));
            }
            let (need, have) = (axes.frame_count() as usize, self.frames.len());
            if have < need {
                errs.push(format!("axes need {need} frames, the table has {have}"));
            }
        }
        if errs.is_empty() {
            Ok(())
        } else {
            Err(errs)
        }
    }
}

/// The newest `frames.json` version this engine reads (`2` = sheet v2:
/// `depth_mode`/`depth_range_m`/`team_mask` per sheet, `dir_interp` per
/// table).  Version 1 tables are v2 tables with every new field at its
/// default.
pub const FRAME_TABLE_VERSION: u32 = 2;

/// A manifest entry for an SDF font.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct SdfFontManifestEntry {
    pub name: String,
    pub metrics: String,
}

/// A manifest entry for a wheeled-vehicle definition sidecar.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct VehicleManifestEntry {
    pub name: String,
    pub src: String,
}

/// A manifest entry for a 3D glTF model (`.glb`), staged via the catalog's
/// `models[]` (`src` = `/models/<name>.glb`) and referenced by name from a
/// [`Model`](crate::components::Model) component.
///
/// The model's albedo texture is embedded in the `.glb` and decoded to RGBA8
/// at load.  Externalizing it to Basis/KTX2 later grows this entry a texture
/// field; it is a packaging change, not an engine rework.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct ModelManifestEntry {
    pub name: String,
    pub src: String,
}

/// A manifest entry for a Blender-exported data artifact (vehicle anchors,
/// animation offsets, etc.), staged via the catalog's `data[]` and referenced
/// by name from authored defs.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct DataManifestEntry {
    pub name: String,
    pub src: String,
}

/// A wheeled-vehicle definition: a body plus independent wheel parts, each with
/// per-direction ground-origin anchors.  Emitted by the Blender exporter and
/// consumed by `Engine::spawn_vehicle`.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct VehicleDef {
    pub name: String,
    pub directions: u32,
    /// Name of the Blender-exported anchors data artifact (staged via the
    /// catalog's `data[]`).  Empty = no anchors (parts fall back to 0.5/0.5).
    #[serde(default)]
    pub anchors: String,
    #[serde(default)]
    pub columns: u32,
    #[serde(default)]
    pub rows: u32,
    /// Pixel size of one frame cell (the sprite's drawn size at scale 1).
    #[serde(default)]
    pub cell: [f32; 2],
    /// Number of body pitch frames per direction (1 = flat/level only).  The
    /// body sheet stacks `pitch_levels` direction blocks vertically, so the
    /// body `tile_set_size` is `[columns, rows * pitch_levels]`.
    #[serde(default)]
    pub pitch_levels: u32,
    /// Max body pitch angle (degrees, symmetric ±) the frames span, emitted by
    /// the exporter.  The engine quantizes the simulated pitch angle against
    /// this ceiling.
    #[serde(default)]
    pub pitch_max_deg: f32,
    /// Number of body roll frames per (pitch, direction) (1 = no roll).  The
    /// body sheet stacks `pitch_levels · roll_levels` direction blocks
    /// vertically, so the body `tile_set_size` is
    /// `[columns, rows · pitch_levels · roll_levels]`.
    #[serde(default)]
    pub roll_levels: u32,
    /// Max body roll angle (degrees, symmetric ±) the frames span, emitted by
    /// the exporter.
    #[serde(default)]
    pub roll_max_deg: f32,
    /// Collision footprint for pathfinding: integer tile offsets from the body
    /// anchor cell that the vehicle occupies.  `None` auto-derives the
    /// footprint from the wheel extent at spawn (see `Engine::spawn_vehicle`);
    /// `Some` is an explicit override.  A* treats a cell as passable only when
    /// every offset is walkable (issue #35).
    #[serde(default)]
    pub path_footprint: Option<Vec<(i32, i32)>>,
    /// Max body heading change while driving, in degrees per second.  Drives
    /// the bounded-turn follow controller (issue #35).  Defaults high so the
    /// movement approximates the old instant-turn behaviour for simple defs.
    #[serde(default)]
    pub turn_rate_deg_per_sec: f32,
    /// Max drop (in metres) the suspension absorbs without damage.  The A* may
    /// route a downward "jump" over a small cliff whose drop is within this
    /// distance; larger drops are impassable (issue #35).  `0` disables jumps.
    #[serde(default)]
    pub safe_fall_m: f32,
    /// Number of steering angles rendered for the front tires (1 = no steering).
    /// A steering-tire sheet stacks `steer_levels` direction blocks vertically,
    /// so its `tile_set_size` is `[columns, rows · steer_levels]`.
    #[serde(default)]
    pub steer_levels: u32,
    /// Max front-wheel steering angle (degrees, symmetric ±) the frames span,
    /// emitted by the exporter.  The engine quantizes the steering demand
    /// against this ceiling.
    #[serde(default)]
    pub steer_max_deg: f32,
    /// Max front-wheel steering-angle rate while driving, in degrees per second.
    /// The follow controller integrates the steering angle toward its demand at
    /// this rate, so the tires sweep between steer frames instead of snapping.
    #[serde(default)]
    pub steer_rate_deg_per_sec: f32,
    /// Reverse speed in tiles per second (used when the vehicle backs up to
    /// recover from a goal/waypoint behind it).  `0` disables reversing.
    #[serde(default)]
    pub reverse_speed: f32,
    /// A* turn penalty: cost added per 45° of heading change between successive
    /// steps (forward is cheapest, a 180° reversal costs the most).  `0`
    /// disables the penalty.  Threaded to the pathfinder so routes prefer
    /// gentle turns.
    #[serde(default)]
    pub turn_cost: f32,
    /// Steering-tire parts, in `[fl, fr, …]` order, matched to `parts` wheels by
    /// index (`tires[0]` steers `parts[1]`, the front-left wheel).  Empty = no
    /// steering.  A tire shares its wheel's ground-origin anchor (the steering
    /// yaw is about the axle's vertical axis, so the anchor is steer-invariant).
    #[serde(default)]
    pub tires: Vec<VehiclePartDef>,
    /// v1 parts: the body then the four wheels `[fl, fr, rl, rr]`.  Empty in a
    /// v2 def (which uses `layers`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<VehiclePartDef>,
    /// Effect each wheel kicks up while driving (resolved against the loaded
    /// effect libraries).  Per wheel, its spawn rate and launch speed scale
    /// with ground speed, and every change in speed adds bursts.  `None` = no
    /// dust (and no emitters).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dust_effect: Option<String>,
    /// One-shot effect fired under the body when it lands after being
    /// airborne, scaled by the impact speed.  `None` = no touchdown puff.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub touchdown_effect: Option<String>,
    /// Definition version: `1` (the default: 8 directions, 5 `parts`) or `2`
    /// (`layers`).  See [`crate::vehicle_def`].
    #[serde(default = "default_vehicle_def_version", skip_serializing_if = "is_v1")]
    pub version: u32,
    /// Sprite pixels per metre the sheets were baked at (`0` = the engine's
    /// `PPM_TARGET`).
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub px_per_m: f32,
    /// v2 sprite layers (empty for a v1 def; see [`VehicleDef::effective_layers`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<crate::vehicle_def::VehicleLayerDef>,
    /// Named attachment points (emitters, lights, muzzles), keyed by name.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub sockets: std::collections::BTreeMap<String, crate::vehicle_def::SocketDef>,
    /// How the spawned layers share a ghost group.
    #[serde(default, skip_serializing_if = "is_instance_ghost")]
    pub ghost_group: crate::vehicle_def::GhostGroupMode,
    /// Default anchor-relative depth range (metres) for every layer whose
    /// sheet and layer leave it unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth_range_m: Option<f32>,
    /// Tracked vehicle: turn in place (zero turning radius) instead of
    /// arcing.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pivot_turn: bool,
}

fn default_vehicle_def_version() -> u32 {
    1
}

fn is_v1(v: &u32) -> bool {
    *v == 1
}

fn is_instance_ghost(m: &crate::vehicle_def::GhostGroupMode) -> bool {
    *m == crate::vehicle_def::GhostGroupMode::Instance
}

impl Default for VehicleDef {
    /// An empty v1 def (what serde fills for absent keys), for tests and
    /// struct-update syntax.
    fn default() -> Self {
        Self {
            name: String::new(),
            directions: 8,
            anchors: String::new(),
            columns: 0,
            rows: 0,
            cell: [0.0; 2],
            pitch_levels: 0,
            pitch_max_deg: 0.0,
            roll_levels: 0,
            roll_max_deg: 0.0,
            path_footprint: None,
            turn_rate_deg_per_sec: 0.0,
            safe_fall_m: 0.0,
            steer_levels: 0,
            steer_max_deg: 0.0,
            steer_rate_deg_per_sec: 0.0,
            reverse_speed: 0.0,
            turn_cost: 0.0,
            tires: Vec::new(),
            parts: Vec::new(),
            dust_effect: None,
            touchdown_effect: None,
            version: 1,
            px_per_m: 0.0,
            layers: Vec::new(),
            sockets: Default::default(),
            ghost_group: Default::default(),
            depth_range_m: None,
            pivot_turn: false,
        }
    }
}

impl VehicleDef {
    /// Whether this is a v2 (layered) definition.
    pub fn is_v2(&self) -> bool {
        self.version >= 2 || !self.layers.is_empty()
    }

    /// Pixels per metre of the sheets (`px_per_m`, or `PPM_TARGET` when unset).
    pub fn px_per_m(&self) -> f32 {
        if self.px_per_m > 0.0 {
            self.px_per_m
        } else {
            crate::tilemap::PPM_TARGET
        }
    }

    /// The layers to spawn: the v2 `layers`, or the v1 `parts`/`tires` mapped
    /// onto layers ([`crate::vehicle_def::legacy_layers`]).
    pub fn effective_layers(&self) -> Vec<crate::vehicle_def::VehicleLayerDef> {
        if self.is_v2() {
            self.layers.clone()
        } else {
            crate::vehicle_def::legacy_layers(self)
        }
    }

    /// Check the def against its version's contract.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        crate::vehicle_def::validate(self)
    }
}

/// One part (body or wheel) of a [`VehicleDef`].  The part's per-direction
/// ground-origin anchors live in the vehicle's anchors data artifact (see
/// [`VehicleAnchors`]), referenced by [`VehicleDef::anchors`].
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct VehiclePartDef {
    pub name: String,
    pub texture: String,
}

/// A vehicle's Blender-exported anchors data artifact: each part's per-direction
/// ground-origin anchor, `[ax, ay]` normalized to the frame (x from left, y from
/// top), keyed by part/tire name.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct VehicleAnchors {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub parts: std::collections::BTreeMap<String, Vec<[f32; 2]>>,
    #[serde(default)]
    pub tires: std::collections::BTreeMap<String, Vec<[f32; 2]>>,
}

/// Per-scene vehicle stat tuning, keyed by (namespace-qualified) vehicle name.
/// Applied as a typed field merge over the shared [`VehicleDef`] (no JSON
/// round-trip).
#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct VehicleOverrides {
    #[serde(default)]
    pub turn_rate_deg_per_sec: Option<f32>,
    #[serde(default)]
    pub safe_fall_m: Option<f32>,
    #[serde(default)]
    pub steer_rate_deg_per_sec: Option<f32>,
    #[serde(default)]
    pub reverse_speed: Option<f32>,
    #[serde(default)]
    pub turn_cost: Option<f32>,
    /// Opt a scene's copy of the vehicle into wheel dust (see
    /// [`VehicleDef::dust_effect`]).
    #[serde(default)]
    pub dust_effect: Option<String>,
    /// Opt a scene's copy of the vehicle into a touchdown puff (see
    /// [`VehicleDef::touchdown_effect`]).
    #[serde(default)]
    pub touchdown_effect: Option<String>,
}

impl VehicleOverrides {
    /// Merge the set (non-`None`) overrides onto `def` by direct field
    /// assignment.
    pub fn apply_to(&self, def: &mut VehicleDef) {
        if let Some(v) = self.turn_rate_deg_per_sec {
            def.turn_rate_deg_per_sec = v;
        }
        if let Some(v) = self.safe_fall_m {
            def.safe_fall_m = v;
        }
        if let Some(v) = self.steer_rate_deg_per_sec {
            def.steer_rate_deg_per_sec = v;
        }
        if let Some(v) = self.reverse_speed {
            def.reverse_speed = v;
        }
        if let Some(v) = self.turn_cost {
            def.turn_cost = v;
        }
        if let Some(v) = &self.dust_effect {
            def.dust_effect = Some(v.clone());
        }
        if let Some(v) = &self.touchdown_effect {
            def.touchdown_effect = Some(v.clone());
        }
    }
}

/// A sparse, frame-keyed visual offset.  The engine linearly interpolates
/// between consecutive keyframes (matching the Blender ``location`` fcurves),
/// so a long descent ships only its true breakpoints.
#[derive(Clone, Debug)]
pub struct OffsetKeyframe {
    /// Animation timeline frame (an index into [`AnimationData::sequence`]).
    pub frame: u32,
    /// Screen-space offset at that frame (already scaled by `pixels_per_meter`).
    pub offset: [f32; 3],
}

/// A named, typed, sparse keyframe channel in an animation's unified curves
/// blob (`animation.bin`).  One animation carries several channels — sprite
/// motion (`offset`) alongside light configuration (`light.position`,
/// `light.color`, `light.intensity`, `light.radius`, `light.dir`,
/// `light.cone`) — each linearly interpolated between keyframes by the
/// animator, so frame + motion + light stay in lockstep.
#[derive(Clone, Debug)]
pub struct AnimChannel {
    /// Channel name (`offset`, `light.position`, `light.color`, ...).
    pub name: String,
    /// Floats per keyframe: `1` (scalar), `3` (vec3), or `4` (vec4).
    pub component: u8,
    /// Sparse `(frame, value)` keyframes, sorted by frame.
    pub keys: Vec<(u32, Vec<f32>)>,
}

/// A manifest entry for an animation.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct AnimationData {
    pub name: String,
    pub src: String,
    pub rate: f32,
    pub sequence: Vec<u32>,
    /// Per-sequence-entry visual offsets in iso units (the legacy dense format).
    /// Missing entries are zero.
    #[serde(default)]
    pub offsets: Vec<[f32; 3]>,
    /// Sparse, frame-keyed offsets (the versioned blob).  Interpolated between
    /// keyframes by the animator; takes precedence over [`Self::offsets`] when
    /// non-empty.  Never present in the JSON manifest — filled from the ROM's
    /// `animations/` metadata at boot.
    #[serde(skip)]
    pub offset_keyframes: Vec<OffsetKeyframe>,
    /// Unified typed channels loaded from the ROM's animation metadata blob
    /// (`animation.bin`).  Carries sprite motion (`offset`) and every light
    /// channel; never present in the JSON manifest.
    #[serde(skip)]
    pub channels: Vec<AnimChannel>,
    /// Optional path to a ROM resource with per-frame renderer metadata (e.g.
    /// Blender `rig_location` offsets), loaded into [`Self::offsets`] /
    /// [`Self::offset_keyframes`] at boot.
    #[serde(default)]
    pub metadata: Option<String>,
}

/// Where a fractional timeline position falls in a channel's keys.
enum KeySpan {
    /// Exactly a key's value (clamped before the first / past the last).
    Key(usize),
    /// Between keys `i` and `i + 1`, at fraction `t`.
    Lerp(usize, f32),
}

/// A channel sample as a fixed array: the first `len` (at most 4) floats.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ChannelValue {
    pub v: [f32; 4],
    pub len: usize,
}

impl AnimChannel {
    /// The keys around `counter`.  Keys are sorted by frame (the format's
    /// contract), so a binary search finds the first span whose end reaches
    /// `counter`, the span a front-to-back scan picks (ties included).
    fn span(&self, counter: f32) -> Option<KeySpan> {
        let keys = &self.keys;
        let n = keys.len();
        if n == 0 {
            return None;
        }
        if counter <= keys[0].0 as f32 {
            return Some(KeySpan::Key(0));
        }
        if counter >= keys[n - 1].0 as f32 {
            return Some(KeySpan::Key(n - 1));
        }
        let i = keys[1..].partition_point(|k| (k.0 as f32) < counter);
        let (f0, f1) = (keys[i].0 as f32, keys[i + 1].0 as f32);
        if !(counter >= f0 && counter <= f1) {
            // NaN: the scan matches no span and falls through to the last key.
            return Some(KeySpan::Key(n - 1));
        }
        let t = if f1 > f0 { (counter - f0) / (f1 - f0) } else { 0.0 };
        Some(KeySpan::Lerp(i, t))
    }

    /// The channel at `counter` as a fixed array (no allocation): the same
    /// values as [`AnimationData::channel_sample`], truncated to 4 floats.
    pub fn sample_fixed(&self, counter: f32) -> Option<ChannelValue> {
        let mut out = ChannelValue::default();
        match self.span(counter)? {
            KeySpan::Key(k) => {
                let v = &self.keys[k].1;
                out.len = v.len().min(4);
                out.v[..out.len].copy_from_slice(&v[..out.len]);
            }
            KeySpan::Lerp(i, t) => {
                let (v0, v1) = (&self.keys[i].1, &self.keys[i + 1].1);
                for (o, (a, b)) in out.v.iter_mut().zip(v0.iter().zip(v1)) {
                    *o = a + (b - a) * t;
                    out.len += 1;
                }
            }
        }
        Some(out)
    }
}

impl AnimationData {
    /// Sample a named channel at a fractional timeline position, linearly
    /// interpolating between the surrounding keyframes (clamping beyond the
    /// ends).  Returns `None` when the channel is absent or empty.
    pub fn channel_sample(&self, name: &str, counter: f32) -> Option<Vec<f32>> {
        let ch = self.channels.iter().find(|c| c.name == name)?;
        Some(match ch.span(counter)? {
            KeySpan::Key(k) => ch.keys[k].1.clone(),
            KeySpan::Lerp(i, t) => {
                let (v0, v1) = (&ch.keys[i].1, &ch.keys[i + 1].1);
                v0.iter().zip(v1).map(|(a, b)| a + (b - a) * t).collect()
            }
        })
    }
}

/// The resource manifest (matches `public/manifest.json`).
#[derive(Clone, Debug, serde::Deserialize)]
pub struct Manifest {
    /// Shader declarations.  The engine owns the built-in catalog: the ROM may
    /// omit this (or send `[]`) and the engine compiles its builtins; a
    /// non-empty list overrides builtins by name.
    #[serde(default)]
    pub shaders: Vec<ShaderInfo>,
    pub textures: Vec<TextureManifestEntry>,
    #[serde(default)]
    pub sdf_fonts: Vec<SdfFontManifestEntry>,
    #[serde(default)]
    pub animations: Vec<AnimationData>,
    #[serde(default)]
    pub vehicles: Vec<VehicleManifestEntry>,
    /// 3D glTF models (`.glb`), referenced by name from `Model` components.
    #[serde(default)]
    pub models: Vec<ModelManifestEntry>,
    /// Blender-exported data artifacts (anchors, offsets, …), referenced by
    /// name from authored defs.
    #[serde(default)]
    pub data: Vec<DataManifestEntry>,
}

/// One component in a serialized entity from `state.json`.
///
/// The `type` field identifies the component; everything else is
/// deserialized by the component's own serde impl.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct EntityComponentData {
    #[serde(rename = "type")]
    pub comp_type: String,
    #[serde(flatten)]
    pub fields: serde_json::Value,
}

/// A serialized entity from `state.json`.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct EntityData {
    pub components: Vec<EntityComponentData>,
}

/// Top-level `state.json` shape.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct StateData {
    pub entities: std::collections::HashMap<String, EntityData>,
}

/// Glyph metrics for a single character in an SDF atlas.
#[derive(Clone, Copy, Debug, serde::Deserialize)]
pub struct GlyphMetrics {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub x_offset: f32,
    pub y_offset: f32,
    pub x_advance: f32,
}

/// SDF font metrics (from the `.json` generated by classic-assets'
/// `render/make_sdf_font.py`).
#[derive(Clone, Debug, serde::Deserialize)]
pub struct SdfFontMetrics {
    pub name: String,
    pub family: String,
    pub atlas_size: [f32; 2],
    pub glyph_size: f32,
    #[serde(default)]
    pub spread: f32,
    pub baseline: f32,
    pub line_height: f32,
    pub glyphs: std::collections::HashMap<String, GlyphMetrics>,
}

/// Loading-progress weight constants.
pub const MANIFEST_WEIGHT: f32 = 2.0;
pub const SHADER_FETCH_WEIGHT: f32 = 2.0;
pub const SHADER_COMPILE_WEIGHT: f32 = 1.0;
pub const BUFFERS_WEIGHT: f32 = 1.0;
pub const TEXTURE_WEIGHT: f32 = 1.0;
pub const SDF_FONT_WEIGHT: f32 = 1.0;
pub const ANIMATIONS_WEIGHT: f32 = 1.0;

/// Estimate total loading-progress weight for a manifest.
pub fn estimate_manifest_weight(m: &Manifest) -> f32 {
    MANIFEST_WEIGHT
        + m.shaders.len() as f32 * (SHADER_FETCH_WEIGHT + SHADER_COMPILE_WEIGHT)
        + BUFFERS_WEIGHT
        + m.textures.len() as f32 * TEXTURE_WEIGHT
        + m.sdf_fonts.len() as f32 * SDF_FONT_WEIGHT
        + ANIMATIONS_WEIGHT
}

/// The 2D viewport size driven by canvas dimensions.
#[derive(Clone, Copy, Debug)]
pub struct Viewport {
    pub width: f32,
    pub height: f32,
}

impl Viewport {
    pub fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }

    pub fn aspect(&self) -> f32 {
        self.width / self.height
    }

    /// Build an orthographic projection.
    /// Left=0, right=w, bottom=h, top=0, near=-10000, far=10000.
    pub fn ortho_matrix(&self) -> glam::Mat4 {
        glam::Mat4::orthographic_rh(0.0, self.width, self.height, 0.0, -10000.0, 10000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lock the cross-repo authored `VehicleDef` contract: the fields
    /// classic-roms authors in `scene.json` `vehicles[]` (geometry + stats +
    /// an `anchors` data-artifact name) and an absent `path_footprint`
    /// (auto-derived at spawn).
    #[test]
    fn vehicle_def_deserializes_authored_shape() {
        let json = serde_json::json!({
            "name": "lrv",
            "directions": 8,
            "columns": 4,
            "rows": 2,
            "pitch_levels": 5,
            "pitch_max_deg": 20.0,
            "roll_levels": 3,
            "roll_max_deg": 20.0,
            "cell": [331, 331],
            "turn_rate_deg_per_sec": 90.0,
            "safe_fall_m": 1.5,
            "anchors": "lrv_anchors",
            "parts": [
                { "name": "body", "texture": "lrvBody" }
            ]
        });
        let def: VehicleDef = serde_json::from_value(json).expect("deserialize authored def");
        assert_eq!(def.name, "lrv");
        assert_eq!(def.turn_rate_deg_per_sec, 90.0);
        assert_eq!(def.safe_fall_m, 1.5);
        assert_eq!(def.anchors, "lrv_anchors");
        assert!(def.path_footprint.is_none(), "absent path_footprint -> None (auto-derive)");

        // An explicit footprint deserializes into Some.
        let json = serde_json::json!({
            "name": "lrv",
            "directions": 8,
            "parts": [ { "name": "body", "texture": "lrvBody" } ],
            "path_footprint": [[0, 0], [-1, -1]]
        });
        let def: VehicleDef = serde_json::from_value(json).expect("footprint override");
        assert_eq!(def.path_footprint, Some(vec![(0, 0), (-1, -1)]));
    }

    #[test]
    fn vehicle_anchors_deserializes_and_overrides_apply() {
        let json = serde_json::json!({
            "version": 1,
            "parts": { "body": [[0.5, 0.6618]] },
            "tires": { "tire_fl": [[0.1, 0.2]] }
        });
        let anchors: VehicleAnchors = serde_json::from_value(json).unwrap();
        assert_eq!(anchors.parts.len(), 1);
        assert_eq!(anchors.parts.get("body"), Some(&vec![[0.5, 0.6618]]));
        assert_eq!(anchors.tires.get("tire_fl"), Some(&vec![[0.1, 0.2]]));

        let ov: VehicleOverrides = serde_json::from_value(serde_json::json!({
            "turn_rate_deg_per_sec": 55.0,
            "safe_fall_m": 1.5
        }))
        .unwrap();
        let mut def = VehicleDef {
            name: "lrv".into(),
            directions: 8,
            anchors: String::new(),
            columns: 4,
            rows: 2,
            cell: [331.0, 331.0],
            pitch_levels: 5,
            pitch_max_deg: 20.0,
            roll_levels: 3,
            roll_max_deg: 20.0,
            path_footprint: None,
            turn_rate_deg_per_sec: 720.0,
            safe_fall_m: 0.0,
            steer_levels: 5,
            steer_max_deg: 30.0,
            steer_rate_deg_per_sec: 360.0,
            reverse_speed: 1.3,
            turn_cost: 0.0,
            tires: vec![],
            parts: vec![],
            dust_effect: None,
            touchdown_effect: None,
            ..Default::default()
        };
        ov.apply_to(&mut def);
        assert_eq!(def.turn_rate_deg_per_sec, 55.0);
        assert_eq!(def.safe_fall_m, 1.5);
        assert_eq!(def.steer_rate_deg_per_sec, 360.0);
        assert_eq!(def.dust_effect, None, "unset overrides leave dust off");

        // A scene opts its copy of a shared vehicle into dust + touchdown.
        let ov: VehicleOverrides = serde_json::from_value(serde_json::json!({
            "dust_effect": "lunar-common::lrv_dust",
            "touchdown_effect": "lunar-common::touchdown_puff"
        }))
        .unwrap();
        ov.apply_to(&mut def);
        assert_eq!(def.dust_effect.as_deref(), Some("lunar-common::lrv_dust"));
        assert_eq!(def.touchdown_effect.as_deref(), Some("lunar-common::touchdown_puff"));
        // An authored def without the keys serializes without them.
        def.dust_effect = None;
        def.touchdown_effect = None;
        let out = serde_json::to_value(&def).unwrap();
        assert!(out.get("dust_effect").is_none() && out.get("touchdown_effect").is_none());
    }

    /// A texture manifest entry without `frames` leaves it `None` (backward
    /// compatible with the flat uniform-grid referencing).
    #[test]
    fn texture_entry_frames_defaults_none() {
        let entry: TextureManifestEntry = serde_json::from_value(serde_json::json!({
            "name": "humanoid",
            "src": "res/humanoid.png"
        }))
        .unwrap();
        assert!(entry.frames.is_none());
        assert!(entry.depth.is_none());
    }

    /// A `frames.json` sidecar deserializes into a [`FrameTable`] and resolves
    /// frame pixel rects to normalized UV rects.
    #[test]
    fn frame_table_deserializes_and_normalizes_uv() {
        let table: FrameTable = serde_json::from_value(serde_json::json!({
            "version": 1,
            "sheets": [
                { "name": "humanoid", "src": "res/humanoid.png", "size": [512, 512] },
                { "name": "humanoid_2", "src": "res/humanoid_2.png", "size": [256, 256] }
            ],
            "frames": {
                "idle_0": { "sheet": 0, "rect": [0, 0, 64, 128], "source_size": [64, 128],
                            "trim_offset": [0, 0], "anchor": [0.5, 0.98] },
                "walk_3": { "sheet": 1, "rect": [128, 0, 64, 64] }
            }
        }))
        .unwrap();

        assert_eq!(table.version, 1);
        assert_eq!(table.sheets.len(), 2);
        assert_eq!(table.frames.len(), 2);

        let idle = &table.frames["idle_0"];
        assert_eq!(idle.sheet, 0);
        assert_eq!(idle.source_size, [64, 128]);
        assert_eq!(idle.trim_offset, [0, 0]);
        assert_eq!(idle.anchor, Some([0.5, 0.98]));

        // Defaults applied when omitted.
        let walk = &table.frames["walk_3"];
        assert_eq!(walk.source_size, [0, 0]);
        assert_eq!(walk.trim_offset, [0, 0]);
        assert_eq!(walk.anchor, None);

        // UV normalization: sheet 0 is 512x512.
        assert_eq!(table.uv_rect(idle), Some([0.0, 0.0, 64.0 / 512.0, 128.0 / 512.0]));
        // Sheet 1 is 256x256.
        assert_eq!(table.uv_rect(walk), Some([128.0 / 256.0, 0.0, 192.0 / 256.0, 64.0 / 256.0]));

        // Out-of-range sheet index yields None.
        let bad = AtlasFrame {
            sheet: 9,
            rect: [0, 0, 1, 1],
            source_size: [0, 0],
            trim_offset: [0, 0],
            anchor: None,
        };
        assert_eq!(table.uv_rect(&bad), None);
    }
}
