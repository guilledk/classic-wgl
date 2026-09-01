//! # Skill: `classic-ecs`
//!
//! **Read `.claude/skills/classic-ecs/SKILL.md` before working on this module.**
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
/// one explicitly.
pub const DEFAULT_SDF_FONT: &str = "dejavusans";

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
/// The default nav-overlay tileset name.
pub const DEFAULT_NAV_TILESET: &str = "navTileset";

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
    /// Visual offset selected by the current animation frame, in iso units.
    #[serde(skip)]
    pub frame_offset: Vec3,
    /// Footprint vertices in iso tile coords: `[NE, SE, SW, NW]`.
    #[serde(default = "default_footprint")]
    pub footprint: Vec<Vec2>,
    /// Stencil ghost group id (0 = ungrouped).  Sprites sharing a non-zero id
    /// (e.g. a vehicle's body + wheels) never ghost through each other, while
    /// still ghosting through terrain and other entities.  Assignable
    /// declaratively in `state.json`; `spawn_vehicle` assigns it imperatively.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub ghost_group: u32,
    /// Per-sprite tint (RGBA) multiplied onto the sprite's albedo by the sheet
    /// shader before the Lambertian term.  Defaults to white (a no-op).
    /// Tintable assets (e.g. the grayscale shipping container cargo) set this
    /// at spawn to render the same sheet in different colours.
    #[serde(default = "default_white", skip_serializing_if = "is_white")]
    pub color: [f32; 4],
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
    /// Visual offset selected by the current animation frame, in iso units.
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
    /// Visual offset selected by the current animation frame, in iso units.
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
    /// Body ground-origin anchor per direction, `[ax, ay]` normalized to the
    /// sprite frame (x from left, y from top).
    pub body_anchors: [[f32; 2]; 8],
    /// Per-wheel ground-origin anchor per direction (`[4 wheels][8 dirs][ax, ay]`).
    pub wheel_anchors: [[[f32; 2]; 8]; 4],
    /// Speed in tiles per second.
    #[serde(default = "default_vehicle_speed")]
    pub speed: f32,
    /// Current sprite-sheet direction frame (0..7).
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
    /// Front-rear axle distance in screen pixels (derived from the wheel
    /// offsets and tile scale at spawn).
    #[serde(skip)]
    pub wheelbase_px: f32,
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
    /// Left-right axle distance in screen pixels (derived from the wheel
    /// offsets and tile scale at spawn).
    #[serde(skip)]
    pub track_px: f32,
    /// Collision footprint for pathfinding: integer tile offsets from the body
    /// anchor cell (copied from the vehicle def at spawn).  A* erodes the nav
    /// grid by this footprint before searching (issue #35).
    #[serde(skip)]
    pub path_footprint: Vec<(i32, i32)>,
    /// Max heading change while driving, in radians per second (copied from the
    /// vehicle def at spawn).  Drives the bounded-turn follow controller.
    #[serde(skip)]
    pub turn_rate: f32,
    /// Max safe drop (pixels) the suspension absorbs; the A* may route a
    /// downward jump within this distance (copied from the vehicle def at
    /// spawn).  `0` disables jumps.
    #[serde(skip)]
    pub safe_fall_px: f32,
    /// Max upward wheel compression (pixels) above the body plane.  A wheel
    /// whose terrain rises more than this is clamped, and the body plane lifts
    /// instead.  Derived from the vehicle def at spawn (see `spawn_vehicle`).
    #[serde(skip)]
    pub wheel_travel_up: f32,
    /// Max downward wheel droop (pixels) below the body plane before a wheel
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

    // -- transient simulation state (not serialized) ----------------------
    /// Per-wheel, per-direction tile-space offset from the body, derived from
    /// the anchors at spawn time (`[4 wheels][8 dirs][tx, ty]`).
    #[serde(skip)]
    pub wheel_tile_offsets: [[[f32; 2]; 8]; 4],
    /// Body height above the supporting terrain, in screen-pixel units
    /// (same units as `Engine::height_at`).
    #[serde(skip)]
    pub altitude: f32,
    /// Body vertical velocity, in pixels per second.
    #[serde(skip)]
    pub vel_z: f32,
    /// Smoothed per-wheel terrain height (pixels), `[fl, fr, rl, rr]`, clamped
    /// to a travel envelope around the body plane (`wheel_travel_up` /
    /// `wheel_travel_down`) so wheels never ride over the body or sink.
    #[serde(skip)]
    pub wheel_h: [f32; 4],
    /// Per-wheel smoothing velocity (pixels/second).
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
}

fn default_vehicle_speed() -> f32 {
    2.6
}

impl Default for IsoVehicle {
    fn default() -> Self {
        Self {
            tilemap: String::new(),
            wheel_entities: [String::new(), String::new(), String::new(), String::new()],
            tire_entities: [String::new(), String::new()],
            body_anchors: [[0.5, 0.5]; 8],
            wheel_anchors: [[[0.5, 0.5]; 8]; 4],
            speed: 2.6,
            direction: 0,
            pitch: 0.0,
            pitch_vel: 0.0,
            pitch_index: 0,
            pitch_levels: 1,
            pitch_max: 20.0f32.to_radians(),
            wheelbase_px: 0.0,
            roll: 0.0,
            roll_vel: 0.0,
            roll_index: 0,
            roll_levels: 1,
            roll_max: 20.0f32.to_radians(),
            track_px: 0.0,
            path_footprint: vec![(0, 0)],
            turn_rate: 720.0f32.to_radians(),
            safe_fall_px: 0.0,
            wheel_travel_up: 10.0,
            wheel_travel_down: 20.0,
            tilt_dead_zone: 0.0,
            steer_index: 0,
            steer_levels: 1,
            steer_max: 30.0f32.to_radians(),
            steer_rate: 360.0f32.to_radians(),
            reverse_speed: 1.3,
            turn_cost: 0.0,
            wheel_tile_offsets: [[[0.0, 0.0]; 8]; 4],
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

/// A dynamic point/spot light in the shared world space consumed by the lit
/// shaders (`sheet.frag`, `iso_tilemap.frag`).
///
/// `kind` selects point (omnidirectional) vs spot (directional cone).  Point
/// lights ignore the spot fields (`dir`, `cone_angle`); the GPU `std140`
/// representation encodes the kind as a `cone_angle <= 0` sentinel so both
/// kinds share one layout.  Spotlights are future-proofed here but not yet
/// emitted by any system.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Light {
    #[serde(default)]
    pub kind: LightKind,
    /// Light-space position (metric, +Z up).  This is the frame every lighting
    /// quantity lives in — `classic_core::math::iso_to_light_4`, i.e. the
    /// isometric yaw *without* the `diag(1, 0.5, 1)` isometric squash.  It is
    /// **not** the sheared screen space (`vWorldPos`) and not the squashed
    /// `iso_to_cartesian` space — those make `length`/`normalize`/`dot` mean
    /// something different along y.  The tilemap and sprite shaders derive
    /// `vLightPos` in this same frame.
    pub position: Vec3,
    /// Linear RGB colour.
    pub color: [f32; 3],
    /// Scalar multiplier applied to `color`.
    #[serde(default = "default_light_intensity")]
    pub intensity: f32,
    /// Attenuation radius in world units; `<= 0` disables distance falloff.
    #[serde(default = "default_light_radius")]
    pub radius: f32,
    /// Spot direction (world space); ignored by point lights.
    #[serde(default)]
    pub dir: Vec3,
    /// Spot half-angle in radians; `<= 0` encodes a point (omnidirectional) light.
    #[serde(default)]
    pub cone_angle: f32,
    /// Optional parent entity name.  When set, `position` is interpreted as a
    /// light-space offset **relative to the parent's ground point** and the
    /// light follows the parent each frame; when `None`, `position` is an
    /// absolute light-space position.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
}

fn default_light_intensity() -> f32 {
    1.0
}

fn default_light_radius() -> f32 {
    200.0
}

impl Default for Light {
    fn default() -> Self {
        Self {
            kind: LightKind::Point,
            position: Vec3::ZERO,
            color: [1.0, 1.0, 1.0],
            intensity: 1.0,
            radius: 200.0,
            dir: Vec3::ZERO,
            cone_angle: 0.0,
            parent: None,
        }
    }
}
