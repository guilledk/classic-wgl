//! # Skill: `classic-ecs`
//!
//! **Read `.claude/skills/classic-ecs/SKILL.md` before working on this module.**
//!
use glam::{Mat4, Vec2, Vec3, Vec4};
use serde::{Deserialize, Serialize};

/// Transforms position + scale, the basic renderable spatial component.
///
/// Port of `Transform` from `transforms.ts`.
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

/// Render a solid-colour rectangle.
/// Port of `Rectangle` from `transforms.ts`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RectRender {
    pub color: [f32; 4],
    pub ignore_cam: bool,
}

/// Render a sprite (single frame from a sprite sheet).
/// Port of `Sprite` from `transforms.ts`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpriteRender {
    pub position: Vec3,
    pub scale: Vec3,
    /// Texture name (resolved to an asset handle at load time).
    pub texture: String,
    pub ignore_cam: bool,
    pub frame: f32,
    pub tile_set_size: Vec2,
    pub anchor: Vec2,
}

/// Port of `SdfText` from `sdfText.ts`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SdfTextRender {
    /// SDF font atlas name (without `-sdf` suffix).
    pub atlas_name: String,
    pub color: [f32; 4],
    pub bgcolor: [f32; 4],
    pub outline_color: [f32; 4],
    pub outline_width: f32,
    pub shadow_offset: [f32; 2],
    pub shadow_color: [f32; 4],
    pub shadow_blur: f32,
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
/// Port of `Tilemap` from `isometric.ts`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
    /// Tile data (row-major, `size_x * size_y` elements), inlined as base64.
    #[serde(default, with = "crate::serde_base64::vec_u32")]
    pub data: Vec<u32>,
    /// Per-tile height data (vertex grid, `(size_x+1) * (size_y+1)` elements),
    /// inlined as base64.
    #[serde(default, with = "crate::serde_base64::vec_f32")]
    pub height_data: Vec<f32>,
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
}

fn default_iso_sel() -> Vec3 {
    Vec3::new(-1.0, -1.0, -1.0)
}

impl Tilemap {
    pub fn map_size(&self) -> [i32; 2] {
        [self.size_x, self.size_y]
    }
}

/// Navigation mesh (layered on top of a Tilemap).
/// Port of `IsometricNavMesh` from `isometric.ts`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NavMesh {
    #[serde(default)]
    pub position: Vec3,
    #[serde(default)]
    pub scale: Vec3,
    /// Entity name of the source Tilemap.
    #[serde(rename = "map")]
    pub map_entity: String,
    /// Texture name for the nav tileset (hardcoded `navTileset` in TS).
    #[serde(default)]
    pub tile_set: String,
    /// Walkability grid (row-major, `1` = walkable), inlined as base64.
    #[serde(default, with = "crate::serde_base64::vec_u32")]
    pub data: Vec<u32>,
    pub size_x: i32,
    pub size_y: i32,
}

/// An isometric sprite (billboard in iso space).
/// Port of `IsoSprite` from `isometric.ts`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IsoSprite {
    pub position: Vec3,
    pub scale: Vec3,
    pub texture: String,
    /// Entity name of the tilemap this sprite lives on.
    pub tilemap: String,
    pub frame: f32,
    pub tile_set_size: Vec2,
    /// Anchor point in [0..1] range (e.g. `[0.5, 0.98]` = centre-bottom / feet).
    pub anchor: Vec2,
    /// Footprint vertices in iso tile coords: `[NE, SE, SW, NW]`.
    #[serde(default = "default_footprint")]
    pub footprint: Vec<Vec2>,
}

fn default_footprint() -> Vec<Vec2> {
    vec![Vec2::new(0.5, -0.5), Vec2::new(0.5, 0.5), Vec2::new(-0.5, 0.5), Vec2::new(-0.5, -0.5)]
}

/// An isometric agent (pathfinding sprite).
/// Port of `IsoAgent` from `isometric.ts`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IsoAgent {
    pub position: Vec3,
    pub scale: Vec3,
    pub texture: String,
    pub tilemap: String,
    pub frame: f32,
    pub tile_set_size: Vec2,
    pub anchor: Vec2,
    #[serde(default = "default_footprint")]
    pub footprint: Vec<Vec2>,
    pub speed: f32,
    pub anim_speed: f32,
    #[serde(default)]
    pub anim_prefix: String,
    /// Internal state.
    #[serde(skip)]
    pub path: Vec<Vec2>,
    #[serde(skip)]
    pub target_index: usize,
    #[serde(skip)]
    pub delta: f32,
    #[serde(skip)]
    pub init_dist: f32,
    #[serde(skip)]
    pub direction: f32,
    #[serde(skip)]
    pub anim_index: usize,
    #[serde(skip)]
    pub state: AgentState,
}

impl Default for IsoAgent {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            scale: Vec3::ONE,
            texture: String::new(),
            tilemap: String::new(),
            frame: 0.0,
            tile_set_size: Vec2::ONE,
            anchor: Vec2::new(0.5, 0.98),
            footprint: default_footprint(),
            speed: 2.6,
            anim_speed: 1.0,
            anim_prefix: String::new(),
            path: Vec::new(),
            target_index: 1,
            delta: 0.0,
            init_dist: 0.0,
            direction: 0.0,
            anim_index: 2,
            state: AgentState::Idle,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AgentState {
    #[default]
    Idle,
    FollowPath,
}

/// A frame-animator tied to a sprite.
/// Port of `Animator` from `animator.ts`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Animator {
    /// Target component in "entityName.ComponentName" format.
    pub target: String,
    pub speed: f32,
    #[serde(skip)]
    pub animation: Option<String>,
    #[serde(skip)]
    pub counter: f32,
    #[serde(skip)]
    pub frame: f32,
    #[serde(skip)]
    pub repeat: bool,
    #[serde(skip)]
    pub playing: bool,
}

/// Collision shape.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Shape {
    Circle { diameter: f32 },
    Polygon { verts: Vec<Vec3>, center: Vec3, min: Vec3, max: Vec3 },
}

/// Collider component — serializable physics data (no runtime handlers).
///
/// Port of `Collider` from `collision.ts`.  Interaction handlers
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
        }
    }
}

/// Scene lighting state (ambient / direction / colour).  Held on a dedicated
/// `lighting` entity so it round-trips through `state.json`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LightState {
    pub ambient: [f32; 3],
    pub direction: [f32; 3],
    pub color: [f32; 3],
}

impl Default for LightState {
    fn default() -> Self {
        Self {
            ambient: [0.15, 0.15, 0.2],
            direction: [0.45, -0.35, 0.82],
            color: [1.0, 0.95, 0.85],
        }
    }
}

/// UI node — the visual + layout element for retained-mode UI.
/// Port of `UIElement`/`UIContainer`/`UIArray`/`UIPadding` from `ui.ts`.
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
