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
    /// layout as this texture; 0.5 = anchor plane, 1.0 = closest, 0.0 =
    /// farthest).  When present, the sprite writes `gl_FragDepth` and is
    /// occluded per-pixel instead of by draw order.
    #[serde(default)]
    pub depth: Option<String>,
    /// Depth range (isoDepth units) that the depth map's grayscale [0, 1]
    /// spans, emitted by the exporter.
    #[serde(default)]
    pub depth_range: f32,
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
}

/// One sheet (texture) referenced by a [`FrameTable`].
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
    /// sheet (grayscale `gl_FragDepth` mask; 0.5 = anchor plane).
    #[serde(default)]
    pub depth: Option<String>,
    /// Depth range (isoDepth units) the sheet's depth-map grayscale spans.
    #[serde(default)]
    pub depth_range: f32,
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
}

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

/// A wheeled-vehicle definition: a body plus independent wheel parts, each with
/// per-direction ground-origin anchors.  Emitted by the Blender exporter and
/// consumed by `Engine::spawn_vehicle`.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct VehicleDef {
    pub name: String,
    pub directions: u32,
    #[serde(default = "default_vehicle_columns")]
    pub columns: u32,
    #[serde(default = "default_vehicle_rows")]
    pub rows: u32,
    /// Pixel size of one frame cell (the sprite's drawn size at scale 1).
    #[serde(default)]
    pub cell: [f32; 2],
    /// Number of body pitch frames per direction (1 = flat/level only).  The
    /// body sheet stacks `pitch_levels` direction blocks vertically, so the
    /// body `tile_set_size` is `[columns, rows * pitch_levels]`.
    #[serde(default = "default_vehicle_pitch_levels")]
    pub pitch_levels: u32,
    /// Max body pitch angle (degrees, symmetric ±) the frames span, emitted by
    /// the exporter.  The engine quantizes the simulated pitch angle against
    /// this ceiling.
    #[serde(default = "default_vehicle_pitch_max_deg")]
    pub pitch_max_deg: f32,
    /// Number of body roll frames per (pitch, direction) (1 = no roll).  The
    /// body sheet stacks `pitch_levels · roll_levels` direction blocks
    /// vertically, so the body `tile_set_size` is
    /// `[columns, rows · pitch_levels · roll_levels]`.
    #[serde(default = "default_vehicle_roll_levels")]
    pub roll_levels: u32,
    /// Max body roll angle (degrees, symmetric ±) the frames span, emitted by
    /// the exporter.
    #[serde(default = "default_vehicle_roll_max_deg")]
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
    #[serde(default = "default_vehicle_turn_rate")]
    pub turn_rate_deg_per_sec: f32,
    /// Max drop (in pixels) the suspension absorbs without damage.  The A* may
    /// route a downward "jump" over a small cliff whose drop is within this
    /// distance; larger drops are impassable (issue #35).  `0` disables jumps.
    #[serde(default)]
    pub safe_fall_px: f32,
    /// Number of steering angles rendered for the front tires (1 = no steering).
    /// A steering-tire sheet stacks `steer_levels` direction blocks vertically,
    /// so its `tile_set_size` is `[columns, rows · steer_levels]`.
    #[serde(default = "default_vehicle_steer_levels")]
    pub steer_levels: u32,
    /// Max front-wheel steering angle (degrees, symmetric ±) the frames span,
    /// emitted by the exporter.  The engine quantizes the steering demand
    /// against this ceiling.
    #[serde(default = "default_vehicle_steer_max_deg")]
    pub steer_max_deg: f32,
    /// Max front-wheel steering-angle rate while driving, in degrees per second.
    /// The follow controller integrates the steering angle toward its demand at
    /// this rate, so the tires sweep between steer frames instead of snapping.
    #[serde(default = "default_vehicle_steer_rate")]
    pub steer_rate_deg_per_sec: f32,
    /// Reverse speed in tiles per second (used when the vehicle backs up to
    /// recover from a goal/waypoint behind it).  `0` disables reversing.
    #[serde(default = "default_vehicle_reverse_speed")]
    pub reverse_speed: f32,
    /// A* turn penalty: cost added per 45° of heading change between successive
    /// steps (forward is cheapest, a 180° reversal costs the most).  `0`
    /// disables the penalty.  Threaded to the pathfinder so routes prefer
    /// gentle turns.
    #[serde(default = "default_vehicle_turn_cost")]
    pub turn_cost: f32,
    /// Steering-tire parts, in `[fl, fr, …]` order, matched to `parts` wheels by
    /// index (`tires[0]` steers `parts[1]`, the front-left wheel).  Empty = no
    /// steering.  A tire shares its wheel's ground-origin anchor (the steering
    /// yaw is about the axle's vertical axis, so the anchor is steer-invariant).
    #[serde(default)]
    pub tires: Vec<VehiclePartDef>,
    pub parts: Vec<VehiclePartDef>,
}

fn default_vehicle_columns() -> u32 {
    4
}

fn default_vehicle_rows() -> u32 {
    2
}

fn default_vehicle_pitch_levels() -> u32 {
    1
}

fn default_vehicle_pitch_max_deg() -> f32 {
    20.0
}

fn default_vehicle_roll_levels() -> u32 {
    1
}

fn default_vehicle_roll_max_deg() -> f32 {
    20.0
}

fn default_vehicle_turn_rate() -> f32 {
    720.0
}

fn default_vehicle_steer_levels() -> u32 {
    1
}

fn default_vehicle_steer_rate() -> f32 {
    360.0
}

fn default_vehicle_reverse_speed() -> f32 {
    1.3
}

fn default_vehicle_turn_cost() -> f32 {
    0.0
}

fn default_vehicle_steer_max_deg() -> f32 {
    30.0
}

/// One part (body or wheel) of a [`VehicleDef`].
#[derive(Clone, Debug, serde::Deserialize)]
pub struct VehiclePartDef {
    pub name: String,
    pub texture: String,
    /// Ground-origin anchor per direction, `[ax, ay]` normalized to the frame
    /// (x from left, y from top).
    pub anchors: Vec<[f32; 2]>,
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

impl AnimationData {
    /// Sample a named channel at a fractional timeline position, linearly
    /// interpolating between the surrounding keyframes (clamping beyond the
    /// ends).  Returns `None` when the channel is absent or empty.
    pub fn channel_sample(&self, name: &str, counter: f32) -> Option<Vec<f32>> {
        let ch = self.channels.iter().find(|c| c.name == name)?;
        if ch.keys.is_empty() {
            return None;
        }
        let first = &ch.keys[0];
        if counter <= first.0 as f32 {
            return Some(first.1.clone());
        }
        let last = &ch.keys[ch.keys.len() - 1];
        if counter >= last.0 as f32 {
            return Some(last.1.clone());
        }
        for i in 0..ch.keys.len() - 1 {
            let (f0, v0) = &ch.keys[i];
            let (f1, v1) = &ch.keys[i + 1];
            let (f0, f1) = (*f0 as f32, *f1 as f32);
            if counter >= f0 && counter <= f1 {
                let t = if f1 > f0 { (counter - f0) / (f1 - f0) } else { 0.0 };
                return Some(v0.iter().zip(v1).map(|(a, b)| a + (b - a) * t).collect());
            }
        }
        Some(last.1.clone())
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

    /// Lock the cross-repo `VehicleDef` sidecar contract: the fields the
    /// `classic-roms` xtask injects (`turn_rate_deg_per_sec`, `safe_fall_px`)
    /// plus the exporter's extra `depth_range` (ignored) and an absent
    /// `path_footprint` (auto-derived at spawn).
    #[test]
    fn vehicle_def_deserializes_lrv_sidecar_shape() {
        let json = serde_json::json!({
            "name": "lrv",
            "directions": 8,
            "columns": 4,
            "rows": 2,
            "pitch_levels": 5,
            "pitch_max_deg": 20.0,
            "roll_levels": 3,
            "roll_max_deg": 20.0,
            "depth_range": 0.02400137797794352,
            "cell": [331, 331],
            "turn_rate_deg_per_sec": 90.0,
            "safe_fall_px": 96.0,
            "parts": [
                { "name": "body", "texture": "lrvBody", "anchors": [[0.5, 0.6618]] }
            ]
        });
        let def: VehicleDef = serde_json::from_value(json).expect("deserialize lrv.json");
        assert_eq!(def.name, "lrv");
        assert_eq!(def.turn_rate_deg_per_sec, 90.0);
        assert_eq!(def.safe_fall_px, 96.0);
        assert!(def.path_footprint.is_none(), "absent path_footprint -> None (auto-derive)");

        // An explicit footprint deserializes into Some.
        let json = serde_json::json!({
            "name": "lrv",
            "directions": 8,
            "parts": [ { "name": "body", "texture": "lrvBody", "anchors": [[0.5, 0.6618]] } ],
            "path_footprint": [[0, 0], [-1, -1]]
        });
        let def: VehicleDef = serde_json::from_value(json).expect("footprint override");
        assert_eq!(def.path_footprint, Some(vec![(0, 0), (-1, -1)]));
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
