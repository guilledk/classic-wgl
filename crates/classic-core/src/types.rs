/// Axis-aligned rectangle. Used by quadtree, colliders, and layout.
///
/// Matches the TS `Rect` interface (`types.ts:118-123`).
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

/// Collider event names from `collision.ts:138`.
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

/// One part (body or wheel) of a [`VehicleDef`].
#[derive(Clone, Debug, serde::Deserialize)]
pub struct VehiclePartDef {
    pub name: String,
    pub texture: String,
    /// Ground-origin anchor per direction, `[ax, ay]` normalized to the frame
    /// (x from left, y from top).
    pub anchors: Vec<[f32; 2]>,
}

/// A manifest entry for an animation.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct AnimationData {
    pub name: String,
    pub src: String,
    pub rate: f32,
    pub sequence: Vec<u32>,
    /// Per-sequence-entry visual offsets in iso units. Missing entries are zero.
    #[serde(default)]
    pub offsets: Vec<[f32; 3]>,
    /// Optional path to a ROM resource with per-frame renderer metadata (e.g.
    /// Blender `rig_location` offsets), loaded into [`Self::offsets`] at boot.
    #[serde(default)]
    pub metadata: Option<String>,
}

/// The resource manifest (matches `public/manifest.json`).
#[derive(Clone, Debug, serde::Deserialize)]
pub struct Manifest {
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

/// SDF font metrics (from the `.json` generated by the `sdf-atlas` crate).
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
/// Port of `utils.ts:171-177`.
pub const MANIFEST_WEIGHT: f32 = 2.0;
pub const SHADER_FETCH_WEIGHT: f32 = 2.0;
pub const SHADER_COMPILE_WEIGHT: f32 = 1.0;
pub const BUFFERS_WEIGHT: f32 = 1.0;
pub const TEXTURE_WEIGHT: f32 = 1.0;
pub const SDF_FONT_WEIGHT: f32 = 1.0;
pub const ANIMATIONS_WEIGHT: f32 = 1.0;

/// Estimate total loading-progress weight for a manifest.
/// Port of `utils.ts:179-187`.
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

    /// Build an orthographic projection matching `state.ts:355-363`.
    /// Left=0, right=w, bottom=h, top=0, near=-10000, far=10000.
    pub fn ortho_matrix(&self) -> glam::Mat4 {
        glam::Mat4::orthographic_rh(0.0, self.width, self.height, 0.0, -10000.0, 10000.0)
    }
}
