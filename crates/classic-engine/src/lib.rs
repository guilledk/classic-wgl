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
pub mod ui;

use std::collections::HashMap;
use std::rc::Rc;

use classic_core::collision::PhysicsProvider;
use classic_core::components::{
    AgentState, DebugName, IsoAgent, IsoSprite, NavMesh, RectRender, Role, SdfTextRender, Tilemap,
    UiNode,
};
use classic_core::instrument::Chan;
use classic_core::math::{cartesian_to_iso_4, iso_to_cartesian_4};
use classic_core::pathfinder;
use classic_core::sdf_builder::build_sdf_glyph_buffer;
use classic_core::terrain::lunar::LunarTerrain;
use classic_core::tilemap::{bilinear_height, build_mesh, build_tile_texture};
use classic_core::types::AnimationData;
use classic_core::types::SdfFontMetrics;
use classic_core::{Camera, RoleKind, SpriteRender, Transform};
use classic_gfx::{Gfx, GlBuffer};
use classic_platform::InputState;
use glam::{Mat3, Mat4, Vec3, Vec4};
use glow::HasContext;

type UpdateFn = Box<dyn FnMut(&mut Engine)>;

/// Per-entity GPU resources for a tilemap.
struct TilemapGpu {
    mesh_buf: GlBuffer,
    vertex_count: usize,
    tile_tex: glow::Texture,
}

struct SdfTextGpu {
    glyph_buf: GlBuffer,
    vertex_count: usize,
    text_width: f32,
    text_height: f32,
    last_text: String,
    last_scale: f32,
}

pub struct Engine {
    pub gfx: Option<Gfx>,
    pub world: hecs::World,
    pub camera: Camera,
    pub time: Time,
    pub names: HashMap<String, hecs::Entity>,
    pub name_order: Vec<String>,
    pub physics: PhysicsProvider,
    pub ui_consumed_click: bool,
    pub scroll_speed: f32,
    pub input: InputState,
    pub show_grid: bool,
    /// Selected-agent flag — the generic engine signal the nav click-to-move
    /// handler reads.  The demo editor toggles it.
    pub agent_selected: bool,
    pub light_ambient: [f32; 3],
    pub light_dir: [f32; 3],
    pub light_color: [f32; 3],
    pub animations: HashMap<String, AnimationData>,
    pub sdf_fonts: HashMap<String, SdfFontMetrics>,
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
    /// that overrides the scale (see `init_tilemap_generated`).
    pub base_height_scale: f32,
    /// Height difference between adjacent tiles above which `sync_nav_heights`
    /// marks a tile impassable.  The flat demo map edits heights in integer
    /// steps, hence the default of 2.0; generated terrain is continuous and
    /// needs a much finer threshold to match the slope rule it was built with.
    pub nav_slope_threshold: f32,
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
            physics: PhysicsProvider::new(),
            ui_consumed_click: false,
            scroll_speed: 600.0,
            input: InputState::new(),
            show_grid: false,
            agent_selected: false,
            light_ambient: [0.15, 0.15, 0.2],
            light_dir: [0.45, -0.35, 0.82],
            light_color: [1.0, 0.95, 0.85],
            animations: HashMap::new(),
            sdf_fonts: HashMap::new(),
            rom_manifest_json: None,
            rom_manifest: None,
            rom_resources: None,
            ui: None,
            selection_mode: -1,
            selection_begin_screen: glam::Vec3::new(-1.0, -1.0, -1.0),
            base_height_scale: 32.0,
            nav_slope_threshold: 2.0,
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
        for info in &manifest.manifest.shaders {
            let vs = registry.resolve_vertex(&info.vertex);
            let fs = registry.resolve_fragment(&info.fragment);
            self.gfx
                .as_mut()
                .unwrap()
                .add_shader(&info.name, &vs, &fs, &info.attr, &info.unif)
                .expect("compile shader");
        }

        // Textures from the manifest (via the resource set), skipping the SDF
        // atlas textures (those are uploaded by the SDF font path with LINEAR
        // filtering).
        let atlas_names: std::collections::HashSet<String> =
            resources.fonts().keys().map(|f| format!("{f}-sdf")).collect();
        for (name, bytes) in resources.textures() {
            if atlas_names.contains(name) {
                continue;
            }
            self.load_texture_png(name, bytes);
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
    }

    /// Hydrate the engine from a ROM: compile shaders, upload resources, and
    /// spawn the entity graph.  Records the ROM's manifest + resources so
    /// [`Engine::dump_rom`] can reconstruct it.
    pub fn load_rom(&mut self, gl: Rc<glow::Context>, rom: &classic_rom::Rom) {
        self.init_gfx(gl, &rom.manifest, &rom.resources);
        self.load_state(&rom.state).expect("load ROM state");
        self.rom_manifest_json = Some(rom.manifest_json.clone());
        self.rom_manifest = Some(rom.manifest.clone());
        self.rom_resources = Some(rom.resources.clone());
    }

    /// Reconstruct a [`classic_rom::Rom`] from the loaded manifest + resources
    /// and the current world state.  Returns `None` if no ROM was loaded.
    pub fn dump_rom(&self) -> Option<classic_rom::Rom> {
        Some(classic_rom::Rom {
            manifest: self.rom_manifest.clone()?,
            manifest_json: self.rom_manifest_json.clone()?,
            resources: self.rom_resources.clone()?,
            state: self.dump_state(),
        })
    }

    /// Upload a PNG texture from raw bytes.
    pub fn load_texture_png(&mut self, name: &str, png_bytes: &[u8]) {
        let img = image::load_from_memory(png_bytes).expect("decode PNG");
        let rgba = img.to_rgba8();
        if let Some(gfx) = self.gfx.as_mut() {
            gfx.add_texture_rgba8(name, &rgba, rgba.width(), rgba.height());
        }
    }

    /// Load an SDF font from its metrics JSON and atlas PNG.
    /// The atlas texture is set to LINEAR filtering.
    pub fn load_sdf_font(&mut self, atlas_name: &str, metrics_json: &str, atlas_png: &[u8]) {
        let metrics: SdfFontMetrics =
            serde_json::from_str(metrics_json).expect("parse SDF font metrics JSON");
        self.sdf_fonts.insert(metrics.name.clone(), metrics);

        let img = image::load_from_memory(atlas_png).expect("decode SDF atlas PNG");
        let rgba = img.to_rgba8();
        if let Some(gfx) = self.gfx.as_mut() {
            gfx.add_texture_rgba8(atlas_name, &rgba, rgba.width(), rgba.height());
            if let Some(tex) = gfx.textures.get(atlas_name) {
                tex.set_linear(&gfx.gl);
            }
        }
    }

    /// Decode base64-encoded JSON array into tile data.
    /// Build and upload the tilemap mesh + tile data texture for a named entity.
    /// The tile data comes from the entity's `Tilemap.data` (loaded inline from
    /// `state.json`) and the tileset texture is loaded from the manifest by
    /// [`Engine::init_gfx`].  Terrain is flat (height 1.0 everywhere), matching
    /// the TS `heightData.fill(1)`.  For procedurally generated terrain see
    /// [`Engine::init_tilemap_generated`].
    pub fn init_tilemap(&mut self) {
        let entity = self.entity_by_role(RoleKind::Tilemap).expect("Tilemap-role entity");

        let (size_x, size_y, tiles) = {
            let tm = self.world.get::<&Tilemap>(entity).expect("Tilemap component");
            (tm.size_x, tm.size_y, tm.data.clone())
        };

        // height_data stride is (size_x + 1) — vertex grid, not tile grid.
        // The TS used sizeX * sizeY (tile-based). Rust uses (sizeX+1)*(sizeY+1)
        // to avoid off-by-one edge cases. See docs/TS-PARITY.md.
        let heights = vec![1.0f32; (size_x as usize + 1) * (size_y as usize + 1)];

        self.finish_tilemap_init(entity, tiles, heights, None);
    }

    /// Build and upload a tilemap from procedurally generated terrain, with
    /// an in-memory RGBA tileset instead of a PNG.
    ///
    /// `height_scale` overrides the default (`tile_pixel_size[0]`).  Generated
    /// terrain has a far larger height range than the flat demo map, so the
    /// default scale would exaggerate the relief and stretch the mouse-picking
    /// parallax solve.
    pub fn init_tilemap_generated(
        &mut self,
        terrain: &LunarTerrain,
        tileset_rgba: &[u8],
        tileset_w: u32,
        tileset_h: u32,
        height_scale: Option<f32>,
    ) {
        let entity = self.entity_by_role(RoleKind::Tilemap).expect("Tilemap-role entity");

        let (tile_set_name, size_x, size_y) = {
            let tm = self.world.get::<&Tilemap>(entity).expect("Tilemap component");
            (tm.tile_set.clone(), tm.size_x, tm.size_y)
        };
        assert_eq!(
            (terrain.size_x, terrain.size_y),
            (size_x, size_y),
            "generated terrain size does not match the Tilemap component"
        );

        if let Some(gfx) = self.gfx.as_mut() {
            gfx.add_texture_rgba8(&tile_set_name, tileset_rgba, tileset_w, tileset_h);
        }

        self.finish_tilemap_init(
            entity,
            terrain.tiles.clone(),
            terrain.heights.clone(),
            height_scale,
        );
    }

    /// Shared tail of the `init_tilemap*` family: build the mesh and tile-data
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

    // "type" must be the first key in each dumped component object.
    // The TS positional loader relies on this: it splices out "type"
    // and passes remaining values as positional constructor args.
    // See docs/TS-PARITY.md for the per-component key ordering.
    /// Serialise all named entities to a state JSON string (TS-compatible format).
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

    /// Dump a named entity's components to a JSON string (`{"components": [...]}`).
    pub fn dump_entity_json(&self, name: &str) -> Option<String> {
        let entity = *self.names.get(name)?;
        let components = self.dump_entity_components(entity);
        serde_json::to_string_pretty(&serde_json::json!({ "components": components })).ok()
    }

    /// Dump a single component of a named entity via the registry dumper.
    pub fn dump_component_json(&self, name: &str, comp_type: &str) -> Option<String> {
        let entity = *self.names.get(name)?;
        let dumper = classic_core::registry::ordered_regs()
            .into_iter()
            .find(|r| r.name == comp_type)
            .and_then(|r| r.dump)?;
        let val = dumper(&self.world, entity)?;
        serde_json::to_string_pretty(&val).ok()
    }

    /// Set a single component of a named entity from its serialized JSON,
    /// reusing the registry spawner (deserialize → merge onto the entity).
    pub fn set_component_json(
        &mut self,
        name: &str,
        comp_type: &str,
        json: serde_json::Value,
    ) -> anyhow::Result<()> {
        let spawner = classic_core::registry::lookup(comp_type)
            .ok_or_else(|| anyhow::anyhow!("unknown component type: {comp_type}"))?;
        let entity =
            *self.names.get(name).ok_or_else(|| anyhow::anyhow!("no entity named {name}"))?;
        let mut builder = hecs::EntityBuilder::new();
        spawner(&mut builder, json)?;
        self.world.insert(entity, builder.build())?;
        Ok(())
    }

    /// Read a named entity's 2D position (from its `Transform`).
    pub fn get_pos(&self, name: &str) -> Option<(f32, f32)> {
        let entity = *self.names.get(name)?;
        self.world.get::<&Transform>(entity).ok().map(|tf| (tf.position.x, tf.position.y))
    }

    /// Write a named entity's 2D position (into its `Transform`, creating a
    /// default one if the entity has none yet).
    pub fn set_pos(&mut self, name: &str, x: f32, y: f32) -> bool {
        let Some(&entity) = self.names.get(name) else { return false };
        if self.world.get::<&Transform>(entity).is_err() {
            let _ = self.world.insert_one(
                entity,
                Transform::new(glam::Vec3::new(x, y, 0.0), glam::Vec3::new(1.0, 1.0, 1.0)),
            );
            return true;
        }
        if let Ok(mut tf) = self.world.get::<&mut Transform>(entity) {
            tf.position.x = x;
            tf.position.y = y;
            true
        } else {
            false
        }
    }

    /// Save a file, handling both native (filesystem) and web (Blob download).
    pub fn save_file(&self, name: &str, data: &str) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let dir = &crate::env_config::EnvConfig::get().dump_dir;
            let _ = std::fs::create_dir_all(dir);
            let path = format!("{dir}/{name}");
            if let Err(e) = std::fs::write(&path, data) {
                classic_core::cl_warn!(
                    classic_core::instrument::Chan::Dump,
                    "save_file: failed to write {path}: {e}"
                );
            } else {
                classic_core::cl_warn!(
                    classic_core::instrument::Chan::Dump,
                    "save_file: wrote {path} ({} bytes)",
                    data.len()
                );
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            use wasm_bindgen::JsCast;
            if let Some(window) = web_sys::window() {
                let doc = window.document().unwrap();
                let blob_parts = js_sys::Array::new();
                blob_parts.push(&js_sys::Uint8Array::from(data.as_bytes()).into());
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
            self.ui_consumed_click = true;
        }

        // Per-frame hover highlighting for UI elements.
        if let Some(ref mut ui) = self.ui {
            ui.update_hover(&mut self.world, &self.physics);
        }

        if self.input.was_mouse_pressed(0) && !self.ui_consumed_click {
            self.selection_mode = 1;
            self.selection_begin_screen = Vec3::new(mp.x, mp.y, 0.0);
            if let Some(e) = self.entity_by_role(RoleKind::Tilemap) {
                if let Ok(mut tm) = self.world.get::<&mut Tilemap>(e) {
                    tm.selection_iso_begin = tm.mouse_iso_pos;
                }
            }
            self.physics.begin_selection(Vec3::new(mp.x, mp.y, 0.0));
        }

        // Stretch selection rect every frame while dragging.
        if self.selection_mode == 1 {
            self.physics.update_selection(self.selection_begin_screen, Vec3::new(mp.x, mp.y, 0.0));
        }

        // Frame ordering: demo pre-update hooks run BEFORE on_update closures.
        // The camera's on_update runs first in registration order and would
        // consume the wheel; the demo's text-scroll hook zeroes it first.
        let mut pre = std::mem::take(&mut self.pre_update_hooks);
        for f in pre.iter_mut() {
            f(self);
        }
        self.pre_update_hooks = pre;

        // Take-restore dance: closures fire with &mut Engine, but the Vec
        // is owned by Engine. Taking means closures can call on_update()
        // without borrow conflicts. Restoring preserves them for next frame.
        // Handlers use iter_mut(), NOT std::mem::take — they must survive
        // across frames (click, enter, exit, selection).
        let mut fns = std::mem::take(&mut self.update_fns);
        for f in fns.iter_mut() {
            f(self);
        }
        self.update_fns = fns;

        // ---- CLASSIC_TEST automated test runner (registered by the demo) ----
        if env_config::EnvConfig::get().test_active() {
            let mut runner = self.test_runner.take();
            if let Some(r) = runner.as_mut() {
                r(self);
            }
            self.test_runner = runner;
        }

        // Wheel decay matches TS: 1.4 * delta, then [-1, 1] clamp.
        // Without write-back, decay resets to zero every frame.
        // Wheel decay + clamp (matches TS: 1.4 * deltaTime, then [-1, 1])
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
        self.ui_consumed_click = false;

        if self.input.was_mouse_released(0) && !self.ui_consumed_click {
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

            // Editor paint on selection-end (registered by the demo).
            if just_finished_selection {
                let mut hooks = std::mem::take(&mut self.selection_end_hooks);
                for h in hooks.iter_mut() {
                    h(self);
                }
                self.selection_end_hooks = hooks;
            }
        }

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

        let Some(gfx) = self.gfx.as_mut() else { return };
        let vp = gfx.viewport_w;
        let vh2 = gfx.viewport_h;
        self.camera.size = Vec3::new(vp, vh2, 0.0);

        // begin_frame sets depthFunc/depthMask but does NOT glEnable(DEPTH_TEST).
        // draw_tilemap/draw_iso_sprite toggle it locally. UI/SDF runs without it.
        // Enabling it globally depth-rejects all UI under ortho projection.
        gfx.begin_frame();
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
        for (order, entity, kind) in &items {
            let Ok(tf) = self.world.get::<&Transform>(*entity) else {
                continue;
            };
            match kind {
                DrawKind::Sprite => {
                    let Ok(sprite) = self.world.get::<&SpriteRender>(*entity) else {
                        continue;
                    };
                    let ts = [sprite.tile_set_size.x, sprite.tile_set_size.y];
                    let tex_size = gfx
                        .textures
                        .get(&sprite.texture)
                        .map(|t| (t.size.0 as f32, t.size.1 as f32))
                        .unwrap_or((1.0, 1.0));
                    let sprite_model = Mat4::from_translation(tf.position)
                        * Mat4::from_scale(Vec3::new(
                            tf.scale.x * tex_size.0 / ts[0],
                            tf.scale.y * tex_size.1 / ts[1],
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
                            texture: Some(&sprite.texture),
                            frame: Some(sprite.frame),
                            color: None,
                        });
                    }
                    gfx.draw_sprite(
                        &sprite_model,
                        &cam,
                        &sprite.texture,
                        sprite.frame,
                        &ts,
                        sprite.ignore_cam,
                        1.0,
                    );
                }
                DrawKind::Tilemap => {
                    let is_nav = self
                        .world
                        .get::<&Role>(*entity)
                        .is_ok_and(|r| r.value == RoleKind::NavMesh);

                    if is_nav {
                        let Some(ref gpu) = self.nav_gpu else { continue };
                        if let Ok(nav) = self.world.get::<&NavMesh>(*entity) {
                            let iso = cartesian_to_iso_4().inverse();
                            let iso_matrix = Mat4::from_scale(tf.scale) * iso;
                            let iso3 = Mat3::from_mat4(iso);
                            let normal_matrix = iso3.inverse().transpose();
                            let nav_ts = gfx
                                .textures
                                .get("navTileset")
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
                                    texture: Some("navTileset"),
                                    frame: None,
                                    color: None,
                                });
                            }
                            gfx.draw_tilemap(
                                &Mat4::from_translation(tf.position),
                                &cam,
                                &iso_matrix,
                                &gpu.tile_tex,
                                "navTileset",
                                &nav_ts,
                                &[8.0, 8.0],
                                &[nav.size_x as f32, nav.size_y as f32],
                                &[0.0, 0.0],
                                &[-1.0, -1.0],
                                -1,
                                &[0.0, 0.0, 1.0, 0.3],
                                &normal_matrix,
                                &self.light_ambient,
                                &self.light_dir,
                                &self.light_color,
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
                    // Build iso matrix: same as TS constructor
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
                        self.selection_mode,
                        &[0.0, 1.0, 1.0, 1.0],
                        &normal_matrix,
                        &self.light_ambient,
                        &self.light_dir,
                        &self.light_color,
                        self.show_grid,
                        gpu.vertex_count as i32,
                        &gpu.mesh_buf,
                    );
                }
                DrawKind::IsoSprite => {
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

                    let tex_dim = if let Some(tex) = gfx.textures.get(&iso_sprite.texture) {
                        (
                            tex.size.0 as f32 / iso_sprite.tile_set_size.x.max(0.001),
                            tex.size.1 as f32 / iso_sprite.tile_set_size.y.max(0.001),
                        )
                    } else {
                        continue;
                    };

                    let model = Self::compute_iso_sprite_model(
                        &iso_sprite,
                        &tf,
                        &tilemap_tf,
                        &tilemap,
                        tex_dim,
                    );
                    let depth_corners =
                        Self::compute_iso_depth_corners(tf.position, &iso_sprite.footprint);

                    if let Some(ref mut t) = self.trace {
                        let name = name_by_entity.get(entity).copied().unwrap_or("");
                        t.push(golden::TraceItemParams {
                            order: *order,
                            kind: "IsoSprite",
                            name,
                            model: &model,
                            camera_ignored: false,
                            texture: Some(&iso_sprite.texture),
                            frame: Some(iso_sprite.frame),
                            color: None,
                        });
                    }
                    gfx.draw_iso_sprite(
                        &model,
                        &cam,
                        &iso_sprite.texture,
                        iso_sprite.frame,
                        &[iso_sprite.tile_set_size.x, iso_sprite.tile_set_size.y],
                        &depth_corners,
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
                    let model = Mat4::from_translation(tf.position)
                        * Mat4::from_scale(Vec3::new(w, h, 1.0));
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
                        });
                    }
                    let ts = [sprite.tile_set_size.x, sprite.tile_set_size.y];
                    gfx.draw_sprite(
                        &model,
                        &Mat4::IDENTITY,
                        &sprite.texture,
                        sprite.frame,
                        &ts,
                        true,
                        1.0,
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
                    let font = self.sdf_fonts.get("dejavusans");
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
        let Some(font) = self.sdf_fonts.get("dejavusans").cloned() else {
            return;
        };

        let mut to_measure: Vec<(hecs::Entity, SdfTextRender, f32)> = Vec::new();
        for (e, (tf, sdf)) in self.world.query::<(&Transform, &SdfTextRender)>().iter() {
            if self.world.get::<&UiNode>(e).map(|n| n.parent.is_some()).unwrap_or(false) {
                to_measure.push((e, sdf.clone(), tf.scale.x));
            }
        }

        let mut changed = false;
        for (e, sdf, scale) in &to_measure {
            let buf = build_sdf_glyph_buffer(&font, &sdf.text, *scale, sdf.justify, 0.0);
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
        let (size_x, size_y, nav_data, heights, height_scale) = {
            let nav = match self.world.get::<&NavMesh>(nav_entity) {
                Ok(n) => n,
                Err(_) => return,
            };
            // Use parent tilemap's actual height data so nav tiles sit on terrain surface
            let (hd, hs) = self
                .entity_by_role(RoleKind::Tilemap)
                .and_then(|e| self.world.get::<&Tilemap>(e).ok())
                .map(|tm| (tm.height_data.clone(), tm.height_scale))
                .unwrap_or_else(|| {
                    let h = vec![1.0f32; (nav.size_x as usize + 1) * (nav.size_y as usize + 1)];
                    (h, 64.0)
                });
            (nav.size_x, nav.size_y, nav.data.clone(), hd, hs)
        };

        let Some(gfx) = self.gfx.as_mut() else { return };

        let (mesh_data, vcount) = build_mesh(size_x, size_y, &nav_data, &heights, height_scale);
        let mesh_buf =
            GlBuffer::from_slice(&gfx.gl, glow::ARRAY_BUFFER, &mesh_data, glow::DYNAMIC_DRAW);

        let (tile_pixels, tw, th) = build_tile_texture(size_x, size_y, &nav_data);
        let tile_tex = Engine::upload_data_texture(&gfx.gl, &tile_pixels, tw, th);

        self.nav_gpu = Some(TilemapGpu { mesh_buf, vertex_count: vcount, tile_tex });

        // Add Transform to nav entity so render query (&Transform, &NavMesh) matches.
        // Borrow position + scale from parent tilemap (matches TS IsometricNavMesh constructor).
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

    /// Initialize navigation from the nav mesh entity's inline `NavMesh.data`
    /// (loaded from `state.json`) and wire click-to-move.
    pub fn init_navigation(&mut self) {
        let Some(nav_entity) = self.entity_by_role(RoleKind::NavMesh) else { return };
        let nav_data = {
            let Ok(nav) = self.world.get::<&NavMesh>(nav_entity) else { return };
            nav.data.clone()
        };
        self.init_navigation_data(nav_data);
    }

    /// Install a pre-built navigation grid (`1` = walkable, `0` = blocked) and
    /// wire click-to-move.
    ///
    /// Used by generated scenes, which derive walkability from real terrain
    /// slope during generation and so must not have it recomputed here.
    pub fn init_navigation_data(&mut self, nav_tiles: Vec<u32>) {
        let Some(nav_entity) = self.entity_by_role(RoleKind::NavMesh) else { return };
        let Some(agent_entity) = self.entity_by_role(RoleKind::Agent) else { return };
        let Some(tilemap_entity) = self.entity_by_role(RoleKind::Tilemap) else { return };

        // The supplied grid is authoritative for passability.  A block here
        // used to re-derive walkability from tilemap heights and then discard
        // the result on the very next line; `sync_nav_heights` is now the one
        // place that does that, and only in response to a height edit.
        if let Ok(mut nav) = self.world.get::<&mut NavMesh>(nav_entity) {
            nav.data = nav_tiles;
        }

        // 3. Wire click-to-move.  Each frame, if the mouse was just
        //    clicked, compute an A* path and send it to the agent.
        let agent_ent = agent_entity;
        let nav_ent = nav_entity;
        self.on_update(move |engine| {
            if !engine.input.was_mouse_pressed(0) || engine.ui_consumed_click {
                return;
            }

            let (click_x, click_y, agent_pos, nav_data, s_x, s_y) = {
                let Ok(tm) = engine.world.get::<&Tilemap>(tilemap_entity) else { return };
                let Ok(_agent) = engine.world.get::<&IsoAgent>(agent_ent) else { return };
                let agent_tf = engine.world.get::<&Transform>(agent_ent).unwrap();
                let nav = engine.world.get::<&NavMesh>(nav_ent).unwrap();
                (
                    tm.mouse_iso_pos.x,
                    tm.mouse_iso_pos.y,
                    (agent_tf.position.x, agent_tf.position.y),
                    nav.data.clone(),
                    nav.size_x,
                    nav.size_y,
                )
            };

            let cx = click_x as i32;
            let cy = click_y as i32;
            let ax = agent_pos.0 as i32;
            let ay = agent_pos.1 as i32;

            // Bounds check.
            if cx < 0 || cx >= s_x || cy < 0 || cy >= s_y {
                return;
            }

            // Reject impassable destinations before running A*.  Without this
            // the search cannot succeed but still has to exhaust every
            // reachable cell before it can say so — 21 ms on a 400x400 map,
            // a dropped frame, on every click against a crater wall.  It also
            // happens to be the behaviour you want: clicking a cliff should
            // do nothing rather than walk to somewhere adjacent to it.
            if nav_data.get((cy * s_x + cx) as usize).copied().unwrap_or(0) == 0 {
                classic_core::cl_debug!(
                    classic_core::instrument::Chan::Path,
                    "click-to-move: ({cx}, {cy}) is impassable, ignoring"
                );
                return;
            }

            let _dist = (((cx - ax) * (cx - ax) + (cy - ay) * (cy - ay)) as f32).sqrt();

            if !engine.agent_selected {
                return;
            }

            // Convert nav data to owned i32 slice for the find_path API.
            let nav_i32: Vec<i32> = nav_data.iter().map(|&v| v as i32).collect();
            let size = (s_x, s_y);
            if let Some(raw_path) =
                pathfinder::find_path(&nav_i32, size.0, size.1, (ax, ay), (cx, cy))
            {
                // Offset waypoints by 0.5 to centre within tiles (matches TS).
                let mut path: Vec<_> = raw_path
                    .into_iter()
                    .map(|(x, y)| glam::Vec2::new(x as f32 + 0.5, y as f32 + 0.5))
                    .collect();

                if let Ok(mut agent) = engine.world.get::<&mut IsoAgent>(agent_ent) {
                    // Replace first waypoint with agent's exact current position
                    // (matches TS `this._path[0] = [this.position[0], this.position[1]]`).
                    let agent_tf = engine.world.get::<&Transform>(agent_ent).unwrap();
                    path[0] = glam::Vec2::new(agent_tf.position.x, agent_tf.position.y);

                    let waypoint_count = path.len();
                    agent.path = path;
                    agent.target_index = 1;
                    agent.delta = 0.0;
                    agent.init_dist = glam::Vec2::new(
                        agent.path[1].x - agent.path[0].x,
                        agent.path[1].y - agent.path[0].y,
                    )
                    .length()
                    .max(0.001);
                    agent.state = AgentState::FollowPath;
                    classic_core::cl_debug!(
                        classic_core::instrument::Chan::Nav,
                        "nav: path found with {waypoint_count} waypoints"
                    );
                }
            }
        });
    }

    /// Compute the model matrix for an IsoSprite (matches TS `IsoSprite.modelMatrix()`).
    fn compute_iso_sprite_model(
        iso_sprite: &IsoSprite,
        sprite_tf: &Transform,
        tilemap_tf: &Transform,
        tilemap: &Tilemap,
        tex_dim: (f32, f32),
    ) -> Mat4 {
        let iso_to_cart_world = iso_to_cartesian_4() * Mat4::from_scale(tilemap_tf.scale);

        let mut cart_pos = iso_to_cart_world.transform_point3(sprite_tf.position);
        cart_pos += tilemap_tf.position;

        let h = bilinear_height(
            &tilemap.height_data,
            tilemap.size_x,
            tilemap.size_y,
            sprite_tf.position.x,
            sprite_tf.position.y,
        );
        cart_pos.y -= h * tilemap.height_scale;

        let anchor_delta =
            Vec3::new(-tex_dim.0 * iso_sprite.anchor.x, -tex_dim.1 * iso_sprite.anchor.y, 0.0);

        Mat4::from_translation(cart_pos)
            * Mat4::from_scale(sprite_tf.scale)
            * Mat4::from_translation(anchor_delta)
            * Mat4::from_scale(Vec3::new(tex_dim.0, tex_dim.1, 1.0))
    }

    /// Compute iso depth corners for the footprint (matches TS `IsoSprite.rawDraw()`).
    fn compute_iso_depth_corners(pos: Vec3, footprint: &[glam::Vec2]) -> [f32; 4] {
        let base_depth = (pos.x - pos.y) / 400.0 + 0.5 - pos.z / 14500.0 - 0.005;

        let mut raw_depths = [0.0f32; 4];
        for i in 0..4 {
            let pt = &footprint[i];
            let d = (pos.x + pt.x - pos.y - pt.y) / 400.0 + 0.5 - pos.z / 14500.0 - 0.005;
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
