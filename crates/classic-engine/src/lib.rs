//! # classic-engine — the game engine
//!
//! God-object orchestrator.  Contains the `Engine` struct, frame lifecycle,
//! prefab init-* builders, editor tools, and the CLASSIC_TEST runner.
//!
//! **Skills to read before working here:**
//! - [classic-ecs](.claude/skills/classic-ecs/SKILL.md) — ECS patterns, components, update_fns
//! - [classic-ui](.claude/skills/classic-ui/SKILL.md) — UIManager, layout, collider integration
//! - [classic-physics](.claude/skills/classic-physics/SKILL.md) — click dispatch, selection, pathfinding
//! - [classic-iso](.claude/skills/classic-iso/SKILL.md) — iso coords, sprite rendering, nav mesh
//! - [classic-gfx](.claude/skills/classic-gfx/SKILL.md) — draw_*, GL state, DEPTH_TEST contract
//! - [classic-text](.claude/skills/classic-text/SKILL.md) — SdfText, glyph buffers, justify
//! - [classic-testing](.claude/skills/classic-testing/SKILL.md) — CLASSIC_TEST, golden harness
//! - [classic-debugging](.claude/skills/classic-debugging/SKILL.md) — CLASSIC_LOG, debugging playbook

pub mod env_config;
pub mod golden;
pub mod inventory;
pub mod inventory_ui;
pub mod light;
pub mod selection;
pub mod ui;
pub mod vehicle;

pub use classic_core::fields;

use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;
use std::sync::Arc;

use classic_core::collision::PhysicsProvider;
use classic_core::components::{
    Animator, ColliderData, DebugName, IsoSprite, IsoVehicle, Light, NavMesh, RectRender, Role,
    SdfTextRender, Selectable, TextJustify, Tilemap, UiAlign, UiAnchor, UiNode,
};
use classic_core::instrument::Chan;
use classic_core::math::{cartesian_to_iso_4, iso_to_cartesian_4};
use classic_core::pathfinder;
use classic_core::sdf_builder::build_sdf_glyph_buffer;
use classic_core::tilemap::{
    bilinear_height, build_mesh, build_tile_texture, horizontal_depth_scale, sample_height_mesh,
    HEIGHT_DEPTH_SCALE_M, HORIZONTAL_DEPTH_SCALE, PPM_TARGET,
};
use classic_core::types::AnimationData;
use classic_core::types::FrameTable;
use classic_core::types::OffsetKeyframe;
use classic_core::types::SdfFontMetrics;
use classic_core::{Camera, RoleKind, SpriteRender, Transform};
use classic_gfx::{Gfx, GlBuffer, GlTexture, IsoSpritePass, RenderSettings, SpriteRegion};
use classic_platform::InputState;
use glam::{Mat3, Mat4, Vec2, Vec3, Vec4};
use glow::HasContext;

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
/// manifest texture declares a `depth` map, the engine uploads the grayscale
/// PNG under `depth_tex` and records the `depth_range` (isoDepth units spanned
/// by the map's `[0, 1]` grayscale) so the render loop can write per-pixel
/// `gl_FragDepth` for that sprite.
#[derive(Clone, Debug)]
struct TextureDepth {
    depth_tex: String,
    depth_range: f32,
}

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
    depth_map: Option<(String, f32)>,
    depth_base: f32,
    normal_map: Option<String>,
    ghost_group: u32,
    color: [f32; 4],
    /// Whether the sprite is currently RTS-selected (draws a silhouette edge).
    selected: bool,
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
    /// Per-sheet depth-map GL texture name + depth range, set when the frame's
    /// sheet declares a `depth` companion.
    pub(crate) depth_tex: Option<(String, f32)>,
}

pub struct Engine {
    pub gfx: Option<Gfx>,
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
    /// Dynamic light pool (point/spot lights beyond the sun term).  Gathered +
    /// uploaded to the `LightBlock` UBO once per frame.
    pub light_pool: light::LightPool,
    pub animations: HashMap<String, AnimationData>,
    /// Packed-atlas frame tables keyed by texture name, loaded from the ROM's
    /// `frames` resources at boot (issue #45).  A sprite with `frame_name` set
    /// resolves its frame through the table for the owning texture.
    pub frame_tables: HashMap<String, FrameTable>,
    pub sdf_fonts: HashMap<String, SdfFontMetrics>,
    /// Per-texture depth-mask metadata keyed by color texture name (loaded
    /// from the manifest's `depth` / `depth_range` fields).
    texture_depths: HashMap<String, TextureDepth>,
    /// Per-texture normal-map texture name keyed by color texture name (loaded
    /// from the manifest's `normal` field).  Sprites with a normal map are
    /// shaded with a runtime Lambertian term.
    texture_normals: HashMap<String, String>,
    /// Wheeled-vehicle definitions keyed by name, loaded from the ROM's
    /// `vehicles` resources at boot.
    pub vehicles: HashMap<String, classic_core::types::VehicleDef>,
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
    /// Synchronously-computed path results (synchronous_workers mode / web).
    sync_paths: HashMap<u64, pathfinder::PathPoll>,
    /// Pathfinding worker (spawned lazily on first async request; native
    /// thread or web `Worker` depending on target).
    pathfinder: Option<classic_worker::PathfinderWorker>,
    /// Immutable vehicle nav snapshot (structural nav + heights) shared with
    /// the pathfinding worker.
    vehicle_nav_snapshot: Arc<pathfinder::VehicleNavSnapshot>,
    /// Synchronously-computed vehicle path results (synchronous_workers mode).
    sync_vehicle_paths: HashMap<u64, vehicle::VehicleGotoPoll>,
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
    UiRect,
    UiSprite,
    SdfText,
}

impl Engine {
    /// Create an Engine without a GL context — useful for testing non-rendering
    /// subsystems (UI layout, collision dispatch, tilemap math, etc.).
    pub fn new_for_test() -> Self {
        Self::new()
    }

    pub fn new() -> Self {
        // Component registry is a global RwLock<HashMap>. Tests that share
        // it must use --test-threads=1. spawn/load order determines entity
        // IDs, which affects golden trace stability.
        classic_core::register_all_components();
        classic_core::instrument::init_from_env();
        Self {
            gfx: None,
            world: hecs::World::new(),
            camera: Camera::new(Vec3::ZERO, Vec3::new(1.0, 1.0, 1.0)),
            time: Time::default(),
            names: HashMap::new(),
            name_order: Vec::new(),
            namespace: String::new(),
            physics: PhysicsProvider::new(),
            collider_names: HashMap::new(),
            collider_pids: HashMap::new(),
            selectable_colliders: HashSet::new(),
            subscribed: HashSet::new(),
            guest_events: VecDeque::new(),
            guest_hover: None,
            guest_flags: HashMap::new(),
            scroll_speed: 600.0,
            input: InputState::new(),
            show_grid: false,
            light_ambient: [0.15, 0.15, 0.2],
            light_dir: [0.45, -0.35, 0.82],
            light_color: [1.0, 0.95, 0.85],
            light_pool: light::LightPool::new(),
            animations: HashMap::new(),
            sdf_fonts: HashMap::new(),
            frame_tables: HashMap::new(),
            texture_depths: HashMap::new(),
            texture_normals: HashMap::new(),
            vehicles: HashMap::new(),
            items: classic_core::inventory::ItemRegistry::default(),
            next_ghost_group: 1,
            rom_manifest_json: None,
            rom_manifest: None,
            rom_resources: None,
            ui: None,
            selection: selection::SelectionSet::default(),
            rts_box: None,
            selection_mode: -1,
            selection_begin_screen: glam::Vec3::new(-1.0, -1.0, -1.0),
            base_height_scale: 32.0,
            nav_slope_threshold: 2.0,
            nav_snapshot: Arc::new(pathfinder::NavSnapshot::new(0, 0, Vec::new())),
            nav_version: 0,
            synchronous_workers: false,
            next_path_id: 1,
            sync_paths: HashMap::new(),
            pathfinder: None,
            vehicle_nav_snapshot: Arc::new(pathfinder::VehicleNavSnapshot::new(
                0,
                0,
                Vec::new(),
                Vec::new(),
                0.0,
                45.0,
            )),
            sync_vehicle_paths: HashMap::new(),
            vehicle_path_entities: HashMap::new(),
            preview_paths: HashMap::new(),
            preview_probe: None,
            next_task_id: 0,
            guest_worker: None,
            fields: fields::FieldRegistry::default(),
            inventory_ui: inventory_ui::InventoryUi::default(),
            nav_gpu: None,
            debug_frame: 0,
            pre_update_hooks: Vec::new(),
            selection_end_hooks: Vec::new(),
            overlay_hooks: Vec::new(),
            test_runner: None,
            test_should_close: false,
            test_failed: false,
            golden_capture_frame: 55, // default: one frame after default scenario's last step (54)
            update_fns: Vec::new(),
            trace: None,
            tilemap_gpu: HashMap::new(),
            sdf_text_gpu: HashMap::new(),
            last_vw: 0.0,
            last_vh: 0.0,
        }
    }

    /// Initialise the GL layer from a ROM manifest + resource set: compile the
    /// manifest's shaders (built-ins, overridable by a ROM via the named
    /// shader registry), upload every declared texture, load the SDF fonts, and
    /// register the animations.
    pub fn init_gfx(
        &mut self,
        gl: Rc<glow::Context>,
        manifest: &classic_rom::RomManifest,
        resources: &classic_rom::ResourceSet,
    ) {
        self.gfx = Some(Gfx::new(gl));
        let registry = classic_gfx::ShaderSourceRegistry::builtin();
        // The engine owns the shader declarations: compile the full builtin
        // catalog by default, letting the ROM override any shader by *name*
        // (a manifest `shaders[]` entry with a matching name swaps its
        // vertex/fragment filenames + layout).
        let overrides: HashMap<&str, &classic_core::types::ShaderInfo> =
            manifest.manifest.shaders.iter().map(|info| (info.name.as_str(), info)).collect();
        for builtin in classic_gfx::builtin_shaders() {
            let (vs_name, fs_name, attr, unif): (&str, &str, Vec<&str>, Vec<&str>) = match overrides
                .get(builtin.name)
            {
                Some(info) => (
                    info.vertex.as_str(),
                    info.fragment.as_str(),
                    info.attr.iter().map(String::as_str).collect(),
                    info.unif.iter().map(String::as_str).collect(),
                ),
                None => {
                    (builtin.vertex, builtin.fragment, builtin.attr.to_vec(), builtin.unif.to_vec())
                }
            };
            let vs = registry.resolve_vertex(vs_name);
            let fs = registry.resolve_fragment(fs_name);
            self.gfx
                .as_mut()
                .unwrap()
                .add_shader(builtin.name, &vs, &fs, &attr, &unif)
                .expect("compile shader");
        }

        // Textures from the manifest, skipping the SDF atlas textures (those
        // are uploaded by the SDF font path with LINEAR filtering).  Several
        // manifest entries may share one `src` — every frame-table texture
        // points at its shared colour sheet — so decode + upload each unique
        // `src` once and alias the rest to the same GL texture (the packed-atlas
        // draw path binds the sheet name, not the frame-table name).
        let atlas_names: std::collections::HashSet<String> =
            resources.fonts().keys().map(|f| format!("{f}-sdf")).collect();
        let mut uploaded_by_src: HashMap<String, GlTexture> = HashMap::new();
        for entry in &manifest.manifest.textures {
            if atlas_names.contains(&entry.name) {
                continue;
            }
            let Some(bytes) = resources.textures().get(&entry.name) else {
                continue;
            };
            if let Some(tex) = uploaded_by_src.get(&entry.src) {
                self.gfx.as_mut().unwrap().textures.insert(entry.name.clone(), tex.clone());
                continue;
            }
            // Shared-atlas companion sheets are named `{sheet}-normal` /
            // `{sheet}-depth` by the packer; upload them in their native channel
            // count (RGB8 / R8) instead of RGBA8.
            if entry.name.ends_with("-depth") {
                self.load_texture_luma8(&entry.name, bytes);
            } else if entry.name.ends_with("-normal") {
                self.load_texture_rgb8(&entry.name, bytes);
            } else {
                self.load_texture_png(&entry.name, bytes);
            }
            if let Some(tex) = self.gfx.as_ref().unwrap().textures.get(&entry.name) {
                uploaded_by_src.insert(entry.src.clone(), tex.clone());
            }
        }

        // Per-texture depth maps (grayscale `gl_FragDepth` masks).  Uploaded as
        // sibling textures named `"{name}-depth"` and recorded against the
        // color texture name so `draw_iso_sprite` can look them up.
        for entry in &manifest.manifest.textures {
            if entry.depth.is_some() {
                if let Some(bytes) = resources.depths().get(&entry.name) {
                    let depth_tex = format!("{}-depth", entry.name);
                    self.load_texture_luma8(&depth_tex, bytes);
                    self.texture_depths.insert(
                        entry.name.clone(),
                        TextureDepth { depth_tex, depth_range: entry.depth_range },
                    );
                }
            }
        }

        // Per-texture normal maps (RGB world-space normals).  Uploaded as
        // sibling textures named `"{name}-normal"` and recorded against the
        // color texture name so `draw_iso_sprite` can bind them for the
        // runtime Lambertian term.
        for entry in &manifest.manifest.textures {
            if entry.normal.is_some() {
                if let Some(bytes) = resources.normals().get(&entry.name) {
                    let normal_tex = format!("{}-normal", entry.name);
                    self.load_texture_rgb8(&normal_tex, bytes);
                    self.texture_normals.insert(entry.name.clone(), normal_tex);
                }
            }
        }

        // SDF fonts: metrics JSON + atlas PNG (font name + "-sdf").
        for (font_name, metrics_bytes) in resources.fonts() {
            let atlas_name = format!("{font_name}-sdf");
            let metrics_str = std::str::from_utf8(metrics_bytes).expect("SDF metrics UTF-8");
            if let Some(atlas_png) = resources.textures().get(&atlas_name) {
                self.load_sdf_font(&atlas_name, metrics_str, atlas_png);
            }
        }

        for anim in &manifest.manifest.animations {
            self.animations.insert(anim.name.clone(), anim.clone());
        }

        // Packed-atlas frame tables (`frames.json`) keyed by texture name,
        // loaded from the ROM's `frames` resources.  Companion sheets are
        // uploaded as ordinary textures (declared in the manifest) and are
        // referenced by name from each frame's `sheet` index.
        for (name, bytes) in resources.frames() {
            match serde_json::from_slice::<FrameTable>(bytes) {
                Ok(mut table) => {
                    table.precompute_companions();
                    self.frame_tables.insert(name.clone(), table);
                }
                Err(e) => {
                    classic_core::cl_error!(Chan::Guest, "frame table '{name}' parse failed: {e}");
                }
            }
        }

        // Per-animation renderer metadata (frame offsets) declared in the
        // manifest is loaded from the ROM's `animations/` resources and folded
        // into the registered `AnimationData`.
        for (name, metadata_bytes) in resources.animations() {
            self.load_animation_offsets(name, metadata_bytes);
        }

        // Wheeled-vehicle definitions (JSON sidecars) from the `vehicles`
        // resources, keyed by the manifest-declared name.
        for (name, bytes) in resources.vehicles() {
            match serde_json::from_slice::<classic_core::types::VehicleDef>(bytes) {
                Ok(def) => {
                    self.vehicles.insert(name.clone(), def);
                }
                Err(e) => {
                    classic_core::cl_error!(Chan::Guest, "vehicle '{name}' parse failed: {e}");
                }
            }
        }
    }

    /// Hydrate the engine from a ROM: compile shaders, upload resources, and
    /// spawn the entity graph.  Records the ROM's manifest + resources so
    /// [`Engine::dump_rom`] can reconstruct it.
    pub fn load_rom(&mut self, gl: Rc<glow::Context>, rom: &classic_rom::Rom) {
        self.namespace = rom.manifest.namespace.clone();
        self.init_gfx(gl, &rom.manifest, &rom.resources);
        self.load_state(&rom.state).expect("load ROM state");
        self.load_grids(&rom.resources);
        self.items = classic_core::inventory::ItemRegistry::build(
            &rom.manifest.items,
            &rom.manifest.inventory_types,
        );
        self.rom_manifest_json = Some(rom.manifest_json.clone());
        self.rom_manifest = Some(rom.manifest.clone());
        self.rom_resources = Some(rom.resources.clone());
    }

    /// Qualify an entity name with the active namespace (a no-op when the
    /// namespace is empty).  The single point where multi-ROM namespacing will
    /// be applied: `names`/`name_order` and every name lookup route through this
    /// once several ROMs can load concurrently.
    pub fn entity_key(&self, name: &str) -> String {
        if self.namespace.is_empty() {
            name.to_string()
        } else {
            format!("{}::{name}", self.namespace)
        }
    }

    /// Hydrate the tile/nav/height grids referenced by the entity state from
    /// the ROM's grid resources (raw little-endian numbers keyed by name).
    fn load_grids(&mut self, resources: &classic_rom::ResourceSet) {
        let grids = resources.grids();

        if let Some(tm_entity) = self.entity_by_role(RoleKind::Tilemap) {
            let (tiles_grid, heights_grid) = match self.world.get::<&Tilemap>(tm_entity) {
                Ok(tm) => (tm.tiles_grid.clone(), tm.heights_grid.clone()),
                Err(_) => (None, None),
            };
            if let Some(name) = tiles_grid {
                if let Some(bytes) = grids.get(&name) {
                    self.set_tiles_bulk(&decode_u32(bytes));
                }
            }
            if let Some(name) = heights_grid {
                if let Some(bytes) = grids.get(&name) {
                    self.set_heights_bulk(&decode_f32(bytes));
                }
            }
        }

        if let Some(nav_entity) = self.entity_by_role(RoleKind::NavMesh) {
            let data_grid = match self.world.get::<&NavMesh>(nav_entity) {
                Ok(nav) => nav.data_grid.clone(),
                Err(_) => None,
            };
            if let Some(name) = data_grid {
                if let Some(bytes) = grids.get(&name) {
                    self.set_nav_bulk(&decode_u32(bytes));
                }
            }
        }
    }

    /// Reconstruct a [`classic_rom::Rom`] from the loaded manifest + resources
    /// and the current world state.  Returns `None` if no ROM was loaded.
    pub fn dump_rom(&self) -> Option<classic_rom::Rom> {
        let mut resources = self.rom_resources.clone()?;

        if let Some(tm_entity) = self.entity_by_role(RoleKind::Tilemap) {
            if let Ok(tm) = self.world.get::<&Tilemap>(tm_entity) {
                if let Some(name) = &tm.tiles_grid {
                    resources.insert(
                        classic_rom::ResourceKind::Grid,
                        name.clone(),
                        encode_u32(&tm.data),
                    );
                }
                if let Some(name) = &tm.heights_grid {
                    resources.insert(
                        classic_rom::ResourceKind::Grid,
                        name.clone(),
                        encode_f32(&tm.height_data),
                    );
                }
            }
        }
        if let Some(nav_entity) = self.entity_by_role(RoleKind::NavMesh) {
            if let Ok(nav) = self.world.get::<&NavMesh>(nav_entity) {
                if let Some(name) = &nav.data_grid {
                    resources.insert(
                        classic_rom::ResourceKind::Grid,
                        name.clone(),
                        encode_u32(&nav.data),
                    );
                }
            }
        }

        Some(classic_rom::Rom {
            manifest: self.rom_manifest.clone()?,
            manifest_json: self.rom_manifest_json.clone()?,
            resources,
            state: self.dump_state(),
        })
    }

    /// Load per-frame visual offsets emitted by the animation renderer.
    ///
    /// Two encodings are accepted, distinguished by a 4-byte magic prefix:
    ///
    /// - **Sparse keyframes** (current): `b"KAOS"`, `u8` version (= 1), `u32`
    ///   keyframe_count, `f32` `pixels_per_meter`, then keyframe_count ×
    ///   `(u32 frame_idx, f32 x, f32 y, f32 z)` `rig_location` triplets.  The
    ///   animator linearly interpolates between keyframes.
    /// - **Legacy dense**: `u32` frame_count, `f32` `pixels_per_meter`, then
    ///   frame_count × `[f32 x, f32 y, f32 z]` triplets (one per frame).
    ///
    /// `rig_location` is Blender world `(x = drift, y = drift, z = altitude)`
    /// in metres.  It is converted here to a cartesian screen-space offset:
    /// the altitude maps onto the vertical (screen-Y, negative = up), and the
    /// drift maps onto screen X/Y, all scaled by `pixels_per_meter` so the
    /// rocket's motion matches the sprite's render resolution.
    pub fn load_animation_offsets(&mut self, animation_name: &str, bytes: &[u8]) {
        const MAGIC: &[u8; 4] = b"KAOS";

        let Some(animation) = self.animations.get_mut(animation_name) else {
            return;
        };

        if bytes.len() >= 13 && &bytes[0..4] == MAGIC {
            let version = bytes[4];
            if version != 1 {
                return;
            }
            let count = u32::from_le_bytes([bytes[5], bytes[6], bytes[7], bytes[8]]) as usize;
            let ppm = f32::from_le_bytes([bytes[9], bytes[10], bytes[11], bytes[12]]);
            let mut keyframes = Vec::with_capacity(count);
            let mut o = 13;
            for _ in 0..count {
                if o + 16 > bytes.len() {
                    break;
                }
                let frame =
                    u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
                let x =
                    f32::from_le_bytes([bytes[o + 4], bytes[o + 5], bytes[o + 6], bytes[o + 7]]);
                let y =
                    f32::from_le_bytes([bytes[o + 8], bytes[o + 9], bytes[o + 10], bytes[o + 11]]);
                let z = f32::from_le_bytes([
                    bytes[o + 12],
                    bytes[o + 13],
                    bytes[o + 14],
                    bytes[o + 15],
                ]);
                o += 16;
                keyframes.push(OffsetKeyframe {
                    frame,
                    // Altitude (z) lifts the rocket up = smaller cart_pos.y.
                    offset: Vec3::new(x * ppm, y * ppm - z * ppm, 0.0).to_array(),
                });
            }
            animation.offset_keyframes = keyframes;
            return;
        }

        if bytes.len() < 8 {
            return;
        }
        let frame_count = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        let ppm = f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);

        let mut offsets = Vec::with_capacity(frame_count);
        for i in 0..frame_count {
            let o = 8 + i * 12;
            if o + 12 > bytes.len() {
                break;
            }
            let x = f32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
            let y = f32::from_le_bytes([bytes[o + 4], bytes[o + 5], bytes[o + 6], bytes[o + 7]]);
            let z = f32::from_le_bytes([bytes[o + 8], bytes[o + 9], bytes[o + 10], bytes[o + 11]]);
            // Altitude (z) lifts the rocket up = smaller cart_pos.y.
            offsets.push(Vec3::new(x * ppm, y * ppm - z * ppm, 0.0).to_array());
        }
        animation.offsets = offsets;
    }

    /// Upload a PNG texture from raw bytes.
    pub fn load_texture_png(&mut self, name: &str, png_bytes: &[u8]) {
        let img = image::load_from_memory(png_bytes).expect("decode PNG");
        let rgba = img.to_rgba8();
        if let Some(gfx) = self.gfx.as_mut() {
            gfx.add_texture_rgba8(name, &rgba, rgba.width(), rgba.height());
        }
    }

    /// Upload a grayscale PNG as an R8 texture (depth maps, SDF atlases).
    pub fn load_texture_luma8(&mut self, name: &str, png_bytes: &[u8]) {
        let img = image::load_from_memory(png_bytes).expect("decode PNG");
        let luma = img.to_luma8();
        if let Some(gfx) = self.gfx.as_mut() {
            gfx.add_texture_r8(name, &luma, luma.width(), luma.height());
        }
    }

    /// Upload an RGB PNG as an RGB8 texture (world-space normal maps).
    pub fn load_texture_rgb8(&mut self, name: &str, png_bytes: &[u8]) {
        let img = image::load_from_memory(png_bytes).expect("decode PNG");
        let rgb = img.to_rgb8();
        if let Some(gfx) = self.gfx.as_mut() {
            gfx.add_texture_rgb8(name, &rgb, rgb.width(), rgb.height());
        }
    }

    /// Load an SDF font from its metrics JSON and atlas PNG.
    /// The atlas texture is set to LINEAR filtering.
    pub fn load_sdf_font(&mut self, atlas_name: &str, metrics_json: &str, atlas_png: &[u8]) {
        let metrics: SdfFontMetrics =
            serde_json::from_str(metrics_json).expect("parse SDF font metrics JSON");
        self.sdf_fonts.insert(metrics.name.clone(), metrics);

        let img = image::load_from_memory(atlas_png).expect("decode SDF atlas PNG");
        let luma = img.to_luma8();
        if let Some(gfx) = self.gfx.as_mut() {
            gfx.add_texture_r8(atlas_name, &luma, luma.width(), luma.height());
            if let Some(tex) = gfx.textures.get(atlas_name) {
                tex.set_linear(&gfx.gl);
            }
        }
    }

    /// Shared tail of the `commit_terrain` path: build the mesh and tile-data
    /// texture, upload both, write the data back onto the component, and
    /// register the mouse-to-iso parallax solve.
    ///
    /// The parallax closure must be registered exactly once per tilemap, which
    /// is the main reason this is factored out rather than duplicated.
    fn finish_tilemap_init(
        &mut self,
        entity: hecs::Entity,
        tiles: Vec<u32>,
        heights: Vec<f32>,
        height_scale: Option<f32>,
    ) {
        let (tile_set_name, size_x, size_y, tile_pixel_size) = {
            let tm = self.world.get::<&Tilemap>(entity).expect("Tilemap component");
            (tm.tile_set.clone(), tm.size_x, tm.size_y, tm.tile_pixel_size)
        };

        let height_scale = height_scale.unwrap_or(tile_pixel_size[0] as f32);
        self.base_height_scale = height_scale;
        let (mesh_data, vcount) = build_mesh(size_x, size_y, &tiles, &heights, height_scale);

        let (tile_pixels, tw, th) = build_tile_texture(size_x, size_y, &tiles);

        let gfx = self.gfx.as_mut().expect("gfx not initialized");

        // Upload mesh.
        let mesh_buf =
            GlBuffer::from_slice(&gfx.gl, glow::ARRAY_BUFFER, &mesh_data, glow::STATIC_DRAW);

        // Upload tile data texture.
        let tile_tex = Engine::upload_data_texture(&gfx.gl, &tile_pixels, tw, th);

        // Store tile data on the component.
        if let Ok(mut tm) = self.world.get::<&mut Tilemap>(entity) {
            tm.data = tiles;
            tm.height_data = heights;
            tm.height_scale = height_scale;
            let img_h = gfx.textures.get(&tile_set_name).map(|t| t.size.1).unwrap_or(0);
            let img_w = gfx.textures.get(&tile_set_name).map(|t| t.size.0).unwrap_or(0);
            if img_w > 0 {
                let px = tile_pixel_size[0].max(1);
                tm.tile_set_pixel_size = [img_w, img_h];
                tm.tiles_per_row = img_w / px;
            }
        }

        let name = self.debug_name(entity);
        self.tilemap_gpu.insert(name, TilemapGpu { mesh_buf, vertex_count: vcount, tile_tex });

        // Register updateMousePos: convert screen coords → iso tile coords
        // with 3-iteration height parallax solve.
        let tm_entity = entity;
        self.on_update(move |engine| {
            let (tile_step, cart_to_iso, height_data, size_x, size_y, height_scale) = {
                let tm = engine.world.get::<&Tilemap>(tm_entity).unwrap();
                let scale = tm.scale;
                let tile_step = scale.x * std::f32::consts::FRAC_1_SQRT_2;
                let inv_scale = Vec3::new(1.0 / scale.x, 1.0 / scale.y, 1.0);
                let cart_to_iso = cartesian_to_iso_4() * Mat4::from_scale(inv_scale);
                (
                    tile_step,
                    cart_to_iso,
                    tm.height_data.clone(),
                    tm.size_x,
                    tm.size_y,
                    tm.height_scale,
                )
            };

            let mp = engine.input.mouse_pos;
            let mut iso_pos = Vec3::new(mp.x, mp.y, 0.0);
            iso_pos += engine.camera.fix();
            iso_pos /= engine.camera.scale;
            iso_pos = cart_to_iso.transform_point3(iso_pos);

            let orig = iso_pos;
            for _ in 0..3 {
                let h = bilinear_height(&height_data, size_x, size_y, iso_pos.x, iso_pos.y);
                if h <= 0.0 {
                    break;
                }
                let z_offset = (h * height_scale) / tile_step.max(0.001);
                iso_pos.x = orig.x - z_offset;
                iso_pos.y = orig.y + z_offset;
            }

            // Ground the height: `mouse_iso_pos` becomes a full (x, y, z)
            // position where `z` is the terrain height (metres) under the
            // cursor, sampled with the same triangle-linear interpolation as
            // the rendered mesh.
            iso_pos.z = sample_height_mesh(&height_data, size_x, size_y, iso_pos.x, iso_pos.y);

            if let Ok(mut tm) = engine.world.get::<&mut Tilemap>(tm_entity) {
                tm.mouse_iso_pos = iso_pos;
            }
        });
    }

    pub fn on_update(&mut self, f: impl FnMut(&mut Engine) + 'static) {
        self.update_fns.push(Box::new(f));
    }

    /// Register a pre-update callback, run every frame *before* the
    /// `on_update` closures (used by the demo to route the mouse wheel to the
    /// text panel before the camera zoom handler sees it).
    pub fn on_pre_update(&mut self, f: impl FnMut(&mut Engine) + 'static) {
        self.pre_update_hooks.push(Box::new(f));
    }

    /// Register a callback run when a selection drag just ended (replaces the
    /// hardcoded editor-paint that used to live in `frame()`).
    pub fn on_selection_end(&mut self, f: impl FnMut(&mut Engine) + 'static) {
        self.selection_end_hooks.push(Box::new(f));
    }

    /// Register a debug overlay callback, run in the GL draw phase after the
    /// main render list (footprint polygons, agent ring, compass rose, ...).
    pub fn add_overlay(&mut self, f: impl FnMut(&mut Engine) + 'static) {
        self.overlay_hooks.push(Box::new(f));
    }

    /// Install the CLASSIC_TEST per-frame runner (a single callback invoked
    /// once per frame when `CLASSIC_TEST` is active).
    pub fn set_test_runner(&mut self, f: impl FnMut(&mut Engine) + 'static) {
        self.test_runner = Some(Box::new(f));
    }

    /// Current frame counter (used by the test runner for frame scheduling).
    pub fn frame_number(&self) -> u64 {
        self.debug_frame
    }

    /// Last-frame viewport size (used by the test runner to synthesise
    /// normalised mouse coordinates).
    pub fn viewport_size(&self) -> (f32, f32) {
        (self.last_vw, self.last_vh)
    }

    pub fn load_state(&mut self, json: &str) -> Result<(), anyhow::Error> {
        // Parse via raw Value to preserve JSON key order (serde_json Map is ordered
        // under `preserve_order`). The typed HashMap on StateData drops ordering.
        let root: serde_json::Value = serde_json::from_str(json)?;
        let entities_obj = root
            .get("entities")
            .and_then(|v| v.as_object())
            .ok_or_else(|| anyhow::anyhow!("state.json missing 'entities' key"))?;

        for (name, val) in entities_obj {
            let ed: classic_core::types::EntityData = serde_json::from_value(val.clone())?;
            let mut builder = hecs::EntityBuilder::new();
            for comp in &ed.components {
                let spawner = classic_core::registry::lookup(&comp.comp_type)
                    .ok_or_else(|| anyhow::anyhow!("unknown component type: {}", comp.comp_type))?;
                spawner(&mut builder, comp.fields.clone())?;
            }
            if ed.components.is_empty() {
                builder.add(());
            }
            let entity = self.world.spawn(builder.build());
            self.world.insert_one(entity, DebugName(name.clone())).ok();
            self.names.insert(name.clone(), entity);
            self.name_order.push(name.clone());
        }
        Ok(())
    }

    pub fn debug_name(&self, entity: hecs::Entity) -> String {
        self.world
            .get::<&DebugName>(entity)
            .map(|n| n.0.clone())
            .unwrap_or_else(|_| format!("e#{:?}", entity.id()))
    }

    /// Find the entity tagged with the given [`RoleKind`].
    pub fn entity_by_role(&self, kind: RoleKind) -> Option<hecs::Entity> {
        self.world
            .query::<&Role>()
            .iter()
            .find(|(_, role)| role.value == kind)
            .map(|(entity, _)| entity)
    }

    /// Find the name of the entity tagged with the given [`RoleKind`].
    pub fn name_by_role(&self, kind: RoleKind) -> Option<String> {
        self.entity_by_role(kind).map(|e| self.debug_name(e))
    }

    /// Serialise all named entities to a state JSON string.
    pub fn dump_state(&self) -> String {
        let entities = self.dump_state_value();
        let root = serde_json::json!({ "entities": entities });
        serde_json::to_string_pretty(&root).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
    }

    fn dump_state_value(&self) -> serde_json::Value {
        let mut entities = serde_json::Map::new();

        for name in &self.name_order {
            let Some(&entity) = self.names.get(name) else { continue };
            let components = self.dump_entity_components(entity);
            if !components.is_empty() {
                entities.insert(name.clone(), serde_json::json!({ "components": components }));
            }
        }

        serde_json::Value::Object(entities)
    }

    /// Serialize a single named entity's component list (the `components`
    /// array of a `state.json` entry), using the registry dumpers.
    pub fn dump_entity_components(&self, entity: hecs::Entity) -> Vec<serde_json::Value> {
        let regs = classic_core::registry::ordered_regs();
        let mut components: Vec<serde_json::Value> = Vec::new();
        let mut dumped = std::collections::HashSet::new();

        for reg in &regs {
            if dumped.contains(reg.name) {
                continue;
            }
            if let Some(dump) = reg.dump {
                if let Some(val) = dump(&self.world, entity) {
                    components.push(val);
                    dumped.insert(reg.name);
                    for sub in reg.subsumes {
                        dumped.insert(sub);
                    }
                }
            }
            // Try subsumed components first (they may match)
            for sub in reg.subsumes {
                if dumped.contains(sub) {
                    continue;
                }
                // Check if there's a subsumed reg with a dumper
                if let Some(sub_dump) = regs.iter().find(|r| r.name == *sub).and_then(|r| r.dump) {
                    if let Some(val) = sub_dump(&self.world, entity) {
                        components.push(val);
                        dumped.insert(sub);
                    }
                }
            }
        }

        components
    }

    /// Named-entity query/access helpers for the guest-code layer.  They mirror
    /// the `load_state`/`dump_state` bookkeeping (names + name_order) so guest
    /// code can spawn/despawn/lookup entities and round-trip components through
    /// the registry without per-field glue.
    pub fn has_name(&self, name: &str) -> bool {
        self.names.contains_key(name)
    }

    pub fn entity_names(&self) -> Vec<String> {
        self.name_order.clone()
    }

    /// Spawn an empty named entity (registers it in `names`/`name_order`).
    /// Returns false if the name is already taken.
    pub fn spawn_named(&mut self, name: &str) -> bool {
        if self.names.contains_key(name) {
            return false;
        }
        let entity = self.world.spawn(());
        self.world.insert_one(entity, DebugName(name.to_string())).ok();
        self.names.insert(name.to_string(), entity);
        self.name_order.push(name.to_string());
        true
    }

    /// Despawn a named entity and drop its name registration.
    pub fn despawn_named(&mut self, name: &str) -> bool {
        let Some(entity) = self.names.remove(name) else { return false };
        self.name_order.retain(|n| n != name);
        let _ = self.world.despawn(entity);
        true
    }

    /// Read a named entity's position (from its `Transform`).
    pub fn get_pos(&self, name: &str) -> Option<(f32, f32, f32)> {
        let entity = *self.names.get(name)?;
        self.world
            .get::<&Transform>(entity)
            .ok()
            .map(|tf| (tf.position.x, tf.position.y, tf.position.z))
    }

    /// Write a named entity's position (into its `Transform`, creating a
    /// default one if the entity has none yet).
    pub fn set_pos(&mut self, name: &str, x: f32, y: f32, z: f32) -> bool {
        let Some(&entity) = self.names.get(name) else { return false };
        if self.world.get::<&Transform>(entity).is_err() {
            let _ = self.world.insert_one(
                entity,
                Transform::new(glam::Vec3::new(x, y, z), glam::Vec3::new(1.0, 1.0, 1.0)),
            );
            return true;
        }
        if let Ok(mut tf) = self.world.get::<&mut Transform>(entity) {
            tf.position.x = x;
            tf.position.y = y;
            tf.position.z = z;
            true
        } else {
            false
        }
    }

    /// Set a named entity's `IsoSprite` frame index.  When the sprite's texture
    /// has a packed-atlas frame table, the matching `frame_name` is resolved so
    /// the packed path is used; otherwise the uniform-grid path takes over.
    pub fn set_sprite_frame(&mut self, name: &str, frame: f32) -> bool {
        let Some(&entity) = self.names.get(name) else { return false };
        let Ok(mut sprite) = self.world.get::<&mut IsoSprite>(entity) else { return false };
        sprite.frame = frame;
        sprite.frame_name = if self.frame_tables.contains_key(&sprite.texture) {
            Some(format!("{}_{}", sprite.texture, frame as u32))
        } else {
            None
        };
        true
    }

    /// Read a named entity's `IsoSprite` frame index.
    pub fn get_sprite_frame(&self, name: &str) -> Option<f32> {
        let entity = *self.names.get(name)?;
        self.world.get::<&IsoSprite>(entity).ok().map(|s| s.frame)
    }

    /// Set a named entity's `IsoSprite` tint colour (RGBA).
    pub fn set_sprite_color(&mut self, name: &str, color: [f32; 4]) -> bool {
        let Some(&entity) = self.names.get(name) else { return false };
        let Ok(mut sprite) = self.world.get::<&mut IsoSprite>(entity) else { return false };
        sprite.color = color;
        true
    }

    /// Set a named entity's `IsoSprite` visual offset (`frame_offset`, in world
    /// pixels; negative Y lifts the sprite on screen).  Lets guests elevate a
    /// runtime sprite (e.g. a container sliding out of a rocket).  Only valid
    /// for sprites without an animator or vehicle sim that overwrites
    /// `frame_offset` each frame.
    pub fn set_sprite_offset(&mut self, name: &str, dx: f32, dy: f32, dz: f32) -> bool {
        let Some(&entity) = self.names.get(name) else { return false };
        let Ok(mut sprite) = self.world.get::<&mut IsoSprite>(entity) else { return false };
        sprite.frame_offset = glam::Vec3::new(dx, dy, dz);
        true
    }

    /// Spawn a new `IsoSprite` entity cloned from a template entity (e.g. a
    /// mouse-follow placement ghost), so a guest can drop copies at runtime.
    /// Copies the template's `IsoSprite` and `Transform` (the latter carries the
    /// live position written by `set_pos`), plus any gameplay markers
    /// (`Selectable`, `Inventory`) the template carries; the caller then adjusts
    /// the clone with `set_pos`/`set_sprite_frame`/`set_sprite_color` as usual.
    /// Returns `false` when the name is taken, the template is unknown, or the
    /// template has no `IsoSprite`.
    pub fn spawn_sprite_clone(&mut self, template: &str, name: &str) -> bool {
        if self.names.contains_key(name) {
            return false;
        }
        let Some(&template_entity) = self.names.get(template) else { return false };
        let sprite = match self.world.get::<&IsoSprite>(template_entity) {
            Ok(s) => (*s).clone(),
            Err(_) => return false,
        };
        let transform = self
            .world
            .get::<&Transform>(template_entity)
            .ok()
            .map(|t| (*t).clone())
            .unwrap_or_else(|| Transform::new(sprite.position, sprite.scale));
        let selectable = self.world.get::<&Selectable>(template_entity).ok().map(|s| *s);
        let inventory = self
            .world
            .get::<&classic_core::inventory::Inventory>(template_entity)
            .ok()
            .map(|i| (*i).clone());

        let mut builder = hecs::EntityBuilder::new();
        builder.add(sprite);
        builder.add(transform);
        if let Some(s) = selectable {
            builder.add(s);
        }
        if let Some(inv) = inventory {
            builder.add(inv);
        }
        let entity = self.world.spawn(builder.build());
        self.register_named_entity(name, entity);
        true
    }

    /// The iso tile coordinates under the mouse cursor (from the tilemap).
    pub fn mouse_iso(&self) -> Option<(f32, f32)> {
        let tm_entity = self.entity_by_role(RoleKind::Tilemap)?;
        let tm = self.world.get::<&Tilemap>(tm_entity).ok()?;
        Some((tm.mouse_iso_pos.x, tm.mouse_iso_pos.y))
    }

    /// Show the container-inventory hover tooltip for a named entity, or hide
    /// it when `name` is empty.  The host resolves the entity and renders the
    /// tooltip from its `Inventory`; the guest only supplies *which* entity is
    /// hovered and *when* to show/hide.
    pub fn inventory_ui_show(&mut self, name: &str) {
        let target = if name.is_empty() { None } else { self.names.get(name).copied() };
        self.inventory_ui.set_target(target);
    }

    /// Project an iso tile coordinate to screen space, using the Tilemap-role
    /// entity's scale.  Returns `None` when no Tilemap-role entity exists.
    pub fn iso_to_screen(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        let tm_entity = self.entity_by_role(RoleKind::Tilemap)?;
        let tm = self.world.get::<&Tilemap>(tm_entity).ok()?;
        let iso = Mat4::from_scale(tm.scale) * cartesian_to_iso_4().inverse();
        let p = iso.transform_point3(Vec3::new(x, y, 0.0));
        Some((p.x, p.y))
    }

    /// The world → screen transform for the current frame, derived from the
    /// camera (`T(-fix) · S(scale)`), matching the sprite/terrain projection.
    fn world_to_screen_matrix(&self, vw: f32, vh: f32) -> Mat4 {
        let size = Vec3::new(vw, vh, 0.0);
        let fix = self.camera.position * self.camera.scale - size / Vec3::new(2.0, 2.0, 1.0);
        Mat4::from_translation(-fix) * Mat4::from_scale(self.camera.scale)
    }

    /// Project an iso tile coordinate (at terrain height) to screen pixels
    /// (top-left origin), matching the engine's sprite model + camera math.
    pub fn iso_to_screen_px(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        let tm_entity = self.entity_by_role(RoleKind::Tilemap)?;
        let tm = self.world.get::<&Tilemap>(tm_entity).ok()?;
        let tm_tf = self.world.get::<&Transform>(tm_entity).ok()?;

        let iso_to_cart_world = iso_to_cartesian_4() * Mat4::from_scale(tm_tf.scale);
        let mut world = iso_to_cart_world.transform_point3(Vec3::new(x, y, 0.0));
        world += tm_tf.position;
        let h = bilinear_height(&tm.height_data, tm.size_x, tm.size_y, x, y);
        world.y -= h * tm.height_scale;

        let (vw, vh) = self.viewport_size();
        let cam = self.world_to_screen_matrix(vw, vh);
        let screen = cam.transform_point3(world);
        Some((screen.x, screen.y))
    }

    /// Convert an iso tile coordinate to a **world-space** point (the same
    /// space consumed by the lit shaders' `worldPos` varyings and by
    /// [`Light::position`]).  This is the light-placement coordinate flag:
    /// world space is `iso_to_cartesian * scale * (x, y, 0)` plus the tilemap
    /// origin, with the surface height lifted along `-y` and carried in `z`
    /// (matching the tilemap shader's `worldPos.y -= z; worldPos.z = z`).
    ///
    /// `elevation` is metres above the sampled terrain surface (same units as
    /// `height_data`; `height_scale` converts metres to world px).  Returns
    /// `None` without a Tilemap-role entity.
    pub fn iso_to_world(&self, x: f32, y: f32, elevation: f32) -> Option<Vec3> {
        let tm_entity = self.entity_by_role(RoleKind::Tilemap)?;
        let (tm, tm_tf) = {
            let tm = self.world.get::<&Tilemap>(tm_entity).ok()?;
            let tf = self.world.get::<&Transform>(tm_entity).ok()?;
            (tm, tf)
        };
        let iso_to_cart_world = iso_to_cartesian_4() * Mat4::from_scale(tm_tf.scale);
        let mut cart = iso_to_cart_world.transform_point3(Vec3::new(x, y, 0.0));
        cart += tm_tf.position;
        let h = sample_height_mesh(&tm.height_data, tm.size_x, tm.size_y, x, y);
        let z_px = (h + elevation) * tm.height_scale;
        cart.y -= z_px;
        cart.z = z_px;
        Some(cart)
    }

    /// Terrain height (in world z units) at the given iso tile coordinate.
    pub fn height_at(&self, x: f32, y: f32) -> f32 {
        let Some(tm_entity) = self.entity_by_role(RoleKind::Tilemap) else { return 0.0 };
        let Ok(tm) = self.world.get::<&Tilemap>(tm_entity) else { return 0.0 };
        sample_height_mesh(&tm.height_data, tm.size_x, tm.size_y, x, y) * tm.height_scale
    }

    /// Write one tile index at tile coordinate `(x, y)` (bounds-checked).
    pub fn set_tile(&mut self, x: i32, y: i32, id: u32) -> bool {
        let Some(tm_entity) = self.entity_by_role(RoleKind::Tilemap) else { return false };
        let Ok(mut tm) = self.world.get::<&mut Tilemap>(tm_entity) else { return false };
        if x < 0 || y < 0 || x >= tm.size_x || y >= tm.size_y {
            return false;
        }
        let idx = (y as usize) * tm.size_x as usize + x as usize;
        let Some(t) = tm.data.get_mut(idx) else { return false };
        *t = id;
        true
    }

    /// Write one height vertex at coordinate `(x, y)` (bounds-checked; the
    /// height grid is a `(size_x + 1) × (size_y + 1)` vertex grid).
    pub fn set_height(&mut self, x: i32, y: i32, h: f32) -> bool {
        let Some(tm_entity) = self.entity_by_role(RoleKind::Tilemap) else { return false };
        let Ok(mut tm) = self.world.get::<&mut Tilemap>(tm_entity) else { return false };
        if x < 0 || y < 0 || x > tm.size_x || y > tm.size_y {
            return false;
        }
        let idx = (y as usize) * (tm.size_x as usize + 1) + x as usize;
        let Some(cell) = tm.height_data.get_mut(idx) else { return false };
        *cell = h.max(0.0);
        true
    }

    /// Rebuild the tilemap mesh and re-derive nav walkability after in-place
    /// tile/height edits (the guest-facing terrain-edit tail).
    pub fn rebuild_terrain(&mut self) -> bool {
        if self.entity_by_role(RoleKind::Tilemap).is_none() {
            return false;
        }
        self.rebuild_tilemap_mesh();
        self.sync_nav_heights();
        true
    }

    /// Bulk-write the tilemap tile grid from a guest-provided `u32` array
    /// (row-major, `size_x * size_y`).  Replaces the grid wholesale — the
    /// loaded component may be empty (`state_lunar.json` declares `"data":
    /// null`, generated at runtime).
    pub fn set_tiles_bulk(&mut self, tiles: &[u32]) -> bool {
        let Some(tm_entity) = self.entity_by_role(RoleKind::Tilemap) else { return false };
        let Ok(mut tm) = self.world.get::<&mut Tilemap>(tm_entity) else { return false };
        if tiles.len() != (tm.size_x * tm.size_y) as usize {
            return false;
        }
        tm.data = tiles.to_vec();
        true
    }

    /// Bulk-write the tilemap height vertex grid from a guest-provided `f32`
    /// array (`(size_x + 1) * (size_y + 1)`).  Replaces the grid wholesale.
    pub fn set_heights_bulk(&mut self, heights: &[f32]) -> bool {
        let Some(tm_entity) = self.entity_by_role(RoleKind::Tilemap) else { return false };
        let Ok(mut tm) = self.world.get::<&mut Tilemap>(tm_entity) else { return false };
        if heights.len() != ((tm.size_x + 1) * (tm.size_y + 1)) as usize {
            return false;
        }
        tm.height_data = heights.to_vec();
        true
    }

    /// Bulk-write the nav walkability grid from a guest-provided `u32` array
    /// (`size_x * size_y`, `1` = walkable).  Replaces the grid wholesale.
    pub fn set_nav_bulk(&mut self, nav: &[u32]) -> bool {
        let Some(nav_entity) = self.entity_by_role(RoleKind::NavMesh) else { return false };
        {
            let Ok(mut nm) = self.world.get::<&mut NavMesh>(nav_entity) else { return false };
            if nav.len() != (nm.size_x * nm.size_y) as usize {
                return false;
            }
            nm.data = nav.to_vec();
        }
        self.refresh_nav_snapshot();
        true
    }

    /// Upload a raw RGBA tileset texture for the tilemap (guest-generated
    /// tileset).  Replaces the current tileset under the Tilemap's `tile_set`.
    pub fn set_tileset_bulk(&mut self, rgba: &[u8], w: u32, h: u32) -> bool {
        let Some(tm_entity) = self.entity_by_role(RoleKind::Tilemap) else { return false };
        let Ok(tm) = self.world.get::<&Tilemap>(tm_entity) else { return false };
        let tile_set = tm.tile_set.clone();
        let Some(gfx) = self.gfx.as_mut() else { return false };
        gfx.add_texture_rgba8(&tile_set, rgba, w, h);
        true
    }

    /// Commit the tilemap terrain: install (first call) or rebuild (later
    /// calls) the tilemap mesh + tile data texture and re-upload the nav
    /// overlay.  Used by ROM guests to own their map, whether generated
    /// (bulk-uploaded via the `set_*` imports) or hand-authored (inline
    /// `state.json` data, hydrated here).  Does NOT re-derive walkability (the
    /// guest's nav grid is authoritative).  A tilemap with no height data is
    /// treated as flat (height 1.0 everywhere).
    pub fn commit_terrain(&mut self, height_scale: f32) -> bool {
        let Some(tm_entity) = self.entity_by_role(RoleKind::Tilemap) else {
            return false;
        };
        if let Ok(mut tm) = self.world.get::<&mut Tilemap>(tm_entity) {
            tm.height_scale = height_scale;
        }
        let installed = self.tilemap_gpu.contains_key(&self.debug_name(tm_entity));
        if installed {
            self.rebuild_tilemap_mesh();
        } else {
            let (tiles, mut heights, size_x, size_y) = {
                let tm = self.world.get::<&Tilemap>(tm_entity).unwrap();
                (tm.data.clone(), tm.height_data.clone(), tm.size_x, tm.size_y)
            };
            // A tilemap with no height data (e.g. a hand-authored map with only
            // inline tiles) renders flat at height 1.0.
            if heights.is_empty() {
                heights = vec![1.0f32; (size_x as usize + 1) * (size_y as usize + 1)];
            }
            self.finish_tilemap_init(tm_entity, tiles, heights, Some(height_scale));
        }
        self.rebuild_nav_gpu();
        self.refresh_nav_snapshot();
        true
    }

    /// Set a named entity's `Animator` to play a looping animation.
    pub fn set_anim(&mut self, name: &str, anim: &str) -> bool {
        let Some(&entity) = self.names.get(name) else { return false };
        if let Ok(mut a) = self.world.get::<&mut Animator>(entity) {
            a.animation = Some(anim.to_string());
            a.playing = true;
            a.repeat = true;
            true
        } else {
            false
        }
    }

    /// Restart a named entity's `Animator` from frame zero: reset the transient
    /// `counter`/`frame`/`offset`, then play `anim` (looping if `repeat`).
    pub fn start_anim(&mut self, name: &str, anim: &str, repeat: bool) -> bool {
        let Some(&entity) = self.names.get(name) else { return false };
        if let Ok(mut a) = self.world.get::<&mut Animator>(entity) {
            a.animation = Some(anim.to_string());
            a.repeat = repeat;
            a.playing = true;
            a.counter = 0.0;
            a.frame = 0.0;
            a.offset = Vec3::ZERO;
            true
        } else {
            false
        }
    }

    /// Read a named entity's current animation name and frame.
    pub fn get_anim(&self, name: &str) -> Option<(String, f32)> {
        let entity = *self.names.get(name)?;
        let a = self.world.get::<&Animator>(entity).ok()?;
        Some((a.animation.clone().unwrap_or_default(), a.frame))
    }

    /// Whether a named texture is available (declared in the ROM's resources or
    /// already uploaded to GL).
    pub fn has_texture(&self, name: &str) -> bool {
        let in_gfx = self.gfx.as_ref().map(|g| g.textures.contains_key(name)).unwrap_or(false);
        let in_rom =
            self.rom_resources.as_ref().map(|r| r.textures().contains_key(name)).unwrap_or(false);
        in_gfx || in_rom
    }

    /// Whether a named SDF font is available (declared in the ROM's resources
    /// or already loaded into `sdf_fonts`).
    pub fn has_font(&self, name: &str) -> bool {
        let in_metrics = self.sdf_fonts.contains_key(name);
        let in_rom =
            self.rom_resources.as_ref().map(|r| r.fonts().contains_key(name)).unwrap_or(false);
        in_metrics || in_rom
    }

    /// Whether a named animation is registered.
    pub fn has_animation(&self, name: &str) -> bool {
        self.animations.contains_key(name)
    }

    /// The pixel dimensions of a loaded texture, if any.
    pub fn texture_size(&self, name: &str) -> Option<(u32, u32)> {
        self.gfx.as_ref().and_then(|g| g.textures.get(name)).map(|t| t.size)
    }

    /// A* path over the nav mesh between two integer tile coordinates.
    /// Returns the full path (inclusive of both endpoints) or `None`.
    pub fn find_path(&self, from: (i32, i32), to: (i32, i32)) -> Option<Vec<(i32, i32)>> {
        let nav_entity = self.entity_by_role(RoleKind::NavMesh)?;
        let nav = self.world.get::<&NavMesh>(nav_entity).ok()?;
        let nav_i32: Vec<i32> = nav.data.iter().map(|&v| v as i32).collect();
        pathfinder::find_path(&nav_i32, nav.size_x, nav.size_y, from, to)
    }

    /// Footprint-aware A* over the nav mesh: treat the moving agent as a
    /// multi-tile object by eroding the walkability grid by `footprint` (a set
    /// of integer tile offsets from the anchor cell) before searching.  Used by
    /// `vehicle_goto`; the humanoid `IsoAgent` keeps the plain [`find_path`].
    pub fn find_path_for_footprint(
        &self,
        from: (i32, i32),
        to: (i32, i32),
        footprint: &[(i32, i32)],
    ) -> Option<Vec<(i32, i32)>> {
        let nav_entity = self.entity_by_role(RoleKind::NavMesh)?;
        let nav = self.world.get::<&NavMesh>(nav_entity).ok()?;
        let nav_i32: Vec<i32> = nav.data.iter().map(|&v| v as i32).collect();
        pathfinder::find_path_for_footprint(&nav_i32, nav.size_x, nav.size_y, from, to, footprint)
    }

    /// Footprint-, slope- and jump-aware A* for a wheeled vehicle (the
    /// synchronous fallback, used under `synchronous_workers`).  Builds the
    /// vehicle nav snapshot and delegates to [`pathfinder::find_vehicle_path_snapshot`],
    /// the same single code path the worker runs.
    #[allow(clippy::too_many_arguments)]
    pub fn find_vehicle_path(
        &self,
        from: (i32, i32),
        to: (i32, i32),
        footprint: &[(i32, i32)],
        pitch_max: f32,
        roll_max: f32,
        wheelbase_px: f32,
        track_px: f32,
        safe_fall_px: f32,
        jump_cost: f32,
        turn_cost: f32,
    ) -> Option<Vec<(i32, i32)>> {
        let obstacles = self.compute_nav_obstacles();
        let snapshot = self.build_vehicle_nav_snapshot(&obstacles)?;
        let result = pathfinder::find_vehicle_path_snapshot(
            &snapshot,
            from,
            to,
            footprint,
            pitch_max,
            roll_max,
            wheelbase_px,
            track_px,
            safe_fall_px,
            jump_cost,
            turn_cost,
        );
        classic_core::cl_info!(
            classic_core::instrument::Chan::Path,
            "find_vehicle_path {} -> {}: footprint={} tiles, pitch={:.3}rad fall={}px, found={}",
            format!("{from:?}"),
            format!("{to:?}"),
            footprint.len(),
            pitch_max.min(roll_max),
            safe_fall_px,
            result.is_some(),
        );
        result
    }

    /// Rebuild the shared nav snapshot from the live `NavMesh` component and
    /// re-share it with the pathfinding worker.  Bumps `nav_version`.
    fn refresh_nav_snapshot(&mut self) {
        let Some(nav_entity) = self.entity_by_role(RoleKind::NavMesh) else {
            return;
        };
        let (size_x, size_y, data) = {
            let Ok(nav) = self.world.get::<&NavMesh>(nav_entity) else {
                return;
            };
            (nav.size_x, nav.size_y, nav.data.iter().map(|&v| v as i32).collect::<Vec<_>>())
        };
        // Unified obstacles: footprints of `blocks_nav` colliders block both
        // humanoid and vehicle pathfinding.
        let obstacles = self.compute_nav_obstacles();
        let combined: Vec<i32> = data.iter().zip(&obstacles).map(|(&d, &o)| d & o).collect();
        let snapshot = Arc::new(pathfinder::NavSnapshot::new(size_x, size_y, combined));
        if let Some(worker) = self.pathfinder.as_mut() {
            worker.set_snapshot(Arc::clone(&snapshot));
        }
        self.nav_snapshot = snapshot;

        // Rebuild + push the vehicle nav snapshot (structural nav + heights +
        // height/tile scale) so the worker can run vehicle A* off-thread too.
        if let Some(vehicle_snapshot) = self.build_vehicle_nav_snapshot(&obstacles) {
            if let Some(worker) = self.pathfinder.as_mut() {
                worker.set_vehicle_snapshot(Arc::clone(&vehicle_snapshot));
            }
            self.vehicle_nav_snapshot = vehicle_snapshot;
        }

        self.refresh_guest_worker_nav();
        self.nav_version = self.nav_version.wrapping_add(1);
    }

    /// Build the unified obstacle grid (0 = blocked, 1 = open) from the
    /// footprints of every non-disabled entity whose collider has `blocks_nav`
    /// set.  Used for both humanoid and vehicle pathfinding.
    fn compute_nav_obstacles(&self) -> Vec<i32> {
        let Some(nav_entity) = self.entity_by_role(RoleKind::NavMesh) else { return Vec::new() };
        let Ok(nav) = self.world.get::<&NavMesh>(nav_entity) else { return Vec::new() };
        let (size_x, size_y) = (nav.size_x, nav.size_y);
        let mut grid = vec![1i32; (size_x * size_y) as usize];

        for (entity, (iso, tf)) in self.world.query::<(&IsoSprite, &Transform)>().iter() {
            if self.is_disabled(entity) || iso.footprint.is_empty() {
                continue;
            }
            let name = self.debug_name(entity);
            let Some(&pid) = self.collider_pids.get(&name) else { continue };
            if !self.physics.collider_blocks_nav(pid) {
                continue;
            }
            // Rasterize the footprint AABB (iso tile coords at the entity position).
            let mut min_x = f32::INFINITY;
            let mut max_x = f32::NEG_INFINITY;
            let mut min_y = f32::INFINITY;
            let mut max_y = f32::NEG_INFINITY;
            for pt in &iso.footprint {
                min_x = min_x.min(tf.position.x + pt.x);
                max_x = max_x.max(tf.position.x + pt.x);
                min_y = min_y.min(tf.position.y + pt.y);
                max_y = max_y.max(tf.position.y + pt.y);
            }
            let x0 = (min_x.floor() as i32).clamp(0, size_x - 1);
            let x1 = (max_x.floor() as i32).clamp(0, size_x - 1);
            let y0 = (min_y.floor() as i32).clamp(0, size_y - 1);
            let y1 = (max_y.floor() as i32).clamp(0, size_y - 1);
            for ty in y0..=y1 {
                for tx in x0..=x1 {
                    grid[(ty * size_x + tx) as usize] = 0;
                }
            }
        }
        grid
    }

    /// Build the [`pathfinder::VehicleNavSnapshot`] the worker uses for vehicle
    /// A*, from the live `NavMesh` (structural nav) and `Tilemap` (heights +
    /// scales).  Returns `None` when either component is missing.
    fn build_vehicle_nav_snapshot(
        &self,
        obstacles: &[i32],
    ) -> Option<Arc<pathfinder::VehicleNavSnapshot>> {
        let nav_entity = self.entity_by_role(RoleKind::NavMesh)?;
        let nav = self.world.get::<&NavMesh>(nav_entity).ok()?;
        let size_x = nav.size_x;
        let size_y = nav.size_y;
        // The vehicle `structural` grid gates hard obstacles (blocking
        // colliders); slope climbability is derived per-request from pitch/roll.
        let structural: Vec<i32> = obstacles.to_vec();

        let tm_entity = self.entity_by_role(RoleKind::Tilemap)?;
        let tm = self.world.get::<&Tilemap>(tm_entity).ok()?;
        Some(Arc::new(pathfinder::VehicleNavSnapshot::new(
            size_x,
            size_y,
            structural,
            tm.height_data.clone(),
            tm.height_scale,
            tm.scale.x,
        )))
    }

    /// Force pathfinding to run synchronously on the render thread (the
    /// deterministic test/golden harness) instead of offloading to a worker.
    pub fn set_synchronous_workers(&mut self, synchronous: bool) {
        self.synchronous_workers = synchronous;
    }

    /// The nav-snapshot version, bumped on every rebuild.
    pub fn nav_version(&self) -> u64 {
        self.nav_version
    }

    /// Submit an A* path request over the nav mesh and return its request id.
    ///
    /// When `synchronous_workers` is off, the search runs on a background
    /// worker (native thread or web `Worker`); the result is collected via
    /// [`Engine::poll_path`].  In synchronous mode the search runs inline and
    /// the result is immediately available to `poll_path`.
    pub fn request_path(&mut self, from: (i32, i32), to: (i32, i32)) -> u64 {
        let id = self.next_path_id;
        self.next_path_id = self.next_path_id.wrapping_add(1);

        if !self.synchronous_workers {
            self.ensure_pathfinder();
            if let Some(worker) = self.pathfinder.as_mut() {
                worker.request_path(id, from, to);
                return id;
            }
        }

        let poll = match self.nav_snapshot.find_path(from, to) {
            Some(path) => pathfinder::PathPoll::Path(path),
            None => pathfinder::PathPoll::NoPath,
        };
        self.sync_paths.insert(id, poll);
        id
    }

    /// Poll a previously submitted path request (non-blocking).
    ///
    /// Returns [`pathfinder::PathPoll::Pending`] while the search is still
    /// running, [`pathfinder::PathPoll::Path`] with the route, or
    /// [`pathfinder::PathPoll::NoPath`] if no route exists.
    pub fn poll_path(&mut self, id: u64) -> pathfinder::PathPoll {
        if let Some(poll) = self.sync_paths.remove(&id) {
            return poll;
        }
        if let Some(worker) = self.pathfinder.as_mut() {
            return worker.poll_path(id);
        }
        pathfinder::PathPoll::Pending
    }

    /// Block until all in-flight worker jobs have completed.  Determinism
    /// barrier, called at frame boundaries when `CLASSIC_TEST` is active
    /// (no-op on web, where determinism is handled by the sync fallback).
    pub fn join_workers(&mut self) {
        if let Some(worker) = self.pathfinder.as_ref() {
            worker.join();
        }
        if let Some(worker) = self.guest_worker.as_ref() {
            worker.join();
        }
    }

    /// Spawn the pathfinding worker on first use, sharing the current nav
    /// snapshot and vehicle nav snapshot.
    fn ensure_pathfinder(&mut self) {
        if self.pathfinder.is_none() {
            let mut worker = classic_worker::PathfinderWorker::new(Arc::clone(&self.nav_snapshot));
            worker.set_vehicle_snapshot(Arc::clone(&self.vehicle_nav_snapshot));
            self.pathfinder = Some(worker);
        }
    }

    /// Install the background guest worker (Tier 3), sharing the current nav
    /// snapshot.  The worker runs a second `.wasm` instance against the reduced
    /// pure-import surface (see `classic-worker::guest_worker`).  `synchronous`
    /// forces entries to run inline on the render thread (the deterministic
    /// test/golden harness).
    pub fn install_guest_worker(&mut self, wasm: &[u8], synchronous: bool) -> Result<(), String> {
        let worker =
            classic_worker::GuestWorker::new(wasm, Arc::clone(&self.nav_snapshot), synchronous)?;
        self.guest_worker = Some(worker);
        Ok(())
    }

    /// Submit a background guest task: run the named export of the worker guest
    /// with `arg` as its input bytes.  Returns a task id to poll with
    /// [`Engine::poll_task`].
    pub fn spawn_task(&mut self, entry: &str, arg: Vec<u8>) -> u64 {
        let id = self.next_task_id;
        self.next_task_id = self.next_task_id.wrapping_add(1);
        if let Some(worker) = self.guest_worker.as_mut() {
            worker.spawn_task(id, entry, arg);
        }
        id
    }

    /// Poll a previously submitted background task.  `None` while pending,
    /// `Some(Ok(bytes))` with the result, or `Some(Err(msg))` if it trapped.
    pub fn poll_task(&mut self, id: u64) -> Option<Result<Vec<u8>, String>> {
        self.guest_worker.as_mut().and_then(|worker| worker.poll_task(id))
    }

    /// Re-share the current nav snapshot with the background guest worker (and
    /// the pathfinding worker), e.g. after a terrain rebuild.
    fn refresh_guest_worker_nav(&mut self) {
        if let Some(worker) = self.guest_worker.as_mut() {
            worker.set_nav(Arc::clone(&self.nav_snapshot));
        }
    }

    /// Read the camera position (x, y) and uniform scale.
    pub fn get_camera(&self) -> (f32, f32, f32) {
        (self.camera.position.x, self.camera.position.y, self.camera.scale.x)
    }

    /// Set the camera position (x, y) and uniform scale.
    pub fn set_camera(&mut self, x: f32, y: f32, scale: f32) {
        self.camera.position.x = x;
        self.camera.position.y = y;
        self.camera.scale.x = scale;
        self.camera.scale.y = scale;
    }

    /// Show or hide the tilemap editor grid overlay.
    pub fn set_grid(&mut self, show: bool) {
        self.show_grid = show;
    }

    /// Register a collider and remember its owning entity's name, so
    /// [`Engine::pick_at`] can resolve it.
    pub fn register_named_collider(&mut self, name: &str, collider: ColliderData) -> u32 {
        let pid = self.physics.register_collider(collider);
        self.collider_names.insert(pid, name.to_string());
        self.collider_pids.insert(name.to_string(), pid);
        pid
    }

    /// Attach an axis-aligned rectangle collider to a named entity, at a screen
    /// position and size.  Combined with `subscribe`, this makes arbitrary
    /// (screen-space) entities clickable/hoverable from a guest.
    pub fn spawn_collider(&mut self, name: &str, x: f32, y: f32, w: f32, h: f32) -> bool {
        if !self.names.contains_key(name) {
            return false;
        }
        let verts = vec![
            glam::Vec3::new(0.0, 0.0, 0.0),
            glam::Vec3::new(w, 0.0, 0.0),
            glam::Vec3::new(w, h, 0.0),
            glam::Vec3::new(0.0, h, 0.0),
        ];
        let mut collider = ColliderData::new(classic_core::collision::polygon_from_verts(verts));
        collider.position = glam::Vec3::new(x, y, 0.0);
        collider.scale = glam::Vec3::ONE;
        self.register_named_collider(name, collider);
        true
    }

    /// The name of the top gameplay entity under a screen point, optionally
    /// filtered to entities carrying `filter`'s component (empty = any).  The
    /// filter is a component type name (e.g. `"Inventory"`, `"Selectable"`);
    /// an unknown name matches nothing.
    pub fn pick_at(&self, x: f32, y: f32, filter: &str) -> Option<String> {
        self.physics.point_query(x, y).into_iter().find_map(|pid| {
            let name = self.collider_names.get(&pid)?;
            if filter.is_empty() {
                return Some(name.clone());
            }
            let entity = self.names.get(name)?;
            if self.has_component(*entity, filter) {
                Some(name.clone())
            } else {
                None
            }
        })
    }

    /// Mark (or clear) a named entity's collider as a navigation obstacle.
    /// Rebuilds the nav snapshots so the change is reflected immediately.
    /// Returns `false` when the entity has no registered collider.
    pub fn set_collider_blocks_nav(&mut self, name: &str, blocks: bool) -> bool {
        if let Some(&pid) = self.collider_pids.get(name) {
            self.physics.set_collider_blocks_nav(pid, blocks);
            self.refresh_nav_snapshot();
            true
        } else {
            false
        }
    }

    /// Whether `entity` carries the named component.  The empty name matches
    /// every entity; recognized component type names map to a runtime check.
    fn has_component(&self, entity: hecs::Entity, name: &str) -> bool {
        match name {
            "" => true,
            "Inventory" => self.world.get::<&classic_core::inventory::Inventory>(entity).is_ok(),
            "Selectable" => self.world.get::<&Selectable>(entity).is_ok(),
            "IsoSprite" => self.world.get::<&IsoSprite>(entity).is_ok(),
            "IsoVehicle" => self.world.get::<&IsoVehicle>(entity).is_ok(),
            "Sprite" => self.world.get::<&SpriteRender>(entity).is_ok(),
            "SdfTextRender" => self.world.get::<&SdfTextRender>(entity).is_ok(),
            "Tilemap" => self.world.get::<&Tilemap>(entity).is_ok(),
            _ => false,
        }
    }

    /// The name of the top *subscribed* entity under a screen point, if any.
    fn pick_subscribed(&self, x: f32, y: f32) -> Option<String> {
        self.physics.point_query(x, y).into_iter().find_map(|pid| {
            self.collider_names.get(&pid).cloned().filter(|n| self.subscribed.contains(n))
        })
    }

    /// Subscribe a named entity to interaction events (click/enter/exit).
    pub fn subscribe(&mut self, name: &str) -> bool {
        if !self.names.contains_key(name) {
            return false;
        }
        self.subscribed.insert(name.to_string());
        true
    }

    /// Pop the next queued guest event, if any.
    pub fn poll_event(&mut self) -> Option<GuestEvent> {
        self.guest_events.pop_front()
    }

    /// Set a host-provided boolean flag visible to ROM guests.
    pub fn set_guest_flag(&mut self, name: &str, value: bool) {
        self.guest_flags.insert(name.to_string(), value);
    }

    /// Read a host-provided boolean flag (false when unset).
    pub fn guest_flag(&self, name: &str) -> bool {
        self.guest_flags.get(name).copied().unwrap_or(false)
    }

    /// Read the light uniforms (ambient, direction, color).
    pub fn get_light(&self) -> ([f32; 3], [f32; 3], [f32; 3]) {
        (self.light_ambient, self.light_dir, self.light_color)
    }

    /// Set the light uniforms (ambient, direction, color).
    pub fn set_light(&mut self, ambient: [f32; 3], dir: [f32; 3], color: [f32; 3]) {
        self.light_ambient = ambient;
        self.light_dir = dir;
        self.light_color = color;
    }

    /// Spawn a dynamic light in the pool, returning its handle (or `None` when
    /// the pool is full).  A `ttl` of `None` makes the light persistent; a
    /// finite `ttl` (seconds) auto-releases it after decaying.
    pub fn spawn_light(&mut self, light: Light, ttl: Option<f32>) -> Option<u32> {
        self.light_pool.spawn(light, ttl)
    }

    /// Overwrite an active pooled light's parameters by handle.
    pub fn update_light(&mut self, handle: u32, light: Light) -> bool {
        self.light_pool.set(handle, light)
    }

    /// Release a pooled light back to the free-list.
    pub fn release_light(&mut self, handle: u32) -> bool {
        self.light_pool.release(handle)
    }

    /// Spawn a named screen-space solid-color rectangle (a HUD element).
    pub fn spawn_rect(
        &mut self,
        name: &str,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: [f32; 4],
    ) -> bool {
        if self.names.contains_key(name) {
            return false;
        }
        let entity = self.world.spawn((
            Transform::new(glam::Vec3::new(x, y, 0.0), glam::Vec3::new(w, h, 1.0)),
            RectRender { color, ignore_cam: true },
        ));
        self.world.insert_one(entity, DebugName(name.to_string())).ok();
        self.names.insert(name.to_string(), entity);
        self.name_order.push(name.to_string());
        true
    }

    /// Spawn a named screen-space SDF text label.
    pub fn spawn_text(
        &mut self,
        name: &str,
        x: f32,
        y: f32,
        text: &str,
        scale: f32,
        color: [f32; 4],
    ) -> bool {
        if self.names.contains_key(name) {
            return false;
        }
        let entity = self.world.spawn((
            Transform::new(glam::Vec3::new(x, y, 0.0), glam::Vec3::new(scale, scale, 1.0)),
            SdfTextRender {
                atlas_name: classic_core::components::DEFAULT_SDF_FONT.into(),
                color,
                outline_color: [0.0, 0.0, 0.0, 0.0],
                outline_width: 0.0,
                ignore_cam: true,
                text: text.to_string(),
                justify: classic_core::components::TextJustify::Left,
                weight: 0.0,
                gamma: 1.0,
            },
        ));
        self.world.insert_one(entity, DebugName(name.to_string())).ok();
        self.names.insert(name.to_string(), entity);
        self.name_order.push(name.to_string());
        true
    }

    /// Update a named SDF text label's string.
    pub fn set_text(&mut self, name: &str, text: &str) -> bool {
        let Some(&entity) = self.names.get(name) else { return false };
        if let Ok(mut sdf) = self.world.get::<&mut SdfTextRender>(entity) {
            sdf.text = text.to_string();
            true
        } else {
            false
        }
    }

    /// Register an already-spawned entity under a guest-visible name.
    fn register_named_entity(&mut self, name: &str, entity: hecs::Entity) {
        self.world.insert_one(entity, DebugName(name.to_string())).ok();
        self.names.insert(name.to_string(), entity);
        self.name_order.push(name.to_string());
    }

    // ---- UIManager registration (guest-managed responsive UI) -------------
    //
    // These wrap the `UIManager` factories so a guest can create UI elements
    // that participate in layout (anchoring/array/padding/resize) under a name
    // it controls, without reimplementing any responsiveness.

    /// Spawn a named UI container (solid-color rectangle managed by layout).
    pub fn ui_container(&mut self, name: &str, w: f32, h: f32, color: [f32; 4]) -> bool {
        if self.names.contains_key(name) {
            return false;
        }
        let entity = {
            let Some(ui) = self.ui.as_mut() else { return false };
            ui.spawn_container(&mut self.world, w, h, color)
        };
        self.register_named_entity(name, entity);
        true
    }

    /// Spawn a named UI SDF text label managed by layout.
    pub fn ui_text(
        &mut self,
        name: &str,
        text: &str,
        scale: f32,
        max_width: f32,
        color: [f32; 4],
        justify: TextJustify,
    ) -> bool {
        if self.names.contains_key(name) {
            return false;
        }
        let entity = {
            let Some(ui) = self.ui.as_mut() else { return false };
            ui.spawn_sdf_text(&mut self.world, text, scale, max_width, color, justify)
        };
        self.register_named_entity(name, entity);
        true
    }

    /// Spawn a named UI button (container + centered text + click collider).
    /// The button is registered in the collider-name map and auto-subscribed,
    /// so its clicks surface through the guest event queue.
    pub fn ui_button(&mut self, name: &str, text: &str, w: f32, h: f32, color: [f32; 4]) -> bool {
        if self.names.contains_key(name) {
            return false;
        }
        let (entity, pid) = {
            let Some(ui) = self.ui.as_mut() else { return false };
            let entity = ui.spawn_button(
                &mut self.world,
                &mut self.physics,
                w,
                h,
                color,
                ui::ButtonOptions {
                    text: Some(text.to_string()),
                    text_scale: 0.4,
                    text_color: [1.0, 1.0, 1.0, 1.0],
                    sdf_text: true,
                    hover: true,
                    click_priority: 1,
                    ..Default::default()
                },
            );
            let pid = ui.collider_pid_for(entity);
            (entity, pid)
        };
        self.register_named_entity(name, entity);
        if let Some(pid) = pid {
            self.collider_names.insert(pid, name.to_string());
        }
        self.subscribed.insert(name.to_string());
        true
    }

    /// Spawn a named UI array container (vertical or horizontal stacking).
    pub fn ui_array(
        &mut self,
        name: &str,
        vertical: bool,
        align: UiAlign,
        spacing: f32,
        color: [f32; 4],
    ) -> bool {
        if self.names.contains_key(name) {
            return false;
        }
        let entity = {
            let Some(ui) = self.ui.as_mut() else { return false };
            ui.spawn_array(&mut self.world, vertical, align, spacing, color)
        };
        self.register_named_entity(name, entity);
        true
    }

    /// Spawn a named UI padding wrapper.
    pub fn ui_padding(
        &mut self,
        name: &str,
        top: f32,
        right: f32,
        bottom: f32,
        left: f32,
        color: [f32; 4],
    ) -> bool {
        if self.names.contains_key(name) {
            return false;
        }
        let entity = {
            let Some(ui) = self.ui.as_mut() else { return false };
            ui.spawn_padding(&mut self.world, top, right, bottom, left, color)
        };
        self.register_named_entity(name, entity);
        true
    }

    /// Spawn a named texture-sprite UI element.
    pub fn ui_sprite(
        &mut self,
        name: &str,
        texture: &str,
        w: f32,
        h: f32,
        frame: f32,
        tile_set_size: [f32; 2],
    ) -> bool {
        if self.names.contains_key(name) {
            return false;
        }
        let entity = {
            let Some(ui) = self.ui.as_mut() else { return false };
            ui.spawn_sprite(&mut self.world, texture, w, h, frame, tile_set_size)
        };
        self.register_named_entity(name, entity);
        true
    }

    /// Attach a named UI element as a child of another (anchor-based layout).
    pub fn ui_add_child(
        &mut self,
        parent: &str,
        child: &str,
        self_anchor: UiAnchor,
        child_anchor: UiAnchor,
    ) -> bool {
        let Some(&p) = self.names.get(parent) else { return false };
        let Some(&c) = self.names.get(child) else { return false };
        let Some(ui) = self.ui.as_mut() else { return false };
        ui.container_add_child(&mut self.world, p, c, self_anchor, child_anchor);
        true
    }

    /// Attach a named UI element to the root container (viewport-anchored).
    pub fn ui_add_to_root(
        &mut self,
        name: &str,
        self_anchor: UiAnchor,
        child_anchor: UiAnchor,
    ) -> bool {
        let Some(&c) = self.names.get(name) else { return false };
        let Some(ui) = self.ui.as_mut() else { return false };
        ui.root_add_child(&mut self.world, c, self_anchor, child_anchor);
        true
    }

    /// Set a named UI element's size.
    pub fn ui_set_size(&mut self, name: &str, w: f32, h: f32) -> bool {
        let Some(&e) = self.names.get(name) else { return false };
        {
            let Ok(mut n) = self.world.get::<&mut UiNode>(e) else { return false };
            n.size = glam::Vec2::new(w, h);
        }
        if let Some(ui) = self.ui.as_mut() {
            ui.mark_dirty();
        }
        true
    }

    /// Set a named UI element's anchor.
    pub fn ui_set_anchor(&mut self, name: &str, anchor: UiAnchor) -> bool {
        let Some(&e) = self.names.get(name) else { return false };
        {
            let Ok(mut n) = self.world.get::<&mut UiNode>(e) else { return false };
            n.anchor = anchor;
        }
        if let Some(ui) = self.ui.as_mut() {
            ui.mark_dirty();
        }
        true
    }

    /// Set a named UI rectangle's color.
    pub fn ui_set_color(&mut self, name: &str, color: [f32; 4]) -> bool {
        let Some(&e) = self.names.get(name) else { return false };
        if let Ok(mut r) = self.world.get::<&mut RectRender>(e) {
            r.color = color;
            true
        } else {
            false
        }
    }

    /// Set whether a named UI element is fixed (skips responsive layout).
    pub fn ui_set_fixed(&mut self, name: &str, fixed: bool) -> bool {
        let Some(&e) = self.names.get(name) else { return false };
        {
            let Ok(mut n) = self.world.get::<&mut UiNode>(e) else { return false };
            n.fixed = fixed;
        }
        if let Some(ui) = self.ui.as_mut() {
            ui.mark_dirty();
        }
        true
    }

    /// Save raw bytes to a file, handling both native (filesystem) and web
    /// (Blob download).
    pub fn save_bytes(&self, name: &str, bytes: &[u8]) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let dir = &crate::env_config::EnvConfig::get().dump_dir;
            let _ = std::fs::create_dir_all(dir);
            let path = format!("{dir}/{name}");
            if let Err(e) = std::fs::write(&path, bytes) {
                classic_core::cl_warn!(
                    classic_core::instrument::Chan::Dump,
                    "save_bytes: failed to write {path}: {e}"
                );
            } else {
                classic_core::cl_warn!(
                    classic_core::instrument::Chan::Dump,
                    "save_bytes: wrote {path} ({} bytes)",
                    bytes.len()
                );
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            use wasm_bindgen::JsCast;
            if let Some(window) = web_sys::window() {
                let doc = window.document().unwrap();
                let blob_parts = js_sys::Array::new();
                blob_parts.push(&js_sys::Uint8Array::from(bytes).into());
                let blob = web_sys::Blob::new_with_str_sequence(&blob_parts).unwrap();
                let url = web_sys::Url::create_object_url_with_blob(&blob).unwrap();
                let a = doc.create_element("a").unwrap();
                a.set_attribute("download", name).unwrap();
                a.set_attribute("href", &url).unwrap();
                a.dyn_ref::<web_sys::HtmlElement>().unwrap().click();
                web_sys::Url::revoke_object_url(&url).unwrap();
            }
        }
    }

    /// Save a UTF-8 text file (delegates to [`Engine::save_bytes`]).
    pub fn save_file(&self, name: &str, data: &str) {
        self.save_bytes(name, data.as_bytes());
    }

    /// Serialize the current world as a ROM archive and save it to
    /// `<entrypoint>.rom` — the canonical editor save (F10).
    pub fn save_rom(&self) -> bool {
        let Some(rom) = self.dump_rom() else {
            classic_core::cl_warn!(
                classic_core::instrument::Chan::Dump,
                "save_rom: no ROM loaded to save"
            );
            return false;
        };
        let name = format!("{}.rom", rom.manifest.entrypoint);
        match rom.pack() {
            Ok(bytes) => {
                self.save_bytes(&name, &bytes);
                true
            }
            Err(e) => {
                classic_core::cl_warn!(
                    classic_core::instrument::Chan::Dump,
                    "save_rom: failed to pack ROM: {e}"
                );
                false
            }
        }
    }

    pub fn frame(&mut self, input: &mut InputState, vw: f32, vh: f32, delta: f32) {
        let config = env_config::EnvConfig::get();
        let vw = config.forced_width.unwrap_or(vw);
        let vh = config.forced_height.unwrap_or(vh);
        let delta = config.fixed_dt.unwrap_or(delta);

        classic_core::cl_every!(
            Chan::Frame,
            60,
            log::Level::Info,
            "frame dt={:.3} fps={}",
            delta,
            self.time.fps
        );

        self.input = input.clone();
        self.time.delta = delta;
        if delta > 0.0 {
            self.time.fps = (1.0 / delta) as u32;
        }
        let mp = self.input.mouse_pos;

        if (vw - self.last_vw).abs() > 0.5 || (vh - self.last_vh).abs() > 0.5 {
            self.last_vw = vw;
            self.last_vh = vh;
            self.physics.resize_screen(vw, vh);
            if let Some(gfx) = self.gfx.as_mut() {
                gfx.resize(vw, vh);
            }
            if let Some(ref mut um) = self.ui {
                um.resize(&mut self.world, vw, vh);
            }
        }

        // Offscreen FBO for headless/golden runs.
        if let Some(gfx) = self.gfx.as_mut() {
            let need_offscreen = config.offscreen || config.headless;
            if need_offscreen && gfx.render_target.is_none() {
                gfx.set_render_target(vw as u32, vh as u32);
            } else if need_offscreen {
                if let Some(ref mut rt) = gfx.render_target {
                    let w = vw as u32;
                    let h = vh as u32;
                    if rt.width != w || rt.height != h {
                        rt.resize(&gfx.gl, w, h);
                    }
                }
            }
        }

        // Refresh UI layout every frame (after resize, on-update closures,
        // and before physics/rendering).
        if let Some(ref mut ui) = self.ui {
            if ui.dirty {
                ui.refresh_layout(&mut self.world);
                ui.sync_colliders(&self.world, &mut self.physics);
                ui.dirty = false;
            }
        }

        // Sync world-space colliders for selectable entities, then project them
        // (and any other World colliders) to screen before rebuilding the tree.
        self.sync_selectable_colliders();
        self.physics.set_world_to_screen(self.world_to_screen_matrix(vw, vh));
        self.physics.begin_frame();
        classic_core::cl_debug!(classic_core::instrument::Chan::Collision, "begin_frame");
        self.physics.mouse.position = Vec3::new(mp.x, mp.y, 0.0);
        self.physics.mouse.update_rect();
        self.physics.consumed_click = false;
        // mouse_clicked MUST be set before perform_calls. Without it,
        // collider click handlers fire every frame on hover, not just on press.
        self.physics.mouse_clicked = self.input.was_mouse_pressed(0);
        // perform_calls dispatches clicks, enter/exit events, and selection.
        self.physics.perform_calls();

        // ui_consumed_click blocks map editing on UI palette clicks.
        // Set AFTER perform_calls; reset AFTER the final release-path guard.
        if self.physics.consumed_click {
            self.set_guest_flag("ui_consumed_click", true);
        }

        // Per-frame hover highlighting for UI elements.
        if let Some(ref mut ui) = self.ui {
            ui.update_hover(&mut self.world, &self.physics);
        }

        // Guest interaction events: click + enter/exit for subscribed entities.
        if !self.subscribed.is_empty() {
            if self.physics.mouse_clicked {
                if let Some(name) = self.pick_subscribed(mp.x, mp.y) {
                    self.guest_events.push_back(GuestEvent { kind: 0, name });
                }
            }
            let current = self.pick_subscribed(mp.x, mp.y);
            if current != self.guest_hover {
                if let Some(h) = self.guest_hover.clone() {
                    self.guest_events.push_back(GuestEvent { kind: 2, name: h });
                }
                if let Some(c) = current.clone() {
                    self.guest_events.push_back(GuestEvent { kind: 1, name: c });
                }
                self.guest_hover = current;
            }
        }

        if self.input.was_mouse_pressed(0) && !self.guest_flag("ui_consumed_click") {
            self.selection_mode = 1;
            self.selection_begin_screen = Vec3::new(mp.x, mp.y, 0.0);
            if let Some(e) = self.entity_by_role(RoleKind::Tilemap) {
                if let Ok(mut tm) = self.world.get::<&mut Tilemap>(e) {
                    tm.selection_iso_begin = tm.mouse_iso_pos;
                }
            }
            self.physics.begin_selection(Vec3::new(mp.x, mp.y, 0.0));
        }

        // Right-click clears the RTS selection (and, via the guest's
        // `selected_names`, any in-progress drop preview).
        if self.input.was_mouse_pressed(1) && !self.guest_flag("ui_consumed_click") {
            self.selection_clear();
        }

        // Stretch selection rect every frame while dragging.
        if self.selection_mode == 1 {
            self.physics.update_selection(self.selection_begin_screen, Vec3::new(mp.x, mp.y, 0.0));
        }

        // RTS rubber band (screen-space rectangle), shown only while dragging and
        // a terrain-paint tool is not active.
        self.rts_box = if self.selection_mode == 1 && self.guest_flag("rts_selection") {
            Some((
                Vec2::new(self.selection_begin_screen.x, self.selection_begin_screen.y),
                Vec2::new(mp.x, mp.y),
            ))
        } else {
            None
        };

        // Frame ordering: demo pre-update hooks run BEFORE on_update closures.
        // The camera's on_update runs first in registration order and would
        // consume the wheel; the demo's text-scroll hook zeroes it first.
        let mut pre = std::mem::take(&mut self.pre_update_hooks);
        for f in pre.iter_mut() {
            f(self);
        }
        pre.append(&mut self.pre_update_hooks);
        self.pre_update_hooks = pre;

        // Take-restore dance: closures fire with &mut Engine, but the Vec
        // is owned by Engine. Taking means closures can call on_update()
        // without borrow conflicts. Restoring preserves them for next frame.
        // Closures registered *during* the loop (e.g. the tilemap's mouse-iso
        // solve, installed lazily by `commit_terrain`) land in the emptied
        // `self.update_fns`; append them before restoring so they survive.
        // Handlers use iter_mut(), NOT std::mem::take — they must survive
        // across frames (click, enter, exit, selection).
        let mut fns = std::mem::take(&mut self.update_fns);
        for f in fns.iter_mut() {
            f(self);
        }
        fns.append(&mut self.update_fns);
        self.update_fns = fns;

        // Wheeled-vehicle simulation runs after the guest update closures
        // (so a `vehicle_goto` issued this frame is honoured) and before the
        // render list is built.
        self.update_vehicles();

        // ---- CLASSIC_TEST automated test runner (registered by the demo) ----
        if env_config::EnvConfig::get().test_active() {
            let mut runner = self.test_runner.take();
            if let Some(r) = runner.as_mut() {
                r(self);
            }
            self.test_runner = runner;
        }

        // Determinism barrier: under CLASSIC_TEST, wait for in-flight worker
        // jobs (pathfinding) so frame boundaries are deterministic.
        if env_config::EnvConfig::get().test_active() {
            self.join_workers();
        }

        // Wheel decay: 1.4 * delta, then [-1, 1] clamp.
        // Without write-back, decay resets to zero every frame.
        let mw = &mut self.input.mouse_wheel;
        *mw = (mw.abs() - 1.4 * self.time.delta).max(0.0) * mw.signum();
        if mw.abs() < 0.01 {
            *mw = 0.0;
        }
        *mw = (*mw).clamp(-1.0, 1.0);
        // Write back to platform so decay persists across frames.
        input.mouse_wheel = self.input.mouse_wheel;

        let ui_debug = env_config::EnvConfig::get().ui_debug && self.debug_frame < 120;
        if ui_debug {
            let Some(ref ui) = self.ui else { return };
            log::info!(
                "=== frame {} vp={:.0}x{:.0} ===",
                self.debug_frame,
                ui.viewport_w,
                ui.viewport_h
            );
            let mut ents: Vec<_> = self
                .world
                .query::<(&Transform, &classic_core::components::UiNode)>()
                .iter()
                .map(|(e, (tf, n))| (e, tf.clone(), n.clone()))
                .collect();
            ents.sort_by(|a, b| a.2.kind.kind_str().cmp(b.2.kind.kind_str()));
            for (e, tf, node) in &ents {
                log::info!(
                    "  [{:?}] {:?} pos=({:.0},{:.0}) size=({:.0},{:.0}) z={:.0} enabled={} parent={:?} children={}",
                    node.kind.kind_str(),
                    e.id(),
                    tf.position.x, tf.position.y,
                    node.size.x, node.size.y,
                    tf.position.z,
                    self.world.get::<&classic_core::components::Disabled>(*e).is_err(),
                    node.parent,
                    node.children.len(),
                );
            }
        }
        self.debug_frame += 1;
        classic_core::instrument::set_frame(self.debug_frame);

        // Reset here, after the last read in the mouse-release guard.
        // If reset earlier (e.g. at the top of frame()), click-through
        // protection on editor-paint is dead.
        self.set_guest_flag("ui_consumed_click", false);

        if self.input.was_mouse_released(0) && !self.guest_flag("ui_consumed_click") {
            let just_finished_selection = self.selection_mode == 1;
            if self.selection_mode == 1 {
                self.selection_mode = -1;
                if let Some(e) = self.entity_by_role(RoleKind::Tilemap) {
                    if let Ok(mut tm) = self.world.get::<&mut Tilemap>(e) {
                        tm.selection_iso_end = tm.mouse_iso_pos;
                    }
                }
            }
            self.physics.end_selection();

            // RTS selection (host-owned): click = point-select, drag = box-select,
            // shift = additive.  Gated on `rts_selection` (cleared while a
            // terrain-paint tool owns the drag gesture).
            if just_finished_selection && self.guest_flag("rts_selection") {
                let begin = Vec2::new(self.selection_begin_screen.x, self.selection_begin_screen.y);
                let end = Vec2::new(mp.x, mp.y);
                let additive =
                    self.input.is_key_down("ShiftLeft") || self.input.is_key_down("ShiftRight");
                if (end - begin).length() < RTS_DRAG_THRESHOLD_PX {
                    self.select_at(end.x, end.y, additive);
                } else {
                    self.select_box((begin.x, begin.y), (end.x, end.y), additive);
                }
            }
            self.rts_box = None;

            // Editor paint on selection-end (registered by the demo).
            if just_finished_selection {
                let mut hooks = std::mem::take(&mut self.selection_end_hooks);
                for h in hooks.iter_mut() {
                    h(self);
                }
                self.selection_end_hooks = hooks;
            }
        }

        // Container-inventory hover tooltip (host-owned).  Reconcile it before
        // the render list is built so show/hide, content, and position are
        // atomic with this frame's render — no one-frame lag on hover changes
        // or camera pan/zoom.  Take-and-restore so `sync` can re-borrow `self`.
        let mut inventory_ui = std::mem::take(&mut self.inventory_ui);
        inventory_ui.sync(self);
        self.inventory_ui = inventory_ui;

        // Render-list: sprites + tilemaps + iso sprites
        let mut items: Vec<(f32, hecs::Entity, DrawKind)> = Vec::new();
        for (e, (tf, sprite)) in self.world.query::<(&Transform, &SpriteRender)>().iter() {
            if self.is_disabled(e) {
                continue;
            }
            let is_ui_sprite = self
                .world
                .get::<&classic_core::components::UiNode>(e)
                .map(|n| matches!(n.kind, classic_core::components::UiKind::Sprite))
                .unwrap_or(false);
            if is_ui_sprite {
                items.push((tf.position.z, e, DrawKind::UiSprite));
            } else {
                let z = if sprite.ignore_cam { -20000.0 } else { tf.position.z };
                items.push((z, e, DrawKind::Sprite));
            }
        }
        for (e, (_, _tm)) in self.world.query::<(&Transform, &Tilemap)>().iter() {
            if self.is_disabled(e) {
                continue;
            }
            items.push((20000.0, e, DrawKind::Tilemap));
        }
        for (e, (_, _)) in self.world.query::<(&Transform, &NavMesh)>().iter() {
            if self.is_disabled(e) {
                continue;
            }
            if self.world.get::<&Role>(e).is_ok_and(|r| r.value == RoleKind::NavMesh)
                && self.nav_gpu.is_some()
            {
                items.push((19999.0, e, DrawKind::Tilemap));
            }
        }
        for (e, (tf, _)) in self.world.query::<(&Transform, &IsoSprite)>().iter() {
            if self.is_disabled(e) {
                continue;
            }
            let iso_order = tf.position.x - tf.position.y;
            items.push((iso_order, e, DrawKind::IsoSprite));
        }
        for (e, (tf, _)) in self.world.query::<(&Transform, &RectRender)>().iter() {
            if self.is_disabled(e) {
                continue;
            }
            items.push((tf.position.z, e, DrawKind::UiRect));
        }
        for (e, (tf, _)) in self.world.query::<(&Transform, &SdfTextRender)>().iter() {
            if self.is_disabled(e) {
                continue;
            }
            items.push((tf.position.z, e, DrawKind::SdfText));
        }
        // Descending sort by sort-key (z-order or iso-order).
        // Uses sort_by (not sort_unstable_by) for deterministic golden traces.
        items.sort_by(|a, b| b.0.total_cmp(&a.0));

        // The isometric sprites, in the same (sorted) order they appear in
        // `items`, drawn in two explicit passes (normals then ghosts).
        let iso_items: Vec<(f32, hecs::Entity)> = items
            .iter()
            .filter(|(_, _, k)| matches!(k, DrawKind::IsoSprite))
            .map(|(o, e, _)| (*o, *e))
            .collect();

        classic_core::cl_debug!(
            classic_core::instrument::Chan::Render,
            "render: {} draw items",
            items.len()
        );

        // Pre-compute entity debug names before we borrow gfx mutably,
        // so we can still look up names during the draw loop below.
        let entity_names: Vec<(hecs::Entity, String)> =
            items.iter().map(|(_, e, _)| (*e, self.debug_name(*e))).collect();
        let name_by_entity: HashMap<hecs::Entity, &str> =
            entity_names.iter().map(|(e, n)| (*e, n.as_str())).collect();

        // The tilemap's paint highlight only shows when a terrain tool owns the
        // drag; under RTS selection the tilemap selection is off.
        let paint_mode = if self.guest_flag("rts_selection") { -1 } else { self.selection_mode };

        // Decay transient lights and gather the active set for the UBO upload.
        self.light_pool.decay(delta);
        let lights = self.light_pool.gather();

        let Some(gfx) = self.gfx.as_mut() else { return };
        let vp = gfx.viewport_w;
        let vh2 = gfx.viewport_h;
        self.camera.size = Vec3::new(vp, vh2, 0.0);

        // begin_frame sets depthFunc/depthMask but does NOT glEnable(DEPTH_TEST).
        // draw_tilemap/draw_iso_sprite toggle it locally. UI/SDF runs without it.
        // Enabling it globally depth-rejects all UI under ortho projection.
        gfx.begin_frame();
        // Upload the dynamic light block once per frame (consumed by the lit
        // tilemap + sprite shaders).
        gfx.upload_lights(&lights);
        let cam = self.camera.matrix();

        // Create trace collector when golden mode is active and we're on the capture frame.
        let golden_active = !config.golden_mode.is_empty();
        if golden_active && self.debug_frame == self.golden_capture_frame {
            self.trace = Some(golden::TraceCollector::new(
                "baseline",
                vp,
                vh2,
                &cam,
                self.camera.position,
                self.camera.scale,
            ));
        }

        // Model matrix z MUST stay inside [-10000, 10000] — the orthographic
        // projection clips everything outside. The sort key can differ from
        // the model z. Cursor uses sort_z=-20000 but model_z=-10000.
        // Expand the selection set to include a selected vehicle's wheels and
        // steering tires, so the silhouette outlines the whole vehicle, not
        // just its body.
        let mut visual_selected: HashSet<hecs::Entity> =
            self.selection.selected.iter().copied().collect();
        {
            let selected: Vec<hecs::Entity> = self.selection.selected.iter().copied().collect();
            for entity in selected {
                if let Ok(veh) = self.world.get::<&IsoVehicle>(entity) {
                    for name in veh.wheel_entities.iter().chain(veh.tire_entities.iter()) {
                        if name.is_empty() {
                            continue;
                        }
                        if let Some(&part) = self.names.get(name) {
                            visual_selected.insert(part);
                        }
                    }
                }
            }
        }

        // Precompute isometric-sprite draw params once, shared by the normal
        // and ghost passes below.
        let mut iso_draws: Vec<IsoDraw> = Vec::new();
        for (order, entity) in &iso_items {
            let Ok(tf) = self.world.get::<&Transform>(*entity) else {
                continue;
            };
            let Ok(iso_sprite) = self.world.get::<&IsoSprite>(*entity) else {
                continue;
            };
            let Some(&tm_entity) = self.names.get(&iso_sprite.tilemap) else {
                continue;
            };
            let Ok(tilemap_tf) = self.world.get::<&Transform>(tm_entity) else {
                continue;
            };
            let Ok(tilemap) = self.world.get::<&Tilemap>(tm_entity) else {
                continue;
            };
            // Resolve a packed-atlas frame (issue #45); falls back to the
            // uniform-grid path when no `frame_name` / table match.
            let frame_ref = iso_sprite
                .frame_name
                .as_deref()
                .and_then(|n| Self::resolve_frame(&self.frame_tables, &iso_sprite.texture, n));

            // (quad size, anchor px, sheet name, uv params).  Packed frames are
            // drawn at their source cell size with the trimmed content offset;
            // the uniform-grid path uses the cell size directly.
            let (tex_dim, anchor_px, sheet_name, uv) = match &frame_ref {
                Some(fr) => {
                    let sw =
                        if fr.source_size[0] > 0 { fr.source_size[0] as f32 } else { fr.size[0] };
                    let sh =
                        if fr.source_size[1] > 0 { fr.source_size[1] as f32 } else { fr.size[1] };
                    let (cw, ch) = (fr.size[0], fr.size[1]);
                    let (bx, by) = (fr.trim_offset[0] as f32, fr.trim_offset[1] as f32);
                    let a_trim = Self::effective_anchor(iso_sprite.anchor, fr);
                    let anchor_px = Vec2::new(a_trim.x * cw + bx, a_trim.y * ch + by);
                    (
                        (sw, sh),
                        anchor_px,
                        fr.sheet_name.clone(),
                        Some((fr.uv_rect, [bx, by], [sw, sh], [cw, ch])),
                    )
                }
                None => {
                    let Some(tex) = gfx.textures.get(&iso_sprite.texture) else {
                        continue;
                    };
                    let td = (
                        tex.size.0 as f32 / iso_sprite.tile_set_size.x.max(0.001),
                        tex.size.1 as f32 / iso_sprite.tile_set_size.y.max(0.001),
                    );
                    let anchor_px =
                        Vec2::new(td.0 * iso_sprite.anchor.x, td.1 * iso_sprite.anchor.y);
                    (td, anchor_px, iso_sprite.texture.clone(), None)
                }
            };

            let model = Self::compute_iso_sprite_model(
                &iso_sprite,
                &tf,
                &tilemap_tf,
                &tilemap,
                tex_dim,
                anchor_px,
            );
            let h_depth = horizontal_depth_scale(tilemap.size_x, tilemap.size_y);
            let depth_corners =
                Self::compute_iso_depth_corners(tf.position, &iso_sprite.footprint, h_depth);
            // Per-sheet normal/depth companions (from the resolved frame's
            // sheet) win; fall back to the per-texture `entry.normal`/`depth`
            // manifest fields for assets not on a shared atlas.
            let depth_map = frame_ref.as_ref().and_then(|fr| fr.depth_tex.clone()).or_else(|| {
                self.texture_depths
                    .get(&iso_sprite.texture)
                    .map(|d| (d.depth_tex.clone(), d.depth_range))
            });
            let normal_map = frame_ref
                .as_ref()
                .and_then(|fr| fr.normal_tex.clone())
                .or_else(|| self.texture_normals.get(&iso_sprite.texture).cloned());
            iso_draws.push(IsoDraw {
                order: *order,
                name: name_by_entity.get(entity).copied().unwrap_or("").to_string(),
                model,
                texture: sheet_name,
                frame: iso_sprite.frame,
                tile_set_size: [iso_sprite.tile_set_size.x, iso_sprite.tile_set_size.y],
                uv,
                depth_corners,
                depth_map,
                depth_base: Self::compute_iso_base_depth(tf.position, h_depth),
                normal_map,
                ghost_group: iso_sprite.ghost_group,
                color: iso_sprite.color,
                selected: visual_selected.contains(entity),
            });
        }

        // Phase 1: terrain (tilemap + nav mesh) — writes the depth buffer.
        for (order, entity, kind) in &items {
            if !matches!(kind, DrawKind::Tilemap) {
                continue;
            }
            let Ok(tf) = self.world.get::<&Transform>(*entity) else {
                continue;
            };
            if let DrawKind::Tilemap = kind {
                let is_nav =
                    self.world.get::<&Role>(*entity).is_ok_and(|r| r.value == RoleKind::NavMesh);

                if is_nav {
                    let Some(ref gpu) = self.nav_gpu else { continue };
                    if let Ok(nav) = self.world.get::<&NavMesh>(*entity) {
                        let iso = cartesian_to_iso_4().inverse();
                        let iso_matrix = Mat4::from_scale(tf.scale) * iso;
                        let iso3 = Mat3::from_mat4(iso);
                        let normal_matrix = iso3.inverse().transpose();
                        let nav_ts = gfx
                            .textures
                            .get(&nav.tile_set)
                            .map(|t| [t.size.0 as f32 / 8.0, t.size.1 as f32 / 8.0])
                            .unwrap_or([2.0, 1.0]);
                        if let Some(ref mut t) = self.trace {
                            let name = name_by_entity.get(entity).copied().unwrap_or("");
                            t.push(golden::TraceItemParams {
                                order: *order,
                                kind: "Tilemap",
                                name,
                                model: &Mat4::from_translation(tf.position),
                                camera_ignored: false,
                                texture: Some(&nav.tile_set),
                                frame: None,
                                color: None,
                                depth: None,
                                depth_range: None,
                                normal: None,
                            });
                        }
                        gfx.draw_tilemap(
                            &Mat4::from_translation(tf.position),
                            &cam,
                            &iso_matrix,
                            &gpu.tile_tex,
                            &nav.tile_set,
                            &nav_ts,
                            &[8.0, 8.0],
                            &[nav.size_x as f32, nav.size_y as f32],
                            &[0.0, 0.0],
                            &[-1.0, -1.0],
                            -1,
                            &[0.0, 0.0, 1.0, 0.3],
                            &RenderSettings {
                                ambient: self.light_ambient,
                                light_dir: self.light_dir,
                                light_color: self.light_color,
                                depth_scale: [
                                    horizontal_depth_scale(nav.size_x, nav.size_y),
                                    HEIGHT_DEPTH_SCALE_M,
                                ],
                                ppm: PPM_TARGET,
                                normal_matrix,
                            },
                            false,
                            gpu.vertex_count as i32,
                            &gpu.mesh_buf,
                        );
                    }
                    continue;
                }

                let Ok(tm) = self.world.get::<&Tilemap>(*entity) else {
                    continue;
                };
                // Look up GPU data by entity name (avoid borrow conflict with gfx).
                let entity_name = name_by_entity.get(entity).copied().unwrap_or("");
                let Some(gpu) = self.tilemap_gpu.get(entity_name) else {
                    continue;
                };
                // Build the iso matrix.
                let iso = cartesian_to_iso_4().inverse();
                let iso_matrix = Mat4::from_scale(tf.scale) * iso;

                // Normal matrix: transpose(inverse(mat3(iso)))
                let iso3 = Mat3::from_mat4(iso);
                let normal_matrix = iso3.inverse().transpose();

                let tps = tm.tile_pixel_size;
                let tile_pixel_size = [tps[0] as f32, tps[1] as f32];
                let tile_set_size = [
                    tm.tile_set_pixel_size[0] as f32 / tile_pixel_size[0],
                    tm.tile_set_pixel_size[1] as f32 / tile_pixel_size[1],
                ];

                if let Some(ref mut t) = self.trace {
                    let name = name_by_entity.get(entity).copied().unwrap_or("");
                    t.push(golden::TraceItemParams {
                        order: *order,
                        kind: "Tilemap",
                        name,
                        model: &Mat4::from_translation(tf.position),
                        camera_ignored: false,
                        texture: Some(&tm.tile_set),
                        frame: None,
                        color: None,
                        depth: None,
                        depth_range: None,
                        normal: None,
                    });
                }

                gfx.draw_tilemap(
                    &Mat4::from_translation(tf.position),
                    &cam,
                    &iso_matrix,
                    &gpu.tile_tex,
                    &tm.tile_set,
                    &tile_set_size,
                    &tile_pixel_size,
                    &[tm.size_x as f32, tm.size_y as f32],
                    &[tm.mouse_iso_pos.x, tm.mouse_iso_pos.y],
                    &[tm.selection_iso_begin.x, tm.selection_iso_begin.y],
                    paint_mode,
                    &[0.0, 1.0, 1.0, 1.0],
                    &RenderSettings {
                        ambient: self.light_ambient,
                        light_dir: self.light_dir,
                        light_color: self.light_color,
                        depth_scale: [
                            horizontal_depth_scale(tm.size_x, tm.size_y),
                            HEIGHT_DEPTH_SCALE_M,
                        ],
                        ppm: PPM_TARGET,
                        normal_matrix,
                    },
                    self.show_grid,
                    gpu.vertex_count as i32,
                    &gpu.mesh_buf,
                );
            }
        }

        // Phase 2: isometric normal passes — draw on top of terrain, writing
        // depth (depth-mapped sprites) and stencil ghost-group ids.  A single
        // `RenderSettings` (shared by both sprite passes) carries the light
        // preset; the sprite shader only consumes the ambient/dir/color terms
        // (the normal is baked world-space, so `normal_matrix` is unused by
        // sprites, but the struct needs a value).
        let iso = cartesian_to_iso_4().inverse();
        let sprite_settings = RenderSettings {
            ambient: self.light_ambient,
            light_dir: self.light_dir,
            light_color: self.light_color,
            depth_scale: [HORIZONTAL_DEPTH_SCALE, HEIGHT_DEPTH_SCALE_M],
            ppm: PPM_TARGET,
            normal_matrix: Mat3::from_mat4(iso).inverse().transpose(),
        };
        for draw in &iso_draws {
            if let Some(ref mut t) = self.trace {
                t.push(golden::TraceItemParams {
                    order: draw.order,
                    kind: "IsoSprite",
                    name: &draw.name,
                    model: &draw.model,
                    camera_ignored: false,
                    texture: Some(&draw.texture),
                    frame: Some(draw.frame),
                    color: None,
                    depth: draw.depth_map.as_ref().map(|(t, _)| t.as_str()),
                    depth_range: draw.depth_map.as_ref().map(|(_, r)| *r),
                    normal: draw.normal_map.as_deref(),
                });
            }
            gfx.draw_iso_sprite(
                &draw.model,
                &cam,
                &draw.texture,
                draw.region(),
                &draw.depth_corners,
                draw.depth_map.as_ref().map(|(t, r)| (t.as_str(), *r)),
                draw.depth_base,
                draw.normal_map.as_deref(),
                &[draw.color[0], draw.color[1], draw.color[2]],
                &sprite_settings,
                draw.ghost_group,
                IsoSpritePass::Normal,
                draw.selected,
                &SELECTION_COLOR,
                OUTLINE_RADIUS_PX,
            );
        }

        // Phase 3: isometric ghost passes — 40% alpha where behind the depth
        // buffer, skipping pixels the sprite's own ghost group already occludes.
        for draw in &iso_draws {
            gfx.draw_iso_sprite(
                &draw.model,
                &cam,
                &draw.texture,
                draw.region(),
                &draw.depth_corners,
                draw.depth_map.as_ref().map(|(t, r)| (t.as_str(), *r)),
                draw.depth_base,
                draw.normal_map.as_deref(),
                &[draw.color[0], draw.color[1], draw.color[2]],
                &sprite_settings,
                draw.ghost_group,
                IsoSpritePass::Ghost,
                draw.selected,
                &SELECTION_COLOR,
                OUTLINE_RADIUS_PX,
            );
        }

        // Phase 4: UI + sprites + text (no depth test — draw-order layering).
        for (order, entity, kind) in &items {
            if matches!(kind, DrawKind::Tilemap | DrawKind::IsoSprite) {
                continue;
            }
            let Ok(tf) = self.world.get::<&Transform>(*entity) else {
                continue;
            };
            match kind {
                DrawKind::Sprite => {
                    let Ok(sprite) = self.world.get::<&SpriteRender>(*entity) else {
                        continue;
                    };
                    let ts = [sprite.tile_set_size.x, sprite.tile_set_size.y];
                    let frame_ref = sprite
                        .frame_name
                        .as_deref()
                        .and_then(|n| Self::resolve_frame(&self.frame_tables, &sprite.texture, n));
                    let (sprite_size, sheet_name, uv) = match &frame_ref {
                        Some(fr) => {
                            let sw = if fr.source_size[0] > 0 {
                                fr.source_size[0] as f32
                            } else {
                                fr.size[0]
                            };
                            let sh = if fr.source_size[1] > 0 {
                                fr.source_size[1] as f32
                            } else {
                                fr.size[1]
                            };
                            let (cw, ch) = (fr.size[0], fr.size[1]);
                            let (bx, by) = (fr.trim_offset[0] as f32, fr.trim_offset[1] as f32);
                            (
                                (sw, sh),
                                fr.sheet_name.clone(),
                                Some((fr.uv_rect, [bx, by], [sw, sh], [cw, ch])),
                            )
                        }
                        None => {
                            let tex_size = gfx
                                .textures
                                .get(&sprite.texture)
                                .map(|t| (t.size.0 as f32, t.size.1 as f32))
                                .unwrap_or((1.0, 1.0));
                            ((tex_size.0 / ts[0], tex_size.1 / ts[1]), sprite.texture.clone(), None)
                        }
                    };
                    let sprite_model = Mat4::from_translation(tf.position)
                        * Mat4::from_scale(Vec3::new(
                            tf.scale.x * sprite_size.0,
                            tf.scale.y * sprite_size.1,
                            1.0,
                        ));
                    if let Some(ref mut t) = self.trace {
                        let name = name_by_entity.get(entity).copied().unwrap_or("");
                        t.push(golden::TraceItemParams {
                            order: *order,
                            kind: "Sprite",
                            name,
                            model: &sprite_model,
                            camera_ignored: sprite.ignore_cam,
                            texture: Some(&sheet_name),
                            frame: Some(sprite.frame),
                            color: None,
                            depth: None,
                            depth_range: None,
                            normal: None,
                        });
                    }
                    let region = match &uv {
                        Some((uv_rect, trim_offset, source_size, content_size)) => {
                            SpriteRegion::Uv { uv_rect, trim_offset, source_size, content_size }
                        }
                        None => SpriteRegion::Grid { frame: sprite.frame, tile_set_size: ts },
                    };
                    gfx.draw_sprite(
                        &sprite_model,
                        &cam,
                        &sheet_name,
                        region,
                        sprite.ignore_cam,
                        1.0,
                        &sprite_settings,
                    );
                }
                DrawKind::UiRect => {
                    let Ok(rect) = self.world.get::<&RectRender>(*entity) else {
                        continue;
                    };
                    let (w, h) = self
                        .world
                        .get::<&classic_core::components::UiNode>(*entity)
                        .map(|n| (n.size.x, n.size.y))
                        .unwrap_or((tf.scale.x, tf.scale.y));
                    let model = Mat4::from_translation(tf.position)
                        * Mat4::from_scale(Vec3::new(w, h, 1.0));
                    if let Some(ref mut t) = self.trace {
                        let name = name_by_entity.get(entity).copied().unwrap_or("");
                        t.push(golden::TraceItemParams {
                            order: *order,
                            kind: "UiRect",
                            name,
                            model: &model,
                            camera_ignored: rect.ignore_cam,
                            texture: None,
                            frame: None,
                            color: Some(rect.color),
                            depth: None,
                            depth_range: None,
                            normal: None,
                        });
                    }
                    let cam_mat = if rect.ignore_cam { Mat4::IDENTITY } else { cam };
                    gfx.draw_rect(&model, &cam_mat, &rect.color, rect.ignore_cam);
                }
                DrawKind::UiSprite => {
                    let Ok(sprite) = self.world.get::<&SpriteRender>(*entity) else {
                        continue;
                    };
                    let (w, h) = self
                        .world
                        .get::<&classic_core::components::UiNode>(*entity)
                        .map(|n| (n.size.x, n.size.y))
                        .unwrap_or((tf.scale.x, tf.scale.y));
                    let frame_ref = sprite
                        .frame_name
                        .as_deref()
                        .and_then(|n| Self::resolve_frame(&self.frame_tables, &sprite.texture, n));
                    let mut uv: Option<IsoUv> = None;
                    let model = match &frame_ref {
                        Some(fr) => {
                            let sw = if fr.source_size[0] > 0 {
                                fr.source_size[0] as f32
                            } else {
                                fr.size[0]
                            };
                            let sh = if fr.source_size[1] > 0 {
                                fr.source_size[1] as f32
                            } else {
                                fr.size[1]
                            };
                            let (cw, ch) = (fr.size[0], fr.size[1]);
                            // Fit the trimmed content into the `(w, h)` box,
                            // preserving aspect and centered.  The trim offset
                            // is compensated so the content (not the source
                            // cell) lands in the middle of the box — icon
                            // frames are trimmed out of a larger source cell.
                            let scale =
                                if cw > 0.0 && ch > 0.0 { (w / cw).min(h / ch) } else { 1.0 };
                            let (bx, by) = (fr.trim_offset[0] as f32, fr.trim_offset[1] as f32);
                            let off_x = (w - cw * scale) / 2.0 - bx * scale;
                            let off_y = (h - ch * scale) / 2.0 - by * scale;
                            uv = Some((fr.uv_rect, [bx, by], [sw, sh], [cw, ch]));
                            Mat4::from_translation(Vec3::new(
                                tf.position.x + off_x,
                                tf.position.y + off_y,
                                tf.position.z,
                            )) * Mat4::from_scale(Vec3::new(sw * scale, sh * scale, 1.0))
                        }
                        None => {
                            Mat4::from_translation(tf.position)
                                * Mat4::from_scale(Vec3::new(w, h, 1.0))
                        }
                    };
                    let region = match &uv {
                        Some((uv_rect, trim_offset, source_size, content_size)) => {
                            SpriteRegion::Uv { uv_rect, trim_offset, source_size, content_size }
                        }
                        None => {
                            let ts = [sprite.tile_set_size.x, sprite.tile_set_size.y];
                            SpriteRegion::Grid { frame: sprite.frame, tile_set_size: ts }
                        }
                    };
                    if let Some(ref mut t) = self.trace {
                        let name = name_by_entity.get(entity).copied().unwrap_or("");
                        t.push(golden::TraceItemParams {
                            order: *order,
                            kind: "UiSprite",
                            name,
                            model: &model,
                            camera_ignored: true,
                            texture: Some(&sprite.texture),
                            frame: Some(sprite.frame),
                            color: None,
                            depth: None,
                            depth_range: None,
                            normal: None,
                        });
                    }
                    gfx.draw_sprite(
                        &model,
                        &Mat4::IDENTITY,
                        &sprite.texture,
                        region,
                        true,
                        1.0,
                        &sprite_settings,
                    );
                }
                DrawKind::SdfText => {
                    let Ok(sdf) = self.world.get::<&SdfTextRender>(*entity) else {
                        continue;
                    };
                    let atlas_name = format!("{}-sdf", sdf.atlas_name);
                    if !gfx.textures.contains_key(&atlas_name) {
                        continue;
                    }
                    let font = self.sdf_fonts.get(&sdf.atlas_name);
                    let Some(font) = font else { continue };

                    let scale = tf.scale.x;
                    let dirty = {
                        self.sdf_text_gpu
                            .get(entity)
                            .map(|st| {
                                st.last_text != sdf.text || (st.last_scale - scale).abs() > 0.001
                            })
                            .unwrap_or(true)
                    };
                    if dirty {
                        let buf = build_sdf_glyph_buffer(font, &sdf.text, scale, sdf.justify, 0.0);
                        let gb = GlBuffer::from_slice(
                            &gfx.gl,
                            glow::ARRAY_BUFFER,
                            &buf.vertices,
                            glow::DYNAMIC_DRAW,
                        );
                        self.sdf_text_gpu.insert(
                            *entity,
                            SdfTextGpu {
                                glyph_buf: gb,
                                vertex_count: buf.vertex_count,
                                text_width: buf.text_width,
                                text_height: buf.text_height,
                                last_text: sdf.text.clone(),
                                last_scale: scale,
                            },
                        );
                        if let Ok(mut node) =
                            self.world.get::<&mut classic_core::components::UiNode>(*entity)
                        {
                            node.size.x = buf.text_width;
                            node.size.y = buf.text_height;
                            if let Some(ref mut um) = self.ui {
                                um.mark_dirty();
                            }
                        }
                    }

                    let Some(st) = self.sdf_text_gpu.get(entity) else { continue };
                    if st.vertex_count == 0 {
                        continue;
                    }

                    let x_off = {
                        let is_ui = self
                            .world
                            .get::<&classic_core::components::UiNode>(*entity)
                            .map(|n| n.parent.is_some())
                            .unwrap_or(false);
                        if is_ui {
                            0.0
                        } else {
                            match sdf.justify {
                                classic_core::components::TextJustify::Left => 0.0,
                                classic_core::components::TextJustify::Center => {
                                    -st.text_width / 2.0
                                }
                                classic_core::components::TextJustify::Right => -st.text_width,
                            }
                        }
                    };
                    let model =
                        Mat4::from_translation(Vec3::new(
                            tf.position.x + x_off,
                            tf.position.y,
                            tf.position.z,
                        )) * Mat4::from_scale(Vec3::new(st.text_width, st.text_height, tf.scale.z));

                    let clip = self
                        .world
                        .get::<&classic_core::components::UiNode>(*entity)
                        .ok()
                        .map(|n| n.clip_rect)
                        .filter(|r| *r != Vec4::ZERO);
                    if let Some(r) = clip {
                        unsafe {
                            gfx.gl.enable(glow::SCISSOR_TEST);
                            gfx.gl.scissor(
                                r.x as i32,
                                (vh2 - r.y - r.w) as i32,
                                r.z as i32,
                                r.w as i32,
                            );
                        }
                    }

                    if let Some(ref mut t) = self.trace {
                        let name = name_by_entity.get(entity).copied().unwrap_or("");
                        t.push(golden::TraceItemParams {
                            order: *order,
                            kind: "SdfText",
                            name,
                            model: &model,
                            camera_ignored: sdf.ignore_cam,
                            texture: Some(&atlas_name),
                            frame: None,
                            color: Some(sdf.color),
                            depth: None,
                            depth_range: None,
                            normal: None,
                        });
                    }
                    let sdf_cam = if sdf.ignore_cam { Mat4::IDENTITY } else { cam };
                    gfx.draw_sdf(
                        &model,
                        &sdf_cam,
                        &atlas_name,
                        &sdf.color,
                        &sdf.outline_color,
                        sdf.outline_width,
                        font.spread,
                        &font.atlas_size,
                        sdf.weight,
                        sdf.gamma,
                        st.vertex_count as i32,
                        &st.glyph_buf,
                        sdf.ignore_cam,
                    );

                    if clip.is_some() {
                        unsafe {
                            gfx.gl.disable(glow::SCISSOR_TEST);
                        }
                    }
                }
                _ => {}
            }
        }

        // --- golden trace finalization ---
        if let Some(t) = self.trace.take() {
            let trace = t.finish();
            let json = golden::serialize_trace(&trace);
            let cwd = std::env::current_dir().unwrap_or_default();
            let baseline_dir = cwd.join(&config.golden_dir);
            let baseline_path = baseline_dir.join("baseline.trace.jsonl");
            match config.golden_mode.as_str() {
                "update" => {
                    let _ = std::fs::create_dir_all(&baseline_dir);
                    if let Err(e) = std::fs::write(&baseline_path, &json) {
                        classic_core::cl_warn!(
                            classic_core::instrument::Chan::Golden,
                            "golden: failed to write {}: {e}",
                            baseline_path.display()
                        );
                    } else {
                        classic_core::cl_info!(
                            classic_core::instrument::Chan::Golden,
                            "golden: wrote {} ({} items)",
                            baseline_path.display(),
                            trace.items.len()
                        );
                    }
                }
                "check" => {
                    match std::fs::read_to_string(&baseline_path) {
                        Ok(expected) => {
                            if let Err(diffs) = golden::compare_traces(&json, &expected) {
                                classic_core::cl_error!(
                                    classic_core::instrument::Chan::Golden,
                                    "golden: baseline mismatch"
                                );
                                for d in &diffs {
                                    classic_core::cl_warn!(
                                        classic_core::instrument::Chan::Golden,
                                        "  {d}"
                                    );
                                }
                                self.test_failed = true;
                                // Write actual trace to target/ so the CI artifact upload picks it up.
                                let artifact_dir = cwd.join("target/classic-test");
                                let _ = std::fs::create_dir_all(&artifact_dir);
                                let actual_path = artifact_dir.join("baseline.actual.trace.jsonl");
                                let _ = std::fs::write(&actual_path, &json);
                            } else {
                                classic_core::cl_info!(
                                    classic_core::instrument::Chan::Golden,
                                    "golden: baseline trace matches ({})",
                                    trace.items.len()
                                );
                            }
                        }
                        Err(_) => {
                            classic_core::cl_error!(
                                classic_core::instrument::Chan::Golden,
                                "golden: baseline not found at {}.  Run CLASSIC_GOLDEN=update to create it.",
                                baseline_path.display(),
                            );
                            self.test_failed = true;
                        }
                    }
                }
                _ => {}
            }

            // --- pixel golden ---
            if config.golden_png {
                if let Some(ref rt) = gfx.render_target {
                    // glFinish to ensure all draw commands have completed.
                    unsafe {
                        gfx.gl.finish();
                    }
                    let mut pixels = rt.read_pixels_rgba(&gfx.gl);
                    // Vertical flip: GL reads bottom-first, PNG expects top-first.
                    let row_bytes = (rt.width * 4) as usize;
                    let mut flipped = vec![0u8; pixels.len()];
                    for y in 0..rt.height {
                        let src_row = y as usize * row_bytes;
                        let dst_row = (rt.height - 1 - y) as usize * row_bytes;
                        flipped[dst_row..dst_row + row_bytes]
                            .copy_from_slice(&pixels[src_row..src_row + row_bytes]);
                    }
                    std::mem::swap(&mut pixels, &mut flipped);

                    let tol = config.golden_tol;
                    match config.golden_mode.as_str() {
                        "update" => {
                            let dir = config.golden_dir.as_str();
                            let _ = std::fs::create_dir_all(dir);
                            let path = format!("{dir}/baseline.png");
                            if let Err(e) = image::save_buffer(
                                &path,
                                &pixels,
                                rt.width,
                                rt.height,
                                image::ColorType::Rgba8,
                            ) {
                                classic_core::cl_warn!(
                                    classic_core::instrument::Chan::Golden,
                                    "golden: failed to write {path}: {e}"
                                );
                            } else {
                                classic_core::cl_info!(
                                    classic_core::instrument::Chan::Golden,
                                    "golden: wrote {path} ({}x{})",
                                    rt.width,
                                    rt.height
                                );
                            }
                        }
                        "check" => {
                            let path = format!("{}/baseline.png", config.golden_dir);
                            match image::open(&path) {
                                Ok(img) => {
                                    let expected = img.to_rgba8();
                                    let total = (rt.width * rt.height) as usize;
                                    let exp_raw = expected.as_raw();
                                    let mut diff_count = 0usize;
                                    for i in 0..total.min(exp_raw.len() / 4) {
                                        let ai = i * 4;
                                        let bi = i * 4;
                                        let dr = (pixels[ai] as i32 - exp_raw[bi] as i32)
                                            .unsigned_abs()
                                            as u8;
                                        let dg = (pixels[ai + 1] as i32 - exp_raw[bi + 1] as i32)
                                            .unsigned_abs()
                                            as u8;
                                        let db = (pixels[ai + 2] as i32 - exp_raw[bi + 2] as i32)
                                            .unsigned_abs()
                                            as u8;
                                        let da = (pixels[ai + 3] as i32 - exp_raw[bi + 3] as i32)
                                            .unsigned_abs()
                                            as u8;
                                        if dr > tol || dg > tol || db > tol || da > tol {
                                            diff_count += 1;
                                            if diff_count <= 10 {
                                                classic_core::cl_warn!(
                                                    classic_core::instrument::Chan::Golden,
                                                    "  pixel[{i}]=[{},{},{},{}] expected=[{},{},{},{}]",
                                                    pixels[ai],
                                                    pixels[ai + 1],
                                                    pixels[ai + 2],
                                                    pixels[ai + 3],
                                                    exp_raw[bi],
                                                    exp_raw[bi + 1],
                                                    exp_raw[bi + 2],
                                                    exp_raw[bi + 3],
                                                );
                                            }
                                        }
                                    }
                                    let pct = (diff_count as f64 / total as f64) * 100.0;
                                    if pct > 0.1 {
                                        classic_core::cl_error!(
                                            classic_core::instrument::Chan::Golden,
                                            "golden: pixel mismatch {}/{} ({:.2}%) > 0.1%",
                                            diff_count,
                                            total,
                                            pct,
                                        );
                                        self.test_failed = true;
                                    } else {
                                        classic_core::cl_info!(
                                            classic_core::instrument::Chan::Golden,
                                            "golden: pixels match ({} diffs out of {}, {:.2}%)",
                                            diff_count,
                                            total,
                                            pct,
                                        );
                                    }
                                }
                                Err(_) => {
                                    classic_core::cl_warn!(
                                        classic_core::instrument::Chan::Golden,
                                        "golden: no baseline PNG at {path}, skipping pixel check"
                                    );
                                }
                            }
                        }
                        _ => {}
                    }
                } else {
                    classic_core::cl_warn!(
                        classic_core::instrument::Chan::Golden,
                        "golden: CLASSIC_GOLDEN_PNG=1 but no offscreen render target"
                    );
                }
            }
        }

        // (SDF text is now rendered inline above, in z-order.)
        // Debug overlays (footprints, agent ring, compass) are drawn by the
        // demo via the overlay hook.  The `gfx` borrow ends here (NLL), so the
        // hooks can re-borrow `self`; each hook re-borrows `gfx` internally.
        let mut overlays = std::mem::take(&mut self.overlay_hooks);
        for o in overlays.iter_mut() {
            o(self);
        }
        self.overlay_hooks = overlays;
    }

    /// Pre-measure all UI-managed SDF text labels so their UiNode.size
    /// is correct from the first frame. Call once after all UI init is complete.
    /// Without this, text inside button containers appears at wrong positions for
    /// one frame because spawn_sdf_text creates entities with
    /// UiNode.size = (max_width, 0) — the anchor math in position_children_of
    /// uses these stale dimensions until the render pass updates them.
    pub fn measure_all_ui_labels(&mut self) {
        let mut to_measure: Vec<(hecs::Entity, SdfTextRender, f32)> = Vec::new();
        for (e, (tf, sdf)) in self.world.query::<(&Transform, &SdfTextRender)>().iter() {
            if self.world.get::<&UiNode>(e).map(|n| n.parent.is_some()).unwrap_or(false) {
                to_measure.push((e, sdf.clone(), tf.scale.x));
            }
        }

        let mut changed = false;
        for (e, sdf, scale) in &to_measure {
            let Some(font) = self.sdf_fonts.get(&sdf.atlas_name) else { continue };
            let buf = build_sdf_glyph_buffer(font, &sdf.text, *scale, sdf.justify, 0.0);
            if let Ok(mut node) = self.world.get::<&mut UiNode>(*e) {
                if (node.size.x - buf.text_width).abs() > 0.1
                    || (node.size.y - buf.text_height).abs() > 0.1
                {
                    node.size.x = buf.text_width;
                    node.size.y = buf.text_height;
                    changed = true;
                }
            }
        }

        if changed {
            if let Some(ref mut ui) = self.ui {
                ui.refresh_layout(&mut self.world);
                ui.sync_colliders(&self.world, &mut self.physics);
            }
        }
    }

    /// Build and upload GPU resources for the nav mesh overlay.
    pub fn init_nav_mesh_render(&mut self) {
        let Some(nav_entity) = self.entity_by_role(RoleKind::NavMesh) else {
            return;
        };
        let (size_x, size_y, nav_data) = {
            let nav = match self.world.get::<&NavMesh>(nav_entity) {
                Ok(n) => n,
                Err(_) => return,
            };
            (nav.size_x, nav.size_y, nav.data.clone())
        };

        // A generated map has no nav grid until its guest uploads one (deferred
        // to `commit_terrain` → `rebuild_nav_gpu`).  A grid of the wrong size is
        // equally not ready.  Skip building the overlay until then; the nav mesh
        // overlay is rebuilt once the guest commits.
        if nav_data.len() != (size_x * size_y) as usize {
            return;
        }

        // Use parent tilemap's actual height data so nav tiles sit on terrain surface.
        let (heights, height_scale) = self
            .entity_by_role(RoleKind::Tilemap)
            .and_then(|e| self.world.get::<&Tilemap>(e).ok())
            .map(|tm| (tm.height_data.clone(), tm.height_scale))
            .filter(|(h, _)| h.len() == (size_x as usize + 1) * (size_y as usize + 1))
            .unwrap_or_else(|| (vec![1.0f32; (size_x as usize + 1) * (size_y as usize + 1)], 64.0));

        let Some(gfx) = self.gfx.as_mut() else { return };

        let (mesh_data, vcount) = build_mesh(size_x, size_y, &nav_data, &heights, height_scale);
        let mesh_buf =
            GlBuffer::from_slice(&gfx.gl, glow::ARRAY_BUFFER, &mesh_data, glow::DYNAMIC_DRAW);

        let (tile_pixels, tw, th) = build_tile_texture(size_x, size_y, &nav_data);
        let tile_tex = Engine::upload_data_texture(&gfx.gl, &tile_pixels, tw, th);

        self.nav_gpu = Some(TilemapGpu { mesh_buf, vertex_count: vcount, tile_tex });

        // Add Transform to nav entity so render query (&Transform, &NavMesh) matches.
        // Borrow position + scale from parent tilemap.
        {
            let (pos, scl) = self
                .entity_by_role(RoleKind::Tilemap)
                .and_then(|e| self.world.get::<&Transform>(e).ok())
                .map(|tf| (tf.position, tf.scale))
                .unwrap_or((glam::Vec3::ZERO, glam::Vec3::ONE));
            let _ = self.world.insert_one(nav_entity, Transform::new(pos, scl));
        }
    }

    /// After height edits, recalculate nav mesh walkability and rebuild GPU resources.
    pub fn sync_nav_heights(&mut self) {
        let Some(nav_entity) = self.entity_by_role(RoleKind::NavMesh) else {
            return;
        };
        let Some(tm_entity) = self.entity_by_role(RoleKind::Tilemap) else {
            return;
        };
        let (sx, sy) = {
            let Ok(nav) = self.world.get::<&NavMesh>(nav_entity) else {
                return;
            };
            (nav.size_x, nav.size_y)
        };
        let hd = {
            let Ok(tm) = self.world.get::<&Tilemap>(tm_entity) else {
                return;
            };
            tm.height_data.clone()
        };
        let threshold = self.nav_slope_threshold;
        let stride = sx as usize + 1;
        let at = |tx: i32, ty: i32| -> f32 {
            hd.get(ty as usize * stride + tx as usize).copied().unwrap_or(0.0)
        };

        let mut changed = false;
        if let Ok(mut nav) = self.world.get::<&mut NavMesh>(nav_entity) {
            for ty in 0..sy {
                for tx in 0..sx {
                    let idx = (ty * sx + tx) as usize;
                    let h = at(tx, ty);
                    let mut walkable: u32 = 1;
                    // The four orthogonal neighbours.  The `ty + 1` bound
                    // previously compared against `sx`, so on any non-square
                    // map the southern edge was tested against the wrong
                    // dimension — invisible while the demo map was both
                    // square and perfectly flat.
                    let neighbours = [
                        (tx > 0).then(|| at(tx - 1, ty)),
                        (tx + 1 < sx).then(|| at(tx + 1, ty)),
                        (ty > 0).then(|| at(tx, ty - 1)),
                        (ty + 1 < sy).then(|| at(tx, ty + 1)),
                    ];
                    for n in neighbours.into_iter().flatten() {
                        if (h - n).abs() > threshold {
                            walkable = 0;
                        }
                    }
                    if nav.data.len() > idx {
                        if nav.data[idx] != walkable {
                            changed = true;
                        }
                        nav.data[idx] = walkable;
                    }
                }
            }
        }
        if changed {
            self.rebuild_nav_gpu();
        }
        self.refresh_nav_snapshot();
    }

    /// Upload RGBA `pixels` as a `NEAREST`-filtered `CLAMP_TO_EDGE` 2D texture.
    /// Used for tilemap data and nav-mesh data textures.
    fn upload_data_texture(
        gl: &std::rc::Rc<glow::Context>,
        pixels: &[u8],
        width: u32,
        height: u32,
    ) -> glow::Texture {
        let tex = unsafe { gl.create_texture() }.expect("create data texture");
        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA as i32,
                width as i32,
                height as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(pixels)),
            );
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::NEAREST as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::NEAREST as i32);
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );
        }
        tex
    }

    /// Rebuild nav mesh GPU buffers from current NavMesh component data.
    pub fn rebuild_nav_gpu(&mut self) {
        let Some(nav_entity) = self.entity_by_role(RoleKind::NavMesh) else {
            return;
        };
        let (sx, sy, data) = {
            let nav = match self.world.get::<&NavMesh>(nav_entity) {
                Ok(n) => n,
                Err(_) => return,
            };
            (nav.size_x, nav.size_y, nav.data.clone())
        };
        // Take the terrain's own heights, exactly as `init_nav_mesh_render`
        // does.  Rebuilding the overlay on a flat grid instead left it
        // detached from the surface after any nav edit — unnoticeable on the
        // flat demo map, glaring over a crater field.
        let (hs, heights) = self
            .entity_by_role(RoleKind::Tilemap)
            .and_then(|e| self.world.get::<&Tilemap>(e).ok())
            .map(|tm| (tm.height_scale, tm.height_data.clone()))
            .filter(|(_, h)| h.len() == (sx as usize + 1) * (sy as usize + 1))
            .unwrap_or_else(|| (64.0, vec![1.0f32; (sx as usize + 1) * (sy as usize + 1)]));
        let Some(gfx) = self.gfx.as_mut() else { return };

        let (mesh_data, vcount) = build_mesh(sx, sy, &data, &heights, hs);
        let mesh_buf =
            GlBuffer::from_slice(&gfx.gl, glow::ARRAY_BUFFER, &mesh_data, glow::DYNAMIC_DRAW);
        let (tile_pixels, tw, th) = build_tile_texture(sx, sy, &data);
        let tile_tex = Engine::upload_data_texture(&gfx.gl, &tile_pixels, tw, th);
        self.nav_gpu = Some(TilemapGpu { mesh_buf, vertex_count: vcount, tile_tex });
    }

    /// Rebuild the tilemap mesh from current data + heights and re-upload to GPU.
    pub fn rebuild_tilemap_mesh(&mut self) {
        let Some(tm_entity) = self.entity_by_role(RoleKind::Tilemap) else {
            classic_core::cl_warn!(
                classic_core::instrument::Chan::Editor,
                "rebuild_tilemap_mesh: no Tilemap-role entity"
            );
            return;
        };
        let (size_x, size_y, tiles, heights, height_scale) = {
            let tm = match self.world.get::<&Tilemap>(tm_entity) {
                Ok(t) => t,
                Err(_) => {
                    classic_core::cl_warn!(
                        classic_core::instrument::Chan::Editor,
                        "rebuild_tilemap_mesh: no Tilemap on the Tilemap-role entity"
                    );
                    return;
                }
            };
            (tm.size_x, tm.size_y, tm.data.clone(), tm.height_data.clone(), tm.height_scale)
        };

        let gfx = match self.gfx.as_mut() {
            Some(g) => g,
            None => {
                classic_core::cl_warn!(
                    classic_core::instrument::Chan::Editor,
                    "rebuild_tilemap_mesh: gfx not initialized"
                );
                return;
            }
        };

        let (mesh_data, vcount) = build_mesh(size_x, size_y, &tiles, &heights, height_scale);
        let mesh_buf =
            GlBuffer::from_slice(&gfx.gl, glow::ARRAY_BUFFER, &mesh_data, glow::DYNAMIC_DRAW);

        let (tile_pixels, tw, th) = build_tile_texture(size_x, size_y, &tiles);
        let tile_tex = Engine::upload_data_texture(&gfx.gl, &tile_pixels, tw, th);

        let entity_name = self.debug_name(tm_entity);
        if let Some(gpu) = self.tilemap_gpu.get_mut(&entity_name) {
            gpu.mesh_buf = mesh_buf;
            gpu.vertex_count = vcount;
            gpu.tile_tex = tile_tex;
            classic_core::cl_info!(
                classic_core::instrument::Chan::Editor,
                "rebuild_tilemap_mesh: {vcount} vertices uploaded for '{entity_name}'"
            );
        }
    }

    /// Helper: add or remove Disabled marker component to toggle entity visibility.
    /// Recursively sets children and syncs collider enabled state.
    pub fn set_enabled(&mut self, entity: hecs::Entity, enabled: bool) {
        // Collect collider PIDs before ECS mutations (avoids borrow conflict)
        let pids: Vec<u32> = if let Some(ref ui) = self.ui {
            ui.collect_collider_pids(&self.world, entity)
        } else {
            Vec::new()
        };

        let has_disabled = self.world.get::<&classic_core::components::Disabled>(entity).is_ok();
        if enabled && has_disabled {
            let _ = self.world.remove_one::<classic_core::components::Disabled>(entity);
        } else if !enabled && !has_disabled {
            let _ = self.world.insert_one(entity, classic_core::components::Disabled);
        }
        let children: Vec<hecs::Entity> = self
            .world
            .get::<&classic_core::components::UiNode>(entity)
            .map(|n| n.children.iter().map(|c| c.entity).collect())
            .unwrap_or_default();
        for child in children {
            self.set_enabled(child, enabled);
        }

        // Sync collider enabled state with physics
        for pid in &pids {
            self.physics.set_collider_enabled(*pid, enabled);
        }
    }

    /// Toggle a named entity's visibility (add/remove the `Disabled` marker).
    pub fn set_enabled_named(&mut self, name: &str, enabled: bool) -> bool {
        let Some(&entity) = self.names.get(name) else { return false };
        self.set_enabled(entity, enabled);
        true
    }

    /// Check whether an entity (or any of its ancestors) is disabled.
    pub fn is_disabled(&self, entity: hecs::Entity) -> bool {
        if self.world.get::<&classic_core::components::Disabled>(entity).is_ok() {
            return true;
        }
        let mut parent =
            self.world.get::<&classic_core::components::UiNode>(entity).ok().and_then(|n| n.parent);
        while let Some(p) = parent {
            if self.world.get::<&classic_core::components::Disabled>(p).is_ok() {
                return true;
            }
            parent =
                self.world.get::<&classic_core::components::UiNode>(p).ok().and_then(|n| n.parent);
        }
        false
    }

    /// Resolve a `frame_name` for a texture through its packed-atlas frame
    /// table, returning the bound sheet texture name, the normalized UV rect,
    /// the frame's content pixel size, its trim/anchor metadata, and any
    /// per-sheet normal/depth companion GL texture names.  Returns `None` if
    /// the texture has no frame table or the name is unknown.
    pub(crate) fn resolve_frame(
        tables: &HashMap<String, FrameTable>,
        texture: &str,
        frame_name: &str,
    ) -> Option<ResolvedFrame> {
        let table = tables.get(texture)?;
        let frame = table.frames.get(frame_name)?;
        let sheet = table.sheets.get(frame.sheet as usize)?;
        let uv = table.uv_rect(frame)?;
        let (normal_tex, depth_name) =
            table.companions.get(frame.sheet as usize).cloned().unwrap_or((None, None));
        let depth_tex = depth_name.map(|d| (d, sheet.depth_range));
        Some(ResolvedFrame {
            sheet_name: sheet.name.clone(),
            uv_rect: uv,
            size: [frame.rect[2] as f32, frame.rect[3] as f32],
            source_size: frame.source_size,
            trim_offset: frame.trim_offset,
            anchor: frame.anchor,
            normal_tex,
            depth_tex,
        })
    }

    /// Compute the anchor to use when drawing a (possibly trimmed) packed
    /// frame.  A packer-provided anchor wins; otherwise the component's anchor
    /// (a `[0..1]` ratio of the original source cell) is translated into
    /// trimmed-frame space using `source_size` + `trim_offset`, so the
    /// ground-contact point stays put when empty space is trimmed away.
    fn effective_anchor(component_anchor: Vec2, frame: &ResolvedFrame) -> Vec2 {
        if let Some(a) = frame.anchor {
            return Vec2::new(a[0], a[1]);
        }
        if frame.source_size[0] == 0 || frame.source_size[1] == 0 {
            return component_anchor;
        }
        let cw = frame.source_size[0] as f32;
        let ch = frame.source_size[1] as f32;
        let bx0 = frame.trim_offset[0] as f32;
        let by0 = frame.trim_offset[1] as f32;
        let fw = frame.size[0].max(1.0);
        let fh = frame.size[1].max(1.0);
        Vec2::new((component_anchor.x * cw - bx0) / fw, (component_anchor.y * ch - by0) / fh)
    }

    /// Compute the model matrix for an IsoSprite.
    /// `tex_dim` is the quad size in pixels; `anchor_px` is the ground-contact
    /// point in that same pixel space (the quad is shifted so it lands on the
    /// sprite's position).
    fn compute_iso_sprite_model(
        iso_sprite: &IsoSprite,
        sprite_tf: &Transform,
        tilemap_tf: &Transform,
        tilemap: &Tilemap,
        tex_dim: (f32, f32),
        anchor_px: Vec2,
    ) -> Mat4 {
        let iso_to_cart_world = iso_to_cartesian_4() * Mat4::from_scale(tilemap_tf.scale);

        let mut cart_pos = iso_to_cart_world.transform_point3(sprite_tf.position);
        cart_pos += iso_sprite.frame_offset;
        cart_pos += tilemap_tf.position;

        let h = sample_height_mesh(
            &tilemap.height_data,
            tilemap.size_x,
            tilemap.size_y,
            sprite_tf.position.x,
            sprite_tf.position.y,
        );
        cart_pos.y -= h * tilemap.height_scale;

        let anchor_delta = Vec3::new(-anchor_px.x, -anchor_px.y, 0.0);

        Mat4::from_translation(cart_pos)
            * Mat4::from_scale(sprite_tf.scale)
            * Mat4::from_translation(anchor_delta)
            * Mat4::from_scale(Vec3::new(tex_dim.0, tex_dim.1, 1.0))
    }

    /// Compute the anchor-plane iso depth for a sprite position (the depth a
    /// depth map's 0.5 grayscale corresponds to), in **window space** `[0, 1]`.
    /// Matches the `base_depth` term in [`Self::compute_iso_depth_corners`].
    /// `h_depth` is the tilemap's horizontal depth scale (see
    /// [`classic_core::tilemap::horizontal_depth_scale`]).
    fn compute_iso_base_depth(pos: Vec3, h_depth: f32) -> f32 {
        (pos.x - pos.y) / h_depth + 0.5 + (pos.z / PPM_TARGET) / HEIGHT_DEPTH_SCALE_M
    }

    /// Compute iso depth corners for the footprint, in **window space** `[0, 1]`.  `h_depth` is the tilemap's horizontal depth
    /// scale, kept identical to the terrain's `depth_scale.x` uniform so sprite
    /// occlusion matches the tilemap.
    fn compute_iso_depth_corners(pos: Vec3, footprint: &[glam::Vec2], h_depth: f32) -> [f32; 4] {
        let base_depth = Self::compute_iso_base_depth(pos, h_depth);
        let default_footprint = [
            glam::Vec2::new(0.5, -0.5),
            glam::Vec2::new(0.5, 0.5),
            glam::Vec2::new(-0.5, 0.5),
            glam::Vec2::new(-0.5, -0.5),
        ];
        let footprint = if footprint.len() >= 4 { &footprint[..4] } else { &default_footprint };

        let mut raw_depths = [0.0f32; 4];
        for i in 0..4 {
            let pt = &footprint[i];
            let d = (pos.x + pt.x - pos.y - pt.y) / h_depth
                + 0.5
                + (pos.z / PPM_TARGET) / HEIGHT_DEPTH_SCALE_M;
            raw_depths[i] = d.min(base_depth);
        }

        let min_fp = raw_depths.iter().cloned().fold(f32::MAX, f32::min);

        // shader layout: x=SW, y=SE, z=NW, w=NE
        // footprint order: [NE, SE, SW, NW]
        [min_fp, min_fp, raw_depths[3], raw_depths[0]]
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

/// Decode a little-endian `u32` grid byte blob.
fn decode_u32(bytes: &[u8]) -> Vec<u32> {
    bytes.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect()
}

/// Decode a little-endian `f32` grid byte blob.
fn decode_f32(bytes: &[u8]) -> Vec<f32> {
    bytes.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect()
}

/// Encode a `u32` grid to little-endian bytes.
fn encode_u32(vals: &[u32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(vals.len() * 4);
    for v in vals {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Encode an `f32` grid to little-endian bytes.
fn encode_f32(vals: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(vals.len() * 4);
    for v in vals {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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

        // Frame 0: 50 m altitude → 400 units up (negative screen Y).
        assert!((offsets[0][1] - (-400.0)).abs() < 0.001, "got {:?}", offsets[0]);
        // Drift x maps directly (scaled by ppm).
        assert!((offsets[1][0] - (-8.0)).abs() < 0.001, "got {:?}", offsets[1]);
        // Altitude 0 → zero vertical offset.
        assert!((offsets[2][1]).abs() < 0.001, "got {:?}", offsets[2]);
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
        // Frame 0: 50 m altitude at ppm=64 → 3200 units up (negative screen Y).
        assert_eq!(anim.offset_keyframes[0].frame, 0);
        assert!(
            (anim.offset_keyframes[0].offset[1] - (-3200.0)).abs() < 0.01,
            "got {:?}",
            anim.offset_keyframes[0]
        );
        // Frame 240: touchdown → zero vertical offset.
        assert_eq!(anim.offset_keyframes[1].frame, 240);
        assert!(anim.offset_keyframes[1].offset[1].abs() < 0.01);
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
    fn compute_iso_base_depth_matches_anchor_plane_formula() {
        let pos = glam::Vec3::new(100.0, 20.0, 64.0);
        let expected = (100.0 - 20.0) / HORIZONTAL_DEPTH_SCALE
            + 0.5
            + (64.0 / PPM_TARGET) / HEIGHT_DEPTH_SCALE_M;
        assert!(
            (Engine::compute_iso_base_depth(pos, HORIZONTAL_DEPTH_SCALE) - expected).abs() < 1e-9
        );
    }

    #[test]
    fn iso_to_world_lifts_elevation_above_terrain() {
        classic_core::register_all_components();
        let mut engine = Engine::new_for_test();

        // A flat 4x4 tilemap at the origin with a uniform 1-metre plateau,
        // scale [45,45,1] and a 64 px/metre height scale (the lunar PPM).
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

        // Ground level: the light sits at the 1 m surface → z = 1 * 64.
        let ground = engine.iso_to_world(0.0, 0.0, 0.0).unwrap();
        assert!((ground.z - 64.0).abs() < 1e-3, "got z {}", ground.z);

        // 2 m above: z = 3 * 64, and y is lifted (moved toward -y) by 2 * 64.
        let raised = engine.iso_to_world(0.0, 0.0, 2.0).unwrap();
        assert!((raised.z - 192.0).abs() < 1e-3, "got z {}", raised.z);
        assert!((ground.y - raised.y - 128.0).abs() < 1e-2, "got y {}", raised.y);
        assert!((ground.x - raised.x).abs() < 1e-4, "x should not move");

        // Without a Tilemap-role entity, the mapping is unavailable.
        let empty = Engine::new_for_test();
        assert!(empty.iso_to_world(0.0, 0.0, 0.0).is_none());
    }
}
