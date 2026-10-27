//! # Skill: `classic-ecs`
//!
//! **Read `.agents/skills/classic-ecs/SKILL.md` before working on this module.**
//!
use glam::{Mat4, Vec2, Vec3, Vec4};
use serde::{Deserialize, Serialize};

/// Transforms position + scale, the basic renderable spatial component.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Transform {
    pub position: Vec3,
    pub scale: Vec3,
}

impl Transform {
    pub fn new(position: Vec3, scale: Vec3) -> Self {
        Self { position, scale }
    }

    pub fn model_matrix(&self) -> Mat4 {
        Mat4::from_translation(self.position) * Mat4::from_scale(self.scale)
    }
}

/// Marker component: when present, the entity is disabled (skipped by systems).
/// Maps `entity.enabled = false`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Disabled;

/// Debug identity component — a stable human-readable name for an entity.
/// Used by logging, golden traces, and UI debug output.
#[derive(Clone, Debug)]
pub struct DebugName(pub String);

impl std::fmt::Display for DebugName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The role an entity plays in a scene.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleKind {
    Tilemap,
    NavMesh,
    Agent,
    Cursor,
}

/// Tag component marking an entity's role, so host features (editor, nav,
/// agent, cursor) find entities without hardcoded names.  ROMs tag their own
/// entities in `state.json`; the engine queries by [`RoleKind`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Role {
    pub value: RoleKind,
}

impl Role {
    pub fn new(value: RoleKind) -> Self {
        Self { value }
    }
}

/// Marks an entity as selectable by the RTS selection system (host-owned).
///
/// `group` buckets selectables for set operations (e.g. units vs buildings);
/// `priority` breaks click-hit ties (higher wins).  The selection set lives on
/// the `Engine`, not here — this component only advertises selectability.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Selectable {
    pub priority: i32,
    pub group: u32,
}

/// Render a solid-colour rectangle.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RectRender {
    pub color: [f32; 4],
    pub ignore_cam: bool,
}

/// Render a sprite (single frame from a sprite sheet).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpriteRender {
    pub position: Vec3,
    pub scale: Vec3,
    /// Texture name (resolved to an asset handle at load time).
    pub texture: String,
    pub ignore_cam: bool,
    pub frame: f32,
    /// Optional frame name resolved through the texture's `frames.json` table
    /// (issue #45).  When set, `frame`/`tile_set_size` are ignored in favour of
    /// the frame's packed UV rect + size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_name: Option<String>,
    pub tile_set_size: Vec2,
    pub anchor: Vec2,
}

/// The default SDF font atlas name used when a text element doesn't specify
/// one explicitly.  The font ships in the shared `common` ROM, so the default
/// is namespace-qualified (the bare name would no longer resolve under the
/// multi-ROM split).
pub const DEFAULT_SDF_FONT: &str = "common::dejavusans";

/// SDF text.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SdfTextRender {
    /// SDF font atlas name (without `-sdf` suffix).
    pub atlas_name: String,
    pub color: [f32; 4],
    pub outline_color: [f32; 4],
    pub outline_width: f32,
    pub ignore_cam: bool,
    pub text: String,
    pub justify: TextJustify,
    pub weight: f32,
    pub gamma: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextJustify {
    #[default]
    Left,
    Center,
    Right,
}

/// Iso tilemap — the terrain grid.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Tilemap {
    pub position: Vec3,
    pub scale: Vec3,
    pub size_x: i32,
    pub size_y: i32,
    /// Texture name for the tileset sheet.
    pub tile_set: String,
    /// Pixel dimensions of one tile in the sheet.
    pub tile_pixel_size: [u32; 2],
    /// Max tile index.
    pub max_tile: u32,
    /// Name of the ROM grid resource holding the tile data (raw little-endian
    /// `u32`), if any.  Hydrated into `data` at boot by the host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tiles_grid: Option<String>,
    /// Name of the ROM grid resource holding the height vertex grid (raw
    /// little-endian `f32`), if any.  Hydrated into `height_data` at boot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heights_grid: Option<String>,
    /// Tile data (row-major, `size_x * size_y` elements).  Not serialized —
    /// persisted as the `tiles_grid` resource instead.
    #[serde(skip)]
    pub data: Vec<u32>,
    /// Per-vertex height data (vertex grid, `(size_x+1) * (size_y+1)` elements),
    /// in **metres** (the exporter's unit).  Not serialized — persisted as the
    /// `heights_grid` resource instead.
    #[serde(skip)]
    pub height_data: Vec<f32>,
    /// Pixels per metre — the conversion from `height_data` metres to screen
    /// pixels (`z_px = height_data · height_scale`).  Metre-authored scenes use
    /// [`PPM_TARGET`](crate::tilemap::PPM_TARGET) (64 px/m).
    #[serde(default)]
    pub height_scale: f32,
    /// Pixels-per-tile-row in the tileset image.
    #[serde(skip)]
    pub tile_set_pixel_size: [u32; 2],
    /// Tiles-per-row in the tileset.
    #[serde(skip)]
    pub tiles_per_row: u32,
    /// Mouse position in iso tile coordinates (updated every frame).
    #[serde(skip)]
    pub mouse_iso_pos: Vec3,
    #[serde(skip, default = "default_iso_sel")]
    pub selection_iso_begin: Vec3,
    #[serde(skip, default = "default_iso_sel")]
    pub selection_iso_end: Vec3,
    /// Opt-in material blending between neighbouring tiles (see
    /// [`TileBlend`]).  `None` (the default, and what an absent `blend` key
    /// deserializes to) keeps the classic one-id-per-tile, hard-edged look.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blend: Option<TileBlend>,
}

fn default_iso_sel() -> Vec3 {
    Vec3::new(-1.0, -1.0, -1.0)
}

/// Terrain material blending for a [`Tilemap`].
///
/// With blending on, the tilemap fragment shader stops sampling only the
/// tile it falls in: it fetches the 2x2 tile ids around a noise-warped,
/// half-tile-shifted lattice point and resolves their atlas cells, so a
/// material boundary follows a smooth, organic contour (cutting across the
/// tile staircase, marching-squares style) instead of square tile edges.
/// All lengths are in **tiles**.
///
/// Tile ids are grouped into material *classes* by [`TileBlend::materials`]
/// (the variants of one material share a class; an id no entry lists is its
/// own class).  Across classes the transition is a band about `width` wide
/// and, when `height_strength > 0`, resolved by height: the tileset's alpha is
/// then per-texel height (not opacity) and, with the class priority, decides
/// which material shows near the contour.  The variants of one class resolve
/// the same way on smooth whole-tile weights, which hides the tile grid on
/// single-material ground.  Minified terrain samples a capped mip chain of
/// the tileset (built by the engine) instead of aliasing.
///
/// Opt-in: a tilemap without `blend` renders exactly as before.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TileBlend {
    /// Width of the cross-class transition band, in tiles (`0` = crisp edge;
    /// a wide band exposes the tile staircase as a blur, so keep it narrow).
    #[serde(default = "TileBlend::default_width")]
    pub width: f32,
    /// Edge-warp noise frequency, in cycles per tile.
    #[serde(default = "TileBlend::default_noise_scale")]
    pub noise_scale: f32,
    /// Edge-warp noise amplitude, in tiles (`0` = straight transitions).
    #[serde(default = "TileBlend::default_noise_strength")]
    pub noise_strength: f32,
    /// How strongly the per-texel height (tileset alpha) and the material
    /// priority decide a transition.  `0` disables height-based blending and
    /// leaves the tileset alpha meaning opacity.
    #[serde(default)]
    pub height_strength: f32,
    /// Material classes; see [`TileBlendMaterial`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub materials: Vec<TileBlendMaterial>,
}

impl TileBlend {
    // Defaults are the values tuned on the lunar map: a narrow band, and a
    // warp that meanders a contour over a few tiles (warping only moves the
    // id lookup; the textures themselves are never distorted).
    fn default_width() -> f32 {
        0.3
    }
    fn default_noise_scale() -> f32 {
        0.35
    }
    fn default_noise_strength() -> f32 {
        1.2
    }
}

impl Default for TileBlend {
    fn default() -> Self {
        Self {
            width: Self::default_width(),
            noise_scale: Self::default_noise_scale(),
            noise_strength: Self::default_noise_strength(),
            height_strength: 0.0,
            materials: Vec::new(),
        }
    }
}

/// One material class of a [`TileBlend`]: the tile ids that are variants of
/// the same material, plus how that material behaves at a boundary.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TileBlendMaterial {
    /// Free-form label (documentation only; a scene can name its classes).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Tile ids in this class (`1..=255`).  An id listed by more than one
    /// entry belongs to the first.
    pub ids: Vec<u32>,
    /// Height bias in `[0, 1]` added to the per-texel height before the
    /// height blend: a higher-priority material wins its transitions (rock
    /// pokes through dust rather than dust through rock).
    #[serde(default)]
    pub priority: f32,
    /// Keep this material's edges crisp: wherever it meets another class the
    /// transition band collapses to a hard (still noise-warped) edge with no
    /// height mix.  Its variants still cross-fade among themselves.
    #[serde(default)]
    pub hard: bool,
}

impl Tilemap {
    pub fn map_size(&self) -> [i32; 2] {
        [self.size_x, self.size_y]
    }
}

/// Navigation mesh (layered on top of a Tilemap).
/// The default nav-overlay tileset name.  Ships in the shared `common` ROM, so
/// it is namespace-qualified (a bare name would not resolve under the split).
pub const DEFAULT_NAV_TILESET: &str = "common::navTileset";

fn default_nav_tileset() -> String {
    DEFAULT_NAV_TILESET.to_string()
}

/// Navigation mesh (walkability overlay rendered on top of a Tilemap).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NavMesh {
    #[serde(default)]
    pub position: Vec3,
    #[serde(default)]
    pub scale: Vec3,
    /// Entity name of the source Tilemap.
    pub map_entity: String,
    /// Texture name for the nav tileset.
    #[serde(default = "default_nav_tileset")]
    pub tile_set: String,
    /// Name of the ROM grid resource holding the walkability grid (raw
    /// little-endian `u32`), if any.  Hydrated into `data` at boot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_grid: Option<String>,
    /// Walkability grid (row-major, `1` = walkable).  Not serialized —
    /// persisted as the `data_grid` resource instead.
    #[serde(skip)]
    pub data: Vec<u32>,
    pub size_x: i32,
    pub size_y: i32,
}

/// An isometric sprite (billboard in iso space).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IsoSprite {
    pub position: Vec3,
    pub scale: Vec3,
    pub texture: String,
    /// Entity name of the tilemap this sprite lives on.
    pub tilemap: String,
    pub frame: f32,
    /// Optional frame name resolved through the texture's `frames.json` table
    /// (issue #45).  When set, `frame`/`tile_set_size` are ignored in favour of
    /// the frame's packed UV rect + size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_name: Option<String>,
    pub tile_set_size: Vec2,
    /// Anchor point in [0..1] range (e.g. `[0.5, 0.98]` = centre-bottom / feet).
    pub anchor: Vec2,
    /// Visual offset selected by the current animation frame, in Blender-world
    /// metres: horizontal drift in x/y, altitude in z.
    #[serde(skip)]
    pub frame_offset: Vec3,
    /// Footprint vertices in iso tile coords: `[NE, SE, SW, NW]`.
    #[serde(default = "default_footprint")]
    pub footprint: Vec<Vec2>,
    /// Stencil ghost group id (0 = ungrouped; 1..=254, as 255 is the models'
    /// group).  Sprites sharing a non-zero id (e.g. a vehicle's body + wheels)
    /// never ghost through each other, while still ghosting through terrain
    /// and other entities.  It scopes sprite-on-sprite ghosting only: a model
    /// behind the sprite ghosts over it unless the model opts out
    /// (`Model.ghost`).  Assignable declaratively in `state.json`;
    /// `spawn_vehicle` assigns it imperatively.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub ghost_group: u32,
    /// Per-sprite tint (RGBA) multiplied onto the sprite's albedo by the sheet
    /// shader before the Lambertian term.  Defaults to white (a no-op).
    /// Tintable assets (e.g. the grayscale shipping container cargo) set this
    /// at spawn to render the same sheet in different colours.
    #[serde(default = "default_white", skip_serializing_if = "is_white")]
    pub color: [f32; 4],
    /// Direction interpolation (sheet v2 `dir_interp`): when set, the sprite
    /// draws a blend of `frame` / `frame_name` (direction A, at `anchor`) and
    /// [`DirBlend::frame_b`] warped to a heading between them.  Written by the
    /// vehicle system each frame; never dumped (a review scene may set it).
    #[serde(default, skip_serializing)]
    pub dir_blend: Option<DirBlend>,
    /// Skip the ghost pass (a vehicle layer with `ghost: false`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_ghost: bool,
    /// Team colour (sheet v2 `team_mask`): multiplies the albedo by the team
    /// mask weight in the normal sheet's alpha, separately from `color`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_tint: Option<[f32; 3]>,
    /// Emissive strength: scales the marker-normal (unlit) pixels; `None` = 1
    /// (today's look), above 1 they brighten into the bloom pass.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emissive: Option<f32>,
}

/// The second baked direction of a direction-interpolated [`IsoSprite`]
/// and the rotation that takes each neighbour to the drawn heading.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DirBlend {
    /// Frame index of direction B (`{texture}_{frame_b}` in the frame table).
    pub frame_b: f32,
    /// Direction B's anchor, `[0..1]` of its source cell.
    pub anchor_b: Vec2,
    /// Drawn heading minus direction A's heading (tile-space radians, `>= 0`).
    pub dtheta_a: f32,
    /// Drawn heading minus direction B's heading (tile-space radians, `<= 0`).
    pub dtheta_b: f32,
    /// The world-frame tilt residual both neighbours are warped by after
    /// their yaw (`sheet.frag#INTERP`: `R = Q · Rz(φ)`), as a unit
    /// quaternion's `xyz` (`w = √(1 − |xyz|²) ≥ 0`): the drawn pitch / roll
    /// against the baked tilt level's, from
    /// [`tilt_residual`](crate::vehicle_def::tilt_residual).  Zero (identity)
    /// unless the sheet opts into `tilt_interp`.
    #[serde(default, skip_serializing_if = "is_zero3")]
    pub tilt: [f32; 3],
    /// A third source for pixels neither neighbour covers at a large tilt
    /// residual ([`third_tilt`](crate::vehicle_def::third_tilt)): the nearer
    /// yaw neighbour's direction at an adjacent tilt level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub third: Option<DirThird>,
}

/// [`DirBlend::third`]: a frame of the nearer yaw neighbour's direction at
/// an adjacent tilt level, warped by its own tilt residual and that
/// neighbour's yaw.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DirThird {
    /// Frame index (`{texture}_{frame}`).
    pub frame: f32,
    /// Its anchor, `[0..1]` of its source cell.
    pub anchor: Vec2,
    /// The residual quaternion `xyz` (`w >= 0`) from its tilt level.
    pub tilt: [f32; 3],
    /// Paired with direction B (else A): its yaw residual and textures.
    pub from_b: bool,
}

fn is_zero3(v: &[f32; 3]) -> bool {
    *v == [0.0; 3]
}

impl DirBlend {
    /// A pure-yaw blend (no tilt residual).
    pub fn yaw(frame_b: f32, anchor_b: Vec2, dtheta_a: f32, dtheta_b: f32) -> Self {
        Self { frame_b, anchor_b, dtheta_a, dtheta_b, tilt: [0.0; 3], third: None }
    }
}

fn is_zero(v: &u32) -> bool {
    *v == 0
}

fn default_white() -> [f32; 4] {
    [1.0, 1.0, 1.0, 1.0]
}

fn is_white(c: &[f32; 4]) -> bool {
    *c == default_white()
}

fn default_footprint() -> Vec<Vec2> {
    vec![Vec2::new(0.5, -0.5), Vec2::new(0.5, 0.5), Vec2::new(-0.5, 0.5), Vec2::new(-0.5, -0.5)]
}

/// An isometric agent (pathfinding sprite).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IsoAgent {
    pub position: Vec3,
    pub scale: Vec3,
    pub texture: String,
    pub tilemap: String,
    pub frame: f32,
    /// Optional frame name resolved through the texture's `frames.json` table
    /// (issue #45).  When set, `frame`/`tile_set_size` are ignored in favour of
    /// the frame's packed UV rect + size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_name: Option<String>,
    pub tile_set_size: Vec2,
    pub anchor: Vec2,
    /// Visual offset selected by the current animation frame, in Blender-world
    /// metres: horizontal drift in x/y, altitude in z.
    #[serde(skip)]
    pub frame_offset: Vec3,
    #[serde(default = "default_footprint")]
    pub footprint: Vec<Vec2>,
    pub speed: f32,
    pub anim_speed: f32,
    #[serde(default)]
    pub anim_prefix: String,
}

impl Default for IsoAgent {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            scale: Vec3::ONE,
            texture: String::new(),
            tilemap: String::new(),
            frame: 0.0,
            frame_name: None,
            tile_set_size: Vec2::ONE,
            anchor: Vec2::new(0.5, 0.98),
            frame_offset: Vec3::ZERO,
            footprint: default_footprint(),
            speed: 2.6,
            anim_speed: 1.0,
            anim_prefix: String::new(),
        }
    }
}

fn default_model_speed() -> f32 {
    1.0
}

fn default_scale_one() -> Vec3 {
    Vec3::ONE
}

/// A 3D glTF model entity: a node-hierarchy mesh (node-parented, not
/// vertex-skinned) drawn in the engine's world-metre space.
///
/// The animation is **self-contained**: clips live inside the `.glb` model
/// resource (`model`), `clip` names one of them, and `time`/`speed`/`repeat`/
/// `playing` drive it (no `Animator`).  The model origin sits on the terrain
/// ground point of `position`; the exported glb carries its own clearance (the
/// deployed feet touch `z = 0` at the origin), so there is no ground offset.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Model {
    /// Iso tile position `(tx, ty, z)`; the origin is placed on the terrain at
    /// `(tx, ty)` via `iso_world_pos`.
    pub position: Vec3,
    /// Per-axis scale (the model is authored in metres; `1` = real scale).
    #[serde(default = "default_scale_one")]
    pub scale: Vec3,
    /// Model resource name (a `models[]` manifest entry).
    pub model: String,
    /// Entity name of the tilemap this model stands on.
    pub tilemap: String,
    /// Clip to play, named *inside* the `.glb` (`None` = bind pose).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip: Option<String>,
    /// Clip playback speed multiplier.
    #[serde(default = "default_model_speed")]
    pub speed: f32,
    /// Whether the clip loops.
    #[serde(default)]
    pub repeat: bool,
    /// Whether the clip is advancing.
    #[serde(default)]
    pub playing: bool,
    /// Team colour (RGB), multiplied onto the albedo of the model's
    /// team-coloured texels only: those inside a mesh material's
    /// `classic_team_rect` (see [`crate::model::TEAM_RECT_EXTRA`]), which the
    /// exporter bakes neutral.  The tint is in the **same space as the
    /// texture's stored values** (sRGB-encoded colours for sRGB textures,
    /// `0..1`), not linear: like `IsoSprite.color` it multiplies the sampled
    /// texel as-is (no colour-space conversion), before lighting.
    /// `None` (the default, omitted from dumps) leaves every texel untouched,
    /// as does a model whose materials carry no team rect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tint: Option<[f32; 3]>,
    /// Multiplier on every self-lit region's `strength` (the meshes'
    /// `classic_emissive` rects, see [`crate::model::EMISSIVE_EXTRA`]): `0`
    /// switches the glow off (a powered-down building), `None` (the default,
    /// omitted from dumps) is `1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emissive: Option<f32>,
    /// Whether the model ghosts (redraws at 40% alpha) where sprites or
    /// terrain occlude it.  `false` for a deck or floor that sprites stand on
    /// (a landing pad): a box on the deck is drawn over the deck, so the deck
    /// would otherwise show through it.  A sprite's `IsoSprite.ghost_group`
    /// scopes sprite-on-sprite ghosting only; it never stops a model ghost.
    /// `true` by default (omitted from dumps).
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub ghost: bool,
    /// Clip time in seconds (transient).
    #[serde(skip)]
    pub time: f32,
    /// The animated root node's world translation in **world metres** (drift in
    /// x/y, rig-origin altitude in z), refreshed each frame by
    /// `Engine::update_models` so parented lights track the model.
    #[serde(skip)]
    pub frame_offset: Vec3,
    /// Per-node world transforms (glTF space) of the current pose, refreshed
    /// each frame and consumed by the draw path (transient).
    #[serde(skip)]
    pub node_world: Vec<Mat4>,
    /// What `node_world` + `frame_offset` were last sampled from (transient):
    /// while it still matches, `Engine::update_models` skips the re-sample.
    #[serde(skip)]
    pub pose_key: Option<crate::model::PoseKey>,
}

impl Model {
    /// A model at the bind pose with no clip.
    pub fn new(model: impl Into<String>, tilemap: impl Into<String>) -> Self {
        Self {
            position: Vec3::ZERO,
            scale: Vec3::ONE,
            model: model.into(),
            tilemap: tilemap.into(),
            clip: None,
            speed: 1.0,
            repeat: false,
            playing: false,
            tint: None,
            emissive: None,
            ghost: true,
            time: 0.0,
            frame_offset: Vec3::ZERO,
            node_world: Vec::new(),
            pose_key: None,
        }
    }
}

/// A frame-animator tied to a sprite.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Animator {
    /// Target component in "entityName.ComponentName" format.
    pub target: String,
    pub speed: f32,
    /// Animation to play (None = idle / not playing).  Serialized so ROMs can
    /// declare a starting animation in `state.json` rather than imperatively.
    #[serde(default)]
    pub animation: Option<String>,
    #[serde(skip)]
    pub counter: f32,
    #[serde(skip)]
    pub frame: f32,
    /// Visual offset selected by the current animation frame, in Blender-world
    /// metres: horizontal drift in x/y, altitude in z.
    #[serde(skip)]
    pub offset: Vec3,
    /// Whether the animation loops.  Serialized so ROMs can start one-shot
    /// animations (e.g. a rocket landing sequence).
    #[serde(default)]
    pub repeat: bool,
    /// Whether the animator is advancing.  Serialized so ROMs can start an
    /// animation from `state.json`.
    #[serde(default)]
    pub playing: bool,
}

/// An isometric wheeled vehicle: a body sprite plus four independently
/// suspended wheel sprites, lightly physically simulated (point-mass vertical
/// physics + per-wheel terrain tracking) by the host `Engine::update_vehicles`
/// system.
///
/// The component lives on the **body** entity (which also carries `IsoSprite`
/// and `Transform`) and references the four wheel entities by name.  Part
/// ground-origin anchors come from the vehicle definition sidecar (generated by
/// the Blender exporter); the host derives per-direction wheel tile offsets
/// from those anchors at spawn time, so nothing is measured by hand.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IsoVehicle {
    /// Entity name of the tilemap this vehicle drives on.
    pub tilemap: String,
    /// Wheel entity names, in `[front_left, front_right, rear_left, rear_right]`
    /// order.
    pub wheel_entities: [String; 4],
    /// Steering-tire entity names, in `[front_left, front_right, …]` order,
    /// matched to the wheels by index (a tire is the rotating disk over its
    /// wheel's static suspension arm).  Empty entries = no tire for that wheel.
    #[serde(default)]
    pub tire_entities: [String; 2],
    /// Sprite-sheet direction count (the vehicle def's `directions`: 8, 16
    /// or 24); every per-direction array below has this many entries.
    #[serde(default = "default_vehicle_directions", skip_serializing_if = "is_eight")]
    pub directions: u32,
    /// Body ground-origin anchor per direction, `[ax, ay]` normalized to the
    /// sprite frame (x from left, y from top).
    pub body_anchors: Vec<[f32; 2]>,
    /// Per-wheel ground-origin anchor per direction (`[4 wheels][dirs][ax, ay]`).
    pub wheel_anchors: [Vec<[f32; 2]>; 4],
    /// Speed in tiles per second.
    #[serde(default = "default_vehicle_speed")]
    pub speed: f32,
    /// Current sprite-sheet direction frame (`0..directions`).
    #[serde(default)]
    pub direction: u32,
    /// Continuous body pitch angle (radians), signed nose-up positive.  Driven
    /// by a spring-damper so the body bobs with momentum instead of snapping.
    #[serde(skip)]
    pub pitch: f32,
    /// Body pitch angular velocity (radians/second).
    #[serde(skip)]
    pub pitch_vel: f32,
    /// Quantized pitch frame (0..`pitch_levels`), written into the body
    /// sprite's frame index each update.
    #[serde(skip)]
    pub pitch_index: u32,
    /// Number of body pitch frames per direction (copied from the vehicle def
    /// at spawn).
    #[serde(skip)]
    pub pitch_levels: u32,
    /// Max body pitch angle (radians) the frames span (copied from the vehicle
    /// def at spawn).
    #[serde(skip)]
    pub pitch_max: f32,
    /// Front-rear axle distance in world metres (derived from the wheel
    /// offsets and tile metre length at spawn).
    #[serde(skip)]
    pub wheelbase_m: f32,
    /// Continuous body roll angle (radians), signed left-up positive.  Driven
    /// by the same spring-damper as pitch.
    #[serde(skip)]
    pub roll: f32,
    /// Body roll angular velocity (radians/second).
    #[serde(skip)]
    pub roll_vel: f32,
    /// Quantized roll frame (0..`roll_levels`), combined with pitch and
    /// direction into the body sprite's frame index.
    #[serde(skip)]
    pub roll_index: u32,
    /// Number of body roll frames per (pitch, direction) (copied from the
    /// vehicle def at spawn).
    #[serde(skip)]
    pub roll_levels: u32,
    /// Max body roll angle (radians) the frames span (copied from the vehicle
    /// def at spawn).
    #[serde(skip)]
    pub roll_max: f32,
    /// Left-right axle distance in world metres (derived from the wheel
    /// offsets and tile metre length at spawn).
    #[serde(skip)]
    pub track_m: f32,
    /// Collision footprint: integer tile offsets from the body anchor cell
    /// (copied from the vehicle def at spawn, else the body's axis box).  The
    /// vehicle search erodes the nav grid by it (issue #35) unless
    /// [`Self::path_body`] is set; selection boxes read it either way.
    #[serde(skip)]
    pub path_footprint: Vec<(i32, i32)>,
    /// The body rectangle (tiles, chassis frame) the vehicle search turns to
    /// each move's heading; `None` = search with [`Self::path_footprint`].
    #[serde(skip)]
    pub path_body: Option<crate::pathfinder::BodyRect>,
    /// Max heading change while driving, in radians per second (copied from the
    /// vehicle def at spawn).  Drives the bounded-turn follow controller.
    #[serde(skip)]
    pub turn_rate: f32,
    /// Max safe drop (metres) the suspension absorbs; the A* may route a
    /// downward jump within this distance (copied from the vehicle def at
    /// spawn).  `0` disables jumps.
    #[serde(skip)]
    pub safe_fall_m: f32,
    /// Max upward wheel compression (metres) above the body plane.  A wheel
    /// whose terrain rises more than this is clamped, and the body plane lifts
    /// instead.  Derived from the vehicle def at spawn (see `spawn_vehicle`).
    #[serde(skip)]
    pub wheel_travel_up: f32,
    /// Max downward wheel droop (metres) below the body plane before a wheel
    /// hangs.  Derived from the vehicle def at spawn (see `spawn_vehicle`).
    #[serde(skip)]
    pub wheel_travel_down: f32,
    /// Minimum body pitch/roll slope (radians) before the body takes a tilt
    /// frame; sub-threshold slopes are absorbed by wheel compression instead
    /// (an OpenRA-style terrain-orientation margin).  Derived from the frame
    /// quantization at spawn.
    #[serde(skip)]
    pub tilt_dead_zone: f32,
    /// Quantized steering frame (0..`steer_levels`), centre = straight, written
    /// into the front-tire sprite frames each update.
    #[serde(skip)]
    pub steer_index: u32,
    /// Number of steering frames per direction (copied from the vehicle def at
    /// spawn).
    #[serde(skip)]
    pub steer_levels: u32,
    /// Max steering angle (radians) the frames span (copied from the vehicle def
    /// at spawn).
    #[serde(skip)]
    pub steer_max: f32,
    /// Max steering-angle rate (radians/second) the follow controller integrates
    /// the steering angle at (copied from the vehicle def at spawn).
    #[serde(skip)]
    pub steer_rate: f32,
    /// Reverse speed in tiles per second (copied from the vehicle def at spawn).
    /// `0` disables reversing.
    #[serde(skip)]
    pub reverse_speed: f32,
    /// A* turn penalty (copied from the vehicle def at spawn); see `VehicleDef`.
    #[serde(skip)]
    pub turn_cost: f32,
    /// Wheel dust effect (copied from the vehicle def at spawn; settable per
    /// instance).  The engine spawns one `Emitter` per wheel, named
    /// `<body>Dust{Fl,Fr,Rl,Rr}` and parented to the body, the first frame
    /// this is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dust_effect: Option<String>,
    /// Touchdown one-shot effect (copied from the vehicle def at spawn).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub touchdown_effect: Option<String>,

    // -- transient simulation state (not serialized) ----------------------
    /// Per-wheel, per-direction tile-space offset from the body, derived from
    /// the anchors at spawn time (`[4 wheels][dirs][tx, ty]`).
    #[serde(skip)]
    pub wheel_tile_offsets: [Vec<[f32; 2]>; 4],
    /// Body height above the supporting terrain, in world metres (same units as
    /// `Engine::height_at`).
    #[serde(skip)]
    pub altitude: f32,
    /// Body vertical velocity, in metres per second.
    #[serde(skip)]
    pub vel_z: f32,
    /// Smoothed per-wheel terrain height (metres), `[fl, fr, rl, rr]`, clamped
    /// to a travel envelope around the body plane (`wheel_travel_up` /
    /// `wheel_travel_down`) so wheels never ride over the body or sink.
    #[serde(skip)]
    pub wheel_h: [f32; 4],
    /// Per-wheel smoothing velocity (metres/second).
    #[serde(skip)]
    pub wheel_v: [f32; 4],
    /// A* waypoints the host follows (guest-set via `vehicle_goto`), in integer
    /// tile coordinates.
    #[serde(skip)]
    pub path: Vec<[i32; 2]>,
    /// Index of the waypoint currently being driven toward.
    #[serde(skip)]
    pub path_idx: usize,
    /// Whether the body is airborne (off the supporting terrain).  Persisted so
    /// `vehicle_goto` can reject new paths until the wheels are back on the
    /// ground (issue #40).
    #[serde(skip)]
    pub airborne: bool,
    /// Continuous body heading (tile-space radians, `atan2(dy, dx)`).  Steered
    /// by the bounded-turn follow controller; `direction` is its 8-way
    /// quantization for sprite-frame selection (issue #35).
    #[serde(skip)]
    pub heading: f32,
    /// Continuous front-wheel steering angle (radians, positive = turn left),
    /// the *state* the tires visually track (integrated toward the demand at
    /// `steer_rate`).
    #[serde(skip)]
    pub steer: f32,
    /// Whether the vehicle is currently reversing (`true`) or driving forward.
    #[serde(skip)]
    pub reversing: bool,
    /// Ground speed (metres/second) last frame, for the dust burst term.
    #[serde(skip)]
    pub dust_speed: f32,
    // -- v2 layered vehicles (`VehicleDef` version 2) ---------------------
    /// The (namespace-qualified) vehicle def this was spawned from; empty for
    /// a v1 vehicle.
    #[serde(skip)]
    pub def_name: String,
    /// Per-layer runtime state, in def order (empty for a v1 vehicle).
    #[serde(skip)]
    pub layers: Vec<VehicleLayerState>,
    /// The four ground contacts in chassis metres (`x` forward, `y` left),
    /// `[fl, fr, rl, rr]`: when set, contacts rotate exactly with the
    /// heading instead of lerping `wheel_tile_offsets`.
    #[serde(skip)]
    pub contact_offsets_m: Vec<[f32; 2]>,
    /// Signed distance driven (metres): wheel and track spin.
    #[serde(skip)]
    pub odometer_m: f32,
    /// Accumulated heading change (radians): the left / right track and
    /// wheel differential.
    #[serde(skip)]
    pub turned: f32,
    /// Turn in place instead of arcing (tracked vehicles).
    #[serde(skip)]
    pub pivot_turn: bool,
    /// A heading to face (tile-space radians), set by `vehicle_face`: a
    /// `pivot_turn` vehicle turns to it in place at `turn_rate`, any other
    /// snaps to it on the next step.  Cleared on arrival and by any other
    /// movement command.
    #[serde(skip)]
    pub face: Option<f32>,
    /// A point (tile coordinates) to back up to in a straight line along the
    /// current heading, set by `vehicle_reverse_to`.  Cleared on arrival and
    /// by any other movement command.
    #[serde(skip)]
    pub reverse_to: Option<[f32; 2]>,
    /// A pose route's gear per waypoint (`true` = the segment into it is
    /// driven backward), as long as `path`; empty for a plain route
    /// (`vehicle_goto_pose`).
    #[serde(skip)]
    pub path_reverse: Vec<bool>,
    /// A pose route's goal heading (tile radians): the route arrives facing
    /// it (`vehicle_goto_pose`).
    #[serde(skip)]
    pub goal_heading: Option<f32>,
}

/// Runtime state of one v2 vehicle layer (`classic_core::vehicle_def`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VehicleLayerState {
    /// The layer's def name.
    pub name: String,
    /// Its sprite entity (the body for the hull).
    pub entity: String,
    /// Index into the def's `layers`.
    pub def_index: usize,
    pub visible: bool,
    /// RGBA multiplied onto the layer (`vehicle_layer_color`).
    pub color: [f32; 4],
    /// Turret yaw relative to the hull (tile radians) and its slew target.
    pub yaw: f32,
    pub yaw_target: f32,
    /// Turret gun pitch (radians, up positive).
    pub pitch: f32,
    /// A map point (tile `x, y`) the turret keeps aiming at.
    pub aim: Option<(f32, f32)>,
    /// The playing clip, if any.
    pub clip: Option<ClipPlay>,
}

/// A layer clip in playback (`vehicle_play`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClipPlay {
    pub clip: String,
    /// Seconds since the clip started.
    pub time: f32,
    /// Multiplies the clip fps; negative plays backward.
    pub speed: f32,
    /// `0` loops; `n` plays `n` times then holds the end frame.
    pub repeat: i32,
}

fn default_vehicle_speed() -> f32 {
    2.6
}

fn is_eight(v: &u32) -> bool {
    *v == 8
}

fn default_vehicle_directions() -> u32 {
    8
}

impl Default for IsoVehicle {
    fn default() -> Self {
        Self {
            tilemap: String::new(),
            wheel_entities: [String::new(), String::new(), String::new(), String::new()],
            tire_entities: [String::new(), String::new()],
            directions: 8,
            body_anchors: vec![[0.5, 0.5]; 8],
            wheel_anchors: std::array::from_fn(|_| vec![[0.5, 0.5]; 8]),
            speed: 2.6,
            direction: 0,
            pitch: 0.0,
            pitch_vel: 0.0,
            pitch_index: 0,
            pitch_levels: 1,
            pitch_max: 20.0f32.to_radians(),
            wheelbase_m: 0.0,
            roll: 0.0,
            roll_vel: 0.0,
            roll_index: 0,
            roll_levels: 1,
            roll_max: 20.0f32.to_radians(),
            track_m: 0.0,
            path_footprint: vec![(0, 0)],
            path_body: None,
            turn_rate: 720.0f32.to_radians(),
            safe_fall_m: 0.0,
            wheel_travel_up: 10.0 / 64.0,
            wheel_travel_down: 20.0 / 64.0,
            tilt_dead_zone: 0.0,
            steer_index: 0,
            steer_levels: 1,
            steer_max: 30.0f32.to_radians(),
            steer_rate: 360.0f32.to_radians(),
            reverse_speed: 1.3,
            turn_cost: 0.0,
            dust_effect: None,
            touchdown_effect: None,
            wheel_tile_offsets: std::array::from_fn(|_| vec![[0.0, 0.0]; 8]),
            altitude: 0.0,
            vel_z: 0.0,
            wheel_h: [0.0; 4],
            wheel_v: [0.0; 4],
            path: Vec::new(),
            path_idx: 0,
            airborne: false,
            heading: 0.0,
            steer: 0.0,
            reversing: false,
            dust_speed: 0.0,
            def_name: String::new(),
            layers: Vec::new(),
            contact_offsets_m: Vec::new(),
            odometer_m: 0.0,
            turned: 0.0,
            pivot_turn: false,
            face: None,
            reverse_to: None,
            path_reverse: Vec::new(),
            goal_heading: None,
        }
    }
}

/// Collision shape.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Shape {
    Circle { diameter: f32 },
    Polygon { verts: Vec<Vec3>, center: Vec3, min: Vec3, max: Vec3 },
}

/// The coordinate space a collider's geometry is authored in.
///
/// [`ColliderSpace::Screen`] colliders (UI, the mouse, the selection rubber
/// band) are already in viewport pixel space.  [`ColliderSpace::World`]
/// colliders (gameplay footprints) are in world/cartesian space and are
/// projected to screen every frame by the physics system before any query, so
/// a single screen-space quadtree serves both.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColliderSpace {
    #[default]
    Screen,
    World,
}

/// Collider component — serializable physics data (no runtime handlers).
///
/// Interaction handlers
/// (`Box<dyn FnMut>`) are *not* part of this component; they are stored on the
/// [`PhysicsProvider`](crate::collision::PhysicsProvider) keyed by PID, so the
/// component round-trips through `state.json` without baked closures.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ColliderData {
    pub shape: Shape,
    pub position: Vec3,
    pub scale: Vec3,
    pub rotation: f32,
    /// Assigned by PhysicsProvider on registration.
    pub pid: u32,
    pub consumes_click: bool,
    pub click_priority: i32,
    /// Coordinate space of `shape`/`position`/`scale`.
    #[serde(default)]
    pub space: ColliderSpace,
    /// Whether this collider's footprint blocks navigation (human + vehicle
    /// pathfinding).  Toggled at runtime by guests via
    /// `set_collider_blocks_nav`; the engine rasterizes blocking footprints
    /// into the nav grid.
    #[serde(default)]
    pub blocks_nav: bool,
}

impl ColliderData {
    pub fn new(shape: Shape) -> Self {
        Self {
            shape,
            position: Vec3::ZERO,
            scale: Vec3::ONE,
            rotation: 0.0,
            pid: 0,
            consumes_click: false,
            click_priority: 0,
            space: ColliderSpace::Screen,
            blocks_nav: false,
        }
    }

    /// A world-space collider (e.g. a gameplay footprint), projected to screen
    /// by the physics system before querying.
    pub fn world(shape: Shape) -> Self {
        let mut c = Self::new(shape);
        c.space = ColliderSpace::World;
        c
    }
}

/// UI node — the visual + layout element for retained-mode UI.
#[derive(Clone, Debug)]
pub struct UiNode {
    pub parent: Option<hecs::Entity>,
    pub children: Vec<UiChild>,
    pub size: Vec2,
    pub anchor: UiAnchor,
    pub fixed: bool,
    pub clip_children: bool,
    pub scroll_y: f32,
    pub clip_rect: Vec4,
    pub kind: UiKind,
}

impl Default for UiNode {
    fn default() -> Self {
        UiNode {
            parent: None,
            children: Vec::new(),
            size: Vec2::ZERO,
            anchor: UiAnchor::TopLeft,
            fixed: false,
            clip_children: false,
            scroll_y: 0.0,
            clip_rect: Vec4::ZERO,
            kind: UiKind::Container,
        }
    }
}

#[derive(Clone, Debug)]
pub struct UiChild {
    pub entity: hecs::Entity,
    pub self_anchor: UiAnchor,
    pub child_anchor: UiAnchor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiAnchor {
    TopLeft,
    TopCenter,
    TopRight,
    MidLeft,
    MidCenter,
    MidRight,
    BotLeft,
    BotCenter,
    BotRight,
}

#[derive(Clone, Debug, PartialEq)]
pub enum UiKind {
    Container,
    Array { vertical: bool, align: UiAlign, spacing: f32 },
    Grid { columns: u32, col_gap: f32, row_gap: f32, row_align: UiAlign },
    Padding { top: f32, right: f32, bottom: f32, left: f32 },
    Text,
    SdfText,
    Sprite,
}

impl UiKind {
    pub fn kind_str(&self) -> &'static str {
        match self {
            UiKind::Container => "container",
            UiKind::Array { .. } => "array",
            UiKind::Grid { .. } => "grid",
            UiKind::Padding { .. } => "padding",
            UiKind::Text => "text",
            UiKind::SdfText => "sdfText",
            UiKind::Sprite => "sprite",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiAlign {
    Left,
    Center,
    Right,
}

impl UiAnchor {
    /// Compute the offset of this anchor point from the top-left of a
    /// `(w, h)` box.  Y grows downward (matches the ortho projection).
    pub fn offset(&self, w: f32, h: f32) -> Vec2 {
        match self {
            UiAnchor::TopLeft => Vec2::new(0.0, 0.0),
            UiAnchor::TopCenter => Vec2::new(w / 2.0, 0.0),
            UiAnchor::TopRight => Vec2::new(w, 0.0),
            UiAnchor::MidLeft => Vec2::new(0.0, h / 2.0),
            UiAnchor::MidCenter => Vec2::new(w / 2.0, h / 2.0),
            UiAnchor::MidRight => Vec2::new(w, h / 2.0),
            UiAnchor::BotLeft => Vec2::new(0.0, h),
            UiAnchor::BotCenter => Vec2::new(w / 2.0, h),
            UiAnchor::BotRight => Vec2::new(w, h),
        }
    }
}

/// Kind of a dynamic light.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LightKind {
    #[default]
    Point,
    Spot,
}

/// A dynamic point/spot light in Blender-canonical **world space** (metres).
///
/// `kind` selects point (omnidirectional) vs spot (directional cone).  Point
/// lights ignore the spot fields (`dir`, `cone_angle`); the GPU `std140`
/// representation encodes the kind as a `cone_angle <= 0` sentinel so both
/// kinds share one layout.  Spots are declared in `scene.json` (the
/// `lighttest` ROM carries two); the guest `light_spawn` API cannot set `dir`
/// or `cone_angle`, so a guest-spawned spot degenerates to a point light.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Light {
    #[serde(default)]
    pub kind: LightKind,
    /// World-metre position (`+tx → +X`, `+ty → −Y`, +Z up) — the same
    /// Blender-canonical frame the terrain and sprite geometry live in.
    /// [`Engine::iso_to_world`] places it; lighting evaluates it directly in
    /// world space (there is no separate light space).  It is **not** the
    /// screen camera space.
    pub position: Vec3,
    /// Linear RGB colour.
    pub color: [f32; 3],
    /// Scalar multiplier applied to `color`.
    #[serde(default = "default_light_intensity")]
    pub intensity: f32,
    /// Attenuation radius in **world metres** (the same unit as `position`),
    /// uploaded verbatim by `gather_lights`.  `<= 0` disables distance falloff.
    #[serde(default = "default_light_radius")]
    pub radius: f32,
    /// Spot beam direction in world space, from the light **toward** what it
    /// lights (so a downward spot is `[0, 0, -1]`); ignored by point lights.
    #[serde(default)]
    pub dir: Vec3,
    /// Spot half-angle in radians of the full-intensity cone (the edge softens
    /// out to `1.25 ×` it); `<= 0` encodes a point (omnidirectional) light.
    #[serde(default)]
    pub cone_angle: f32,
    /// Optional parent entity name.  When set, `position` is interpreted as a
    /// world-metre offset **relative to the parent's ground point** and the
    /// light follows the parent each frame; when `None`, `position` is an
    /// absolute world-metre position.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Optional glTF node of a `Model` parent: `position` is then an offset in
    /// that node's frame (full TRS), so the light follows the node's lean and
    /// rotation, not just the rig origin.  On a vehicle parent it names a
    /// **socket** (`VehicleDef::sockets`; JSON key `socket` works too): the
    /// offset is then in the socket's frame, turning with heading, pitch,
    /// roll, turret yaw and gun pitch.
    #[serde(default, alias = "socket", skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
}

fn default_light_intensity() -> f32 {
    1.0
}

fn default_light_radius() -> f32 {
    // World metres (the legacy 200 px at `PPM_TARGET` = 64 px/m).
    3.125
}

impl Default for Light {
    fn default() -> Self {
        Self {
            kind: LightKind::Point,
            position: Vec3::ZERO,
            color: [1.0, 1.0, 1.0],
            intensity: 1.0,
            radius: default_light_radius(),
            dir: Vec3::ZERO,
            cone_angle: 0.0,
            parent: None,
            node: None,
        }
    }
}

/// A particle/VFX emitter: an instance of a named effect from the ROM's effect
/// library (see `classic_core::effects`), placed in the world like a [`Light`]
/// and driven by `emitter.*` animation channels through an
/// `Animator { target: "<entity>.Emitter" }`.
///
/// The drive fields (`rate`, `speed`, `scale`, `color`, `intensity`, `burst`)
/// are multipliers over the effect definition, so a Blender clip keys only the
/// time-varying envelope.  `burst` is a cumulative counter: every integer step
/// it crosses fires one burst of each layer (frame-drop and loop-wrap safe).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Emitter {
    /// Effect name (resolved against the loaded effect libraries).
    pub effect: String,
    /// World-metre position: absolute, or an offset from the parent's ground
    /// point (plus its animated `frame_offset`) when `parent` is set — the
    /// same convention as [`Light::position`].
    #[serde(default)]
    pub position: Vec3,
    /// Emission direction (emitter frame); `+Z` = up.
    #[serde(default = "default_emitter_dir")]
    pub dir: Vec3,
    /// Optional parent entity name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Optional glTF node of a `Model` parent: `position`/`dir` are then in
    /// that node's frame (full TRS), so the emitter follows lean and rotation.
    ///
    /// A trailing `*` makes it a prefix pattern (`"fx_nozzle_*"`): the one
    /// effect instance then spawns from **every** matching node of the
    /// parent's current glb: each node runs the effect's layers at their full
    /// rate (one plume per node), births dealt round-robin into one shared
    /// ring per layer (see [`Emitter::node_prefix`]).
    ///
    /// On a vehicle parent it names a **socket** (`VehicleDef::sockets`;
    /// JSON key `socket` works too), whose frame turns with heading, pitch,
    /// roll, turret yaw and gun pitch.
    #[serde(default, alias = "socket", skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    /// Spatial scale (spawn volume, speeds and sizes).
    #[serde(default = "default_one")]
    pub scale: f32,
    /// Spawn-rate multiplier (`emitter.rate`).
    #[serde(default = "default_one")]
    pub rate: f32,
    /// Speed multiplier (`emitter.speed`).
    #[serde(default = "default_one")]
    pub speed: f32,
    /// Colour tint (`emitter.color`).
    #[serde(default = "default_emitter_color")]
    pub color: [f32; 3],
    /// Brightness multiplier for emissive layers and light layers
    /// (`emitter.intensity`).
    #[serde(default = "default_one")]
    pub intensity: f32,
    /// Cumulative burst counter (`emitter.burst`).
    #[serde(default)]
    pub burst: f32,
    /// Master switch; a disabled emitter stops spawning but lets its live
    /// particles finish.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_emitter_dir() -> Vec3 {
    Vec3::Z
}

fn default_one() -> f32 {
    1.0
}

fn default_emitter_color() -> [f32; 3] {
    [1.0, 1.0, 1.0]
}

fn default_true() -> bool {
    true
}

fn is_true(v: &bool) -> bool {
    *v
}

impl Emitter {
    /// An enabled emitter of `effect` at an absolute world position.
    pub fn new(effect: impl Into<String>, position: Vec3) -> Self {
        Self {
            effect: effect.into(),
            position,
            dir: Vec3::Z,
            parent: None,
            node: None,
            scale: 1.0,
            rate: 1.0,
            speed: 1.0,
            color: [1.0; 3],
            intensity: 1.0,
            burst: 0.0,
            enabled: true,
        }
    }

    /// The prefix of a multi-node `node` pattern (`"fx_nozzle_*"` →
    /// `"fx_nozzle_"`), or `None` for an exact node name / no node.
    pub fn node_prefix(&self) -> Option<&str> {
        self.node.as_deref().and_then(node_pattern_prefix)
    }
}

/// The prefix of a glTF node pattern with a trailing `*` (`"fx_nozzle_*"` →
/// `Some("fx_nozzle_")`); `None` for an exact node name.
pub fn node_pattern_prefix(node: &str) -> Option<&str> {
    node.strip_suffix('*')
}
