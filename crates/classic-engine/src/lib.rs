//! # classic-engine — the game engine
//!
//! God-object orchestrator.  Contains the `Engine` struct, frame lifecycle,
//! prefab init-* builders, editor tools, and the CLASSIC_TEST runner.
//!
//! `Engine`'s methods are split by concern: `lifecycle` (construction +
//! `frame`), `hooks` (callback registration + the host API), `boot_api` (ROM
//! boot and hydration), `render` (GPU rebuilds + sprite resolution) and `model`
//! (3D glTF `Model` clip playback + draw prep).
//!
//! **Skills to read before working here:**
//! - [classic-ecs](.agents/skills/classic-ecs/SKILL.md) — ECS patterns, components, update_fns
//! - [classic-ui](.agents/skills/classic-ui/SKILL.md) — UIManager, layout, collider integration
//! - [classic-physics](.agents/skills/classic-physics/SKILL.md) — click dispatch, selection, pathfinding
//! - [classic-iso](.agents/skills/classic-iso/SKILL.md) — iso coords, sprite rendering, nav mesh
//! - [classic-gfx](.agents/skills/classic-gfx/SKILL.md) — draw_*, GL state, DEPTH_TEST contract
//! - [classic-text](.agents/skills/classic-text/SKILL.md) — SdfText, glyph buffers, justify
//! - [classic-testing](.agents/skills/classic-testing/SKILL.md) — CLASSIC_TEST, golden harness
//! - [classic-debugging](.agents/skills/classic-debugging/SKILL.md) — CLASSIC_LOG, debugging playbook

pub mod boot;
pub mod boot_loader;
pub mod env_config;
pub mod golden;
pub mod inventory;
pub mod inventory_ui;
pub mod light;
pub mod selection;
pub mod shadow;
pub mod ui;
pub mod vehicle;

mod boot_api;
mod hooks;
mod lifecycle;
mod model;
mod render;

pub use classic_core::fields;

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use classic_core::collision::PhysicsProvider;
use classic_core::pathfinder;
use classic_core::types::AnimationData;
use classic_core::types::FrameTable;
use classic_core::types::SdfFontMetrics;
use classic_core::Camera;
use classic_gfx::{Gfx, GlBuffer, SpriteRegion};
use classic_platform::InputState;
use glam::{Mat4, Vec2};

type UpdateFn = Box<dyn FnMut(&mut Engine)>;

/// Screen-space drag distance below which a selection gesture is a click
/// (point-select) rather than a drag box.
const RTS_DRAG_THRESHOLD_PX: f32 = 4.0;

/// The RTS selection silhouette colour (bright green) and its outline width in
/// content pixels.
const SELECTION_COLOR: [f32; 3] = [0.25, 1.0, 0.35];
const OUTLINE_RADIUS_PX: f32 = 1.0;

/// An interaction event queued for a ROM guest.
#[derive(Clone, Debug)]
pub struct GuestEvent {
    /// 0 = click, 1 = enter (hover start), 2 = exit (hover end).
    pub kind: u32,
    /// The subscribed entity's name.
    pub name: String,
}

/// Per-entity GPU resources for a tilemap.
struct TilemapGpu {
    mesh_buf: GlBuffer,
    vertex_count: usize,
    tile_tex: glow::Texture,
}

/// Per-texture depth-mask metadata, keyed by the color texture name.  When a
/// manifest texture declares a `depth` map, the engine uploads it under
/// `depth_tex`; the sheet stores the camera view depth directly (window
/// `[0, 1]`), so the render loop writes it to `gl_FragDepth` as-is.
#[derive(Clone, Debug)]
struct TextureDepth {
    depth_tex: String,
}

/// A pending GPU-compressed (`.basis`) texture upload: one unique `src` sheet
/// plus every manifest entry key that aliases it.  Collected by
/// `begin_boot` and uploaded in a second phase (synchronously on native,
/// awaited through the web transcoder worker on wasm).  Lives in `boot` so the
/// plan can own it.
use boot::BasisTextureJob;

/// Packed-atlas UV draw params: `(uv_rect, trim_offset, source_size, content_size)`.
type IsoUv = ([f32; 4], [f32; 2], [f32; 2], [f32; 2]);

/// Precomputed per-sprite draw parameters for the isometric normal + ghost
/// passes, so both passes share one model/depth computation per frame.
struct IsoDraw {
    order: f32,
    name: String,
    model: Mat4,
    texture: String,
    frame: f32,
    tile_set_size: [f32; 2],
    /// Packed-atlas UV params, or `None` for the uniform-grid path.
    uv: Option<IsoUv>,
    depth_corners: [f32; 4],
    /// The sprite's ground-anchor window depth `[0, 1]` (the depth-map gray is
    /// baked origin-relative, so the fragment offsets it by this anchor depth).
    depth_base: f32,
    depth_map: Option<String>,
    normal_map: Option<String>,
    ghost_group: u32,
    color: [f32; 4],
    /// Whether the sprite is currently RTS-selected (draws a silhouette edge).
    selected: bool,
    /// World -> camera view space (metres): `iso_camera_matrix`.
    world_matrix: Mat4,
}

impl IsoDraw {
    /// The texture region this draw addresses: the packed-atlas UV rect when a
    /// frame was resolved, else the uniform-grid frame fallback.
    fn region(&self) -> SpriteRegion<'_> {
        match &self.uv {
            Some((uv_rect, trim_offset, source_size, content_size)) => {
                SpriteRegion::Uv { uv_rect, trim_offset, source_size, content_size }
            }
            None => SpriteRegion::Grid { frame: self.frame, tile_set_size: self.tile_set_size },
        }
    }
}

struct SdfTextGpu {
    glyph_buf: GlBuffer,
    vertex_count: usize,
    text_width: f32,
    text_height: f32,
    last_text: String,
    last_scale: f32,
}

/// A frame resolved through a packed-atlas frame table.
pub(crate) struct ResolvedFrame {
    pub(crate) sheet_name: String,
    pub(crate) uv_rect: [f32; 4],
    /// Content pixel size (frame rect w/h).
    pub(crate) size: [f32; 2],
    /// Untrimmed source cell size (0 = unknown).
    pub(crate) source_size: [u32; 2],
    /// Offset of the trimmed content within the source cell.
    pub(crate) trim_offset: [i32; 2],
    /// Optional packer-provided anchor, already in trimmed-frame `[0..1]`.
    pub(crate) anchor: Option<[f32; 2]>,
    /// Per-sheet normal-map GL texture name (`"{sheet_name}-normal"`), set when
    /// the frame's sheet declares a `normal` companion.
    pub(crate) normal_tex: Option<String>,
    /// Per-sheet depth-map GL texture name, set when the frame's sheet declares
    /// a `depth` companion (stores camera view depth directly).
    pub(crate) depth_tex: Option<String>,
}

/// The namespace-resolvable resource categories, mirroring the guest SDK's
/// `has_resource` kinds plus the frame-table and vehicle registries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceKind {
    Texture,
    Font,
    Animation,
    FrameTable,
    Vehicle,
    /// A 3D glTF model (`models[]`), referenced by `Model.model`.
    Model,
}

pub struct Engine {
    pub gfx: Option<Gfx>,
    /// Whether the full builtin shader catalog has been compiled into `gfx`.
    /// `init_gfx` (the pre-boot loading screen) compiles only the `solid` +
    /// `sdf` shaders and leaves this `false`, so [`Engine::ensure_gfx`] still
    /// runs to compile the full catalog (with manifest overrides) once a ROM
    /// manifest is available.
    gfx_full: bool,
    pub world: hecs::World,
    pub camera: Camera,
    pub time: Time,
    pub names: HashMap<String, hecs::Entity>,
    pub name_order: Vec<String>,
    /// Namespace prefix for the loaded ROM's entities (empty = global names).
    /// Groundwork for multi-ROM loading: when non-empty, `entity_key` qualifies
    /// names as `"{namespace}::{name}"` so several ROMs can coexist.
    pub namespace: String,
    pub physics: PhysicsProvider,
    /// Collider pid → entity name, populated by `register_named_collider` so the
    /// guest `pick_at` can resolve a screen point to a gameplay entity.
    collider_names: HashMap<u32, String>,
    /// Entity name → collider pid (the reverse of `collider_names`), so the
    /// engine can update a named entity's collider in place instead of
    /// re-registering it every frame.
    collider_pids: HashMap<String, u32>,
    /// Names whose collider is owned by `sync_selectable_colliders` (world-space
    /// footprint colliders), so the disabled-cleanup pass can disable stale ones
    /// without touching screen-space `spawn_collider` colliders.
    selectable_colliders: HashSet<String>,
    /// The host-owned RTS selection set (see `selection.rs`).
    pub selection: selection::SelectionSet,
    /// Active RTS drag-box rubber band, `Some((begin, end))` in screen space
    /// while dragging, `None` otherwise.
    pub rts_box: Option<(Vec2, Vec2)>,
    /// Entity names the guest has subscribed to for interaction events.
    subscribed: HashSet<String>,
    /// Events queued for the guest, drained via `poll_event`.
    guest_events: VecDeque<GuestEvent>,
    /// The subscribed entity currently under the mouse (for enter/exit).
    guest_hover: Option<String>,
    /// Host-provided boolean flags exposed to ROM guests (e.g. `agent_selected`,
    /// `ui_consumed_click`).  Demo content writes these; the guest SDK reads
    /// them back through the generic `agent_selected`/`ui_consumed_click`
    /// imports.
    pub guest_flags: HashMap<String, bool>,
    pub scroll_speed: f32,
    pub input: InputState,
    pub show_grid: bool,
    pub light_ambient: [f32; 3],
    pub light_dir: [f32; 3],
    pub light_color: [f32; 3],
    /// Dynamic light handle table (point/spot lights beyond the sun term).
    /// Each handle maps to a spawned `Light` ECS entity; the active set is
    /// gathered (resolving parent attachments) and uploaded to the `LightBlock`
    /// UBO once per frame.
    pub light_handles: light::LightHandles,
    pub animations: HashMap<String, AnimationData>,
    /// Packed-atlas frame tables keyed by texture name, loaded from the ROM's
    /// `frames` resources at boot (issue #45).  A sprite with `frame_name` set
    /// resolves its frame through the table for the owning texture.
    pub frame_tables: HashMap<String, FrameTable>,
    pub sdf_fonts: HashMap<String, SdfFontMetrics>,
    /// Per-texture depth-mask metadata keyed by color texture name (loaded
    /// from the manifest's `depth` field).
    texture_depths: HashMap<String, TextureDepth>,
    /// Per-texture normal-map texture name keyed by color texture name (loaded
    /// from the manifest's `normal` field).  Sprites with a normal map are
    /// shaded with a runtime Lambertian term.
    texture_normals: HashMap<String, String>,
    /// Qualified texture names registered by `load_manifest_resources`, kept
    /// GL-free so texture existence + namespace resolution work without a
    /// `Gfx` (the unit-test path).  Union across every loaded ROM.
    pub texture_names: HashSet<String>,
    /// Wheeled-vehicle definitions keyed by name, loaded from the ROM's
    /// `vehicles` resources at boot.
    pub vehicles: HashMap<String, classic_core::types::VehicleDef>,
    /// Blender-exported vehicle anchors data artifacts keyed by name, loaded
    /// from the ROM's `data` resources (referenced by `VehicleDef::anchors`).
    pub vehicle_anchors: HashMap<String, classic_core::types::VehicleAnchors>,
    /// Parsed 3D glTF models keyed by (qualified) name, loaded from the ROM's
    /// `models` resources at boot: node hierarchy + clips + CPU mesh data (the
    /// embedded texture pixels are released once uploaded to GL).
    pub models: HashMap<String, classic_core::model::ModelAsset>,
    /// GPU model meshes keyed by `"{model}::{mesh_index}"`, uploaded from
    /// [`Self::models`] when a `Gfx` context is present.
    model_gpu: HashMap<String, classic_gfx::ModelMeshGpu>,
    /// The ROM-namespaced item catalog, interned once at `load_rom`.  Read-only
    /// after load; the inventory mechanics look items up by [`ItemId`].
    pub items: classic_core::inventory::ItemRegistry,
    /// Next per-instance stencil ghost-group id handed out by `spawn_vehicle`
    /// (1..=255; 0 is reserved for ungrouped sprites).
    next_ghost_group: u32,
    /// ROM manifest (raw + parsed) and resources, captured by `load_rom` so
    /// `dump_rom` can reconstruct a [`classic_rom::Rom`] with the current state.
    pub rom_manifest_json: Option<String>,
    pub rom_manifest: Option<classic_rom::RomManifest>,
    pub rom_resources: Option<classic_rom::ResourceSet>,
    /// The full multi-ROM dependency DAG captured by `load_roms` (topological
    /// order, deps first).  `dump_roms` reconstructs the DAG from this; the
    /// single-ROM legacy path records exactly one entry.
    pub loaded_roms: Vec<classic_rom::LoadedRom>,
    pub ui: Option<ui::UIManager>,
    pub selection_mode: i32,
    pub selection_begin_screen: glam::Vec3,
    /// Height scale the tilemap mesh was built with, before the height
    /// widget's multiplier.  Recorded so the widget can scale relative to it
    /// instead of assuming `tile_pixel_size[0]`, which is wrong for any scene
    /// that overrides the scale (see [`Engine::commit_terrain`]).
    pub base_height_scale: f32,
    /// Height difference between adjacent tiles above which `sync_nav_heights`
    /// marks a tile impassable.  The flat demo map edits heights in integer
    /// steps, hence the default of 2.0; generated terrain is continuous and
    /// needs a much finer threshold to match the slope rule it was built with.
    pub nav_slope_threshold: f32,
    /// Immutable nav-grid snapshot shared with the pathfinding worker.
    nav_snapshot: Arc<pathfinder::NavSnapshot>,
    /// Monotonic counter bumped each time `nav_snapshot` is rebuilt.
    nav_version: u64,
    /// Force pathfinding to run synchronously (deterministic test harness).
    synchronous_workers: bool,
    /// Next path-request id handed to a guest.  Shared by the humanoid
    /// `request_path` and vehicle `vehicle_goto` (the worker's result map is a
    /// single `PathId` namespace), starting at 1 so a vehicle id is always `> 0`
    /// (the ABI's "airborne" code is `0`).
    next_path_id: u64,
    /// Pathfinding worker (spawned lazily on first request): a background
    /// native thread or web `Worker`, or an inline queue under
    /// `synchronous_workers`.
    pathfinder: Option<classic_worker::PathfinderWorker>,
    /// Immutable vehicle nav snapshot (structural nav + heights) shared with
    /// the pathfinding worker.
    vehicle_nav_snapshot: Arc<pathfinder::VehicleNavSnapshot>,
    /// Vehicle entity for each in-flight vehicle path request id.
    vehicle_path_entities: HashMap<u64, hecs::Entity>,
    /// Candidate vehicle path waypoints, keyed by vehicle name, computed by the
    /// non-mutating `vehicle_probe` and drawn by the demo overlay as a preview.
    pub preview_paths: HashMap<String, Vec<[i32; 2]>>,
    /// The single in-flight (or cached) vehicle reachability probe, if any.
    preview_probe: Option<vehicle::PreviewProbe>,
    /// Next background-task id handed to a guest.
    next_task_id: u64,
    /// Background guest worker (Tier 3): a second `.wasm` instance running pure
    /// guest entry points off-thread (installed by the demo layer, which owns
    /// the worker module bytes).
    guest_worker: Option<classic_worker::GuestWorker>,
    /// Host-owned named-field registry (grid kernels operate over these).
    pub fields: fields::FieldRegistry,
    /// Host-owned container-inventory tooltip renderer (hover target + icon/
    /// amount overlay).  Drives the overlay drawn each frame in `frame()`.
    pub inventory_ui: inventory_ui::InventoryUi,
    nav_gpu: Option<TilemapGpu>,
    debug_frame: u64,
    pre_update_hooks: Vec<UpdateFn>,
    selection_end_hooks: Vec<UpdateFn>,
    overlay_hooks: Vec<UpdateFn>,
    test_runner: Option<UpdateFn>,
    pub test_should_close: bool,
    pub test_failed: bool,
    pub golden_capture_frame: u64,
    update_fns: Vec<UpdateFn>,
    #[allow(dead_code)]
    trace: Option<golden::TraceCollector>,
    tilemap_gpu: HashMap<String, TilemapGpu>,
    sdf_text_gpu: HashMap<hecs::Entity, SdfTextGpu>,
    last_vw: f32,
    last_vh: f32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Time {
    pub prev: f64,
    pub delta: f32,
    pub fps: u32,
    pub elapsed: f64,
}

impl Time {
    pub fn tick(&mut self, now_secs: f64) {
        self.delta = (now_secs - self.prev) as f32;
        self.prev = now_secs;
        self.elapsed += self.delta as f64;
        if self.delta > 0.0 {
            self.fps = (1.0 / self.delta) as u32;
        }
    }
}

pub enum DrawKind {
    Sprite,
    Tilemap,
    IsoSprite,
    /// A 3D glTF model (`Model`): real geometry in the depth-tested world pass.
    Model,
    UiRect,
    UiSprite,
    SdfText,
}

#[cfg(test)]
mod tests {
    use super::*;
    use classic_core::components::{
        Animator, IsoSprite, Light, LightKind, NavMesh, Role, SdfTextRender, Tilemap,
    };
    use classic_core::math::{iso_view_depth, iso_world_pos, DEPTH_FAR, DEPTH_NEAR};
    use classic_core::tilemap::sample_height_mesh;
    use classic_core::{RoleKind, SpriteRender, Transform};
    use glam::Vec3;

    #[test]
    fn animation_offsets_map_altitude_to_screen_y() {
        classic_core::register_all_components();
        let mut engine = Engine::new();
        engine.animations.insert(
            "rocketLanding".to_string(),
            AnimationData {
                name: "rocketLanding".to_string(),
                src: "rocketLanding".to_string(),
                rate: 24.0,
                sequence: vec![0, 1],
                offsets: vec![],
                offset_keyframes: vec![],
                channels: vec![],
                metadata: None,
            },
        );

        // Little-endian: u32 frame_count, f32 ppm, then triples of [x, y, z].
        let mut metadata = Vec::new();
        metadata.extend_from_slice(&3u32.to_le_bytes());
        metadata.extend_from_slice(&8.0f32.to_le_bytes());
        for (x, y, z) in
            [(0.0f32, 0.0f32, 50.0f32), (-1.0f32, 0.5f32, 10.0f32), (0.0f32, 0.0f32, 0.0f32)]
        {
            metadata.extend_from_slice(&x.to_le_bytes());
            metadata.extend_from_slice(&y.to_le_bytes());
            metadata.extend_from_slice(&z.to_le_bytes());
        }
        engine.load_animation_offsets("rocketLanding", &metadata);

        let offsets = &engine.animations["rocketLanding"].offsets;
        assert_eq!(offsets.len(), 3);

        // Frame 0: 50 m altitude lands in z (the `ppm` field is now ignored).
        assert!((offsets[0][2] - 50.0).abs() < 0.001, "got {:?}", offsets[0]);
        // Drift x/y map verbatim (metres).
        assert!((offsets[1][0] - (-1.0)).abs() < 0.001, "got {:?}", offsets[1]);
        assert!((offsets[1][1] - 0.5).abs() < 0.001, "got {:?}", offsets[1]);
        // Altitude 0 → zero vertical offset.
        assert!((offsets[2][2]).abs() < 0.001, "got {:?}", offsets[2]);
    }

    #[test]
    fn animation_offsets_parse_sparse_keyframes() {
        classic_core::register_all_components();
        let mut engine = Engine::new();
        engine.animations.insert(
            "rocketLanding".to_string(),
            AnimationData {
                name: "rocketLanding".to_string(),
                src: "rocketLanding".to_string(),
                rate: 24.0,
                sequence: vec![0, 1],
                offsets: vec![],
                offset_keyframes: vec![],
                channels: vec![],
                metadata: None,
            },
        );

        // Sparse blob: magic b"KAOS", u8 version=1, u32 count, f32 ppm, then
        // count × (u32 frame, f32 x, f32 y, f32 z).
        let mut metadata = Vec::new();
        metadata.extend_from_slice(b"KAOS");
        metadata.push(1u8);
        metadata.extend_from_slice(&2u32.to_le_bytes());
        metadata.extend_from_slice(&64.0f32.to_le_bytes());
        for (frame, x, y, z) in [(0u32, 0.0f32, 0.0f32, 50.0f32), (240, 0.0, 0.0, 0.0)] {
            metadata.extend_from_slice(&frame.to_le_bytes());
            metadata.extend_from_slice(&x.to_le_bytes());
            metadata.extend_from_slice(&y.to_le_bytes());
            metadata.extend_from_slice(&z.to_le_bytes());
        }
        engine.load_animation_offsets("rocketLanding", &metadata);

        let anim = &engine.animations["rocketLanding"];
        assert!(anim.offsets.is_empty(), "sparse blob must not fill `offsets`");
        assert_eq!(anim.offset_keyframes.len(), 2);
        // Frame 0: 50 m altitude lands in z (metres).
        assert_eq!(anim.offset_keyframes[0].frame, 0);
        assert!(
            (anim.offset_keyframes[0].offset[2] - 50.0).abs() < 0.01,
            "got {:?}",
            anim.offset_keyframes[0]
        );
        // Frame 240: touchdown → zero altitude.
        assert_eq!(anim.offset_keyframes[1].frame, 240);
        assert!(anim.offset_keyframes[1].offset[2].abs() < 0.01);
    }

    #[test]
    fn effective_anchor_translates_trim() {
        // Untrimmed frame: source == content, no offset → anchor unchanged.
        let full = ResolvedFrame {
            sheet_name: "a".into(),
            uv_rect: [0.0; 4],
            size: [64.0, 64.0],
            source_size: [64, 64],
            trim_offset: [0, 0],
            anchor: None,
            normal_tex: None,
            depth_tex: None,
        };
        let a = Engine::effective_anchor(Vec2::new(0.5, 0.5), &full);
        assert!((a.x - 0.5).abs() < 1e-6 && (a.y - 0.5).abs() < 1e-6, "got {a:?}");

        // Trimmed frame: anchor is a [0..1] ratio of the source cell, so it
        // shifts within the trimmed content.
        let trimmed = ResolvedFrame {
            sheet_name: "a".into(),
            uv_rect: [0.0; 4],
            size: [466.0, 772.0],
            source_size: [512, 928],
            trim_offset: [8, 62],
            anchor: None,
            normal_tex: None,
            depth_tex: None,
        };
        let a = Engine::effective_anchor(Vec2::new(0.5, 0.5), &trimmed);
        assert!((a.x - (0.5 * 512.0 - 8.0) / 466.0).abs() < 1e-6, "got {a:?}");
        assert!((a.y - (0.5 * 928.0 - 62.0) / 772.0).abs() < 1e-6, "got {a:?}");

        // Packer-provided anchor (already in trimmed space) wins.
        let packed = ResolvedFrame {
            sheet_name: "a".into(),
            uv_rect: [0.0; 4],
            size: [466.0, 772.0],
            source_size: [512, 928],
            trim_offset: [8, 62],
            anchor: Some([0.25, 0.75]),
            normal_tex: None,
            depth_tex: None,
        };
        let a = Engine::effective_anchor(Vec2::new(0.5, 0.5), &packed);
        assert!((a.x - 0.25).abs() < 1e-6 && (a.y - 0.75).abs() < 1e-6, "got {a:?}");

        // Unknown source size → no translation.
        let unknown = ResolvedFrame {
            sheet_name: "a".into(),
            uv_rect: [0.0; 4],
            size: [466.0, 772.0],
            source_size: [0, 0],
            trim_offset: [0, 0],
            anchor: None,
            normal_tex: None,
            depth_tex: None,
        };
        let a = Engine::effective_anchor(Vec2::new(0.5, 0.5), &unknown);
        assert!((a.x - 0.5).abs() < 1e-6 && (a.y - 0.5).abs() < 1e-6, "got {a:?}");
    }

    #[test]
    fn world_depth_matches_camera_view_depth() {
        // The window-space depth must be the camera view depth normalised over
        // `DEPTH_NEAR`/`DEPTH_FAR`: `(DEPTH_NEAR - dot(back, world)) / span`.
        let pos = glam::Vec3::new(100.0, 20.0, 64.0);
        let world = iso_world_pos(pos.x, pos.y, pos.z);
        let expected = (DEPTH_NEAR - iso_view_depth(world)) / (DEPTH_NEAR - DEPTH_FAR);
        assert!((Engine::world_depth(world) - expected).abs() < 1e-9);
    }

    #[test]
    fn iso_to_world_lifts_elevation_above_terrain() {
        classic_core::register_all_components();
        let mut engine = Engine::new_for_test();

        // A flat 4x4 tilemap at the origin with a uniform 1-metre plateau.
        let tilemap = Tilemap {
            position: Vec3::ZERO,
            scale: Vec3::new(45.0, 45.0, 1.0),
            size_x: 4,
            size_y: 4,
            tile_set: "tileset".into(),
            tile_pixel_size: [32, 32],
            max_tile: 16,
            tiles_grid: None,
            heights_grid: None,
            data: vec![0u32; 16],
            height_data: vec![1.0f32; 25],
            height_scale: 64.0,
            tile_set_pixel_size: [0, 0],
            tiles_per_row: 0,
            mouse_iso_pos: Vec3::ZERO,
            selection_iso_begin: Vec3::new(-1.0, -1.0, -1.0),
            selection_iso_end: Vec3::new(-1.0, -1.0, -1.0),
        };
        let entity = engine.world.spawn((
            tilemap,
            Transform::new(Vec3::ZERO, Vec3::new(45.0, 45.0, 1.0)),
            Role::new(RoleKind::Tilemap),
        ));
        engine.names.insert("tilemap".into(), entity);

        // Ground level: the point sits at the 1 m surface → z = 1 (metres).
        let ground = engine.iso_to_world(0.0, 0.0, 0.0).unwrap();
        assert!((ground.z - 1.0).abs() < 1e-3, "got z {}", ground.z);

        // 2 m above: z = 3.  Elevation is carried in z alone — world space is
        // +Z up, so raising a light must not move it in x or y.
        let raised = engine.iso_to_world(0.0, 0.0, 2.0).unwrap();
        assert!((raised.z - 3.0).abs() < 1e-3, "got z {}", raised.z);
        assert!((ground.y - raised.y).abs() < 1e-4, "y should not move");
        assert!((ground.x - raised.x).abs() < 1e-4, "x should not move");

        // Without a Tilemap-role entity, the mapping is unavailable.
        let empty = Engine::new_for_test();
        assert!(empty.iso_to_world(0.0, 0.0, 0.0).is_none());
    }

    /// The world-quad sprite model must place the ground anchor at
    /// `iso_world_pos(x, y, h + altitude) + drift + tilemap.position`: the
    /// `frame_offset` drift in world x/y, the altitude in world z.
    #[test]
    fn sprite_world_model_places_drift_and_altitude_in_world_metres() {
        let tilemap = Tilemap {
            position: Vec3::ZERO,
            scale: Vec3::new(45.0, 45.0, 1.0),
            size_x: 4,
            size_y: 4,
            tile_set: "tileset".into(),
            tile_pixel_size: [32, 32],
            max_tile: 16,
            tiles_grid: None,
            heights_grid: None,
            data: vec![0u32; 16],
            height_data: vec![2.0f32; 25], // 2-metre plateau
            height_scale: 64.0,
            tile_set_pixel_size: [0, 0],
            tiles_per_row: 0,
            mouse_iso_pos: Vec3::ZERO,
            selection_iso_begin: Vec3::new(-1.0, -1.0, -1.0),
            selection_iso_end: Vec3::new(-1.0, -1.0, -1.0),
        };
        let tilemap_tf = Transform::new(Vec3::ZERO, Vec3::new(45.0, 45.0, 1.0));

        // World-metre frame offset (drift x/y, altitude z) — the step-C encoding.
        let frame_offset = Vec3::new(0.25, -0.125, 0.5);
        let iso_sprite = IsoSprite {
            position: Vec3::new(3.5, 2.5, 0.0),
            scale: Vec3::new(1.5, 2.0, 1.0),
            texture: "tex".into(),
            tilemap: "tilemap".into(),
            frame: 0.0,
            frame_name: None,
            tile_set_size: Vec2::new(8.0, 8.0),
            anchor: Vec2::new(0.5, 0.98),
            frame_offset,
            footprint: vec![
                Vec2::new(0.5, -0.5),
                Vec2::new(0.5, 0.5),
                Vec2::new(-0.5, 0.5),
                Vec2::new(-0.5, -0.5),
            ],
            ghost_group: 0,
            color: [1.0, 1.0, 1.0, 1.0],
        };
        let sprite_tf = Transform::new(iso_sprite.position, iso_sprite.scale);
        let tex_dim = (48.0, 96.0);
        let anchor_px = Vec2::new(24.0, 94.0);

        let model = Engine::compute_iso_sprite_model(
            &iso_sprite,
            &sprite_tf,
            &tilemap_tf,
            &tilemap,
            tex_dim,
            anchor_px,
        );

        // The ground anchor (the quad point at the anchor UV) lands at the
        // world-metre ground position: terrain height + altitude in z, drift
        // in x/y.
        let h = sample_height_mesh(
            &tilemap.height_data,
            tilemap.size_x,
            tilemap.size_y,
            sprite_tf.position.x,
            sprite_tf.position.y,
        );
        let expected_anchor =
            iso_world_pos(sprite_tf.position.x, sprite_tf.position.y, h + frame_offset.z)
                + Vec3::new(frame_offset.x, frame_offset.y, 0.0)
                + tilemap_tf.position;

        let ua = anchor_px.x / tex_dim.0;
        let wa = anchor_px.y / tex_dim.1;
        let anchor_world = model.transform_point3(Vec3::new(ua, wa, 0.0));
        assert!(
            (anchor_world - expected_anchor).length() < 1e-3,
            "anchor {anchor_world:?} != expected {expected_anchor:?}"
        );
    }

    fn push_channel(v: &mut Vec<u8>, name: &str, component: u8, keys: &[(u32, &[f32])]) {
        v.push(name.len() as u8);
        v.extend_from_slice(name.as_bytes());
        v.push(component);
        v.extend_from_slice(&(keys.len() as u32).to_le_bytes());
        for (frame, vals) in keys {
            v.extend_from_slice(&frame.to_le_bytes());
            for val in *vals {
                v.extend_from_slice(&val.to_le_bytes());
            }
        }
    }

    #[test]
    fn animation_channels_parse_and_interpolate() {
        classic_core::register_all_components();
        let mut engine = Engine::new();
        engine.animations.insert(
            "rocketLanding".to_string(),
            AnimationData {
                name: "rocketLanding".to_string(),
                src: "rocketLanding".to_string(),
                rate: 24.0,
                sequence: vec![0, 1],
                offsets: vec![],
                offset_keyframes: vec![],
                channels: vec![],
                metadata: None,
            },
        );

        // Unified blob: magic b"KACH", u8 version=1, f32 ppm, u32 channel_count.
        let mut blob = Vec::new();
        blob.extend_from_slice(b"KACH");
        blob.push(1u8);
        blob.extend_from_slice(&64.0f32.to_le_bytes());
        blob.extend_from_slice(&2u32.to_le_bytes());
        push_channel(&mut blob, "offset", 3, &[(0, &[0.0, 0.0, 50.0]), (240, &[0.0, 0.0, 0.0])]);
        push_channel(&mut blob, "light.intensity", 1, &[(0, &[0.0]), (120, &[1.0]), (240, &[0.0])]);
        engine.load_animation_channels("rocketLanding", &blob);

        let anim = &engine.animations["rocketLanding"];
        assert_eq!(anim.channels.len(), 2);
        assert_eq!(anim.channels[0].name, "offset");
        assert_eq!(anim.channels[1].name, "light.intensity");

        // The `offset` channel is folded into `offset_keyframes` (world metres:
        // drift x/y, altitude z).
        assert_eq!(anim.offset_keyframes.len(), 2);
        assert_eq!(anim.offset_keyframes[0].offset[2], 50.0);

        // Interpolation + clamping of a scalar channel.
        assert_eq!(anim.channel_sample("light.intensity", 60.0), Some(vec![0.5]));
        assert_eq!(anim.channel_sample("light.intensity", 300.0), Some(vec![0.0]));
        assert_eq!(anim.channel_sample("light.intensity", -5.0), Some(vec![0.0]));
        assert_eq!(anim.channel_sample("missing", 0.0), None);
    }

    #[test]
    fn parented_light_resolves_to_world() {
        classic_core::register_all_components();
        let mut engine = Engine::new_for_test();

        let tilemap = Tilemap {
            position: Vec3::ZERO,
            scale: Vec3::new(45.0, 45.0, 1.0),
            size_x: 4,
            size_y: 4,
            tile_set: "tileset".into(),
            tile_pixel_size: [32, 32],
            max_tile: 16,
            tiles_grid: None,
            heights_grid: None,
            data: vec![0u32; 16],
            height_data: vec![1.0f32; 25],
            height_scale: 64.0,
            tile_set_pixel_size: [0, 0],
            tiles_per_row: 0,
            mouse_iso_pos: Vec3::ZERO,
            selection_iso_begin: Vec3::new(-1.0, -1.0, -1.0),
            selection_iso_end: Vec3::new(-1.0, -1.0, -1.0),
        };
        let tm = engine.world.spawn((
            tilemap,
            Transform::new(Vec3::ZERO, Vec3::new(45.0, 45.0, 1.0)),
            Role::new(RoleKind::Tilemap),
        ));
        engine.names.insert("tilemap".into(), tm);

        let parent_tile = Vec3::new(2.0, 1.0, 0.0);
        let parent = engine.world.spawn((Transform::new(parent_tile, Vec3::ONE),));
        engine.names.insert("rocket".into(), parent);

        let local = Vec3::new(3.0, -4.0, 12.0);
        let light = Light {
            kind: LightKind::Point,
            position: local,
            color: [1.0, 0.6, 0.2],
            intensity: 2.0,
            radius: 150.0,
            dir: Vec3::ZERO,
            cone_angle: 0.0,
            parent: Some("rocket".into()),
        };
        engine.world.spawn((light.clone(),));

        let gathered = engine.gather_lights();
        assert_eq!(gathered.len(), 1);

        let base = engine.iso_to_world(parent_tile.x, parent_tile.y, 0.0).unwrap();
        let expected = base + local;
        assert!((gathered[0].position - expected).length() < 1e-2);

        // An unparented light stays at its authored (world) position.
        let mut engine = Engine::new_for_test();
        let world_pos = Vec3::new(50.0, 60.0, 70.0);
        engine.world.spawn((Light { position: world_pos, parent: None, ..light.clone() },));
        let gathered = engine.gather_lights();
        assert_eq!(gathered.len(), 1);
        assert!((gathered[0].position - world_pos).length() < 1e-2);
        // `radius` is world metres, uploaded verbatim.
        assert_eq!(gathered[0].radius, light.radius);

        // A dangling parent must *not* silently turn the offset into an
        // absolute position — the light is dropped (and a warning logged).
        let mut engine = Engine::new_for_test();
        engine.world.spawn((Light {
            position: world_pos,
            parent: Some("ghost".into()),
            ..light.clone()
        },));
        assert_eq!(engine.gather_lights().len(), 0, "dangling parent must skip the light");
    }

    #[test]
    fn hidden_light_or_parent_is_skipped() {
        // A hidden (`Disabled`) parent darkens its attached light — the lunar
        // guest hides the rocket between cycles, and its burn light must not
        // linger at the last launch values — and a hidden light is skipped
        // too.  Re-enabling restores both.
        let mut engine = Engine::new_for_test();
        let tilemap = Tilemap {
            position: Vec3::ZERO,
            scale: Vec3::ONE,
            size_x: 8,
            size_y: 8,
            tile_set: "tileset".into(),
            tile_pixel_size: [32, 32],
            max_tile: 16,
            tiles_grid: None,
            heights_grid: None,
            data: vec![0u32; 64],
            height_data: vec![0.0f32; 81],
            height_scale: 64.0,
            tile_set_pixel_size: [0, 0],
            tiles_per_row: 0,
            mouse_iso_pos: Vec3::ZERO,
            selection_iso_begin: Vec3::splat(-1.0),
            selection_iso_end: Vec3::splat(-1.0),
        };
        let tm = engine.world.spawn((
            tilemap,
            Transform::new(Vec3::ZERO, Vec3::ONE),
            Role::new(RoleKind::Tilemap),
        ));
        engine.names.insert("tilemap".into(), tm);
        let parent = engine.world.spawn((Transform::new(Vec3::new(2.0, 1.0, 0.0), Vec3::ONE),));
        engine.names.insert("rocket".into(), parent);
        let light = Light {
            kind: LightKind::Point,
            position: Vec3::new(0.0, 0.0, -1.0),
            color: [1.0, 0.55, 0.15],
            intensity: 1.0,
            radius: 8.125,
            dir: Vec3::ZERO,
            cone_angle: 0.0,
            parent: Some("rocket".into()),
        };
        engine.world.spawn((light.clone(),));
        let free = engine.world.spawn((Light { parent: None, ..light },));
        assert_eq!(engine.gather_lights().len(), 2);

        engine.set_enabled(parent, false);
        assert_eq!(engine.gather_lights().len(), 1, "hidden parent darkens its light");
        engine.set_enabled(parent, true);
        assert_eq!(engine.gather_lights().len(), 2);

        engine.set_enabled(free, false);
        let gathered = engine.gather_lights();
        assert_eq!(gathered.len(), 1, "hidden light is skipped");
        assert_eq!(gathered[0].parent.as_deref(), Some("rocket"));
        engine.set_enabled(free, true);
        assert_eq!(engine.gather_lights().len(), 2);
    }

    #[test]
    fn parented_light_tracks_parent_frame_offset() {
        // A parented light must follow the parent sprite's animated
        // `frame_offset` (altitude → z, horizontal drift → x/y), not just the
        // tile.  The frame offset is world metres here.
        let mut engine = Engine::new_for_test();
        let tilemap = Tilemap {
            position: Vec3::ZERO,
            scale: Vec3::new(45.0, 45.0, 1.0),
            size_x: 8,
            size_y: 8,
            tile_set: "tileset".into(),
            tile_pixel_size: [32, 32],
            max_tile: 16,
            tiles_grid: None,
            heights_grid: None,
            data: vec![0u32; 64],
            height_data: vec![0.0f32; 81],
            height_scale: 64.0,
            tile_set_pixel_size: [0, 0],
            tiles_per_row: 0,
            mouse_iso_pos: Vec3::ZERO,
            selection_iso_begin: Vec3::new(-1.0, -1.0, -1.0),
            selection_iso_end: Vec3::new(-1.0, -1.0, -1.0),
        };
        let tm = engine.world.spawn((
            tilemap,
            Transform::new(Vec3::ZERO, Vec3::new(45.0, 45.0, 1.0)),
            Role::new(RoleKind::Tilemap),
        ));
        engine.names.insert("tilemap".into(), tm);

        let parent_tile = Vec3::new(2.0, 2.0, 0.0);
        // World-metre frame offset (drift x/y, altitude z).
        let frame_offset = Vec3::new(0.25, -0.125, 0.5);
        let parent = engine.world.spawn((
            Transform::new(parent_tile, Vec3::ONE),
            IsoSprite {
                position: parent_tile,
                scale: Vec3::ONE,
                texture: "t".into(),
                tilemap: "tilemap".into(),
                frame: 0.0,
                frame_name: None,
                tile_set_size: Vec2::new(1.0, 1.0),
                anchor: Vec2::new(0.5, 0.5),
                footprint: vec![],
                ghost_group: 0,
                color: [1.0, 1.0, 1.0, 1.0],
                frame_offset,
            },
        ));
        engine.names.insert("rocket".into(), parent);

        let offset = Vec3::new(0.1, 0.2, 2.0);
        let light = Light {
            kind: LightKind::Point,
            position: offset,
            color: [1.0, 1.0, 1.0],
            intensity: 1.0,
            radius: 800.0,
            dir: Vec3::ZERO,
            cone_angle: 0.0,
            parent: Some("rocket".into()),
        };
        engine.world.spawn((light.clone(),));

        let gathered = engine.gather_lights();
        let base = engine.iso_to_world(parent_tile.x, parent_tile.y, 0.0).unwrap();
        // `frame_offset` is world metres; fold it into the base.  `gather_lights`
        // returns world space now (lighting is done in world metres).
        let expected = base + frame_offset + offset;
        assert!(
            (gathered[0].position - expected).length() < 1e-2,
            "got {} expected {}",
            gathered[0].position,
            expected
        );
    }

    /// A two-ROM DAG: `common` (entity `tile`) and the root `scene` (entity
    /// `rocket`), each under its own namespace.
    pub(crate) fn two_rom_dag() -> classic_rom::LoadedRoms {
        classic_rom::LoadedRoms {
            root: "scene".into(),
            order: vec![
                classic_rom::LoadedRom {
                    name: "common".into(),
                    namespace: "common".into(),
                    rom: test_rom("common", "common", r#"{"entities":{"tile":{"components":[]}}}"#),
                    sha256: None,
                },
                classic_rom::LoadedRom {
                    name: "scene".into(),
                    namespace: "scene".into(),
                    rom: test_rom("scene", "scene", r#"{"entities":{"rocket":{"components":[]}}}"#),
                    sha256: None,
                },
            ],
        }
    }

    fn test_rom(name: &str, namespace: &str, state_json: &str) -> classic_rom::Rom {
        let manifest_json = format!(
            r#"{{"entrypoint": "{name}", "namespace": "{namespace}",
                "shaders": [], "textures": [], "animations": []}}"#
        );
        let manifest: classic_rom::RomManifest = serde_json::from_str(&manifest_json).unwrap();
        classic_rom::Rom {
            manifest,
            manifest_json,
            resources: classic_rom::ResourceSet::default(),
            state: state_json.to_string(),
        }
    }

    #[test]
    fn load_state_qualifies_entity_names_under_namespace() {
        let mut e = Engine::new_for_test();
        e.namespace = "lunar".into();
        e.load_state(r#"{"entities":{"rocket":{"components":[]}}}"#).unwrap();
        assert!(e.names.contains_key("lunar::rocket"));
        assert_eq!(e.name_order, vec!["lunar::rocket"]);

        // An empty namespace is a no-op (the legacy single-ROM path).
        let mut e2 = Engine::new_for_test();
        e2.load_state(r#"{"entities":{"rocket":{"components":[]}}}"#).unwrap();
        assert!(e2.names.contains_key("rocket"));
        assert!(!e2.names.contains_key("::rocket"));
    }

    #[test]
    fn hydrate_roms_tracks_dag_in_topological_order() {
        let mut e = Engine::new_for_test();
        let loaded = two_rom_dag();

        e.hydrate_roms(&loaded, &classic_rom::NullBootSink);

        // Entities from both ROMs are hydrated, namespace-qualified.
        assert!(e.names.contains_key("common::tile"));
        assert!(e.names.contains_key("scene::rocket"));

        // The DAG is recorded and round-trips through `dump_roms`.
        assert_eq!(e.loaded_roms.len(), 2);
        let dumped = e.dump_roms().unwrap();
        assert_eq!(dumped.root, "scene");
        assert_eq!(dumped.order.len(), 2);
        assert_eq!(dumped.order[0].namespace, "common");
        assert_eq!(dumped.order[1].namespace, "scene");

        // The root ROM is mirrored into the single-ROM fields.
        assert_eq!(e.rom_manifest.as_ref().unwrap().entrypoint, "scene");
    }

    #[test]
    fn apply_vehicle_overrides_merges_root_tuning_into_shared_def() {
        let mut e = Engine::new_for_test();
        let def: classic_core::types::VehicleDef = serde_json::from_str(
            r#"{"name":"lrv","directions":8,"anchors":"lrv_anchors",
                "parts":[{"name":"body","texture":"lrvBody"}]}"#,
        )
        .unwrap();
        e.vehicles.insert("lunar-common::lrv".into(), def);

        let overrides: std::collections::HashMap<String, classic_core::types::VehicleOverrides> =
            serde_json::from_value(serde_json::json!({
                "lunar-common::lrv": {"turn_rate_deg_per_sec": 55.0, "safe_fall_m": 1.5}
            }))
            .unwrap();
        e.apply_vehicle_overrides(&overrides);

        let merged = &e.vehicles["lunar-common::lrv"];
        assert_eq!(merged.turn_rate_deg_per_sec, 55.0);
        assert_eq!(merged.safe_fall_m, 1.5);
        // Non-overridden fields keep the shared def's values.
        assert_eq!(merged.name, "lrv");
        assert_eq!(merged.parts.len(), 1);

        // An override for a vehicle the DAG didn't hydrate is ignored.
        let unknown: std::collections::HashMap<String, classic_core::types::VehicleOverrides> =
            serde_json::from_value(
                serde_json::json!({"missing::lrv": {"turn_rate_deg_per_sec": 1.0}}),
            )
            .unwrap();
        e.apply_vehicle_overrides(&unknown);
        assert!(!e.vehicles.contains_key("missing::lrv"));
    }

    #[test]
    fn hydrate_roms_qualifies_light_parent() {
        // `gather_lights` looks `Light.parent` up in `names` verbatim, so the
        // cross-ref pass must qualify it: a bare own-ROM parent resolves into
        // the ROM's namespace, a qualified one is kept as-is.  Unqualified, a
        // namespaced ROM's parented lights (the rocket burn light, basetest's
        // `controlLight`) are skipped every frame.
        let loaded = classic_rom::LoadedRoms {
            root: "scene".into(),
            order: vec![
                classic_rom::LoadedRom {
                    name: "common".into(),
                    namespace: "common".into(),
                    rom: test_rom("common", "common", r#"{"entities":{"tile":{"components":[]}}}"#),
                    sha256: None,
                },
                classic_rom::LoadedRom {
                    name: "scene".into(),
                    namespace: "scene".into(),
                    rom: test_rom(
                        "scene",
                        "scene",
                        r#"{"entities":{
                            "rocket":{"components":[]},
                            "burn":{"components":[{"type":"Light","position":[0,0,0],"color":[1,1,1],"parent":"rocket"}]},
                            "lamp":{"components":[{"type":"Light","position":[0,0,0],"color":[1,1,1],"parent":"common::tile"}]},
                            "free":{"components":[{"type":"Light","position":[0,0,0],"color":[1,1,1]}]}}}"#,
                    ),
                    sha256: None,
                },
            ],
        };
        let mut e = Engine::new_for_test();
        e.hydrate_roms(&loaded, &classic_rom::NullBootSink);
        let parent =
            |e: &Engine, n: &str| e.world.get::<&Light>(e.names[n]).unwrap().parent.clone();
        assert_eq!(parent(&e, "scene::burn").as_deref(), Some("scene::rocket"));
        assert_eq!(parent(&e, "scene::lamp").as_deref(), Some("common::tile"));
        assert_eq!(parent(&e, "scene::free"), None);
    }

    #[test]
    fn resolve_entity_name_applies_namespace_rule() {
        let mut e = Engine::new_for_test();
        e.load_state(r#"{"entities":{"globalEnt":{"components":[]}}}"#).unwrap();
        e.namespace = "lunar".into();
        e.load_state(r#"{"entities":{"rocket":{"components":[]}}}"#).unwrap();

        // Qualified names resolve exactly.
        assert_eq!(e.resolve_entity_name("scene", "lunar::rocket"), Some("lunar::rocket".into()));
        // Bare names resolve in the referring namespace first.
        assert_eq!(e.resolve_entity_name("lunar", "rocket"), Some("lunar::rocket".into()));
        // ...then fall back to the global namespace.
        assert_eq!(e.resolve_entity_name("lunar", "globalEnt"), Some("globalEnt".into()));
        // Unknown names fail.
        assert_eq!(e.resolve_entity_name("lunar", "missing"), None);
    }

    #[test]
    fn resolve_resource_applies_namespace_rule() {
        let mut e = Engine::new_for_test();
        // A global texture (empty namespace).
        e.texture_names.insert("cursor".to_string());
        // A namespaced ROM's resources.
        e.texture_names.insert("scene::rocket".to_string());
        e.animations.insert(
            "scene::walk".to_string(),
            AnimationData {
                name: "walk".to_string(),
                src: "humanoid".to_string(),
                rate: 24.0,
                sequence: vec![0],
                offsets: vec![],
                offset_keyframes: vec![],
                channels: vec![],
                metadata: None,
            },
        );
        e.sdf_fonts.insert(
            "scene::dejavusans".to_string(),
            SdfFontMetrics {
                name: "dejavusans".to_string(),
                family: "DejaVu Sans".to_string(),
                atlas_size: [512.0, 512.0],
                glyph_size: 32.0,
                spread: 4.0,
                baseline: 0.0,
                line_height: 1.0,
                glyphs: std::collections::HashMap::new(),
            },
        );

        // Qualified names resolve exactly.
        assert_eq!(
            e.resolve_resource("x", ResourceKind::Texture, "scene::rocket"),
            Some("scene::rocket".into())
        );
        // Bare names resolve in the referring namespace first.
        assert_eq!(
            e.resolve_resource("scene", ResourceKind::Texture, "rocket"),
            Some("scene::rocket".into())
        );
        // ...then fall back to the global namespace.
        assert_eq!(
            e.resolve_resource("scene", ResourceKind::Texture, "cursor"),
            Some("cursor".into())
        );
        // Animations and fonts resolve through their own registries.
        assert_eq!(
            e.resolve_resource("scene", ResourceKind::Animation, "walk"),
            Some("scene::walk".into())
        );
        assert_eq!(
            e.resolve_resource("scene", ResourceKind::Font, "dejavusans"),
            Some("scene::dejavusans".into())
        );
        // Unknown names fail.
        assert_eq!(e.resolve_resource("scene", ResourceKind::Texture, "missing"), None);
    }

    #[test]
    fn rewrite_resource_refs_qualifies_resource_references() {
        let scene_state = r#"{
            "entities": {
                "rocket": { "components": [ { "type": "IsoSprite", "position": [0,0,0],
                    "scale": [1,1,1], "texture": "rocket", "tilemap": "tilemap",
                    "frame": 0, "tile_set_size": [1,1], "anchor": [0.5,0.98] } ] },
                "marker": { "components": [ { "type": "IsoSprite", "position": [0,0,0],
                    "scale": [1,1,1], "texture": "common::tileSet", "tilemap": "common::tilemap",
                    "frame": 0, "tile_set_size": [1,1], "anchor": [0.5,0.5] } ] },
                "cursorSpr": { "components": [ { "type": "Sprite", "position": [0,0,0],
                    "scale": [1,1,1], "texture": "cursor", "ignore_cam": true, "frame": 0,
                    "tile_set_size": [1,1], "anchor": [0.5,0.5] } ] },
                "label": { "components": [ { "type": "SdfText", "atlas_name": "dejavusans",
                    "color": [1,1,1,1], "outline_color": [0,0,0,1], "outline_width": 0.0,
                    "ignore_cam": true, "text": "x", "justify": "Left", "weight": 0.5,
                    "gamma": 0.0 } ] },
                "anim": { "components": [ { "type": "Animator", "target": "rocket.IsoSprite",
                    "speed": 1.0, "animation": "walk" } ] }
            }
        }"#;

        let mut e = Engine::new_for_test();
        // Populate the resource registries as `load_manifest_resources` would.
        e.texture_names.insert("scene::rocket".to_string());
        e.texture_names.insert("common::tileSet".to_string());
        e.texture_names.insert("cursor".to_string());
        e.animations.insert(
            "scene::walk".to_string(),
            AnimationData {
                name: "walk".to_string(),
                src: "humanoid".to_string(),
                rate: 24.0,
                sequence: vec![0],
                offsets: vec![],
                offset_keyframes: vec![],
                channels: vec![],
                metadata: None,
            },
        );
        e.sdf_fonts.insert(
            "scene::dejavusans".to_string(),
            SdfFontMetrics {
                name: "dejavusans".to_string(),
                family: "DejaVu Sans".to_string(),
                atlas_size: [512.0, 512.0],
                glyph_size: 32.0,
                spread: 4.0,
                baseline: 0.0,
                line_height: 1.0,
                glyphs: std::collections::HashMap::new(),
            },
        );

        let keys = e.load_state_in("scene", scene_state).unwrap();
        e.rewrite_resource_refs("scene", &keys);

        // A bare texture resolves in the referring (own) namespace.
        let rocket = *e.names.get("scene::rocket").unwrap();
        assert_eq!(e.world.get::<&IsoSprite>(rocket).unwrap().texture, "scene::rocket");
        // A qualified cross-ROM reference resolves exactly (left untouched).
        let marker = *e.names.get("scene::marker").unwrap();
        assert_eq!(e.world.get::<&IsoSprite>(marker).unwrap().texture, "common::tileSet");
        // A bare name absent from the own namespace falls back to global.
        let cursor_spr = *e.names.get("scene::cursorSpr").unwrap();
        assert_eq!(e.world.get::<&SpriteRender>(cursor_spr).unwrap().texture, "cursor");
        // The SDF font atlas_name resolves as a font.
        let label = *e.names.get("scene::label").unwrap();
        assert_eq!(e.world.get::<&SdfTextRender>(label).unwrap().atlas_name, "scene::dejavusans");
        // The animator's animation name resolves as an animation.
        let anim = *e.names.get("scene::anim").unwrap();
        assert_eq!(
            e.world.get::<&Animator>(anim).unwrap().animation.as_deref(),
            Some("scene::walk")
        );
    }

    #[test]
    fn rewrite_cross_refs_qualifies_entity_references() {
        // A dependency ROM that did not declare a namespace defaults to its
        // entrypoint when it participates in a multi-ROM DAG.
        let scene_state = r#"{
            "entities": {
                "rocket": { "components": [ { "type": "IsoSprite", "position": [0,0,0],
                    "scale": [1,1,1], "texture": "rocket", "tilemap": "common::tilemap",
                    "frame": 0, "tile_set_size": [1,1], "anchor": [0.5,0.98] } ] },
                "selfsprite": { "components": [ { "type": "IsoSprite", "position": [0,0,0],
                    "scale": [1,1,1], "texture": "x", "tilemap": "localTilemap",
                    "frame": 0, "tile_set_size": [1,1], "anchor": [0.5,0.5] } ] },
                "nav": { "components": [ { "type": "IsometricNavMesh", "position": [0,0,0],
                    "scale": [1,1,1], "map_entity": "common::tilemap", "size_x": 10, "size_y": 10 } ] },
                "anim": { "components": [ { "type": "Animator", "target": "rocket.IsoSprite",
                    "speed": 1.0 } ] },
                "localTilemap": { "components": [] }
            }
        }"#;

        let mut e = Engine::new_for_test();
        let loaded = classic_rom::LoadedRoms {
            root: "scene".into(),
            order: vec![
                classic_rom::LoadedRom {
                    name: "common".into(),
                    namespace: String::new(),
                    rom: test_rom("common", "", r#"{"entities":{"tilemap":{"components":[]}}}"#),
                    sha256: None,
                },
                classic_rom::LoadedRom {
                    name: "scene".into(),
                    namespace: "scene".into(),
                    rom: test_rom("scene", "scene", scene_state),
                    sha256: None,
                },
            ],
        };

        e.hydrate_roms(&loaded, &classic_rom::NullBootSink);

        // The undeclared dependency ROM defaulted to its entrypoint namespace.
        assert_eq!(e.rom_namespace("common"), "common");
        assert_eq!(e.rom_namespace("scene"), "scene");
        assert!(e.names.contains_key("common::tilemap"));

        // A qualified cross-ROM reference resolves exactly (left untouched).
        let rocket = *e.names.get("scene::rocket").unwrap();
        assert_eq!(e.world.get::<&IsoSprite>(rocket).unwrap().tilemap, "common::tilemap");

        // A bare reference to a same-ROM entity is qualified with the referring ns.
        let selfsprite = *e.names.get("scene::selfsprite").unwrap();
        assert_eq!(e.world.get::<&IsoSprite>(selfsprite).unwrap().tilemap, "scene::localTilemap");

        // NavMesh.map_entity is rewritten.
        let nav = *e.names.get("scene::nav").unwrap();
        assert_eq!(e.world.get::<&NavMesh>(nav).unwrap().map_entity, "common::tilemap");

        // Animator.target qualifies its entity segment and keeps the component name.
        let anim = *e.names.get("scene::anim").unwrap();
        assert_eq!(e.world.get::<&Animator>(anim).unwrap().target, "scene::rocket.IsoSprite");
    }
}
