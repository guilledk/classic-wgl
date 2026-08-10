//! classic-engine: Runtime that wires core + gfx + platform.

pub mod env_config;
pub mod golden;
pub mod testing;
pub mod ui;

use std::collections::HashMap;
use std::rc::Rc;

use base64::Engine as _;
use classic_core::collision::{polygon_from_verts, PhysicsProvider};
use classic_core::components::{
    AgentState, Animator, Collider, DebugName, IsoAgent, IsoSprite, NavMesh, RectRender,
    SdfTextRender, TextJustify, Tilemap, UiAlign, UiAnchor, UiNode,
};
use classic_core::instrument::Chan;
use classic_core::math::{cartesian_to_iso_4, iso_to_cartesian_4};
use classic_core::pathfinder;
use classic_core::sdf_builder::build_sdf_glyph_buffer;
use classic_core::tilemap::{bilinear_height, build_mesh, build_tile_texture};
use classic_core::types::AnimationData;
use classic_core::types::SdfFontMetrics;
use classic_core::{Camera, SpriteRender, Transform};
use classic_gfx::{Gfx, GlBuffer};
use classic_platform::InputState;
use glam::{Mat3, Mat4, Vec2, Vec3, Vec4};
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

use testing::types::{AssertKind, TestAction, TestStep, TileAssertion};

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
    pub debug_footprints: bool,
    pub agent_selected: bool,
    pub light_ambient: [f32; 3],
    pub light_dir: [f32; 3],
    pub light_color: [f32; 3],
    pub animations: HashMap<String, AnimationData>,
    pub sdf_fonts: HashMap<String, SdfFontMetrics>,
    pub ui: Option<ui::UIManager>,
    pub editor_target: String,
    pub editor_tile: u32,
    pub editor_nav_tile: u32,
    pub editor_height: i32,
    pub height_scale_multiplier: i32,
    pub height_edit_mode: String,
    pub selection_mode: i32,
    pub selection_begin_screen: glam::Vec3,
    pub panel_menu_open: bool,
    pub light_preset: String,
    pub light_azimuth: f32,
    pub light_elevation: f32,
    pub tile_palette_e: Option<hecs::Entity>,
    pub nav_palette_e: Option<hecs::Entity>,
    pub height_widget_e: Option<hecs::Entity>,
    pub light_widget_e: Option<hecs::Entity>,
    pub text_showcase_e: Option<hecs::Entity>,
    pub text_demo_content_h: f32,
    pub menu_panel_e: Option<hecs::Entity>,
    iso_compass_buf: Option<GlBuffer>,
    iso_coord_x_e: Option<hecs::Entity>,
    iso_coord_y_e: Option<hecs::Entity>,
    iso_coord_z_e: Option<hecs::Entity>,
    nav_gpu: Option<TilemapGpu>,
    debug_frame: u64,
    test_step_index: usize,
    test_results: Vec<String>,
    test_drag_state: Option<(glam::Vec2, glam::Vec2, u64, u64)>, // (from, to, total_hold, start_frame)
    test_editor_state: Option<(String, i32, String, u32)>, // (target, height_delta, height_mode, tile_id)
    test_complete_reported: bool,
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
            debug_footprints: false,
            agent_selected: false,
            light_ambient: [0.15, 0.15, 0.2],
            light_dir: [0.45, -0.35, 0.82],
            light_color: [1.0, 0.95, 0.85],
            animations: HashMap::new(),
            sdf_fonts: HashMap::new(),
            ui: None,
            editor_target: "none".into(),
            editor_tile: 0,
            editor_nav_tile: 0,
            editor_height: 0,
            height_scale_multiplier: 1,
            height_edit_mode: "blend".into(),
            selection_mode: -1,
            selection_begin_screen: glam::Vec3::new(-1.0, -1.0, -1.0),
            panel_menu_open: false,
            light_preset: "sunny".into(),
            light_azimuth: 45.0,
            light_elevation: 45.0,
            tile_palette_e: None,
            nav_palette_e: None,
            height_widget_e: None,
            light_widget_e: None,
            text_showcase_e: None,
            text_demo_content_h: 0.0,
            menu_panel_e: None,
            iso_compass_buf: None,
            iso_coord_x_e: None,
            iso_coord_y_e: None,
            iso_coord_z_e: None,
            nav_gpu: None,
            debug_frame: 0,
            test_step_index: 0,
            test_results: Vec::new(),
            test_drag_state: None,
            test_editor_state: None,
            test_complete_reported: false,
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

    pub fn init_gfx(&mut self, gl: Rc<glow::Context>, manifest_json: &str) {
        let manifest: classic_core::types::Manifest =
            serde_json::from_str(manifest_json).expect("parse manifest.json");
        let mut gfx = Gfx::new(gl);
        for info in &manifest.shaders {
            let vs = Gfx::resolve_vertex_source(&info.vertex);
            let fs = Gfx::resolve_fragment_source(&info.fragment);
            gfx.add_shader(&info.name, vs, fs, &info.attr, &info.unif).expect("compile shader");
        }
        for anim in &manifest.animations {
            self.animations.insert(anim.name.clone(), anim.clone());
        }
        self.gfx = Some(gfx);
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
    pub fn decode_map_data(base64_str: &str) -> Vec<u32> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(base64_str.trim().as_bytes())
            .expect("base64 decode map data");
        let json: Vec<u32> = serde_json::from_slice(&bytes).expect("parse map data JSON");
        json
    }

    /// Build and upload the tilemap mesh + tile data texture for a named entity.
    pub fn init_tilemap(&mut self, entity_name: &str, tileset_png: &[u8], map_data_b64: &str) {
        let entity = *self.names.get(entity_name).expect("tilemap entity");

        // Extract tilemap data before mutable borrows.
        let (tile_set_name, size_x, size_y, tile_pixel_size) = {
            let tm = self.world.get::<&Tilemap>(entity).expect("Tilemap component");
            (tm.tile_set.clone(), tm.size_x, tm.size_y, tm.tile_pixel_size)
        };

        // Load tileset texture.
        self.load_texture_png(&tile_set_name, tileset_png);

        // Decode map data.
        let tiles = Self::decode_map_data(map_data_b64);

        // Generate height data — flat 1.0 matches TS `heightData.fill(1)`.
        // (Simplex noise terrain preserved below for later re-enable.)
        // let s = classic_core::simplex_noise::SimplexNoise::new("demo");
        // let mut heights = vec![0.0f32; (size_x as usize + 1) * (size_y as usize + 1)];
        // for ty in 0..=size_y {
        //     for tx in 0..=size_x {
        //         let h = classic_core::simplex_noise::noise_range(&s, tx as f32, ty as f32, 0.0, 4.0);
        //         heights[(ty * (size_x + 1) + tx) as usize] = h.max(0.0).floor();
        //     }
        // }
        let heights = vec![1.0f32; (size_x as usize + 1) * (size_y as usize + 1)];

        let height_scale = tile_pixel_size[0] as f32;
        let (mesh_data, vcount) = build_mesh(size_x, size_y, &tiles, &heights, height_scale);

        let (tile_pixels, tw, th) = build_tile_texture(size_x, size_y, &tiles);

        let gfx = self.gfx.as_mut().expect("gfx not initialized");

        // Upload mesh.
        let mesh_buf =
            GlBuffer::from_slice(&gfx.gl, glow::ARRAY_BUFFER, &mesh_data, glow::STATIC_DRAW);

        // Upload tile data texture.
        let tile_tex = {
            let tex = unsafe { gfx.gl.create_texture() }.expect("create texture");
            unsafe {
                gfx.gl.bind_texture(glow::TEXTURE_2D, Some(tex));
                gfx.gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA as i32,
                    tw as i32,
                    th as i32,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(&tile_pixels)),
                );
                gfx.gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MIN_FILTER,
                    glow::NEAREST as i32,
                );
                gfx.gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MAG_FILTER,
                    glow::NEAREST as i32,
                );
                gfx.gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_WRAP_S,
                    glow::CLAMP_TO_EDGE as i32,
                );
                gfx.gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_WRAP_T,
                    glow::CLAMP_TO_EDGE as i32,
                );
                gfx.gl.bind_texture(glow::TEXTURE_2D, None);
            }
            tex
        };

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

        self.tilemap_gpu.insert(
            entity_name.to_string(),
            TilemapGpu { mesh_buf, vertex_count: vcount, tile_tex },
        );

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

    /// Serialise all named entities to a state JSON string (TS-compatible format).
    pub fn dump_state(&self) -> String {
        let entities = self.dump_state_value();
        let root = serde_json::json!({ "entities": entities });
        serde_json::to_string_pretty(&root).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
    }

    fn dump_state_value(&self) -> serde_json::Value {
        let mut entities = serde_json::Map::new();
        let regs = classic_core::registry::ordered_regs();

        for name in &self.name_order {
            let Some(&entity) = self.names.get(name) else { continue };
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
                    if let Some(sub_dump) =
                        regs.iter().find(|r| r.name == *sub).and_then(|r| r.dump)
                    {
                        if let Some(val) = sub_dump(&self.world, entity) {
                            components.push(val);
                            dumped.insert(sub);
                        }
                    }
                }
            }

            if !components.is_empty() {
                entities.insert(name.clone(), serde_json::json!({ "components": components }));
            }
        }

        serde_json::Value::Object(entities)
    }

    /// Dump tile data as base64-encoded JSON array (for `map001.txt`-style sidecar).
    pub fn dump_map_data(&self) -> Option<String> {
        let &e = self.names.get("tilemap")?;
        let Ok(tm) = self.world.get::<&Tilemap>(e) else { return None };
        let json = serde_json::to_string(&tm.data).ok()?;
        let b64 =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, json.as_bytes());
        Some(b64)
    }

    /// Dump nav mesh data as base64-encoded JSON array (for `map001.nav.txt`-style sidecar).
    pub fn dump_nav_data(&self) -> Option<String> {
        let &e = self.names.get("tilemapNavigation")?;
        let Ok(nm) = self.world.get::<&NavMesh>(e) else { return None };
        let json = serde_json::to_string(&nm.data).ok()?;
        let b64 =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, json.as_bytes());
        Some(b64)
    }

    /// Dump height data as base64-encoded JSON array.
    pub fn dump_height_data(&self) -> Option<String> {
        let &e = self.names.get("tilemap")?;
        let Ok(tm) = self.world.get::<&Tilemap>(e) else { return None };
        let json = serde_json::to_string(&tm.height_data).ok()?;
        let b64 =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, json.as_bytes());
        Some(b64)
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
        self.physics.mouse_clicked = self.input.was_mouse_pressed(0);
        self.physics.perform_calls();

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
            if let Some(&e) = self.names.get("tilemap") {
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

        // Route mouse wheel: if over the text demo container, apply scroll
        // and zero the wheel so the camera on_update doesn't also zoom.
        // Must run BEFORE on_update closures (camera runs first in order).
        if self.input.mouse_wheel.abs() > 0.01
            && self.editor_target == "textDemo"
            && self.text_showcase_e.is_some()
        {
            if let Some(ref ui) = self.ui {
                let panel_w: f32 = 520.0;
                let panel_h: f32 = 440.0;
                let border: f32 = 10.0;
                let px = ui.viewport_w - panel_w - border;
                let py = ui.viewport_h - panel_h - border;
                let mouse = self.input.mouse_pos;
                let in_bounds = mouse.x >= px
                    && mouse.x <= px + panel_w
                    && mouse.y >= py
                    && mouse.y <= py + panel_h;
                if in_bounds {
                    let ds = self.input.mouse_wheel * 30.0;
                    let max_scroll = (self.text_demo_content_h - panel_h).max(0.0);
                    if let Some(e) = self.text_showcase_e {
                        if let Ok(mut node) = self.world.get::<&mut UiNode>(e) {
                            node.scroll_y = (node.scroll_y - ds).clamp(0.0, max_scroll);
                        }
                    }
                    self.input.mouse_wheel = 0.0;
                }
            }
        }

        let mut fns = std::mem::take(&mut self.update_fns);
        for f in fns.iter_mut() {
            f(self);
        }
        self.update_fns = fns;

        // ---- CLASSIC_TEST automated test runner ----
        if env_config::EnvConfig::get().test_active() {
            static STEPS: std::sync::LazyLock<Vec<TestStep>> = std::sync::LazyLock::new(|| {
                let name = env_config::EnvConfig::get().test.clone();
                Engine::build_test_scenario(&name)
            });
            // Set golden capture frame to 1 frame after the last test step.
            if let Some(last) = STEPS.last() {
                self.golden_capture_frame = last.frame + 1;
            }
            self.run_test_frame(&STEPS);
        }

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

        self.ui_consumed_click = false;

        if self.input.was_mouse_released(0) && !self.ui_consumed_click {
            let just_finished_selection = self.selection_mode == 1;
            if self.selection_mode == 1 {
                self.selection_mode = -1;
                if let Some(&e) = self.names.get("tilemap") {
                    if let Ok(mut tm) = self.world.get::<&mut Tilemap>(e) {
                        tm.selection_iso_end = tm.mouse_iso_pos;
                    }
                }
            }
            self.physics.end_selection();

            // Editor paint on selection-end (bypasses collider quadtree for tilemap)
            if just_finished_selection {
                self.apply_editor_selection();
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
            if self.debug_name(e) == "tilemapNavigation" && self.nav_gpu.is_some() {
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
                    let entity_name = name_by_entity.get(entity).copied().unwrap_or("");
                    let is_nav = entity_name == "tilemapNavigation";

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
            let baseline_dir = cwd.join("tests/golden/baseline");
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
                            let dir = "tests/golden/baseline";
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
                            let path = "tests/golden/baseline/baseline.png";
                            match image::open(path) {
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
        // ---- debug: footprint polygons + anchor crosshairs ----
        if self.debug_footprints {
            let x_cross: [f32; 12] =
                [-8.0, -8.0, 0.0, 8.0, 8.0, 0.0, -8.0, 8.0, 0.0, 8.0, -8.0, 0.0];
            let x_cross_buf =
                GlBuffer::from_slice(&gfx.gl, glow::ARRAY_BUFFER, &x_cross, glow::STATIC_DRAW);

            let tm_entity = self.names.get("tilemap").copied();
            if let Some(tm_e) = tm_entity {
                let (iso_to_cart_world, tilemap_pos, size_x, size_y, hd, hs) = {
                    let tm = self.world.get::<&Tilemap>(tm_e).unwrap();
                    let tm_tf = self.world.get::<&Transform>(tm_e).unwrap();
                    let iso_to_cart = iso_to_cartesian_4() * Mat4::from_scale(tm_tf.scale);
                    (
                        iso_to_cart,
                        tm_tf.position,
                        tm.size_x,
                        tm.size_y,
                        tm.height_data.clone(),
                        tm.height_scale,
                    )
                };

                let at = |tx: i32, ty: i32| -> f32 {
                    let tx = tx.clamp(0, size_x) as usize;
                    let ty = ty.clamp(0, size_y) as usize;
                    hd.get(ty * (size_x as usize + 1) + tx).copied().unwrap_or(0.0)
                };

                for (_e, (iso_sprite, tf)) in self.world.query::<(&IsoSprite, &Transform)>().iter()
                {
                    // ---- footprint polygon ----
                    let mut world_fp: Vec<f32> = Vec::with_capacity(iso_sprite.footprint.len() * 3);
                    for pt in &iso_sprite.footprint {
                        let px = tf.position.x + pt.x;
                        let py = tf.position.y + pt.y;
                        let ftx = px.floor() as i32;
                        let fty = py.floor() as i32;
                        let fx = px - ftx as f32;
                        let fy = py - fty as f32;
                        let h_nw = at(ftx, fty);
                        let h_ne = at(ftx + 1, fty);
                        let h_sw = at(ftx, fty + 1);
                        let h_se = at(ftx + 1, fty + 1);
                        let h = h_nw
                            + (h_ne - h_nw) * fx
                            + (h_sw - h_nw) * fy
                            + (h_nw - h_ne - h_sw + h_se) * fx * fy;

                        let mut v = glam::Vec3::new(px, py, 0.0);
                        v = iso_to_cart_world.transform_point3(v);
                        v += tilemap_pos;
                        v.y -= h * hs;
                        world_fp.extend_from_slice(&[v.x, v.y, v.z]);
                    }

                    if world_fp.is_empty() {
                        continue;
                    }
                    let fp_buf = GlBuffer::from_slice(
                        &gfx.gl,
                        glow::ARRAY_BUFFER,
                        &world_fp,
                        glow::STREAM_DRAW,
                    );
                    let vcount = (world_fp.len() / 3) as i32;

                    gfx.draw_line_loop(
                        &fp_buf,
                        vcount,
                        &Mat4::IDENTITY,
                        &cam,
                        &[0.0, 1.0, 0.5, 0.7],
                    );

                    // ---- anchor crosshair ----
                    let ax = tf.position.x;
                    let ay = tf.position.y;
                    let aftx = ax.floor() as i32;
                    let afty = ay.floor() as i32;
                    let afx = ax - aftx as f32;
                    let afy = ay - afty as f32;
                    let ah_nw = at(aftx, afty);
                    let ah_ne = at(aftx + 1, afty);
                    let ah_sw = at(aftx, afty + 1);
                    let ah_se = at(aftx + 1, afty + 1);
                    let ah = ah_nw
                        + (ah_ne - ah_nw) * afx
                        + (ah_sw - ah_nw) * afy
                        + (ah_nw - ah_ne - ah_sw + ah_se) * afx * afy;

                    let mut anchor_world = glam::Vec3::new(ax, ay, 0.0);
                    anchor_world = iso_to_cart_world.transform_point3(anchor_world);
                    anchor_world += tilemap_pos;
                    anchor_world.y -= ah * hs;

                    let anchor_m = Mat4::from_translation(anchor_world);
                    gfx.draw_line_strip(&x_cross_buf, 0, 2, &anchor_m, &cam, &[1.0, 0.0, 1.0, 0.9]);
                    gfx.draw_line_strip(&x_cross_buf, 2, 2, &anchor_m, &cam, &[1.0, 0.0, 1.0, 0.9]);
                }

                // Selection ring around selected agent (yellow diamond).
                if self.agent_selected {
                    if let Some(agent_e) = self.names.get("navAgent").copied() {
                        if let Ok(agent_tf) = self.world.get::<&Transform>(agent_e) {
                            let pos = agent_tf.position;
                            let ring_iso: [(f32, f32); 4] = [
                                (pos.x - 1.0, pos.y),
                                (pos.x, pos.y - 1.0),
                                (pos.x + 1.0, pos.y),
                                (pos.x, pos.y + 1.0),
                            ];
                            let mut ring_verts: Vec<f32> = Vec::with_capacity(12);
                            for &(ix, iy) in &ring_iso {
                                let mut v = glam::Vec3::new(ix, iy, 0.0);
                                v = iso_to_cart_world.transform_point3(v);
                                v += tilemap_pos;
                                let ftx = ix.floor() as i32;
                                let fty = iy.floor() as i32;
                                let fx = ix - ftx as f32;
                                let fy = iy - fty as f32;
                                let h = at(ftx, fty) * (1.0 - fx) * (1.0 - fy)
                                    + at(ftx + 1, fty) * fx * (1.0 - fy)
                                    + at(ftx, fty + 1) * (1.0 - fx) * fy
                                    + at(ftx + 1, fty + 1) * fx * fy;
                                v.y -= h * hs;
                                ring_verts.extend_from_slice(&[v.x, v.y, v.z]);
                            }
                            let rb = GlBuffer::from_slice(
                                &gfx.gl,
                                glow::ARRAY_BUFFER,
                                &ring_verts,
                                glow::STREAM_DRAW,
                            );
                            gfx.draw_line_loop(
                                &rb,
                                4,
                                &Mat4::IDENTITY,
                                &cam,
                                &[1.0, 1.0, 0.0, 0.8],
                            );
                        }
                    }
                }
            }
        }

        // ---- iso compass rose (always visible) ----
        if let Some(ref buf) = self.iso_compass_buf {
            let cx: f32 = 100.0;
            let cy: f32 = 155.0;
            let ax_ox: f32 = 220.0;
            let model = Mat4::from_translation(Vec3::new(cx, cy, -1500.0));
            let ax_model = Mat4::from_translation(Vec3::new(ax_ox, cy, -1500.0));
            let gcol = [0.6, 0.6, 0.5, 0.4];
            let scol = [1.0, 1.0, 0.8, 0.85];
            gfx.draw_line_strip(buf, 0, 2, &model, &Mat4::IDENTITY, &gcol);
            gfx.draw_line_strip(buf, 2, 2, &model, &Mat4::IDENTITY, &gcol);
            for i in 0..4 {
                gfx.draw_line_strip(buf, 4 + i * 2, 2, &model, &Mat4::IDENTITY, &scol);
            }
            gfx.draw_line_strip(buf, 12, 2, &ax_model, &Mat4::IDENTITY, &[1.0, 0.2, 0.2, 1.0]);
            gfx.draw_line_strip(buf, 14, 2, &ax_model, &Mat4::IDENTITY, &[0.2, 1.0, 0.2, 1.0]);
            gfx.draw_line_strip(buf, 16, 2, &ax_model, &Mat4::IDENTITY, &[0.2, 0.2, 1.0, 1.0]);
        }
    }

    pub fn init_camera_wasd(&mut self) {
        self.on_update(|engine| {
            let speed = engine.scroll_speed * engine.time.delta;
            let inp = &engine.input;
            if inp.is_key_down("KeyW") {
                engine.camera.position.y -= speed;
            }
            if inp.is_key_down("KeyS") {
                engine.camera.position.y += speed;
            }
            if inp.is_key_down("KeyA") {
                engine.camera.position.x -= speed;
            }
            if inp.is_key_down("KeyD") {
                engine.camera.position.x += speed;
            }
            if engine.input.mouse_wheel.abs() > 0.01 {
                let dz = engine.input.mouse_wheel * engine.time.delta;
                engine.camera.scale.x += dz;
                engine.camera.scale.y += dz;
                let min = Vec3::new(0.1, 0.1, 1.0);
                engine.camera.scale = engine.camera.scale.max(min);
            }
        });
    }

    pub fn init_cursor(&mut self) {
        let cursor_entity = self.names.get("cursor").copied();
        self.on_update(move |engine| {
            let Some(cursor_e) = cursor_entity else { return };
            let mp = engine.input.mouse_pos;
            if let Ok(mut tf) = engine.world.get::<&mut Transform>(cursor_e) {
                tf.position.x = mp.x;
                tf.position.y = mp.y;
            }
        });
    }

    /// Register the isometric agent update system: idle animation + path-following
    /// state machine.  Only entities with `(IsoAgent, Animator, Transform)` are processed.
    pub fn init_agent_system(&mut self) {
        const ANIM_DIRS: [&str; 8] =
            ["East", "SouthEast", "South", "SouthWest", "West", "NorthWest", "North", "NorthEast"];

        self.on_update(|engine| {
            let delta = engine.time.delta;
            let anim_names: std::collections::HashSet<String> =
                engine.animations.keys().cloned().collect();

            let mut z_updates: Vec<(hecs::Entity, f32)> = Vec::new();

            for (_e, (agent, anim, tf)) in
                engine.world.query::<(&mut IsoAgent, &mut Animator, &mut Transform)>().iter()
            {
                match agent.state {
                    AgentState::Idle => {
                        let dir_name = ANIM_DIRS[agent.anim_index % 8];
                        let anim_name = format!("idle{dir_name}");
                        if anim_names.contains(&anim_name) {
                            anim.animation = Some(anim_name);
                            anim.playing = true;
                            anim.repeat = true;
                        }
                    }
                    AgentState::FollowPath => {
                        if agent.target_index >= agent.path.len()
                            || agent.target_index == 0
                            || agent.path.get(agent.target_index).is_none()
                        {
                            agent.state = AgentState::Idle;
                            continue;
                        }

                        if agent.delta >= 1.0 {
                            agent.delta = 0.0;
                            agent.target_index += 1;

                            if agent.target_index >= agent.path.len() {
                                agent.state = AgentState::Idle;
                                continue;
                            }

                            let from = &agent.path[agent.target_index - 1];
                            let to = &agent.path[agent.target_index];
                            agent.init_dist =
                                glam::Vec2::new(from.x - to.x, from.y - to.y).length().max(0.001);
                        }

                        let from = agent.path[agent.target_index - 1];
                        let to = agent.path[agent.target_index];

                        // Direction
                        let dx = to.x - from.x;
                        let dy = to.y - from.y;
                        let radians = dy.atan2(dx);
                        agent.direction = radians.to_degrees();
                        let mut ix = (agent.direction / 45.0).floor() as i32;
                        ix = ((ix % 8) + 8) % 8;
                        agent.anim_index = ix as usize;

                        let dir_name = ANIM_DIRS[agent.anim_index % 8];
                        let anim_name = format!("walk{dir_name}");
                        if anim_names.contains(&anim_name) {
                            anim.animation = Some(anim_name);
                            anim.playing = true;
                            anim.repeat = true;
                        }

                        // Lerp position
                        let start = glam::Vec3::new(from.x, from.y, tf.position.z);
                        let end = glam::Vec3::new(to.x, to.y, tf.position.z);
                        tf.position.x = start.x + (end.x - start.x) * agent.delta;
                        tf.position.y = start.y + (end.y - start.y) * agent.delta;

                        // Terrain height sampling
                        let tilemap_entity = engine.names.get(&agent.tilemap).copied();
                        if let Some(tm_e) = tilemap_entity {
                            if let Ok(tm) = engine.world.get::<&Tilemap>(tm_e) {
                                let px = tf.position.x;
                                let py = tf.position.y;
                                let ftx = px.floor() as i32;
                                let fty = py.floor() as i32;
                                let fx = px - ftx as f32;
                                let fy = py - fty as f32;

                                let at = |tx: i32, ty: i32| -> f32 {
                                    let tx = tx.clamp(0, tm.size_x) as usize;
                                    let ty = ty.clamp(0, tm.size_y) as usize;
                                    tm.height_data
                                        .get(ty * (tm.size_x as usize + 1) + tx)
                                        .copied()
                                        .unwrap_or(0.0)
                                };

                                let h_nw = at(ftx, fty);
                                let h_ne = at(ftx + 1, fty);
                                let h_sw = at(ftx, fty + 1);
                                let h_se = at(ftx + 1, fty + 1);
                                let hi = h_nw
                                    + (h_ne - h_nw) * fx
                                    + (h_sw - h_nw) * fy
                                    + (h_nw - h_ne - h_sw + h_se) * fx * fy;
                                let target_z = hi * tm.height_scale;

                                // Speed factor from steepness
                                let dx_h = (h_ne - h_nw) * (1.0 - fy) + (h_se - h_sw) * fy;
                                let dy_h = (h_sw - h_nw) * (1.0 - fx) + (h_se - h_ne) * fx;
                                let steepness = (dx_h * dx_h + dy_h * dy_h).sqrt();
                                let speed_factor = 1.0 - (steepness.min(3.0) / 3.0) * 0.5;

                                agent.delta += (agent.speed * speed_factor * delta)
                                    / agent.init_dist.max(0.001);

                                z_updates.push((_e, target_z));
                            }
                        }

                        if tilemap_entity.is_none() {
                            agent.state = AgentState::Idle;
                        }
                    }
                }
            }

            // Phase 2: smooth z interpolation
            for (e, target_z) in z_updates {
                if let Ok(mut tf) = engine.world.get::<&mut Transform>(e) {
                    let z_speed = (delta * 4.0).min(1.0);
                    tf.position.z += (target_z - tf.position.z) * z_speed;
                }
            }
        });
    }

    /// Register the animator system: advances all `Animator` counters and
    /// pushes frame changes to their target IsoSprite / IsoAgent components.
    pub fn init_animator_system(&mut self) {
        self.on_update(|engine| {
            let delta = engine.time.delta;

            let anim_rates: HashMap<String, (f32, usize)> = engine
                .animations
                .iter()
                .map(|(n, a)| (n.clone(), (a.rate, a.sequence.len())))
                .collect();

            let mut frame_writes: Vec<(hecs::Entity, String, f32)> = Vec::new();
            for (_e, anim) in engine.world.query::<&mut Animator>().iter() {
                if !anim.playing && !anim.repeat {
                    continue;
                }
                let Some(ref anim_name) = anim.animation else {
                    continue;
                };
                let Some(&(rate, seq_len)) = anim_rates.get(anim_name.as_str()) else {
                    continue;
                };

                anim.counter += delta * rate * anim.speed;

                if seq_len == 0 {
                    anim.counter = 0.0;
                    anim.frame = 0.0;
                    anim.playing = false;
                    continue;
                }

                let frame_idx = anim.counter.floor() as usize;
                if frame_idx >= seq_len {
                    anim.counter = 0.0;
                    anim.frame = engine
                        .animations
                        .get(anim_name.as_str())
                        .and_then(|a| a.sequence.first().copied())
                        .unwrap_or(0) as f32;
                    if !anim.repeat {
                        anim.playing = false;
                    }
                } else if let Some(&frame) = engine
                    .animations
                    .get(anim_name.as_str())
                    .and_then(|a| a.sequence.get(frame_idx))
                {
                    anim.frame = frame as f32;
                }

                let parts: Vec<&str> = anim.target.splitn(2, '.').collect();
                if parts.len() == 2 {
                    if let Some(&target_e) = engine.names.get(parts[0]) {
                        frame_writes.push((target_e, parts[1].to_string(), anim.frame));
                    }
                }
            }

            for (target_e, comp_type, frame) in &frame_writes {
                match comp_type.as_str() {
                    "IsoAgent" => {
                        if let Ok(mut a) = engine.world.get::<&mut IsoAgent>(*target_e) {
                            a.frame = *frame;
                        }
                        if let Ok(mut s) = engine.world.get::<&mut IsoSprite>(*target_e) {
                            s.frame = *frame;
                        }
                    }
                    "IsoSprite" => {
                        if let Ok(mut s) = engine.world.get::<&mut IsoSprite>(*target_e) {
                            s.frame = *frame;
                        }
                    }
                    _ => {}
                }
            }
        });
    }

    /// Attach footprint Polygon colliders to all static (non-agent) IsoSprite
    /// entities.  Port of `initFootprintColliders` from `prefabs.ts`.
    pub fn init_footprint_colliders(&mut self) {
        // Look up the tilemap entity once.
        let tm_entity = self.names.get("tilemap").copied().expect("tilemap entity");

        let (isosprite_entities, _tilemap_name, iso_to_cart_world, tilemap_pos) = {
            let tilemap = self.world.get::<&Tilemap>(tm_entity).unwrap();
            let tilemap_tf = self.world.get::<&Transform>(tm_entity).unwrap();
            let isosprite_entities: Vec<hecs::Entity> =
                self.world.query::<&IsoSprite>().iter().map(|(e, _)| e).collect();

            let iso_to_cart_world = iso_to_cartesian_4() * Mat4::from_scale(tilemap_tf.scale);

            (isosprite_entities, tilemap.tile_set.clone(), iso_to_cart_world, tilemap_tf.position)
        };

        for entity in isosprite_entities {
            // Skip agents — they get their own collider handling elsewhere.
            if self.world.get::<&IsoAgent>(entity).is_ok() {
                continue;
            }

            let (sprite_iso_pos, footprint) = {
                let s = self.world.get::<&IsoSprite>(entity).unwrap();
                (s.position, s.footprint.clone())
            };

            // Per-vertex bilinear height lookup.
            let tm = self.world.get::<&Tilemap>(tm_entity).unwrap();
            let hd = &tm.height_data;
            let sx = tm.size_x;
            let sy = tm.size_y;
            let hs = tm.height_scale;
            let at = |tx: i32, ty: i32| -> f32 {
                let tx = tx.clamp(0, sx) as usize;
                let ty = ty.clamp(0, sy) as usize;
                hd.get(ty * (sx as usize + 1) + tx).copied().unwrap_or(0.0)
            };

            let mut world_verts: Vec<glam::Vec3> = Vec::with_capacity(footprint.len());
            for pt in &footprint {
                let px = sprite_iso_pos.x + pt.x;
                let py = sprite_iso_pos.y + pt.y;

                let ftx = px.floor() as i32;
                let fty = py.floor() as i32;
                let fx = px - ftx as f32;
                let fy = py - fty as f32;

                let h_nw = at(ftx, fty);
                let h_ne = at(ftx + 1, fty);
                let h_sw = at(ftx, fty + 1);
                let h_se = at(ftx + 1, fty + 1);
                let h = h_nw
                    + (h_ne - h_nw) * fx
                    + (h_sw - h_nw) * fy
                    + (h_nw - h_ne - h_sw + h_se) * fx * fy;

                let mut v = glam::Vec3::new(px, py, 0.0);
                v = iso_to_cart_world.transform_point3(v);
                v += tilemap_pos;
                v.y -= h * hs;
                world_verts.push(v);
            }

            if world_verts.is_empty() {
                continue;
            }

            let shape = polygon_from_verts(world_verts);
            let pid = self.physics.register_collider(Collider::new(shape));
            log::debug!("registered footprint collider pid={pid} for sprite");

            // Set sprite z-offset from terrain height (matches TS prefabs.ts:367).
            let px = sprite_iso_pos.x;
            let py = sprite_iso_pos.y;
            let ftx = px.floor() as i32;
            let fty = py.floor() as i32;
            let fx = px - ftx as f32;
            let fy = py - fty as f32;
            let h_top = at(ftx, fty) + (at(ftx + 1, fty) - at(ftx, fty)) * fx;
            let h_bot = at(ftx, fty + 1) + (at(ftx + 1, fty + 1) - at(ftx, fty + 1)) * fx;
            let terrain_z = (h_top + (h_bot - h_top) * fy) * hs;

            if let Ok(mut tf) = self.world.get::<&mut Transform>(entity) {
                tf.position.z = terrain_z;
            }
        }
    }

    /// Register keyboard toggles for debug overlays (F = footprints).
    pub fn init_debug_toggles(&mut self) {
        self.on_update(|engine| {
            if engine.input.was_key_pressed("KeyF") {
                engine.debug_footprints = !engine.debug_footprints;
                engine.show_grid = engine.debug_footprints;
            }
            // F9: dump state.json
            if engine.input.was_key_pressed("F9") {
                let state = engine.dump_state();
                engine.save_file("state.json", &state);
                let shift =
                    engine.input.is_key_down("ShiftLeft") || engine.input.is_key_down("ShiftRight");
                if shift {
                    if let Some(map_data) = engine.dump_map_data() {
                        engine.save_file("map001.txt", &map_data);
                    }
                    if let Some(nav_data) = engine.dump_nav_data() {
                        engine.save_file("map001.nav.txt", &nav_data);
                    }
                    if let Some(h_data) = engine.dump_height_data() {
                        engine.save_file("map001.height.txt", &h_data);
                    }
                }
            }
        });
    }

    /// Apply a named lighting preset (sunny, cloudy, dawn, night).
    pub fn apply_light_preset(&mut self, key: &str) {
        let preset = match key {
            "sunny" => {
                Some(("Sunny Day", [0.15, 0.15, 0.2], [0.453, 0.211, 0.866], [1.0, 0.95, 0.85]))
            }
            "cloudy" => Some(("Cloudy", [0.35, 0.35, 0.4], [0.0, -0.2, 1.0], [0.7, 0.72, 0.78])),
            "dawn" => Some(("Dawn / Dusk", [0.2, 0.15, 0.25], [0.5, 0.2, 0.3], [1.0, 0.4, 0.2])),
            "night" => Some(("Night", [0.1, 0.12, 0.25], [-0.2, -0.5, 0.8], [0.3, 0.4, 0.7])),
            _ => None,
        };
        let Some((_name, ambient, dir_unnorm, color)) = preset else {
            return;
        };
        let d = glam::Vec3::new(dir_unnorm[0], dir_unnorm[1], dir_unnorm[2]).normalize();
        self.light_preset = key.into();
        self.light_ambient = ambient;
        self.light_dir = [d.x, d.y, d.z];
        self.light_color = color;
        self.light_azimuth = d.x.atan2(-d.y).to_degrees();
        self.light_elevation = d.z.asin().to_degrees();
    }

    /// Recompute light direction from azimuth/elevation angles.
    pub fn update_light_direction(&mut self) {
        let az = self.light_azimuth.to_radians();
        let el = self.light_elevation.to_radians();
        let d = glam::Vec3::new(el.cos() * az.sin(), -el.cos() * az.cos(), el.sin()).normalize();
        self.light_dir = [d.x, d.y, d.z];
    }

    /// Initialize lighting defaults.
    pub fn init_lighting(&mut self) {
        self.apply_light_preset("sunny");
    }

    /// Spawn HUD text entities using the UI layout system.
    pub fn init_ui(&mut self) {
        let vp_w = 1280.0_f32;
        let vp_h = 720.0_f32;
        let mut ui = ui::UIManager::new(vp_w, vp_h, &mut self.world);

        // Top bar
        let top_bar = ui.spawn_container(&mut self.world, vp_w, 68.0, [0.0, 0.0, 0.0, 0.5]);
        ui.root_add_child(&mut self.world, top_bar, UiAnchor::TopCenter, UiAnchor::TopCenter);

        // FPS counter (left)
        let fps_text = ui.spawn_sdf_text(
            &mut self.world,
            "0",
            1.4,
            100.0,
            [0.0, 0.6, 0.0, 1.0],
            TextJustify::Left,
        );
        if let Ok(mut sdf) = self.world.get::<&mut SdfTextRender>(fps_text) {
            sdf.weight = 0.15;
        }
        ui.container_add_child(
            &mut self.world,
            top_bar,
            fps_text,
            UiAnchor::MidLeft,
            UiAnchor::MidLeft,
        );

        // Banner (center)
        let banner = ui.spawn_sdf_text(
            &mut self.world,
            "CLASSIC-ISO",
            1.5,
            600.0,
            [1.0, 0.53, 0.3, 1.0],
            TextJustify::Center,
        );
        if let Ok(mut sdf) = self.world.get::<&mut SdfTextRender>(banner) {
            sdf.outline_color = [0.1, 0.05, 0.0, 1.0];
            sdf.outline_width = 0.12;
        }
        ui.container_add_child(
            &mut self.world,
            top_bar,
            banner,
            UiAnchor::MidCenter,
            UiAnchor::MidCenter,
        );

        // Info text (right)
        let info = ui.spawn_sdf_text(
            &mut self.world,
            "WASD MOVE\nSCROLL ZOOM",
            1.0,
            300.0,
            [1.0, 0.2, 0.6, 1.0],
            TextJustify::Right,
        );
        ui.container_add_child(
            &mut self.world,
            top_bar,
            info,
            UiAnchor::MidRight,
            UiAnchor::MidRight,
        );

        // FPS update closure
        let fps_e = fps_text;
        self.on_update(move |engine| {
            if let Ok(mut sdf) = engine.world.get::<&mut SdfTextRender>(fps_e) {
                let fps = engine.time.fps;
                sdf.text = fps.to_string();
                sdf.color = if fps >= 30 { [0.0, 0.6, 0.0, 1.0] } else { [0.8, 0.0, 0.0, 1.0] };
            }
        });

        // Refresh layout driven by frame() after resize + before physics.
        self.ui = Some(ui);
    }

    /// Spawn the DEV button tool panel with slide-out menu, agent selector,
    /// and backdrop for click-outside-to-close.
    #[allow(clippy::too_many_lines)]
    pub fn init_tool_buttons(&mut self) {
        use std::cell::Cell;
        use std::cell::RefCell;
        use std::rc::Rc;

        let btn_size: f32 = 128.0;
        let agent_size: f32 = 64.0;
        let menu_item_h: f32 = 28.0;
        let menu_padding: f32 = 6.0;
        let menu_gap: f32 = 2.0;
        let menu_font_scale: f32 = 0.45;
        let menu_panel_gap: f32 = 0.0;
        let agent_pad: f32 = 8.0;

        let menu_targets: [(&str, &str); 6] = [
            ("Tile Editor", "tilemap"),
            ("Nav Editor", "navMesh"),
            ("Height Editor", "height"),
            ("Light Config", "light"),
            ("Footprints", "_footprints"),
            ("Text Demo", "textDemo"),
        ];

        let max_label_len = menu_targets.iter().map(|m| m.0.len()).max().unwrap_or(12);
        let glyph_w = 18.0_f32;
        let menu_w = max_label_len as f32 * glyph_w + menu_padding * 2.0;
        let n = menu_targets.len() as f32;
        let menu_h = n * menu_item_h + menu_gap * (n - 1.0) + menu_padding * 2.0;

        // Shared state used by click handlers + on_update sync
        let is_open = Rc::new(Cell::new(false));
        let editor_tgt = Rc::new(RefCell::new(String::from("none")));
        let agent_sel = Rc::new(Cell::new(false));
        let dbg_feet = Rc::new(Cell::new(false));

        // Spawn all UI entities inside a block so the ui borrow is released
        // before calling set_enabled (which borrows self).
        let (agent_btn, btn_arr, menu_panel, backdrop, item_rows);
        {
            let Some(ref mut ui) = self.ui else { return };

            // Transparent vertical array: agent on top, dev below, center-aligned
            let btn_array = ui.spawn_array(
                &mut self.world,
                true,
                UiAlign::Center,
                agent_pad,
                [0.0, 0.0, 0.0, 0.0],
            );

            // Agent [A] button
            let ag;
            {
                let ags = agent_sel.clone();
                let et = editor_tgt.clone();
                ag = ui.spawn_button(
                    &mut self.world,
                    &mut self.physics,
                    agent_size,
                    agent_size,
                    [0.1, 0.6, 0.1, 0.8],
                    ui::ButtonOptions {
                        text: Some("A".into()),
                        text_scale: 0.4,
                        sdf_text: true,
                        hover: true,
                        click_priority: 1,
                        click_action: Some(Box::new(move || {
                            ags.set(!ags.get());
                            *et.borrow_mut() = "none".into();
                            true
                        })),
                        ..Default::default()
                    },
                );
            }
            agent_btn = ag;
            ui.container_add_child(
                &mut self.world,
                btn_array,
                ag,
                UiAnchor::TopLeft,
                UiAnchor::TopLeft,
            );

            // DEV button sprite
            let dev;
            {
                let iso = is_open.clone();
                let et = editor_tgt.clone();
                let ag_d = agent_sel.clone();
                dev = ui.spawn_button(
                    &mut self.world,
                    &mut self.physics,
                    btn_size,
                    btn_size,
                    [0.0, 0.0, 0.0, 0.0],
                    ui::ButtonOptions {
                        sprite: Some("editorIcons".into()),
                        sprite_frame: 0.0,
                        sprite_tile_set: [4.0, 4.0],
                        hover: true,
                        click_action: Some(Box::new(move || {
                            iso.set(!iso.get());
                            if !iso.get() {
                                *et.borrow_mut() = "none".into();
                            }
                            ag_d.set(false);
                            true
                        })),
                        ..Default::default()
                    },
                );
            }
            ui.container_add_child(
                &mut self.world,
                btn_array,
                dev,
                UiAnchor::TopLeft,
                UiAnchor::TopLeft,
            );

            // Menu panel
            menu_panel = ui.spawn_container(&mut self.world, menu_w, menu_h, [0.1, 0.1, 0.1, 0.95]);

            // Menu item rows
            let mut rows: Vec<(hecs::Entity, usize)> = Vec::new();
            for (idx, (label, target)) in menu_targets.iter().enumerate() {
                let row_w = menu_w - menu_padding * 2.0;
                let t_str = (*target).to_string();
                let et = editor_tgt.clone();
                let ags = agent_sel.clone();
                let iso = is_open.clone();
                let df = dbg_feet.clone();

                let click_fn: Box<dyn FnMut() -> bool> = if t_str == "_footprints" {
                    Box::new(move || {
                        df.set(!df.get());
                        iso.set(false);
                        true
                    })
                } else {
                    Box::new(move || {
                        let mut t = et.borrow_mut();
                        *t = if *t == t_str { "none".into() } else { t_str.clone() };
                        ags.set(false);
                        iso.set(false);
                        true
                    })
                };

                let row = ui.spawn_button(
                    &mut self.world,
                    &mut self.physics,
                    row_w,
                    menu_item_h,
                    [0.15, 0.15, 0.15, 1.0],
                    ui::ButtonOptions {
                        text: Some((*label).into()),
                        text_scale: menu_font_scale,
                        sdf_text: true,
                        click_priority: 3,
                        hover: true,
                        click_action: Some(click_fn),
                        ..Default::default()
                    },
                );
                ui.container_add_child(
                    &mut self.world,
                    menu_panel,
                    row,
                    UiAnchor::TopLeft,
                    UiAnchor::TopLeft,
                );
                rows.push((row, idx));
            }
            item_rows = rows;

            // Set initial menu position to avoid 1-frame glitch on first open.
            // The on_update closure repositions to actual viewport on every frame.
            {
                let init_vh: f32 = 720.0;
                let init_mx = btn_size;
                let init_my = init_vh - btn_size - menu_panel_gap - menu_h;
                if let Ok(mut tf) = self.world.get::<&mut Transform>(menu_panel) {
                    tf.position = glam::Vec3::new(init_mx, init_my, -1100.0);
                }
                let mut row_y = init_my + menu_padding;
                for (row_e, _) in &item_rows {
                    if let Ok(mut tf) = self.world.get::<&mut Transform>(*row_e) {
                        tf.position = glam::Vec3::new(init_mx + menu_padding, row_y, -1100.0);
                    }
                    // Set row label SDF text z-order too
                    if let Ok(node) = self.world.get::<&classic_core::components::UiNode>(*row_e) {
                        for child in &node.children {
                            if let Ok(mut tf) = self.world.get::<&mut Transform>(child.entity) {
                                tf.position.z = -1100.0;
                            }
                        }
                    }
                    row_y += menu_item_h + menu_gap;
                }
            }

            // Backdrop with click handler at lowest priority
            let bd;
            {
                let iso = is_open.clone();
                bd = ui.spawn_container(&mut self.world, 800.0, 600.0, [0.0, 0.0, 0.0, 0.01]);
                // Render on top of DEV/agent buttons but behind menu panel.
                if let Ok(mut tf) = self.world.get::<&mut Transform>(bd) {
                    tf.position.z = -1050.0;
                }
                let bp_pid = ui.add_collider_to_elem(&mut self.world, bd, &mut self.physics);
                self.physics.set_collider_consumes_click(bp_pid, true);
                self.physics.set_collider_click_priority(bp_pid, -1);
                self.physics.add_collider_handler(
                    bp_pid,
                    classic_core::collision::HandlerKind::Click,
                    move || {
                        iso.set(false);
                        true
                    },
                );
            }
            backdrop = bd;
            btn_arr = btn_array;
        }

        // Now safe to call set_enabled (ui borrow released)
        self.set_enabled(menu_panel, false);
        self.set_enabled(backdrop, false);
        self.menu_panel_e = Some(menu_panel);

        let items = item_rows.clone();
        let targets: Vec<String> = menu_targets.iter().map(|t| t.1.to_string()).collect();
        let iso2 = is_open.clone();
        let et2 = editor_tgt.clone();
        let ag2 = agent_sel.clone();
        let df2 = dbg_feet.clone();

        // Per-frame: position elements, sync Rc state → engine, toggle visibility
        self.on_update(move |engine| {
            // Sync shared state → engine (before ui borrow)
            engine.editor_target = et2.borrow().clone();
            engine.agent_selected = ag2.get();
            engine.debug_footprints = df2.get();
            let open = iso2.get();
            engine.panel_menu_open = open;
            engine.set_enabled(menu_panel, open);
            engine.set_enabled(backdrop, open);

            let Some(ref mut ui) = engine.ui else { return };
            let vw = ui.viewport_w;
            let vh = ui.viewport_h;

            // Position button array (agent above dev, centered on X)
            let arr_x = btn_size / 2.0;
            let arr_y = vh - btn_size * 0.5 - agent_size - btn_size - agent_pad;
            if let Ok(mut tf) = engine.world.get::<&mut Transform>(btn_arr) {
                tf.position = glam::Vec3::new(arr_x, arr_y, tf.position.z);
            }
            ui.layout_standalone(btn_arr, &mut engine.world);

            // Update agent button color based on selection state
            let ag_color: [f32; 4] =
                if ag2.get() { [0.1, 0.6, 0.1, 0.8] } else { [0.3, 0.3, 0.3, 0.6] };
            ui.set_button_base_color(agent_btn, ag_color);

            // Position menu panel
            let m_x = btn_size;
            let m_y = vh - btn_size - menu_panel_gap - menu_h;
            if let Ok(mut tf) = engine.world.get::<&mut Transform>(menu_panel) {
                tf.position = glam::Vec3::new(m_x, m_y, -1100.0);
            }

            // Position menu item rows
            let mut row_y = m_y + menu_padding;
            for (row_e, _) in &items {
                if let Ok(mut tf) = engine.world.get::<&mut Transform>(*row_e) {
                    tf.position = glam::Vec3::new(m_x + menu_padding, row_y, -1100.0);
                }
                // Set row label SDF text z-order too
                if let Ok(node) = engine.world.get::<&classic_core::components::UiNode>(*row_e) {
                    for child in &node.children {
                        if let Ok(mut tf) = engine.world.get::<&mut Transform>(child.entity) {
                            tf.position.z = -1100.0;
                        }
                    }
                }
                ui::UIManager::position_children_of(*row_e, &mut engine.world);
                row_y += menu_item_h + menu_gap;
            }

            // Backdrop
            if let Ok(mut tf) = engine.world.get::<&mut Transform>(backdrop) {
                tf.position = glam::Vec3::new(0.0, 0.0, tf.position.z);
            }
            if let Ok(mut node) =
                engine.world.get::<&mut classic_core::components::UiNode>(backdrop)
            {
                node.size.x = vw;
                node.size.y = vh;
            }

            // Active-tool highlighting on menu item rows
            for (row_e, idx) in &items {
                let target = &targets[*idx];
                let color: [f32; 4] = if target == "_footprints" {
                    if engine.debug_footprints {
                        [0.2, 0.35, 0.6, 1.0]
                    } else {
                        [0.15, 0.15, 0.15, 1.0]
                    }
                } else if engine.editor_target == *target {
                    [0.2, 0.35, 0.6, 1.0]
                } else {
                    [0.15, 0.15, 0.15, 1.0]
                };
                ui.set_button_base_color(*row_e, color);
            }
        });
    }

    /// Height editing widget: +/- buttons for height delta and scale multiplier,
    /// plus a set/blend mode toggle.
    #[allow(clippy::too_many_lines)]
    pub fn init_height_widget(&mut self) {
        use std::cell::Cell;
        use std::cell::RefCell;
        use std::rc::Rc;

        let btn_sz: f32 = 28.0;
        let label_w: f32 = 60.0;
        let gap: f32 = 4.0;
        let row_h: f32 = btn_sz;
        let widget_w: f32 = gap * 4.0 + btn_sz * 2.0 + label_w;
        let widget_h: f32 = row_h * 3.0 + gap * 4.0;
        let _border: f32 = 0.0;

        let h_val = Rc::new(Cell::new(0i32));
        let h_scale = Rc::new(Cell::new(1i32));
        let h_mode = Rc::new(RefCell::new(String::from("blend")));

        let Some(ref mut ui) = self.ui else { return };

        let container =
            ui.spawn_container(&mut self.world, widget_w, widget_h, [0.0, 0.0, 0.0, 0.4]);

        // Row 1: height value +/-
        let h_minus;
        {
            let hv = h_val.clone();
            h_minus = ui.spawn_button(
                &mut self.world,
                &mut self.physics,
                btn_sz,
                btn_sz,
                [0.6, 0.1, 0.1, 1.0],
                ui::ButtonOptions {
                    text: Some("-".into()),
                    text_scale: 0.5,
                    sdf_text: true,
                    hover: true,
                    click_action: Some(Box::new(move || {
                        hv.set(hv.get() - 1);
                        true
                    })),
                    ..Default::default()
                },
            );
        }
        let h_label = ui.spawn_sdf_text(
            &mut self.world,
            "0",
            1.0,
            200.0,
            [1.0, 1.0, 1.0, 1.0],
            TextJustify::Center,
        );
        let h_plus;
        {
            let hv = h_val.clone();
            h_plus = ui.spawn_button(
                &mut self.world,
                &mut self.physics,
                btn_sz,
                btn_sz,
                [0.1, 0.6, 0.1, 1.0],
                ui::ButtonOptions {
                    text: Some("+".into()),
                    text_scale: 0.5,
                    sdf_text: true,
                    hover: true,
                    click_action: Some(Box::new(move || {
                        hv.set(hv.get() + 1);
                        true
                    })),
                    ..Default::default()
                },
            );
        }

        // Row 2: scale multiplier s-/s+
        let s_minus;
        {
            let hs = h_scale.clone();
            s_minus = ui.spawn_button(
                &mut self.world,
                &mut self.physics,
                btn_sz,
                btn_sz,
                [0.1, 0.1, 0.6, 1.0],
                ui::ButtonOptions {
                    text: Some("s-".into()),
                    text_scale: 0.4,
                    sdf_text: true,
                    hover: true,
                    click_action: Some(Box::new(move || {
                        hs.set((hs.get() - 1).max(1));
                        true
                    })),
                    ..Default::default()
                },
            );
        }
        let s_label = ui.spawn_sdf_text(
            &mut self.world,
            "x1",
            0.9,
            200.0,
            [1.0, 1.0, 1.0, 1.0],
            TextJustify::Center,
        );
        let s_plus;
        {
            let hs = h_scale.clone();
            s_plus = ui.spawn_button(
                &mut self.world,
                &mut self.physics,
                btn_sz,
                btn_sz,
                [0.1, 0.1, 0.6, 1.0],
                ui::ButtonOptions {
                    text: Some("s+".into()),
                    text_scale: 0.4,
                    sdf_text: true,
                    hover: true,
                    click_action: Some(Box::new(move || {
                        hs.set(hs.get() + 1);
                        true
                    })),
                    ..Default::default()
                },
            );
        }

        // Row 3: set/blend mode toggle
        let mode_btn;
        {
            let hm = h_mode.clone();
            mode_btn = ui.spawn_button(
                &mut self.world,
                &mut self.physics,
                widget_w - gap * 2.0,
                row_h,
                [0.2, 0.2, 0.2, 1.0],
                ui::ButtonOptions {
                    text: Some("blend".into()),
                    text_scale: 0.35,
                    sdf_text: true,
                    hover: true,
                    click_action: Some(Box::new(move || {
                        let mut m = hm.borrow_mut();
                        *m = if *m == "set" { "blend".into() } else { "set".into() };
                        true
                    })),
                    ..Default::default()
                },
            );
        }

        // Wire children to container so set_enabled propagates.
        ui.container_add_child(
            &mut self.world,
            container,
            h_minus,
            UiAnchor::TopLeft,
            UiAnchor::TopLeft,
        );
        ui.container_add_child(
            &mut self.world,
            container,
            h_plus,
            UiAnchor::TopLeft,
            UiAnchor::TopLeft,
        );
        ui.container_add_child(
            &mut self.world,
            container,
            s_minus,
            UiAnchor::TopLeft,
            UiAnchor::TopLeft,
        );
        ui.container_add_child(
            &mut self.world,
            container,
            s_plus,
            UiAnchor::TopLeft,
            UiAnchor::TopLeft,
        );
        ui.container_add_child(
            &mut self.world,
            container,
            mode_btn,
            UiAnchor::TopLeft,
            UiAnchor::TopLeft,
        );
        ui.container_add_child(
            &mut self.world,
            container,
            h_label,
            UiAnchor::TopLeft,
            UiAnchor::TopLeft,
        );
        ui.container_add_child(
            &mut self.world,
            container,
            s_label,
            UiAnchor::TopLeft,
            UiAnchor::TopLeft,
        );

        self.height_widget_e = Some(container);
        self.set_enabled(container, false);

        let con_e = container;
        let h_min_e = h_minus;
        let h_pl_e = h_plus;
        let h_lb_e = h_label;
        let s_mi_e = s_minus;
        let s_pl_e = s_plus;
        let s_lb_e = s_label;
        let md_e = mode_btn;
        let hv2 = h_val.clone();
        let hs2 = h_scale.clone();
        let hm2 = h_mode.clone();

        self.on_update(move |engine| {
            let Some(ref _ui) = engine.ui else { return };
            let cw = _ui.viewport_w;
            let ch = _ui.viewport_h;
            let x0 = cw - _border - widget_w;
            let y0 = ch - _border - widget_h;
            let cx = gap;
            let cy1 = gap;
            let cy2 = row_h + gap * 2.0;
            let cy3 = row_h * 2.0 + gap * 3.0;

            // Sync Rc state → engine
            engine.editor_height = hv2.get();
            engine.height_scale_multiplier = hs2.get();
            engine.height_edit_mode = hm2.borrow().clone();

            // Apply height scale to tilemap when it changes
            let prev_hs = hs2.get();
            if let Some(_e) = engine.names.get("tilemap").copied() {
                if let Ok(mut tm) = engine.world.get::<&mut Tilemap>(_e) {
                    tm.height_scale = tm.tile_pixel_size[0] as f32 * prev_hs as f32;
                }
            }

            if let Ok(mut tf) = engine.world.get::<&mut Transform>(con_e) {
                tf.position = glam::Vec3::new(x0, y0, tf.position.z);
            }
            if let Ok(mut tf) = engine.world.get::<&mut Transform>(h_min_e) {
                tf.position = glam::Vec3::new(x0 + cx, y0 + cy1, tf.position.z);
            }
            ui::UIManager::position_children_of(h_min_e, &mut engine.world);
            if let Ok(mut tf) = engine.world.get::<&mut Transform>(h_lb_e) {
                tf.position = glam::Vec3::new(x0 + cx + btn_sz + gap, y0 + cy1, tf.position.z);
            }
            if let Ok(mut tf) = engine.world.get::<&mut Transform>(h_pl_e) {
                tf.position =
                    glam::Vec3::new(x0 + cx + btn_sz + gap + label_w, y0 + cy1, tf.position.z);
            }
            ui::UIManager::position_children_of(h_pl_e, &mut engine.world);
            if let Ok(mut tf) = engine.world.get::<&mut Transform>(s_mi_e) {
                tf.position = glam::Vec3::new(x0 + cx, y0 + cy2, tf.position.z);
            }
            ui::UIManager::position_children_of(s_mi_e, &mut engine.world);
            if let Ok(mut tf) = engine.world.get::<&mut Transform>(s_lb_e) {
                tf.position = glam::Vec3::new(x0 + cx + btn_sz + gap, y0 + cy2, tf.position.z);
            }
            if let Ok(mut tf) = engine.world.get::<&mut Transform>(s_pl_e) {
                tf.position =
                    glam::Vec3::new(x0 + cx + btn_sz + gap + label_w, y0 + cy2, tf.position.z);
            }
            ui::UIManager::position_children_of(s_pl_e, &mut engine.world);
            if let Ok(mut tf) = engine.world.get::<&mut Transform>(md_e) {
                tf.position = glam::Vec3::new(x0 + cx, y0 + cy3, tf.position.z);
            }
            ui::UIManager::position_children_of(md_e, &mut engine.world);

            // Update labels
            if let Ok(mut sdf) = engine.world.get::<&mut SdfTextRender>(h_lb_e) {
                sdf.text = engine.editor_height.to_string();
            }
            if let Ok(mut sdf) = engine.world.get::<&mut SdfTextRender>(s_lb_e) {
                sdf.text = format!("x{}", engine.height_scale_multiplier);
            }
            // Update mode button text (stored on the child entity, not the container)
            if let Ok(node) = engine.world.get::<&classic_core::components::UiNode>(md_e) {
                if let Some(child) = node.children.first() {
                    if let Ok(mut sdf) = engine.world.get::<&mut SdfTextRender>(child.entity) {
                        sdf.text = engine.height_edit_mode.clone();
                    }
                }
            }
        });
    }

    /// Light config widget: preset cycle + azimuth/elevation adjustment buttons.
    #[allow(clippy::too_many_lines)]
    pub fn init_light_widget(&mut self) {
        use std::cell::Cell;
        use std::cell::RefCell;
        use std::rc::Rc;

        let btn_sz: f32 = 32.0;
        let small_btn: f32 = 24.0;
        let label_w: f32 = 160.0;
        let dir_w: f32 = 160.0;
        let gap: f32 = 4.0;
        let _button_gap: f32 = 10.0;
        let row_h: f32 = btn_sz;
        let preset_row_w: f32 = gap * 4.0 + btn_sz * 2.0 + label_w;
        let adjust_row_w: f32 = gap * 4.0 + dir_w + small_btn * 2.0 + _button_gap * 2.0;
        let widget_w = preset_row_w.max(adjust_row_w);
        let widget_h: f32 = row_h * 3.0 + gap * 4.0;
        let _border: f32 = 0.0;

        const PRESET_ORDER: &[&str] = &["sunny", "cloudy", "dawn", "night"];
        const AZ_STEP: f32 = 15.0;
        const EL_STEP: f32 = 10.0;

        let preset = Rc::new(RefCell::new(String::from("sunny")));
        let last_applied_preset = Rc::new(RefCell::new(String::from("sunny")));
        let azimuth = Rc::new(Cell::new(45.0f32));
        let elevation = Rc::new(Cell::new(45.0f32));

        let Some(ref mut ui) = self.ui else { return };

        let container =
            ui.spawn_container(&mut self.world, widget_w, widget_h, [0.0, 0.0, 0.0, 0.4]);

        // Row 1: preset cycle << >>
        let prev_btn;
        {
            let pr = preset.clone();
            prev_btn = ui.spawn_button(
                &mut self.world,
                &mut self.physics,
                btn_sz,
                btn_sz,
                [0.3, 0.3, 0.6, 1.0],
                ui::ButtonOptions {
                    text: Some("<<".into()),
                    text_scale: 0.35,
                    sdf_text: true,
                    hover: true,
                    click_action: Some(Box::new(move || {
                        let cur = pr.borrow().clone();
                        let idx = PRESET_ORDER.iter().position(|&p| p == cur).unwrap_or(0);
                        let prev =
                            PRESET_ORDER[(idx + PRESET_ORDER.len() - 1) % PRESET_ORDER.len()];
                        *pr.borrow_mut() = prev.into();
                        true
                    })),
                    ..Default::default()
                },
            );
        }
        let preset_label = ui.spawn_sdf_text(
            &mut self.world,
            "Sunny Day",
            0.9,
            300.0,
            [1.0, 1.0, 1.0, 1.0],
            TextJustify::Center,
        );
        let next_btn;
        {
            let pr = preset.clone();
            next_btn = ui.spawn_button(
                &mut self.world,
                &mut self.physics,
                btn_sz,
                btn_sz,
                [0.3, 0.3, 0.6, 1.0],
                ui::ButtonOptions {
                    text: Some(">>".into()),
                    text_scale: 0.35,
                    sdf_text: true,
                    hover: true,
                    click_action: Some(Box::new(move || {
                        let cur = pr.borrow().clone();
                        let idx = PRESET_ORDER.iter().position(|&p| p == cur).unwrap_or(0);
                        let next = PRESET_ORDER[(idx + 1) % PRESET_ORDER.len()];
                        *pr.borrow_mut() = next.into();
                        true
                    })),
                    ..Default::default()
                },
            );
        }

        // Row 2: azimuth
        let az_label = ui.spawn_sdf_text(
            &mut self.world,
            "az: 45deg",
            0.9,
            200.0,
            [1.0, 1.0, 1.0, 1.0],
            TextJustify::Center,
        );
        let az_minus;
        {
            let az = azimuth.clone();
            let pr = preset.clone();
            az_minus = ui.spawn_button(
                &mut self.world,
                &mut self.physics,
                small_btn,
                small_btn,
                [0.6, 0.3, 0.1, 1.0],
                ui::ButtonOptions {
                    text: Some("-".into()),
                    text_scale: 0.35,
                    sdf_text: true,
                    hover: true,
                    click_action: Some(Box::new(move || {
                        az.set((az.get() - AZ_STEP + 360.0) % 360.0);
                        *pr.borrow_mut() = "custom".into();
                        true
                    })),
                    ..Default::default()
                },
            );
        }
        let az_plus;
        {
            let az = azimuth.clone();
            let pr = preset.clone();
            az_plus = ui.spawn_button(
                &mut self.world,
                &mut self.physics,
                small_btn,
                small_btn,
                [0.1, 0.6, 0.3, 1.0],
                ui::ButtonOptions {
                    text: Some("+".into()),
                    text_scale: 0.35,
                    sdf_text: true,
                    hover: true,
                    click_action: Some(Box::new(move || {
                        az.set((az.get() + AZ_STEP) % 360.0);
                        *pr.borrow_mut() = "custom".into();
                        true
                    })),
                    ..Default::default()
                },
            );
        }

        // Row 3: elevation
        let el_label = ui.spawn_sdf_text(
            &mut self.world,
            "el: 45deg",
            0.9,
            200.0,
            [1.0, 1.0, 1.0, 1.0],
            TextJustify::Center,
        );
        let el_minus;
        {
            let el = elevation.clone();
            let pr = preset.clone();
            el_minus = ui.spawn_button(
                &mut self.world,
                &mut self.physics,
                small_btn,
                small_btn,
                [0.6, 0.3, 0.1, 1.0],
                ui::ButtonOptions {
                    text: Some("-".into()),
                    text_scale: 0.35,
                    sdf_text: true,
                    hover: true,
                    click_action: Some(Box::new(move || {
                        el.set((el.get() - EL_STEP).max(0.0));
                        *pr.borrow_mut() = "custom".into();
                        true
                    })),
                    ..Default::default()
                },
            );
        }
        let el_plus;
        {
            let el = elevation.clone();
            let pr = preset.clone();
            el_plus = ui.spawn_button(
                &mut self.world,
                &mut self.physics,
                small_btn,
                small_btn,
                [0.1, 0.6, 0.3, 1.0],
                ui::ButtonOptions {
                    text: Some("+".into()),
                    text_scale: 0.35,
                    sdf_text: true,
                    hover: true,
                    click_action: Some(Box::new(move || {
                        el.set((el.get() + EL_STEP).min(90.0));
                        *pr.borrow_mut() = "custom".into();
                        true
                    })),
                    ..Default::default()
                },
            );
        }

        // Wire children to container so set_enabled propagates.
        ui.container_add_child(
            &mut self.world,
            container,
            prev_btn,
            UiAnchor::TopLeft,
            UiAnchor::TopLeft,
        );
        ui.container_add_child(
            &mut self.world,
            container,
            next_btn,
            UiAnchor::TopLeft,
            UiAnchor::TopLeft,
        );
        ui.container_add_child(
            &mut self.world,
            container,
            az_minus,
            UiAnchor::TopLeft,
            UiAnchor::TopLeft,
        );
        ui.container_add_child(
            &mut self.world,
            container,
            az_plus,
            UiAnchor::TopLeft,
            UiAnchor::TopLeft,
        );
        ui.container_add_child(
            &mut self.world,
            container,
            el_minus,
            UiAnchor::TopLeft,
            UiAnchor::TopLeft,
        );
        ui.container_add_child(
            &mut self.world,
            container,
            el_plus,
            UiAnchor::TopLeft,
            UiAnchor::TopLeft,
        );
        ui.container_add_child(
            &mut self.world,
            container,
            preset_label,
            UiAnchor::TopLeft,
            UiAnchor::TopLeft,
        );
        ui.container_add_child(
            &mut self.world,
            container,
            az_label,
            UiAnchor::TopLeft,
            UiAnchor::TopLeft,
        );
        ui.container_add_child(
            &mut self.world,
            container,
            el_label,
            UiAnchor::TopLeft,
            UiAnchor::TopLeft,
        );

        self.light_widget_e = Some(container);
        self.set_enabled(container, false);

        let con_e = container;
        let pv_e = prev_btn;
        let nx_e = next_btn;
        let pl_e = preset_label;
        let az_l = az_label;
        let az_m = az_minus;
        let az_p = az_plus;
        let el_l = el_label;
        let el_m = el_minus;
        let el_p = el_plus;
        let pr2 = preset.clone();
        let last_preset_clone = last_applied_preset.clone();
        let az2 = azimuth.clone();
        let el2 = elevation.clone();

        self.on_update(move |engine| {
            let Some(ref _ui) = engine.ui else { return };
            let cw = _ui.viewport_w;
            let ch = _ui.viewport_h;
            let x0 = cw - _border - widget_w;
            let y0 = ch - _border - widget_h;
            let cx = gap;
            let cy1 = gap;
            let cy2 = row_h + gap * 2.0;
            let cy3 = row_h * 2.0 + gap * 3.0;

            // Sync Rc state → engine (only when preset actually changes)
            let cur_preset = pr2.borrow().clone();
            let mut last = last_preset_clone.borrow_mut();
            if cur_preset != *last {
                if cur_preset == "custom" {
                    engine.light_azimuth = az2.get();
                    engine.light_elevation = el2.get();
                    engine.update_light_direction();
                    engine.light_preset = "custom".into();
                } else {
                    engine.apply_light_preset(&cur_preset);
                }
                *last = cur_preset;
            } else if cur_preset == "custom" {
                let new_az = az2.get();
                let new_el = el2.get();
                if (new_az - engine.light_azimuth).abs() > 0.1
                    || (new_el - engine.light_elevation).abs() > 0.1
                {
                    engine.light_azimuth = new_az;
                    engine.light_elevation = new_el;
                    engine.update_light_direction();
                }
            }

            if let Ok(mut tf) = engine.world.get::<&mut Transform>(con_e) {
                tf.position = glam::Vec3::new(x0, y0, tf.position.z);
            }
            if let Ok(mut tf) = engine.world.get::<&mut Transform>(pv_e) {
                tf.position = glam::Vec3::new(x0 + cx, y0 + cy1, tf.position.z);
            }
            ui::UIManager::position_children_of(pv_e, &mut engine.world);
            if let Ok(mut tf) = engine.world.get::<&mut Transform>(pl_e) {
                tf.position = glam::Vec3::new(x0 + cx + btn_sz + gap, y0 + cy1, tf.position.z);
            }
            if let Ok(mut tf) = engine.world.get::<&mut Transform>(nx_e) {
                tf.position =
                    glam::Vec3::new(x0 + cx + btn_sz + gap + label_w, y0 + cy1, tf.position.z);
            }
            ui::UIManager::position_children_of(nx_e, &mut engine.world);
            if let Ok(mut tf) = engine.world.get::<&mut Transform>(az_l) {
                tf.position = glam::Vec3::new(x0 + cx, y0 + cy2, tf.position.z);
            }
            if let Ok(mut tf) = engine.world.get::<&mut Transform>(az_m) {
                tf.position = glam::Vec3::new(
                    x0 + widget_w - gap - small_btn * 2.0 - _button_gap,
                    y0 + cy2,
                    tf.position.z,
                );
            }
            ui::UIManager::position_children_of(az_m, &mut engine.world);
            if let Ok(mut tf) = engine.world.get::<&mut Transform>(az_p) {
                tf.position =
                    glam::Vec3::new(x0 + widget_w - gap - small_btn, y0 + cy2, tf.position.z);
            }
            ui::UIManager::position_children_of(az_p, &mut engine.world);
            if let Ok(mut tf) = engine.world.get::<&mut Transform>(el_l) {
                tf.position = glam::Vec3::new(x0 + cx, y0 + cy3, tf.position.z);
            }
            if let Ok(mut tf) = engine.world.get::<&mut Transform>(el_m) {
                tf.position = glam::Vec3::new(
                    x0 + widget_w - gap - small_btn * 2.0 - _button_gap,
                    y0 + cy3,
                    tf.position.z,
                );
            }
            ui::UIManager::position_children_of(el_m, &mut engine.world);
            if let Ok(mut tf) = engine.world.get::<&mut Transform>(el_p) {
                tf.position =
                    glam::Vec3::new(x0 + widget_w - gap - small_btn, y0 + cy3, tf.position.z);
            }
            ui::UIManager::position_children_of(el_p, &mut engine.world);

            if let Ok(mut sdf) = engine.world.get::<&mut SdfTextRender>(pl_e) {
                let name = match engine.light_preset.as_str() {
                    "sunny" => "Sunny Day",
                    "cloudy" => "Cloudy",
                    "dawn" => "Dawn / Dusk",
                    "night" => "Night",
                    _ => "Custom",
                };
                sdf.text = name.into();
            }
            if let Ok(mut sdf) = engine.world.get::<&mut SdfTextRender>(az_l) {
                sdf.text = format!("az: {}deg", engine.light_azimuth.round() as i32);
            }
            if let Ok(mut sdf) = engine.world.get::<&mut SdfTextRender>(el_l) {
                sdf.text = format!("el: {}deg", engine.light_elevation.round() as i32);
            }
        });
    }

    /// Toggle visibility of tool panels based on `editor_target`.
    pub fn init_editor_mode_control(&mut self) {
        self.on_update(|engine| {
            let target = engine.editor_target.clone();
            if let Some(e) = engine.tile_palette_e {
                engine.set_enabled(e, target == "tilemap");
            }
            if let Some(e) = engine.nav_palette_e {
                engine.set_enabled(e, target == "navMesh");
            }
            if let Some(e) = engine.height_widget_e {
                engine.set_enabled(e, target == "height");
            }
            if let Some(e) = engine.light_widget_e {
                engine.set_enabled(e, target == "light");
            }
            if let Some(e) = engine.text_showcase_e {
                engine.set_enabled(e, target == "textDemo");
            }
            if let Some(&e) = engine.names.get("tilemapNavigation") {
                engine.set_enabled(e, target == "navMesh");
            }
        });
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

    /// Text showcase panel: demonstrates SDF text features with
    /// scrollable, scissor-clipped container.
    pub fn init_text_showcase(&mut self) {
        let Some(ref mut ui) = self.ui else { return };
        let border: f32 = 10.0;
        let panel_w: f32 = 520.0;
        let panel_h: f32 = 440.0;
        let init_px = ui.viewport_w - panel_w - border;
        let init_py = ui.viewport_h - panel_h - border;

        let container =
            ui.spawn_container(&mut self.world, panel_w, panel_h, [0.05, 0.05, 0.08, 0.92]);
        if let Ok(mut tf) = self.world.get::<&mut Transform>(container) {
            tf.position = glam::Vec3::new(init_px, init_py, tf.position.z);
        }
        if let Ok(mut node) = self.world.get::<&mut UiNode>(container) {
            node.clip_children = true;
        }

        let text_scale: f32 = 0.7;
        let line_h: f32 = 28.0;
        let line_gap: f32 = 4.0;
        let section_gap: f32 = 16.0;
        let indent: f32 = 6.0;
        let mut cy: f32 = 6.0;

        #[rustfmt::skip]
        let lines: Vec<(&str, f32, TextJustify, [f32; 4])> = vec![
            // --- Scale ramp ---
            ("SDF Font Rendering", 1.3, TextJustify::Left, [1.0, 1.0, 1.0, 1.0]),
            ("Tiny text (0.3)", 0.3, TextJustify::Left, [0.7, 0.7, 0.7, 1.0]),
            ("Small text (0.5)", 0.5, TextJustify::Left, [0.7, 0.7, 0.7, 1.0]),
            ("Medium text (1.0)", 1.0, TextJustify::Center, [0.8, 0.8, 0.9, 1.0]),
            ("Large text (1.8)", 1.8, TextJustify::Left, [0.7, 0.9, 1.0, 1.0]),
            ("Extra large (2.5)", 2.5, TextJustify::Left, [1.0, 0.6, 0.2, 1.0]),
            ("Maximum (3.5)", 3.5, TextJustify::Left, [0.2, 1.0, 0.3, 1.0]),

            // --- Weight & Gamma ---
            ("", 0.0, TextJustify::Left, [0.0; 4]),
            ("Weight 0.0 — thinner strokes", text_scale, TextJustify::Left, [0.8, 0.8, 0.8, 1.0]),
            ("Weight 0.15 — medium", text_scale, TextJustify::Left, [0.8, 0.8, 0.8, 1.0]),
            ("Weight 0.3 — bolder strokes", text_scale, TextJustify::Left, [0.8, 0.8, 0.8, 1.0]),
            ("Gamma 0.5 — sharper edges", text_scale, TextJustify::Left, [0.8, 0.8, 0.9, 1.0]),
            ("Gamma 2.5 — softer edges", text_scale, TextJustify::Left, [0.8, 0.8, 0.9, 1.0]),

            // --- Justification with wrapping ---
            ("", 0.0, TextJustify::Left, [0.0; 4]),
            ("The quick brown fox\njumps over the lazy dog\ntwice to demonstrate text\nwrapping and justification.", text_scale, TextJustify::Left, [0.6, 1.0, 0.6, 1.0]),
            ("The quick brown fox\njumps over the lazy dog\ntwice to demonstrate text\nwrapping and justification.", text_scale, TextJustify::Center, [0.6, 1.0, 0.6, 1.0]),
            ("The quick brown fox\njumps over the lazy dog\ntwice to demonstrate text\nwrapping and justification.", text_scale, TextJustify::Right, [0.6, 1.0, 0.6, 1.0]),

            // --- Colors ---
            ("", 0.0, TextJustify::Left, [0.0; 4]),
            ("Red text", text_scale, TextJustify::Left, [1.0, 0.2, 0.2, 1.0]),
            ("Green text", text_scale, TextJustify::Left, [0.2, 1.0, 0.2, 1.0]),
            ("Blue text", text_scale, TextJustify::Left, [0.3, 0.5, 1.0, 1.0]),
            ("Yellow text", text_scale, TextJustify::Left, [1.0, 0.9, 0.1, 1.0]),
            ("Cyan text", text_scale, TextJustify::Left, [0.2, 0.9, 1.0, 1.0]),
            ("Magenta text", text_scale, TextJustify::Left, [1.0, 0.3, 0.8, 1.0]),

            // --- Outline & Glow & Shadow ---
            ("", 0.0, TextJustify::Left, [0.0; 4]),
            ("Thin outline (0.08)", 1.2, TextJustify::Left, [0.9, 0.5, 0.2, 1.0]),
            ("Thick outline (0.2)", 1.2, TextJustify::Left, [0.9, 0.5, 0.2, 1.0]),
            ("Blue glow", 1.4, TextJustify::Left, [0.2, 0.6, 1.0, 1.0]),
            ("Orange glow", 1.4, TextJustify::Left, [1.0, 0.4, 0.1, 1.0]),
            ("Drop shadow", 1.0, TextJustify::Left, [0.9, 0.9, 0.9, 1.0]),

            // --- Unicode & Symbols ---
            ("", 0.0, TextJustify::Left, [0.0; 4]),
            ("Chess: \u{2654}\u{2655}\u{2656}\u{2657}\u{2658}\u{2659}", text_scale, TextJustify::Left, [1.0, 0.8, 0.4, 1.0]),
            ("Suits: \u{2660}\u{2663}\u{2665}\u{2666}", text_scale, TextJustify::Center, [0.8, 0.8, 0.8, 1.0]),
            ("Arrows: \u{2190}\u{2191}\u{2192}\u{2193}\u{2194}\u{21C4}\u{21BA}", text_scale, TextJustify::Left, [0.6, 0.7, 1.0, 1.0]),
            ("Shapes: \u{25A0}\u{25B2}\u{25C6}\u{25CF}\u{2605}\u{2713}\u{2717}", text_scale, TextJustify::Left, [0.7, 0.9, 0.6, 1.0]),
            ("Greek: \u{0391}\u{0392}\u{0393}\u{0394}\u{03A3}\u{03A9}\u{03B1}\u{03B2}\u{03B3}\u{03C0}", text_scale, TextJustify::Left, [0.7, 0.7, 1.0, 1.0]),
            ("Japanese: \u{65E5}\u{672C}\u{8A9E} \u{6F22}\u{5B57} \u{30AB}\u{30BF}\u{30AB}\u{30CA}", text_scale, TextJustify::Left, [0.8, 0.6, 0.9, 1.0]),
            ("Math: \u{2211} x\u{00B2}  \u{222B}\u{221E}  \u{221A}(-1)  \u{2200}\u{2203}  \u{2260}\u{2264}\u{2265}", text_scale, TextJustify::Left, [0.7, 0.7, 1.0, 1.0]),

            // --- Edge cases ---
            ("", 0.0, TextJustify::Left, [0.0; 4]),
            ("min", 0.4, TextJustify::Left, [0.6, 0.6, 0.6, 1.0]),
            ("Very very very very very very long single line to test overflow behavior", text_scale, TextJustify::Left, [0.7, 0.5, 0.5, 1.0]),
            ("", 0.0, TextJustify::Left, [0.0; 4]),
            ("Scroll with mouse wheel ...", 0.5, TextJustify::Right, [0.4, 0.4, 0.4, 1.0]),
        ];

        let mut sdf_entities: Vec<(hecs::Entity, f32, f32, TextJustify)> = Vec::new();
        for (text, font_scale, justify, color) in &lines {
            if text.is_empty() {
                cy += section_gap;
                continue;
            }
            let e = ui.spawn_sdf_text(
                &mut self.world,
                text,
                *font_scale,
                panel_w - indent * 2.0,
                *color,
                *justify,
            );
            // Add as container child so set_enabled cascades
            ui.container_add_child(
                &mut self.world,
                container,
                e,
                UiAnchor::TopLeft,
                UiAnchor::TopLeft,
            );
            // Apply visual effects based on text content
            if let Ok(mut sdf) = self.world.get::<&mut SdfTextRender>(e) {
                match *text {
                    "Weight 0.0 — thinner strokes" => sdf.weight = 0.0,
                    "Weight 0.15 — medium" => sdf.weight = 0.15,
                    "Weight 0.3 — bolder strokes" => sdf.weight = 0.3,
                    "Gamma 0.5 — sharper edges" => sdf.gamma = 0.5,
                    "Gamma 2.5 — softer edges" => sdf.gamma = 2.5,
                    "Thin outline (0.08)" => {
                        sdf.outline_width = 0.08;
                        sdf.outline_color = [0.1, 0.08, 0.0, 1.0];
                    }
                    "Thick outline (0.2)" => {
                        sdf.outline_width = 0.2;
                        sdf.outline_color = [0.1, 0.08, 0.0, 1.0];
                    }
                    "Blue glow" => {
                        sdf.outline_width = 0.25;
                        sdf.outline_color = [0.0, 0.3, 0.8, 1.0];
                    }
                    "Orange glow" => {
                        sdf.outline_width = 0.25;
                        sdf.outline_color = [0.8, 0.3, 0.0, 1.0];
                    }
                    "Drop shadow" => {
                        sdf.shadow_offset = [3.0, 3.0];
                        sdf.shadow_color = [0.0, 0.0, 0.0, 0.6];
                        sdf.shadow_blur = 0.05;
                    }
                    _ => {}
                }
            }
            sdf_entities.push((e, *font_scale, cy, *justify));
            let line_count = text.matches('\n').count() as f32 + 1.0;
            cy += line_count * font_scale.max(0.5) * line_h + line_gap * line_count;
        }
        let content_h = cy + 6.0;
        self.text_demo_content_h = content_h;

        // Scrollbar thumb
        let thumb_w: f32 = 6.0;
        let thumb_e = ui.spawn_container(&mut self.world, thumb_w, 30.0, [0.4, 0.4, 0.4, 0.8]);

        let ce = container;
        let thumb = thumb_e;

        self.on_update(move |engine| {
            let Some(ref _ui) = engine.ui else { return };
            let vw = _ui.viewport_w;
            let vh = _ui.viewport_h;
            let px2 = vw - panel_w - border;
            let py2 = vh - panel_h - border;

            // Update container position
            if let Ok(mut tf) = engine.world.get::<&mut Transform>(ce) {
                tf.position = glam::Vec3::new(px2, py2, tf.position.z);
            }
            if let Ok(mut node) = engine.world.get::<&mut UiNode>(ce) {
                node.size = Vec2::new(panel_w, panel_h);
            }

            // Scroll is applied by frame() before on_update — read from container.
            let sy = engine.world.get::<&UiNode>(ce).map(|n| n.scroll_y).unwrap_or(0.0);
            let max_scroll = (content_h - panel_h).max(0.0);

            // Reposition SDF text children with scroll offset and set clip rect.
            // Position X by justify using pre-measured UiNode.size.x. Since these
            // are UI children (parent.is_some), the SDF renderer uses x_off=0.
            let clip = Vec4::new(px2, py2, panel_w, panel_h);
            for &(sdf_e, _font_scale, base_y, justify) in &sdf_entities {
                let text_w = engine.world.get::<&UiNode>(sdf_e).map(|n| n.size.x).unwrap_or(0.0);
                let pos_x = match justify {
                    TextJustify::Left => px2 + indent,
                    TextJustify::Center => px2 + panel_w / 2.0 - text_w / 2.0,
                    TextJustify::Right => px2 + panel_w - indent - text_w,
                };
                if let Ok(mut cn) = engine.world.get::<&mut UiNode>(sdf_e) {
                    cn.clip_rect = clip;
                }
                if let Ok(mut tf) = engine.world.get::<&mut Transform>(sdf_e) {
                    tf.position.x = pos_x;
                    tf.position.y = py2 + base_y - sy;
                }
            }

            // Scrollbar thumb
            let thumb_h =
                if max_scroll > 0.0 { (panel_h / content_h * panel_h).max(20.0) } else { panel_h };
            let thumb_y_off =
                if max_scroll > 0.0 { (sy / max_scroll) * (panel_h - thumb_h - 4.0) } else { 0.0 };
            if let Ok(mut tf) = engine.world.get::<&mut Transform>(thumb) {
                tf.position.x = px2 + panel_w - thumb_w - 2.0;
                tf.position.y = py2 + 2.0 + thumb_y_off;
            }
            if let Ok(mut tn) = engine.world.get::<&mut UiNode>(thumb) {
                tn.size = Vec2::new(thumb_w, thumb_h);
            }
        });

        self.text_showcase_e = Some(container);
        self.set_enabled(container, false);
    }

    /// Iso coord overlay — cardinal compass rose + XYZ axes + live iso coords.
    /// Always visible, positioned top-left below the top bar, outside the UI tree.
    pub fn init_iso_coord_overlay(&mut self) {
        log::info!("iso_debug: creating compass overlay (14 labels + GL lines)");
        let z_layer = -1500.0_f32;

        // Helper to spawn a standalone SDF text entity.
        let mut spawn = |text: &str, scale: f32, color: [f32; 4]| -> hecs::Entity {
            self.world.spawn((
                Transform::new(Vec3::new(0.0, 0.0, z_layer), Vec3::new(scale, scale, 1.0)),
                SdfTextRender {
                    atlas_name: "dejavusans".into(),
                    color,
                    text: text.to_string(),
                    ignore_cam: true,
                    justify: TextJustify::Left,
                    weight: 0.0,
                    gamma: 1.0,
                    bgcolor: [0.0; 4],
                    outline_color: [0.0; 4],
                    outline_width: 0.0,
                    shadow_offset: [1.0, 1.0],
                    shadow_color: [0.0, 0.0, 0.0, 0.5],
                    shadow_blur: 0.0,
                },
                DebugName(format!("iso_debug_{}", text.replace([':', ' '], ""))),
            ))
        };

        // Cardinal direction labels
        let n_e = spawn("N", 1.2, [1.0, 1.0, 0.8, 1.0]);
        let e_e = spawn("E", 1.2, [1.0, 1.0, 0.8, 1.0]);
        let s_e = spawn("S", 1.2, [1.0, 1.0, 0.8, 1.0]);
        let w_e = spawn("W", 1.2, [1.0, 1.0, 0.8, 1.0]);

        // Intercardinal labels
        let ne_e = spawn("NE", 0.8, [1.0, 1.0, 0.8, 0.5]);
        let se_e = spawn("SE", 0.8, [1.0, 1.0, 0.8, 0.5]);
        let sw_e = spawn("SW", 0.8, [1.0, 1.0, 0.8, 0.5]);
        let nw_e = spawn("NW", 0.8, [1.0, 1.0, 0.8, 0.5]);

        // Axis labels
        let ax_e = spawn("X", 1.2, [1.0, 0.2, 0.2, 1.0]);
        let ay_e = spawn("Y", 1.2, [0.2, 1.0, 0.2, 1.0]);
        let az_e = spawn("Z", 1.2, [0.2, 0.2, 1.0, 1.0]);

        // Dynamic coordinate labels
        let cx_e = spawn("X: 0.0", 1.0, [1.0, 0.3, 0.3, 1.0]);
        let cy_e = spawn("Y: 0.0", 1.0, [0.3, 1.0, 0.3, 1.0]);
        let cz_e = spawn("Z: 0", 1.0, [0.4, 0.4, 1.0, 1.0]);

        self.iso_coord_x_e = Some(cx_e);
        self.iso_coord_y_e = Some(cy_e);
        self.iso_coord_z_e = Some(cz_e);

        // Build combined GL line buffer for compass rose.
        // screen_iso(dtx, dty, s) = (s*(dtx+dty), s*(dty-dtx)/2)
        let r: f32 = 30.0;
        let al: f32 = 35.0;
        let si =
            |dtx: f32, dty: f32, s: f32| -> (f32, f32) { (s * (dtx + dty), s * (dty - dtx) / 2.0) };

        let mut verts: Vec<f32> = Vec::with_capacity(54);
        // Grid 1 (NE/SW axis → horizontal through centre)
        verts.extend_from_slice(&[-2.0 * r, 0.0, 0.0, 2.0 * r, 0.0, 0.0]);
        // Grid 2 (NW/SE axis → vertical through centre)
        verts.extend_from_slice(&[0.0, -r, 0.0, 0.0, r, 0.0]);

        // Cardinal spokes (centre → direction)
        let (nx, ny) = si(0.0, -1.0, r);
        verts.extend_from_slice(&[0.0, 0.0, 0.0, nx, ny, 0.0]);
        let (ex, ey) = si(1.0, 0.0, r);
        verts.extend_from_slice(&[0.0, 0.0, 0.0, ex, ey, 0.0]);
        let (sx, sy) = si(0.0, 1.0, r);
        verts.extend_from_slice(&[0.0, 0.0, 0.0, sx, sy, 0.0]);
        let (wx, wy) = si(-1.0, 0.0, r);
        verts.extend_from_slice(&[0.0, 0.0, 0.0, wx, wy, 0.0]);

        // XYZ axes from axis origin
        let (axx, axy) = si(1.0, 0.0, al);
        verts.extend_from_slice(&[0.0, 0.0, 0.0, axx, axy, 0.0]);
        let (ayx, ayy) = si(0.0, 1.0, al);
        verts.extend_from_slice(&[0.0, 0.0, 0.0, ayx, ayy, 0.0]);
        verts.extend_from_slice(&[0.0, 0.0, 0.0, 0.0, -al, 0.0]);

        if let Some(ref gfx) = self.gfx {
            self.iso_compass_buf =
                Some(GlBuffer::from_slice(&gfx.gl, glow::ARRAY_BUFFER, &verts, glow::STATIC_DRAW));
        }

        // on_update: reposition labels + update coord text every frame
        let tilemap_name = "tilemap".to_string();
        let coord_x = cx_e;
        let coord_y = cy_e;
        let coord_z = cz_e;

        self.on_update(move |engine| {
            let Some(&tm_entity) = engine.names.get(&tilemap_name) else { return };
            let Ok(tm) = engine.world.get::<&Tilemap>(tm_entity) else { return };

            let mx = tm.mouse_iso_pos.x;
            let my = tm.mouse_iso_pos.y;
            let h = bilinear_height(&tm.height_data, tm.size_x, tm.size_y, mx, my);

            // Update dynamic coord text
            if let Ok(mut sdf) = engine.world.get::<&mut SdfTextRender>(coord_x) {
                sdf.text = format!("X: {:.1}", mx);
            }
            if let Ok(mut sdf) = engine.world.get::<&mut SdfTextRender>(coord_y) {
                sdf.text = format!("Y: {:.1}", my);
            }
            if let Ok(mut sdf) = engine.world.get::<&mut SdfTextRender>(coord_z) {
                sdf.text = format!("Z: {:.0}", h * tm.height_scale);
            }

            // Layout constants
            let cx: f32 = 100.0;
            let cy: f32 = 155.0;
            let ax_ox: f32 = 220.0;
            let ax_oy: f32 = 155.0;
            let coord_x_pos: f32 = 340.0;
            let coord_y_base: f32 = 130.0;
            let gap: f32 = 22.0;
            let r: f32 = 30.0;
            let al: f32 = 35.0;

            let set_pos = |e: hecs::Entity, x: f32, y: f32| {
                if let Ok(mut tf) = engine.world.get::<&mut Transform>(e) {
                    tf.position = Vec3::new(x, y, z_layer);
                }
            };

            // --- Cardinal labels (scale 1.2, ~14w × 20h) ---
            // Spoke tips in screen space (from iso vectors via si()).
            // N tip: (cx-30, cy-15) → label above, centered:  y = tip_y - 20 - 6
            set_pos(n_e, cx - 30.0 - 14.0, cy - 15.0 - 26.0);
            // E tip: (cx+30, cy-15) → label right, vert-centered: x = tip_x + 6
            set_pos(e_e, cx + 30.0 + 6.0, cy - 15.0 - 10.0);
            // S tip: (cx+30, cy+15) → label below, centered:  y = tip_y + 6
            set_pos(s_e, cx + 30.0 + 7.0, cy + 15.0 + 2.0);
            // W tip: (cx-30, cy+15) → label left, vert-centered: x = tip_x - 14 - 6
            set_pos(w_e, cx - 30.0 - 35.0, cy + 15.0 - 10.0);

            // --- Intercardinal labels (scale 0.8, ~12w × 14h) ---
            // NE: straight UP from center, line endpoint at (cx, cy-30)
            set_pos(ne_e, cx - 6.0, cy - 30.0 - 14.0 - 6.0);
            // SE: straight RIGHT, line endpoint at (cx+60, cy)
            set_pos(se_e, cx + 60.0 + 6.0, cy - 7.0);
            // SW: straight DOWN, line endpoint at (cx, cy+30)
            set_pos(sw_e, cx - 6.0, cy + 30.0 + 6.0);
            // NW: straight LEFT, line endpoint at (cx-60, cy)
            set_pos(nw_e, cx - 60.0 - 6.0 - 22.0, cy - 7.0);

            // --- Axis labels (scale 1.2, ~14w × 20h) ---
            // X arm tip: si(1,0,35) = (35, -17.5)
            set_pos(ax_e, ax_ox + 35.0 + 8.0, ax_oy - 17.5 - 10.0);
            // Y arm tip: si(0,1,35) = (35, 17.5)
            set_pos(ay_e, ax_ox + 35.0 + 8.0, ax_oy + 17.5 - 10.0);
            // Z arm tip: (0, -35) — straight up
            set_pos(az_e, ax_ox - 7.0, ax_oy - 35.0 - 20.0 - 10.0);

            // Coordinate labels — stacked vertically at right
            set_pos(coord_x, coord_x_pos, coord_y_base);
            set_pos(coord_y, coord_x_pos, coord_y_base + gap);
            set_pos(coord_z, coord_x_pos, coord_y_base + gap * 2.0);

            classic_core::cl_first!(
                Chan::Iso,
                5,
                log::Level::Info,
                "iso overlay: compass=({},{}), coords=({},{})  r={} al={}",
                cx,
                cy,
                coord_x_pos,
                coord_y_base,
                r,
                al,
            );
        });
    }

    /// Tilemap editor: click to select a tile when height or tile editor is active.
    /// Records the selected tile position for height/tile painting.
    /// Tilemap editor selection visual + painting is handled by the drag
    /// pipeline in `frame()` (selection_mode tracking + apply_editor_selection).
    /// This method is kept as a no-op placeholder for the init call site.

    /// Tile palette: shows the tileset texture with click-to-select and a selector overlay.
    pub fn init_tile_palette(&mut self) {
        let Some(&tm_entity) = self.names.get("tilemap") else { return };
        let (tile_px, tile_py, max_tile, tiles_per_row) = {
            let Ok(tm) = self.world.get::<&Tilemap>(tm_entity) else {
                return;
            };
            (tm.tile_pixel_size[0], tm.tile_pixel_size[1], tm.max_tile, tm.tiles_per_row)
        };
        let Some(ref mut ui) = self.ui else { return };

        let ts_pixel = [tile_px * tiles_per_row, tile_py * tiles_per_row];
        let palette_w = ts_pixel[0] as f32;
        let palette_h = ts_pixel[1] as f32;
        let t_size = [tile_px as f32, tile_py as f32];

        let container =
            ui.spawn_container(&mut self.world, palette_w, palette_h, [0.0, 0.0, 0.0, 0.2]);
        let sprite =
            ui.spawn_sprite(&mut self.world, "tileSet", palette_w, palette_h, 0.0, [1.0, 1.0]);
        ui.container_add_child(
            &mut self.world,
            container,
            sprite,
            UiAnchor::TopLeft,
            UiAnchor::TopLeft,
        );

        let selector =
            ui.spawn_container(&mut self.world, t_size[0], t_size[1], [1.0, 1.0, 1.0, 0.3]);
        ui.container_add_child(
            &mut self.world,
            container,
            selector,
            UiAnchor::TopLeft,
            UiAnchor::TopLeft,
        );

        let pid = ui.add_collider_to_elem(&mut self.world, container, &mut self.physics);
        self.physics.set_collider_consumes_click(pid, true);
        // Dummy click handler so consumes_click triggers consumed_click in perform_calls
        self.physics
            .add_collider_handler(pid, classic_core::collision::HandlerKind::Click, || true);

        let cp_e = container;
        let sel_e = selector;
        let local_x = std::rc::Rc::new(std::cell::Cell::new(0u32));
        let local_y = std::rc::Rc::new(std::cell::Cell::new(0u32));
        let lx2 = local_x.clone();
        let ly2 = local_y.clone();

        self.on_update(move |engine| {
            let Some(ref _ui) = engine.ui else { return };
            if engine.editor_target != "tilemap" {
                return;
            }
            let vw = _ui.viewport_w;
            let vh = _ui.viewport_h;
            let border: f32 = 10.0;
            let px = vw - palette_w - border;
            let py = vh - palette_h - border;
            if let Ok(mut tf) = engine.world.get::<&mut Transform>(cp_e) {
                tf.position = glam::Vec3::new(px, py, tf.position.z);
            }
            ui::UIManager::position_children_of(cp_e, &mut engine.world);

            // Select tile on click (don't check ui_consumed_click — the collider
            // handler sets the flag to block map editing, not palette interaction)
            if engine.input.was_mouse_pressed(0) {
                let mx = engine.input.mouse_pos.x;
                let my = engine.input.mouse_pos.y;
                if mx >= px && mx <= px + palette_w && my >= py && my <= py + palette_h {
                    let lx = ((mx - px) / t_size[0]).floor() as u32;
                    let ly = ((my - py) / t_size[1]).floor() as u32;
                    let tile_idx = lx + ly * tiles_per_row;
                    engine.editor_tile = tile_idx.min(max_tile);
                    lx2.set(lx);
                    ly2.set(ly);
                }
            }

            let sel_x = px + lx2.get() as f32 * t_size[0];
            let sel_y = py + ly2.get() as f32 * t_size[1];
            if let Ok(mut tf) = engine.world.get::<&mut Transform>(sel_e) {
                tf.position = glam::Vec3::new(sel_x, sel_y, tf.position.z);
            }
        });

        self.tile_palette_e = Some(cp_e);
        self.set_enabled(cp_e, false);
    }

    /// Nav palette: shows the nav tileset at 4x scale with click-to-select.
    pub fn init_nav_palette(&mut self) {
        let max_tile: u32 = 2;
        let tiles_per_row: u32 = 2;
        let Some(ref mut ui) = self.ui else { return };

        let nav_tile_px: f32 = 8.0;
        let ui_scale: f32 = 4.0;
        // Read actual texture height from the loaded navTileset PNG
        let tex_h = self
            .gfx
            .as_ref()
            .and_then(|g| g.textures.get("navTileset"))
            .map(|t| t.size.1 as f32)
            .unwrap_or(16.0);
        let palette_w = nav_tile_px * tiles_per_row as f32 * ui_scale;
        let palette_h = tex_h * ui_scale;
        let ts = [nav_tile_px * ui_scale, nav_tile_px * ui_scale];

        let container =
            ui.spawn_container(&mut self.world, palette_w, palette_h, [0.0, 0.0, 0.0, 0.2]);
        let sprite =
            ui.spawn_sprite(&mut self.world, "navTileset", palette_w, palette_h, 0.0, [1.0, 1.0]);
        ui.container_add_child(
            &mut self.world,
            container,
            sprite,
            UiAnchor::TopLeft,
            UiAnchor::TopLeft,
        );

        let selector = ui.spawn_container(&mut self.world, ts[0], ts[1], [1.0, 1.0, 1.0, 0.3]);
        ui.container_add_child(
            &mut self.world,
            container,
            selector,
            UiAnchor::TopLeft,
            UiAnchor::TopLeft,
        );

        let cp_e = container;
        let sel_e = selector;
        let local_x = std::rc::Rc::new(std::cell::Cell::new(0u32));
        let local_y = std::rc::Rc::new(std::cell::Cell::new(0u32));
        let lx2 = local_x.clone();
        let ly2 = local_y.clone();

        // Add collider with dummy click handler so consumes_click blocks map editing
        let pid = ui.add_collider_to_elem(&mut self.world, cp_e, &mut self.physics);
        self.physics.set_collider_consumes_click(pid, true);
        self.physics
            .add_collider_handler(pid, classic_core::collision::HandlerKind::Click, || true);

        self.on_update(move |engine| {
            let Some(ref _ui) = engine.ui else { return };
            if engine.editor_target != "navMesh" {
                return;
            }
            let vw = _ui.viewport_w;
            let vh = _ui.viewport_h;
            let border: f32 = 10.0;
            let px = vw - palette_w - border;
            let py = vh - palette_h - border;
            if let Ok(mut tf) = engine.world.get::<&mut Transform>(cp_e) {
                tf.position = glam::Vec3::new(px, py, tf.position.z);
            }
            ui::UIManager::position_children_of(cp_e, &mut engine.world);

            if engine.input.was_mouse_pressed(0) {
                let mx = engine.input.mouse_pos.x;
                let my = engine.input.mouse_pos.y;
                if mx >= px && mx <= px + palette_w && my >= py && my <= py + palette_h {
                    let lx = ((mx - px) / ts[0]).floor() as u32;
                    let ly = ((my - py) / ts[1]).floor() as u32;
                    engine.editor_nav_tile = (lx + ly * tiles_per_row).min(max_tile);
                    lx2.set(lx);
                    ly2.set(ly);
                }
            }

            let sel_x = px + lx2.get() as f32 * ts[0];
            let sel_y = py + ly2.get() as f32 * ts[1];
            if let Ok(mut tf) = engine.world.get::<&mut Transform>(sel_e) {
                tf.position = glam::Vec3::new(sel_x, sel_y, tf.position.z);
            }
        });

        self.nav_palette_e = Some(container);
        self.set_enabled(container, false);
    }

    /// Build and upload GPU resources for the nav mesh overlay.
    pub fn init_nav_mesh_render(&mut self) {
        let Some(&nav_entity) = self.names.get("tilemapNavigation") else {
            return;
        };
        let (size_x, size_y, nav_data, heights, height_scale) = {
            let nav = match self.world.get::<&NavMesh>(nav_entity) {
                Ok(n) => n,
                Err(_) => return,
            };
            // Use parent tilemap's actual height data so nav tiles sit on terrain surface
            let (hd, hs) = self
                .names
                .get("tilemap")
                .and_then(|&e| self.world.get::<&Tilemap>(e).ok())
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
        let tile_tex = {
            let tex = unsafe { gfx.gl.create_texture() }.expect("create nav texture");
            unsafe {
                gfx.gl.bind_texture(glow::TEXTURE_2D, Some(tex));
                gfx.gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA as i32,
                    tw as i32,
                    th as i32,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(&tile_pixels)),
                );
                gfx.gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MIN_FILTER,
                    glow::NEAREST as i32,
                );
                gfx.gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MAG_FILTER,
                    glow::NEAREST as i32,
                );
                gfx.gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_WRAP_S,
                    glow::CLAMP_TO_EDGE as i32,
                );
                gfx.gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_WRAP_T,
                    glow::CLAMP_TO_EDGE as i32,
                );
            }
            tex
        };

        self.nav_gpu = Some(TilemapGpu { mesh_buf, vertex_count: vcount, tile_tex });

        // Add Transform to nav entity so render query (&Transform, &NavMesh) matches.
        // Borrow position + scale from parent tilemap (matches TS IsometricNavMesh constructor).
        {
            let (pos, scl) = self
                .names
                .get("tilemap")
                .and_then(|&e| self.world.get::<&Transform>(e).ok())
                .map(|tf| (tf.position, tf.scale))
                .unwrap_or((glam::Vec3::ZERO, glam::Vec3::ONE));
            let _ = self.world.insert_one(nav_entity, Transform::new(pos, scl));
        }
    }

    /// After height edits, recalculate nav mesh walkability and rebuild GPU resources.
    fn sync_nav_heights(&mut self) {
        let Some(nav_entity) = self.names.get("tilemapNavigation").copied() else {
            return;
        };
        let Some(tm_entity) = self.names.get("tilemap").copied() else {
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
        let mut changed = false;
        if let Ok(mut nav) = self.world.get::<&mut NavMesh>(nav_entity) {
            for ty in 0..sy {
                for tx in 0..sx {
                    let idx = (ty * sx + tx) as usize;
                    let tidx = ty as usize * (sx as usize + 1) + tx as usize;
                    let h = hd.get(tidx).copied().unwrap_or(0.0);
                    let mut walkable: u32 = 1;
                    if tx > 0 {
                        let h_prev = hd
                            .get(ty as usize * (sx as usize + 1) + (tx - 1) as usize)
                            .copied()
                            .unwrap_or(0.0);
                        if (h - h_prev).abs() > 2.0 {
                            walkable = 0;
                        }
                    }
                    if tx + 1 < sx {
                        let h_next = hd
                            .get(ty as usize * (sx as usize + 1) + (tx + 1) as usize)
                            .copied()
                            .unwrap_or(0.0);
                        if (h - h_next).abs() > 2.0 {
                            walkable = 0;
                        }
                    }
                    if ty > 0 {
                        let h_prev = hd
                            .get((ty - 1) as usize * (sx as usize + 1) + tx as usize)
                            .copied()
                            .unwrap_or(0.0);
                        if (h - h_prev).abs() > 2.0 {
                            walkable = 0;
                        }
                    }
                    if ty + 1 < sx {
                        let h_next = hd
                            .get((ty + 1) as usize * (sx as usize + 1) + tx as usize)
                            .copied()
                            .unwrap_or(0.0);
                        if (h - h_next).abs() > 2.0 {
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

    /// Rebuild nav mesh GPU buffers from current NavMesh component data.
    fn rebuild_nav_gpu(&mut self) {
        let Some(nav_entity) = self.names.get("tilemapNavigation").copied() else {
            return;
        };
        let (sx, sy, data) = {
            let nav = match self.world.get::<&NavMesh>(nav_entity) {
                Ok(n) => n,
                Err(_) => return,
            };
            (nav.size_x, nav.size_y, nav.data.clone())
        };
        let hs = self
            .names
            .get("tilemap")
            .and_then(|&e| self.world.get::<&Tilemap>(e).ok())
            .map(|tm| tm.height_scale)
            .unwrap_or(64.0);
        let heights = vec![1.0f32; (sx as usize + 1) * (sy as usize + 1)];
        let Some(gfx) = self.gfx.as_mut() else { return };

        let (mesh_data, vcount) = build_mesh(sx, sy, &data, &heights, hs);
        let mesh_buf =
            GlBuffer::from_slice(&gfx.gl, glow::ARRAY_BUFFER, &mesh_data, glow::DYNAMIC_DRAW);
        let (tile_pixels, tw, th) = build_tile_texture(sx, sy, &data);
        let tile_tex = {
            let tex = unsafe { gfx.gl.create_texture() }.expect("create nav texture");
            unsafe {
                gfx.gl.bind_texture(glow::TEXTURE_2D, Some(tex));
                gfx.gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA as i32,
                    tw as i32,
                    th as i32,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(&tile_pixels)),
                );
                gfx.gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MIN_FILTER,
                    glow::NEAREST as i32,
                );
                gfx.gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MAG_FILTER,
                    glow::NEAREST as i32,
                );
                gfx.gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_WRAP_S,
                    glow::CLAMP_TO_EDGE as i32,
                );
                gfx.gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_WRAP_T,
                    glow::CLAMP_TO_EDGE as i32,
                );
            }
            tex
        };
        self.nav_gpu = Some(TilemapGpu { mesh_buf, vertex_count: vcount, tile_tex });
    }
    /// Paint tiles or heights in the selection region after a drag ends.
    fn apply_editor_selection(&mut self) {
        let Some(&tm_entity) = self.names.get("tilemap") else { return };
        let (bx, by, ex, ey, tile_count) = {
            let tm = match self.world.get::<&Tilemap>(tm_entity) {
                Ok(t) => t,
                Err(_) => {
                    classic_core::cl_info!(
                        classic_core::instrument::Chan::Editor,
                        "apply_editor_selection: no Tilemap component on entity"
                    );
                    return;
                }
            };
            let b = tm.selection_iso_begin;
            let e = tm.selection_iso_end;
            let from_x = b.x.min(e.x).floor().max(0.0) as i32;
            let from_y = b.y.min(e.y).floor().max(0.0) as i32;
            let to_x = b.x.max(e.x).ceil().min(tm.size_x as f32) as i32;
            let to_y = b.y.max(e.y).ceil().min(tm.size_y as f32) as i32;
            let count = (to_x - from_x).max(0) * (to_y - from_y).max(0);
            (from_x, from_y, to_x, to_y, count)
        };
        classic_core::cl_info!(
            classic_core::instrument::Chan::Editor,
            "apply_editor_selection: target={} region=({},{})-({},{}) tile_count={}",
            self.editor_target,
            bx,
            by,
            ex,
            ey,
            tile_count
        );
        classic_core::cl_debug!(
            classic_core::instrument::Chan::Editor,
            "target={} region=({},{})-({},{})",
            self.editor_target,
            bx,
            by,
            ex,
            ey
        );
        if tile_count == 0 {
            classic_core::cl_info!(
                classic_core::instrument::Chan::Editor,
                "apply_editor_selection: tile_count=0, returning"
            );
            return;
        }

        let updated = if self.editor_target == "height" {
            let val = self.editor_height as f32;
            let is_set = self.height_edit_mode == "set";
            if let Ok(mut tm) = self.world.get::<&mut Tilemap>(tm_entity) {
                for y in by..ey {
                    for x in bx..ex {
                        let idx = (y * (tm.size_x + 1) + x) as usize;
                        if is_set {
                            if let Some(h) = tm.height_data.get_mut(idx) {
                                *h = val.max(0.0);
                            }
                        } else if let Some(h) = tm.height_data.get_mut(idx) {
                            *h = (*h + val).max(0.0);
                        }
                    }
                }
            }
            classic_core::cl_debug!(
                classic_core::instrument::Chan::Editor,
                "painted height region ({},{})-({},{}) delta={} mode={}",
                bx,
                by,
                ex,
                ey,
                self.editor_height,
                self.height_edit_mode,
            );
            true
        } else if self.editor_target == "tilemap" {
            let val = self.editor_tile;
            if let Ok(mut tm) = self.world.get::<&mut Tilemap>(tm_entity) {
                for y in by..ey {
                    for x in bx..ex {
                        let idx = (y * tm.size_x + x) as usize;
                        if let Some(t) = tm.data.get_mut(idx) {
                            *t = val;
                        }
                    }
                }
            }
            classic_core::cl_debug!(
                classic_core::instrument::Chan::Editor,
                "painted tile region ({},{})-({},{}) id={}",
                bx,
                by,
                ex,
                ey,
                val
            );
            true
        } else if self.editor_target == "navMesh" {
            let val = self.editor_nav_tile;
            if let Some(&nav_e) = self.names.get("tilemapNavigation") {
                if let Ok(mut nav) = self.world.get::<&mut NavMesh>(nav_e) {
                    for y in by..ey {
                        for x in bx..ex {
                            let idx = (y * nav.size_x + x) as usize;
                            if let Some(t) = nav.data.get_mut(idx) {
                                *t = val;
                            }
                        }
                    }
                }
            }
            classic_core::cl_debug!(
                classic_core::instrument::Chan::Editor,
                "painted nav region ({},{})-({},{}) id={}",
                bx,
                by,
                ex,
                ey,
                val
            );
            true
        } else {
            false
        };

        if updated {
            classic_core::cl_info!(
                classic_core::instrument::Chan::Editor,
                "apply_editor_selection: paint done, rebuilding mesh"
            );
            if self.editor_target == "navMesh" {
                self.rebuild_nav_gpu();
            } else {
                self.rebuild_tilemap_mesh("tilemap");
                if self.editor_target == "height" {
                    self.sync_nav_heights();
                }
            }
        } else {
            classic_core::cl_info!(
                classic_core::instrument::Chan::Editor,
                "apply_editor_selection: editor_target={}, nothing to paint",
                self.editor_target
            );
        }
    }

    /// Rebuild the tilemap mesh from current data + heights and re-upload to GPU.
    fn rebuild_tilemap_mesh(&mut self, entity_name: &str) {
        classic_core::cl_info!(
            classic_core::instrument::Chan::Editor,
            "rebuild_tilemap_mesh: entering for '{entity_name}'"
        );
        let Some(&tm_entity) = self.names.get(entity_name) else {
            classic_core::cl_warn!(
                classic_core::instrument::Chan::Editor,
                "rebuild_tilemap_mesh: entity '{entity_name}' not found"
            );
            return;
        };
        let (size_x, size_y, tiles, heights, height_scale) = {
            let tm = match self.world.get::<&Tilemap>(tm_entity) {
                Ok(t) => t,
                Err(_) => {
                    classic_core::cl_warn!(
                        classic_core::instrument::Chan::Editor,
                        "rebuild_tilemap_mesh: no Tilemap on '{entity_name}'"
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
        let tile_tex = {
            let tex = unsafe { gfx.gl.create_texture() }.expect("create texture");
            unsafe {
                gfx.gl.bind_texture(glow::TEXTURE_2D, Some(tex));
                gfx.gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA as i32,
                    tw as i32,
                    th as i32,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(&tile_pixels)),
                );
                gfx.gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MIN_FILTER,
                    glow::NEAREST as i32,
                );
                gfx.gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MAG_FILTER,
                    glow::NEAREST as i32,
                );
                gfx.gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_WRAP_S,
                    glow::CLAMP_TO_EDGE as i32,
                );
                gfx.gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_WRAP_T,
                    glow::CLAMP_TO_EDGE as i32,
                );
            }
            tex
        };

        if let Some(gpu) = self.tilemap_gpu.get_mut(entity_name) {
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
    fn set_enabled(&mut self, entity: hecs::Entity, enabled: bool) {
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
    fn is_disabled(&self, entity: hecs::Entity) -> bool {
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

    /// Initialize navigation: load nav mesh data from `map001.nav.txt`,
    /// sync walkable flags from the parent tilemap heights, and wire the
    /// click-to-move handler that computes A* paths for the nav agent.
    pub fn init_navigation(&mut self, nav_data_b64: &str) {
        let nav_entity = self.names.get("tilemapNavigation").copied();
        let tilemap_entity = self.names.get("tilemap").copied();
        let agent_entity = self.names.get("navAgent").copied();

        let Some(nav_entity) = nav_entity else { return };
        let Some(tilemap_entity) = tilemap_entity else { return };
        let Some(agent_entity) = agent_entity else { return };

        // 1. Decode nav map data (same base64 JSON format as tilemap data).
        let nav_tiles = Engine::decode_map_data(nav_data_b64);

        // 2. Sync nav mesh walkable flags from parent tilemap heights.
        //    With flat terrain (height=1 everywhere) this is a no-op, but
        //    the logic is here for when varied heights are re-enabled.
        {
            let tm_entity = tilemap_entity;
            if let Ok(tm) = self.world.get::<&Tilemap>(tm_entity) {
                if let Ok(mut nav) = self.world.get::<&mut NavMesh>(nav_entity) {
                    let s_x = nav.size_x;
                    let s_y = nav.size_y;
                    let hd = &tm.height_data;
                    let mut changed = false;
                    for ty in 0..s_y {
                        for tx in 0..s_x {
                            let idx = (ty * s_x + tx) as usize;
                            let h = hd
                                .get((ty * (tm.size_x + 1) + tx) as usize)
                                .copied()
                                .unwrap_or(0.0);
                            let mut walkable: u32 = 1;
                            if tx > 0 {
                                let h_prev = hd
                                    .get((ty * (tm.size_x + 1) + tx - 1) as usize)
                                    .copied()
                                    .unwrap_or(0.0);
                                if (h - h_prev).abs() > 2.0 {
                                    walkable = 0;
                                }
                            }
                            if tx + 1 < s_x {
                                let h_next = hd
                                    .get((ty * (tm.size_x + 1) + tx + 1) as usize)
                                    .copied()
                                    .unwrap_or(0.0);
                                if (h - h_next).abs() > 2.0 {
                                    walkable = 0;
                                }
                            }
                            if ty > 0 {
                                let h_prev = hd
                                    .get(((ty - 1) * (tm.size_x + 1) + tx) as usize)
                                    .copied()
                                    .unwrap_or(0.0);
                                if (h - h_prev).abs() > 2.0 {
                                    walkable = 0;
                                }
                            }
                            if ty + 1 < s_y {
                                let h_next = hd
                                    .get(((ty + 1) * (tm.size_x + 1) + tx) as usize)
                                    .copied()
                                    .unwrap_or(0.0);
                                if (h - h_next).abs() > 2.0 {
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
                    if changed {
                        log::debug!(
                            "nav mesh sync changed {} tiles",
                            nav.data.iter().filter(|&&v| v == 0).count()
                        );
                    }
                }
            }
        }

        // Overwrite nav data with decoded map (nav is authoritative for passability).
        if let Ok(mut nav) = self.world.get::<&mut NavMesh>(nav_entity) {
            nav.data = nav_tiles.to_vec();
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

    // ---- CLASSIC_TEST runner ----

    fn build_test_scenario(_name: &str) -> Vec<TestStep> {
        // If CLASSIC_TEST_FILE is set, load from that JSON file.
        if let Ok(path) = std::env::var("CLASSIC_TEST_FILE") {
            match std::fs::read_to_string(&path) {
                Ok(json) => {
                    return serde_json::from_str(&json)
                        .unwrap_or_else(|e| panic!("CLASSIC_TEST_FILE {}: {}", path, e));
                }
                Err(e) => panic!("cannot read CLASSIC_TEST_FILE {}: {}", path, e),
            }
        }
        // Single hardcoded scenario (data-driven file loading above takes
        // precedence when CLASSIC_TEST_FILE is set).
        vec![
            // 0: open dev menu, wait a frame for layout, verify text is centered
            TestStep {
                frame: 2,
                actions: vec![TestAction::OpenMenu],
                assertions: vec![],
                log: "open dev menu panel".into(),
            },
            TestStep {
                frame: 4,
                actions: vec![],
                assertions: vec![TileAssertion {
                    kind: AssertKind::UiTextCentered,
                    region: (0, 0, 0, 0),
                    expected: 2.0,
                    log: "menu item text labels centered within rows (tolerance=2px)".into(),
                }],
                log: "verify menu text centered".into(),
            },
            // 1: set height editor to blend mode, delta=2
            TestStep {
                frame: 5,
                actions: vec![TestAction::SetEditor {
                    target: "height".into(),
                    height_delta: 2,
                    height_mode: "blend".into(),
                    tile_id: 0,
                }],
                assertions: vec![],
                log: "set height editor: blend mode, delta=2".into(),
            },
            // 2: drag (10,10)→(14,14) — blend adds 2 to default 1 → expect 3.0
            TestStep {
                frame: 5,
                actions: vec![TestAction::Drag {
                    from: (10.0, 10.0),
                    to: (14.0, 14.0),
                    hold_frames: 4,
                }],
                assertions: vec![],
                log: "drag (10,10)→(14,14), hold=4".into(),
            },
            // 3: wait for mesh rebuild, assert region changed
            TestStep {
                frame: 13,
                actions: vec![],
                assertions: vec![
                    TileAssertion {
                        kind: AssertKind::Height,
                        region: (10, 10, 14, 14),
                        expected: 3.0,
                        log: "height(10,10-14,14)=3.0 (blend +2)".into(),
                    },
                    TileAssertion {
                        kind: AssertKind::Height,
                        region: (0, 0, 2, 2),
                        expected: 1.0,
                        log: "height(0,0-2,2)=1.0 (untouched)".into(),
                    },
                    TileAssertion {
                        kind: AssertKind::Height,
                        region: (16, 10, 18, 12),
                        expected: 1.0,
                        log: "height(16,10-18,12)=1.0 (untouched)".into(),
                    },
                ],
                log: "assert blend region changed, adjacent regions untouched".into(),
            },
            // 4: switch to set mode, apply value 5
            TestStep {
                frame: 16,
                actions: vec![TestAction::SetEditor {
                    target: "height".into(),
                    height_delta: 5,
                    height_mode: "set".into(),
                    tile_id: 0,
                }],
                assertions: vec![],
                log: "set height editor: set mode, value=5".into(),
            },
            // 5: drag (10,10)→(14,14) again — set to 5.0
            TestStep {
                frame: 16,
                actions: vec![TestAction::Drag {
                    from: (10.0, 10.0),
                    to: (14.0, 14.0),
                    hold_frames: 4,
                }],
                assertions: vec![],
                log: "drag (10,10)→(14,14), hold=4".into(),
            },
            // 6: assert set mode results
            TestStep {
                frame: 24,
                actions: vec![],
                assertions: vec![
                    TileAssertion {
                        kind: AssertKind::Height,
                        region: (10, 10, 14, 14),
                        expected: 5.0,
                        log: "height(10,10-14,14)=5.0 (set mode)".into(),
                    },
                    TileAssertion {
                        kind: AssertKind::Height,
                        region: (0, 0, 2, 2),
                        expected: 1.0,
                        log: "height(0,0-2,2)=1.0 (untouched)".into(),
                    },
                ],
                log: "assert set mode applied correct values".into(),
            },
            // 7: blend different region with delta=3 (new tiles: 1+3=4)
            TestStep {
                frame: 27,
                actions: vec![TestAction::SetEditor {
                    target: "height".into(),
                    height_delta: 3,
                    height_mode: "blend".into(),
                    tile_id: 0,
                }],
                assertions: vec![],
                log: "set height editor: blend mode, delta=3".into(),
            },
            // 8: drag (20,10)→(22,12)
            TestStep {
                frame: 27,
                actions: vec![TestAction::Drag {
                    from: (20.0, 10.0),
                    to: (22.0, 12.0),
                    hold_frames: 4,
                }],
                assertions: vec![],
                log: "drag (20,10)→(22,12), hold=4".into(),
            },
            // 9: verify blend on adjacent region, original region unchanged
            TestStep {
                frame: 35,
                actions: vec![],
                assertions: vec![
                    TileAssertion {
                        kind: AssertKind::Height,
                        region: (20, 10, 22, 12),
                        expected: 4.0,
                        log: "height(20,10-22,12)=4.0 (1+3 blend)".into(),
                    },
                    TileAssertion {
                        kind: AssertKind::Height,
                        region: (10, 10, 11, 11),
                        expected: 5.0,
                        log: "height(10,10-11,11)=5.0 (unchanged from set)".into(),
                    },
                ],
                log: "assert blend region correct, set region untouched".into(),
            },
            // 10: tile editor — set tile id to 7
            TestStep {
                frame: 38,
                actions: vec![TestAction::SetEditor {
                    target: "tilemap".into(),
                    height_delta: 0,
                    height_mode: String::new(),
                    tile_id: 7,
                }],
                assertions: vec![],
                log: "set tile editor: tile_id=7".into(),
            },
            // 11: drag single tile (8,8)→(9,9)
            TestStep {
                frame: 38,
                actions: vec![TestAction::Drag {
                    from: (8.0, 8.0),
                    to: (9.0, 9.0),
                    hold_frames: 3,
                }],
                assertions: vec![],
                log: "drag single tile (8,8)→(9,9)".into(),
            },
            // 12: verify tile data changed
            TestStep {
                frame: 45,
                actions: vec![],
                assertions: vec![
                    TileAssertion {
                        kind: AssertKind::Tile,
                        region: (8, 8, 9, 9),
                        expected: 7.0,
                        log: "tile(8,8-9,9)=7".into(),
                    },
                    TileAssertion {
                        kind: AssertKind::Tile,
                        region: (10, 10, 11, 11),
                        expected: 9.0,
                        log: "tile(10,10-11,11)=9 (untouched)".into(),
                    },
                ],
                log: "assert tile paint correct, adjacent unchanged".into(),
            },
            // 13: zero-delta blend (should log tile_count=0, no asserts needed)
            TestStep {
                frame: 48,
                actions: vec![
                    TestAction::SetEditor {
                        target: "height".into(),
                        height_delta: 0,
                        height_mode: "blend".into(),
                        tile_id: 0,
                    },
                    TestAction::Drag { from: (25.0, 10.0), to: (26.0, 10.0), hold_frames: 2 },
                ],
                assertions: vec![TileAssertion {
                    kind: AssertKind::Height,
                    region: (25, 10, 26, 11),
                    expected: 1.0,
                    log: "height(25,10)=1.0 (zero delta, unchanged)".into(),
                }],
                log: "zero-delta blend: no change expected".into(),
            },
            // 7: enable text demo panel, verify container becomes visible
            TestStep {
                frame: 52,
                actions: vec![TestAction::EnableTextDemo],
                assertions: vec![],
                log: "enable text demo panel".into(),
            },
            TestStep {
                frame: 54,
                actions: vec![],
                assertions: vec![TileAssertion {
                    kind: AssertKind::UiEnabled,
                    region: (0, 0, 0, 0),
                    expected: 1.0,
                    log: "text showcase container enabled".into(),
                }],
                log: "verify text demo enabled".into(),
            },
        ]
    }

    fn run_test_frame(&mut self, steps: &[TestStep]) {
        let frame = self.debug_frame;

        // Re-apply editor state before step processing so assertions
        // on this frame see the corrected state (tool_buttons on_update
        // resets editor_target via Rc sync earlier in the frame).
        if let Some((ref target, hd, ref mode, tid)) = self.test_editor_state {
            self.editor_target = target.clone();
            self.editor_height = hd;
            self.height_edit_mode = mode.clone();
            self.editor_tile = tid;
            // Re-enable panels that editor_mode_control on_update may have disabled
            if target == "textDemo" {
                if let Some(e) = self.text_showcase_e {
                    self.set_enabled(e, true);
                }
            }
        }

        // Process any step scheduled for this frame
        while self.test_step_index < steps.len() && steps[self.test_step_index].frame == frame {
            let step = &steps[self.test_step_index];
            classic_core::cl_info!(
                classic_core::instrument::Chan::Test,
                "[FRAME {}] STEP: {}",
                frame,
                step.log
            );

            // Execute actions
            for action in &step.actions {
                match action {
                    TestAction::SetEditor { target, height_delta, height_mode, tile_id } => {
                        self.editor_target = target.clone();
                        self.editor_height = *height_delta;
                        self.height_edit_mode = height_mode.clone();
                        self.editor_tile = *tile_id;
                        self.test_editor_state =
                            Some((target.clone(), *height_delta, height_mode.clone(), *tile_id));
                    }
                    TestAction::Drag { from, to, hold_frames } => {
                        let hold = *hold_frames;
                        self.test_drag_state = Some((
                            glam::Vec2::new(from.0, from.1),
                            glam::Vec2::new(to.0, to.1),
                            hold,
                            frame,
                        ));
                    }
                    TestAction::OpenMenu => {
                        self.panel_menu_open = true;
                        if let Some(mp) = self.menu_panel_e {
                            self.set_enabled(mp, true);
                        }
                    }
                    TestAction::EnableTextDemo => {
                        self.editor_target = "textDemo".into();
                        self.test_editor_state = Some(("textDemo".into(), 0, "set".into(), 0));
                        if let Some(e) = self.text_showcase_e {
                            self.set_enabled(e, true);
                        }
                    }
                    TestAction::MouseMove { x, y } => {
                        self.input.mouse_pos = glam::Vec2::new(*x, *y);
                        self.input.mouse_axis.x = ((*x / self.last_vw) - 0.5) * 2.0;
                        self.input.mouse_axis.y = ((*y / self.last_vh) - 0.5) * 2.0;
                    }
                    TestAction::MouseClick { x, y, button } => {
                        self.input.mouse_pos = glam::Vec2::new(*x, *y);
                        let b = *button as usize;
                        if b < 3 {
                            self.input.mouse_down[b] = true;
                            self.input.mouse_pressed[b] = true;
                        }
                        if b == 0 {
                            self.input.frame_had_click = true;
                        }
                    }
                    TestAction::KeyPress { key, pressed } => {
                        self.input.keys_down.insert(key.clone(), *pressed);
                        if *pressed {
                            self.input.keys_pressed.insert(key.clone(), true);
                        }
                    }
                    TestAction::Wheel { amount } => {
                        self.input.mouse_wheel = *amount;
                    }
                    TestAction::Wait { frames: _wait_frames } => {}
                }
            }

            // Run assertions
            for a in &step.assertions {
                let passed = match a.kind {
                    AssertKind::Height => self.assert_heights(a.region, a.expected),
                    AssertKind::Tile => self.assert_tiles(a.region, a.expected as u32),
                    AssertKind::UiTextCentered => self.assert_ui_text_centered(a.expected),
                    AssertKind::UiEnabled => {
                        let should_be_enabled = a.expected != 0.0;
                        let is_enabled =
                            self.text_showcase_e.map(|e| !self.is_disabled(e)).unwrap_or(false);
                        if is_enabled != should_be_enabled {
                            classic_core::cl_info!(
                                classic_core::instrument::Chan::Test,
                                "  [UI] text showcase enabled={} expected={}",
                                is_enabled,
                                should_be_enabled
                            );
                        }
                        is_enabled == should_be_enabled
                    }
                    AssertKind::CameraAt => {
                        let pos_tol = if a.expected <= 0.0 { 1.0 } else { a.expected };
                        let scale_tol = if a.expected <= 0.0 { 0.01 } else { a.expected };
                        let ex = a.region.0 as f32;
                        let ey = a.region.1 as f32;
                        let ez = a.region.2 as f32;
                        let es = if a.region.3 != 0 { a.region.3 as f32 } else { 1.0 };
                        let dx = (self.camera.position.x - ex).abs();
                        let dy = (self.camera.position.y - ey).abs();
                        let dz = (self.camera.position.z - ez).abs();
                        let ds = (self.camera.scale.x - es).abs();
                        if dx > pos_tol || dy > pos_tol || dz > pos_tol || ds > scale_tol {
                            classic_core::cl_info!(
                                classic_core::instrument::Chan::Test,
                                "  [Camera] pos=({:.1},{:.1},{:.1}) scale={:.2} expected pos=({},{},{}) scale={}",
                                self.camera.position.x,
                                self.camera.position.y,
                                self.camera.position.z,
                                self.camera.scale.x,
                                ex,
                                ey,
                                ez,
                                es,
                            );
                        }
                        dx <= pos_tol && dy <= pos_tol && dz <= pos_tol && ds <= scale_tol
                    }
                    AssertKind::EntityVisible => {
                        let name = if a.log.is_empty() { "entity" } else { &a.log };
                        // Use log as entity name; fall back on assertion for the report.
                        let should_be_visible = a.expected != 0.0;
                        let is_visible =
                            self.names.get(name).map(|&e| !self.is_disabled(e)).unwrap_or(false);
                        if is_visible != should_be_visible {
                            classic_core::cl_info!(
                                classic_core::instrument::Chan::Test,
                                "  [Visible] '{}' visible={} expected={}",
                                name,
                                is_visible,
                                should_be_visible
                            );
                        }
                        is_visible == should_be_visible
                    }
                    AssertKind::EntityPos => {
                        let name = if a.log.is_empty() { "entity" } else { &a.log };
                        // Use log as entity name; fall back on assertion for the report.
                        let tol = if a.expected <= 0.0 { 1.0 } else { a.expected };
                        let ex = a.region.0 as f32;
                        let ey = a.region.1 as f32;
                        let passes = self
                            .names
                            .get(name)
                            .and_then(|&e| self.world.get::<&Transform>(e).ok())
                            .map(|tf| {
                                (tf.position.x - ex).abs() <= tol
                                    && (tf.position.y - ey).abs() <= tol
                            })
                            .unwrap_or(false);
                        if !passes {
                            if let Some(&e) = self.names.get(name) {
                                if let Ok(tf) = self.world.get::<&Transform>(e) {
                                    classic_core::cl_info!(
                                        classic_core::instrument::Chan::Test,
                                        "  [Pos] '{}' pos=({:.1},{:.1}) expected=({},{}) tol={:.1}",
                                        name,
                                        tf.position.x,
                                        tf.position.y,
                                        ex,
                                        ey,
                                        tol,
                                    );
                                }
                            }
                        }
                        passes
                    }
                };
                let result = format!(
                    "[FRAME {}] {}: {} (region=({},{})-({},{}) expected={})",
                    frame,
                    if passed { "PASS" } else { "FAIL" },
                    a.log,
                    a.region.0,
                    a.region.1,
                    a.region.2,
                    a.region.3,
                    a.expected,
                );
                classic_core::cl_info!(classic_core::instrument::Chan::Test, "{}", result);
                self.test_results.push(result);
                if !passed {
                    self.test_should_close = true;
                    self.test_failed = true;
                }
            }

            self.test_step_index += 1;
        }

        // Re-apply editor state every frame (tool_buttons on_update resets it via Rc sync)
        if let Some((ref target, hd, ref mode, tid)) = self.test_editor_state {
            self.editor_target = target.clone();
            self.editor_height = hd;
            self.height_edit_mode = mode.clone();
            self.editor_tile = tid;
        }

        // Process active drag
        if let Some((from, to, hold, start)) = self.test_drag_state {
            let rel = (frame - start) as i64;
            if rel == 0 {
                // press
                if let Some(&e) = self.names.get("tilemap") {
                    if let Ok(mut tm) = self.world.get::<&mut Tilemap>(e) {
                        tm.selection_iso_begin = from.extend(0.0);
                        tm.mouse_iso_pos = from.extend(0.0);
                        self.selection_mode = 1;
                    }
                }
            } else if rel > 0 && (rel as u64) < hold {
                // drag
                if let Some(&e) = self.names.get("tilemap") {
                    if let Ok(mut tm) = self.world.get::<&mut Tilemap>(e) {
                        let t = rel as f32 / hold as f32;
                        let cur = from.lerp(to, t);
                        tm.mouse_iso_pos = cur.extend(0.0);
                    }
                }
            } else if (rel as u64) == hold {
                // release
                if let Some(&e) = self.names.get("tilemap") {
                    if let Ok(mut tm) = self.world.get::<&mut Tilemap>(e) {
                        tm.selection_iso_end = to.extend(0.0);
                        tm.mouse_iso_pos = to.extend(0.0);
                        self.selection_mode = -1;
                    }
                }
                self.apply_editor_selection();
                self.test_drag_state = None;
                self.test_editor_state = None;
            } else {
                self.test_drag_state = None;
            }
        }

        // Report completion when all steps done and no drag in progress
        if self.test_step_index >= steps.len() && self.test_drag_state.is_none() {
            if !self.test_complete_reported {
                let total = self.test_results.len();
                let passed = self.test_results.iter().filter(|r| r.contains("PASS")).count();
                classic_core::cl_info!(
                    classic_core::instrument::Chan::Test,
                    "=== CLASSIC_TEST COMPLETE: {}/{} assertions passed ===",
                    passed,
                    total
                );
                self.test_complete_reported = true;

                // Auto-dump state on exit when CLASSIC_DUMP_ON_EXIT is set.
                if env_config::EnvConfig::get().dump_on_exit {
                    let _ = self.dump_state();
                    let _ = self.dump_map_data();
                    let _ = self.dump_nav_data();
                    let _ = self.dump_height_data();
                }
            }
            self.test_should_close = true;
        }
    }

    fn assert_tiles(&self, region: (i32, i32, i32, i32), expected: u32) -> bool {
        let Some(&e) = self.names.get("tilemap") else { return false };
        let Ok(tm) = self.world.get::<&Tilemap>(e) else {
            return false;
        };
        for y in region.1..region.3 {
            for x in region.0..region.2 {
                let idx = (y * tm.size_x + x) as usize;
                let actual = tm.data.get(idx).copied().unwrap_or(999);
                if actual != expected {
                    classic_core::cl_info!(
                        classic_core::instrument::Chan::Test,
                        "  tile({x},{y}) actual={actual} expected={expected}"
                    );
                    return false;
                }
            }
        }
        true
    }

    fn assert_heights(&self, region: (i32, i32, i32, i32), expected: f32) -> bool {
        let Some(&e) = self.names.get("tilemap") else { return false };
        let Ok(tm) = self.world.get::<&Tilemap>(e) else {
            return false;
        };
        for y in region.1..region.3 {
            for x in region.0..region.2 {
                let idx = (y * (tm.size_x + 1) + x) as usize;
                let actual = tm.height_data.get(idx).copied().unwrap_or(-999.0);
                if (actual - expected).abs() > 0.01 {
                    classic_core::cl_info!(
                        classic_core::instrument::Chan::Test,
                        "  height({x},{y}) actual={actual:.1} expected={expected:.1}"
                    );
                    return false;
                }
            }
        }
        true
    }

    /// Verify that SDF text children of menu panel rows are correctly centered.
    /// `tolerance` is the max allowed distance in pixels between the actual
    /// child position and the expected centered position.
    fn assert_ui_text_centered(&self, tolerance: f32) -> bool {
        let Some(mp) = self.menu_panel_e else {
            classic_core::cl_info!(
                classic_core::instrument::Chan::Test,
                "  [UI] no menu panel entity"
            );
            return false;
        };
        let Some(menu_node) = self.world.get::<&UiNode>(mp).ok() else {
            return false;
        };
        let rows: Vec<hecs::Entity> = menu_node.children.iter().map(|c| c.entity).collect();
        for row_e in &rows {
            let Some(row_node) = self.world.get::<&UiNode>(*row_e).ok() else {
                continue;
            };
            let Some(first_child) = row_node.children.first() else {
                continue;
            };
            let child_e = first_child.entity;
            let Ok(row_tf) = self.world.get::<&Transform>(*row_e) else {
                continue;
            };
            let Ok(child_tf) = self.world.get::<&Transform>(child_e) else {
                continue;
            };
            let (child_w, child_h) = self
                .world
                .get::<&UiNode>(child_e)
                .ok()
                .map(|n| (n.size.x, n.size.y))
                .unwrap_or((0.0, 0.0));

            let expected_x = row_tf.position.x + row_node.size.x / 2.0 - child_w / 2.0;
            let expected_y = row_tf.position.y + row_node.size.y / 2.0 - child_h / 2.0;
            let dx = (child_tf.position.x - expected_x).abs();
            let dy = (child_tf.position.y - expected_y).abs();

            if dx > tolerance || dy > tolerance {
                classic_core::cl_info!(
                    classic_core::instrument::Chan::Test,
                    "  [UI] row {:?} text child @ ({:.1},{:.1}) expected ({:.1},{:.1}) \
                     child_size=({:.1},{:.1}) row_size=({:.1},{:.1}) dx={:.1} dy={:.1}",
                    row_e.id(),
                    child_tf.position.x,
                    child_tf.position.y,
                    expected_x,
                    expected_y,
                    child_w,
                    child_h,
                    row_node.size.x,
                    row_node.size.y,
                    dx,
                    dy,
                );
                return false;
            }
        }
        true
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}
