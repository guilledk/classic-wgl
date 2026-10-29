//! `Engine` construction and the per-frame lifecycle (`Engine::frame`),
//! plus entity enable/disable.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use classic_core::collision::PhysicsProvider;
use classic_core::components::{
    IsoSprite, IsoVehicle, NavMesh, RectRender, Role, SdfTextRender, Tilemap,
};
use classic_core::instrument::Chan;
use classic_core::math::{iso_camera_matrix, iso_view_depth, DEPTH_FAR, DEPTH_NEAR};
use classic_core::pathfinder;
use classic_core::sdf_builder::build_sdf_glyph_buffer;
use classic_core::tilemap::{PPM_TARGET, TILE_M};
use classic_core::{Camera, RoleKind, SpriteRender, Transform};
use classic_gfx::{GlBuffer, IsoSpritePass, RenderSettings, SpriteRegion};
use classic_platform::InputState;
use glam::{Mat4, Vec2, Vec3, Vec4};
use glow::HasContext;

use crate::dir_blend::{packed_geometry, DirChoice};
use crate::{
    env_config, fields, golden, inventory_ui, light, particles, selection, shadow, DrawKind,
    Engine, GuestEvent, IsoDraw, IsoUv, SdfTextGpu, Time, UpdateKind, RTS_DRAG_THRESHOLD_PX,
    SELECTION_COLOR,
};

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
            gfx_full: false,
            world_hidden: false,
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
            // RTS selection is on unless something turns it off (the dev
            // editor while a terrain-paint tool owns the drag): it used to
            // start off and only the dev HUD's update closure turned it on,
            // so a game without the dev layer (`CLASSIC_NO_UI`, or no
            // `host_features`) selected nothing.
            guest_flags: HashMap::from([("rts_selection".to_string(), true)]),
            scroll_speed: 600.0,
            input: InputState::new(),
            scripted_input: Default::default(),
            show_grid: false,
            light_ambient: [0.15, 0.15, 0.2],
            light_dir: [0.566_422, -0.070_803, 0.821_068],
            light_color: [1.0, 0.95, 0.85],
            light_handles: light::LightHandles::new(),
            particles: particles::ParticleSystem::default(),
            emitter_handles: particles::EmitterHandles::new(),
            animations: HashMap::new(),
            sdf_fonts: HashMap::new(),
            frame_tables: HashMap::new(),
            texture_depths: HashMap::new(),
            texture_normals: HashMap::new(),
            texture_names: HashSet::new(),
            vehicles: HashMap::new(),
            vehicle_anchors: HashMap::new(),
            models: HashMap::new(),
            model_generation: 0,
            pose_scratch: Default::default(),
            node_index: Default::default(),
            model_gpu: Default::default(),
            model_textures: HashMap::new(),
            items: classic_core::inventory::ItemRegistry::default(),
            next_ghost_group: 1,
            rom_manifest_json: None,
            rom_manifest: None,
            rom_resources: None,
            loaded_roms: Vec::new(),
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
            pathfinder: None,
            vehicle_nav_snapshot: Arc::new(pathfinder::VehicleNavSnapshot::new(
                0,
                0,
                Vec::new(),
                Vec::new(),
                TILE_M,
            )),
            vehicle_path_entities: HashMap::new(),
            preview_paths: HashMap::new(),
            floor_rects: Default::default(),
            vehicle_blocks: Default::default(),
            vehicle_block_owners: Default::default(),
            vehicle_replans: Vec::new(),
            vehicle_follow_guard: true,
            vehicle_pose_goals: HashMap::new(),
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
            bench: Default::default(),
            test_should_close: false,
            test_failed: false,
            golden_capture_frame: 55, // default: one frame after default scenario's last step (54)
            update_fns: Vec::new(),
            sim_paused: false,
            trace: None,
            tilemap_gpu: HashMap::new(),
            sdf_text_gpu: HashMap::new(),
            last_vw: 0.0,
            last_vh: 0.0,
        }
    }

    pub fn frame(&mut self, input: &mut InputState, vw: f32, vh: f32, delta: f32) {
        // The frame profiler (`CLASSIC_PERF`): off unless configured, and
        // then bracketing every frame (see `classic_gfx::perf`).
        static PERF_INIT: std::sync::Once = std::sync::Once::new();
        PERF_INIT.call_once(|| {
            let config = env_config::EnvConfig::get();
            if let Some(out) = config.perf.clone() {
                classic_gfx::perf::enable(classic_gfx::perf::PerfConfig {
                    out,
                    gpu: config.perf_gpu,
                    // A window's swap paces; the headless loop does not.
                    max_in_flight: config.headless.then_some(2),
                });
            }
        });
        if let Some(gfx) = self.gfx.as_ref() {
            classic_gfx::perf::begin_frame(&gfx.gl);
        }
        self.frame_inner(input, vw, vh, delta);
        self.flush_bench_marker();
        classic_gfx::perf::end_frame();
    }

    fn frame_inner(&mut self, input: &mut InputState, vw: f32, vh: f32, delta: f32) {
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
        // Scripted keys land after the platform snapshot, so a scenario's
        // `keyPress` (issued after the update closures) is seen next frame.
        self.scripted_input.apply(&mut self.input);
        // a scripted pointer also stands in for the platform's mouse axis
        if let Some(p) = self.scripted_input.pointer() {
            let (w, h) = (vw.max(1.0), vh.max(1.0));
            self.input.mouse_axis = glam::Vec2::new((p.x / w - 0.5) * 2.0, (p.y / h - 0.5) * 2.0);
        }
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
        // This frame's pointer on the terrain, before anything reads it
        // (the selection press below, the guests' `mouse_iso`).
        self.update_mouse_iso();

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
        classic_gfx::perf::phase("pre_update");
        let mut pre = std::mem::take(&mut self.pre_update_hooks);
        for f in pre.iter_mut() {
            f(self);
        }
        pre.append(&mut self.pre_update_hooks);
        self.pre_update_hooks = pre;

        // Take-restore dance: closures fire with &mut Engine, but the Vec
        // is owned by Engine. Taking means closures can call on_update()
        // without borrow conflicts. Restoring preserves them for next frame.
        // Closures registered *during* the loop land in the emptied
        // `self.update_fns`; append them before restoring so they survive.
        // Handlers use iter_mut(), NOT std::mem::take — they must survive
        // across frames (click, enter, exit, selection).
        // A paused simulation skips the `Sim` entries; the host ones (camera,
        // cursor, editor, HUD) run on in the same order.
        classic_gfx::perf::phase("update");
        // Read once: a host closure toggling the pause takes effect next frame,
        // so one frame never steps half its simulation.
        let paused = self.sim_paused;
        let mut fns = std::mem::take(&mut self.update_fns);
        for (kind, f) in fns.iter_mut() {
            if paused && *kind == UpdateKind::Sim {
                continue;
            }
            f(self);
        }
        fns.append(&mut self.update_fns);
        self.update_fns = fns;
        // Again after the guests: the camera may have moved under the cursor.
        self.update_mouse_iso();

        // Wheeled-vehicle simulation runs after the guest update closures
        // (so a `vehicle_goto` issued this frame is honoured) and before the
        // render list is built.  Vehicles, particles and the test runner are
        // simulation: a paused frame holds them where they are.
        classic_gfx::perf::phase("vehicles");
        if !paused {
            self.update_vehicles();
        }

        // Particles after every mover (models posed, vehicles stepped) so
        // attached emitters spawn from this frame's positions.  A scene
        // without emitters never creates an instance (a no-op).
        classic_gfx::perf::phase("particles_update");
        if !paused {
            self.update_particles(delta);
        }

        classic_gfx::perf::phase("test_runner");
        // ---- CLASSIC_TEST automated test runner (registered by the demo) ----
        if env_config::EnvConfig::get().test_active() && !paused {
            let mut runner = self.test_runner.take();
            if let Some(r) = runner.as_mut() {
                r(self);
            }
            self.test_runner = runner;
        }

        // Determinism barrier: under CLASSIC_TEST, wait for in-flight worker
        // jobs (pathfinding) so frame boundaries are deterministic.
        if env_config::EnvConfig::get().test_active()
            && !env_config::EnvConfig::get().workers_threaded
        {
            self.join_workers();
        }

        classic_gfx::perf::phase("render_prep");
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
            if let Some(ui) = self.ui.as_ref() {
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
        for (e, (tf, _)) in
            self.world.query::<(&Transform, &classic_core::components::Model)>().iter()
        {
            if self.is_disabled(e) {
                continue;
            }
            // Same `tx - ty` depth-major key as iso sprites.
            items.push((tf.position.x - tf.position.y, e, DrawKind::Model));
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
        if self.world_hidden {
            let world = &self.world;
            items.retain(|(_, e, k)| draws_while_world_hidden(world, *e, k));
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
        classic_gfx::perf::phase("lights");
        // (a paused simulation holds transient lights, like the particles
        // that spawn them)
        if !paused {
            self.light_handles.decay(&mut self.world, delta);
        }
        let lights = self.gather_lights();
        classic_gfx::perf::phase("render_list");
        classic_gfx::perf::count("lights", lights.len() as u64);

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
                    let layers = veh.layers.iter().map(|s| &s.entity);
                    for name in
                        veh.wheel_entities.iter().chain(veh.tire_entities.iter()).chain(layers)
                    {
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

        // Precompute isometric-sprite draw params once, shared by the shadow
        // casters, the normal pass, and the ghost pass below.  Built before the
        // `gfx` mutable borrow so the shadow pass can reuse the same params.
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
            // Direction interpolation: blend the two baked directions, or draw
            // the nearer one (`dir_blend`).
            let choice = Self::dir_choice(&self.frame_tables, &iso_sprite, frame_ref.as_ref());
            let (frame_ref, sprite_anchor) = match &choice {
                DirChoice::UseB { frame, anchor } => (Some(frame.clone()), *anchor),
                _ => (frame_ref, iso_sprite.anchor),
            };
            let (tex_dim, anchor_px, sheet_name, uv) = match &frame_ref {
                Some(fr) => {
                    let (td, ap, uv) = packed_geometry(fr, sprite_anchor);
                    (td, ap, fr.sheet_name.clone(), Some(uv))
                }
                None => {
                    let Some(tex) =
                        self.gfx.as_ref().and_then(|g| g.textures.get(&iso_sprite.texture))
                    else {
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
            // A packed frame resolves to its own sheet, which may not be resident
            // yet even though the frame table is (it is metadata, hydrated before
            // the `.basis` sheets): skip it like the uniform-grid path above.
            if self.gfx.as_ref().is_none_or(|g| !g.textures.contains_key(&sheet_name)) {
                continue;
            }

            let model = Self::compute_iso_sprite_model(
                &iso_sprite,
                &tf,
                &tilemap_tf,
                &tilemap,
                tex_dim,
                anchor_px,
            );
            let anchor_depth = frame_ref.as_ref().is_some_and(|fr| fr.anchor_depth);
            // The shadow caster always draws one baked direction (the nearer).
            let mut shadow_caster = None;
            let (model, blend) = match choice {
                DirChoice::Blend(g) => {
                    if let Some((fb, ab)) = &g.nearest_b {
                        let (td, ap, uv_b) = packed_geometry(fb, *ab);
                        let m = Self::compute_iso_sprite_model(
                            &iso_sprite,
                            &tf,
                            &tilemap_tf,
                            &tilemap,
                            td,
                            ap,
                        );
                        shadow_caster = Some((m, fb.sheet_name.clone(), uv_b));
                    } else if let Some(uv_a) = uv {
                        shadow_caster = Some((model, sheet_name.clone(), uv_a));
                    }
                    let m = Self::compute_iso_sprite_model(
                        &iso_sprite,
                        &tf,
                        &tilemap_tf,
                        &tilemap,
                        g.tex_dim,
                        g.anchor_px,
                    );
                    (m, Some((g.region, g.textures)))
                }
                _ => (model, None),
            };
            // The depth map is re-anchored against this window depth in the
            // fragment shader.  An anchor-relative sheet (v2) is baked about the
            // frame's ground anchor, so the base is the anchor's own depth.  A
            // legacy sheet keeps the model's translation column (the quad's
            // top-left corner, which its origin-relative bake was tuned to).
            let depth_base = if anchor_depth {
                let (td, ap) = match &blend {
                    Some((r, _)) => ((r.quad[2], r.quad[3]), Vec2::new(-r.quad[0], -r.quad[1])),
                    None => (tex_dim, anchor_px),
                };
                Self::world_depth(model.transform_point3(Vec3::new(
                    ap.x / td.0.max(f32::EPSILON),
                    ap.y / td.1.max(f32::EPSILON),
                    0.0,
                )))
            } else {
                // The legacy bake was tuned to the top-left corner of the quad
                // as it stood before sprites drew at their true height (at
                // cos30 of it above the anchor): keep that reference so a
                // legacy depth map lands exactly where it did.
                let anchor = model.transform_point3(Vec3::new(
                    anchor_px.x / tex_dim.0.max(f32::EPSILON),
                    anchor_px.y / tex_dim.1.max(f32::EPSILON),
                    0.0,
                ));
                let t = model.w_axis.truncate();
                let z = anchor.z + (t.z - anchor.z) * 0.866_025_4;
                Self::world_depth(Vec3::new(t.x, t.y, z))
            };
            let world_matrix = iso_camera_matrix();
            let depth_corners = Self::compute_iso_depth_corners(tf.position, &iso_sprite.footprint);
            // Per-sheet normal/depth companions (from the resolved frame's
            // sheet) win; fall back to the per-texture `entry.normal`/`depth`
            // manifest fields for assets not on a shared atlas.
            let depth_map = frame_ref.as_ref().and_then(|fr| fr.depth_tex.clone()).or_else(|| {
                self.texture_depths.get(&iso_sprite.texture).map(|d| d.depth_tex.clone())
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
                depth_base,
                depth_scale: frame_ref.as_ref().map_or(1.0, |fr| fr.depth_scale),
                depth_map,
                normal_map,
                ghost_group: iso_sprite.ghost_group,
                color: iso_sprite.color,
                selected: visual_selected.contains(entity),
                no_ghost: iso_sprite.no_ghost,
                team: iso_sprite
                    .team_tint
                    .filter(|_| frame_ref.as_ref().is_some_and(|fr| fr.team_mask)),
                emissive: iso_sprite.emissive.unwrap_or(1.0),
                world_matrix,
                blend,
                shadow_caster,
            });
        }

        // 3D model draws (every node mesh instance), shared by the shadow
        // casters and the model pass.
        classic_gfx::perf::phase("model_draws");
        let model_draws = self.model_draws(&items, &name_by_entity);
        classic_gfx::perf::phase("shadow_fit");

        // Fit the directional shadow matrix to the primary tilemap's extents
        // plus the sprite casters' world quads (so their shadows aren't clipped
        // at the light box's near plane).  Still before the `gfx` mutable borrow.
        let shadow_pass: Option<shadow::LightMatrix> = if config.shadows {
            self.entity_by_role(RoleKind::Tilemap).and_then(|e| {
                let tm = self.world.get::<&Tilemap>(e).ok()?;
                let tf = self.world.get::<&Transform>(e).ok()?;
                let z_max = tm.height_data.iter().cloned().fold(0.0f32, f32::max);
                let mut casters: Vec<Mat4> = iso_draws.iter().map(|d| d.model).collect();
                casters.extend(model_draws.iter().flat_map(|d| d.shadow_casters()));
                Some(shadow::fit_directional_light_matrix(
                    tf.position,
                    tm.size_x as f32,
                    tm.size_y as f32,
                    z_max,
                    Vec3::from_array(self.light_dir),
                    shadow::SHADOW_PADDING,
                    &casters,
                    classic_gfx::SHADOW_MAP_SIZE as f32,
                ))
            })
        } else {
            None
        };

        let Some(gfx) = self.gfx.as_mut() else { return };
        let vp = gfx.viewport_w;
        let vh2 = gfx.viewport_h;
        self.camera.size = Vec3::new(vp, vh2, 0.0);

        // begin_frame sets depthFunc/depthMask but does NOT glEnable(DEPTH_TEST).
        // draw_tilemap/draw_iso_sprite toggle it locally. UI/SDF runs without it.
        // Enabling it globally depth-rejects all UI under ortho projection.
        classic_gfx::perf::phase("begin_frame");
        gfx.begin_frame();
        // Upload the dynamic light block once per frame (consumed by the lit
        // tilemap + sprite shaders).
        classic_gfx::perf::phase("light_ubo");
        gfx.upload_lights(&lights);
        let cam = self.camera.matrix();
        // Instanced sprite batches (one instance per iso draw), planned so the
        // batched draws reproduce the render-list order (`sprite_batches`).
        classic_gfx::perf::phase("sprite_plan");
        let mut sprite_frame = crate::sprite_batches::plan_sprite_frame(
            &iso_draws,
            gfx,
            &cam,
            !gfx.ghost_ids_supported(),
        );
        let mut sprite_draws = 0u32;
        // Bin them into screen tiles for this camera, so a lit fragment loops
        // over the lights near it only (terrain, models and iso sprites all
        // rasterise through `cam` · `iso_camera_matrix` at `PPM_TARGET`).
        classic_gfx::perf::phase("light_grid");
        gfx.upload_light_grid(&lights, &cam, &iso_camera_matrix(), PPM_TARGET);

        // Directional shadow pass (terrain casters) — before Phase 1.  Renders
        // every non-nav tilemap into the depth texture from the sun's view.
        let shadow_settings: Option<classic_gfx::ShadowSettings> = match &shadow_pass {
            Some(m) => {
                let tex = gfx.shadow_map_texture();
                if let Some(tex) = tex {
                    classic_gfx::perf::phase("shadow_clear");
                    gfx.begin_shadow_pass();
                    classic_gfx::perf::phase("shadow_terrain");
                    for (_, entity, kind) in &items {
                        if !matches!(kind, DrawKind::Tilemap) {
                            continue;
                        }
                        let is_nav = self
                            .world
                            .get::<&Role>(*entity)
                            .is_ok_and(|r| r.value == RoleKind::NavMesh);
                        if is_nav {
                            continue;
                        }
                        let Ok(tf) = self.world.get::<&Transform>(*entity) else {
                            continue;
                        };
                        let name = name_by_entity.get(entity).copied().unwrap_or("");
                        if let Some(gpu) = self.tilemap_gpu.get(name) {
                            gfx.draw_shadow_tilemap(
                                &Mat4::from_translation(tf.position),
                                &m.view_proj,
                                gpu.vertex_count as i32,
                                &gpu.mesh_buf,
                            );
                        }
                    }
                    classic_gfx::perf::phase("shadow_models");
                    // 3D model casters: real geometry, drawn with the terrain's
                    // constant offset (before the sprite slope-scaled switch).
                    let casters = gfx.begin_shadow_models(&m.view_proj);
                    for draw in &model_draws {
                        for inst in &draw.instances {
                            if let Some(mesh) = self.model_gpu.mesh(inst.mesh) {
                                casters.draw(&inst.model, mesh);
                                classic_gfx::perf::count("shadow_model_instances", 1);
                            }
                        }
                    }
                    classic_gfx::perf::phase("shadow_sprites");
                    // Sprite shadow casters: each iso sprite casts its alpha
                    // silhouette into the shadow map (vehicles, agents, props).
                    // Billboards are their own receivers, so they need a
                    // slope-scaled offset the terrain does not.
                    gfx.set_shadow_sprite_offset();
                    gfx.upload_shadow_sprite_instances(&sprite_frame.shadow_instances);
                    sprite_draws +=
                        gfx.draw_shadow_sprite_batches(&m.view_proj, &sprite_frame.shadow_batches);
                    gfx.end_shadow_pass();
                    let texel = 1.0 / classic_gfx::SHADOW_MAP_SIZE as f32;
                    Some(classic_gfx::ShadowSettings {
                        texture: tex,
                        view_proj: m.view_proj,
                        bias: shadow::SHADOW_BIAS,
                        strength: shadow::SHADOW_STRENGTH,
                        texel: [texel, texel],
                        normal_offset: m.world_texel * shadow::SHADOW_NORMAL_OFFSET,
                        debug: config.shadow_debug,
                    })
                } else {
                    None
                }
            }
            None => None,
        };

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

        classic_gfx::perf::phase("terrain");
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
                        let world_matrix = iso_camera_matrix();
                        let nav_ts = gfx
                            .textures
                            .get(&nav.tile_set)
                            .map(|t| [t.size.0 as f32 / 8.0, t.size.1 as f32 / 8.0])
                            .unwrap_or([2.0, 1.0]);
                        let nav_rect = golden::project_rect(
                            &cam,
                            &(Mat4::from_translation(tf.position)
                                * world_matrix
                                * Mat4::from_scale(Vec3::new(
                                    nav.size_x as f32,
                                    nav.size_y as f32,
                                    1.0,
                                ))),
                            false,
                        );
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
                                normal: None,
                                screen: Some(nav_rect),
                            });
                        }
                        gfx.draw_tilemap(
                            &Mat4::from_translation(tf.position),
                            &cam,
                            &world_matrix,
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
                                depth_span: [DEPTH_NEAR, DEPTH_FAR],
                                ppm: PPM_TARGET,
                                shadow: shadow_settings,
                            },
                            None,
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
                // The rasterisation camera; lighting is done in world space.
                let world_matrix = iso_camera_matrix();

                let tps = tm.tile_pixel_size;
                let tile_pixel_size = [tps[0] as f32, tps[1] as f32];
                let tile_set_size = [
                    tm.tile_set_pixel_size[0] as f32 / tile_pixel_size[0],
                    tm.tile_set_pixel_size[1] as f32 / tile_pixel_size[1],
                ];

                let tm_rect = golden::project_rect(
                    &cam,
                    &(Mat4::from_translation(tf.position)
                        * world_matrix
                        * Mat4::from_scale(Vec3::new(tm.size_x as f32, tm.size_y as f32, 1.0))),
                    false,
                );

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
                        normal: None,
                        screen: Some(tm_rect),
                    });
                }

                gfx.draw_tilemap(
                    &Mat4::from_translation(tf.position),
                    &cam,
                    &world_matrix,
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
                        depth_span: [DEPTH_NEAR, DEPTH_FAR],
                        ppm: PPM_TARGET,
                        shadow: shadow_settings,
                    },
                    gpu.blend.as_ref(),
                    self.show_grid,
                    gpu.vertex_count as i32,
                    &gpu.mesh_buf,
                );
            }
        }

        // Phase 1b: 3D models — real geometry drawn into the pixelation target at
        // the sprite texel size (`viewport × min(1, 1/zoom)`), then composited
        // with its true camera view depth before the sprites, so a sprite behind
        // a model depth-fails its normal pass and shows through the ghost pass.
        // Skipped entirely without models (model-less frames are unchanged).
        classic_gfx::perf::phase("models");
        let has_models = model_draws.iter().any(|d| !d.instances.is_empty());
        // A glowing model also needs the bloom pass (below).
        let models_emissive = model_draws.iter().any(|d| d.has_emissive());
        // So does a sprite whose emissive pixels are pushed past 1.
        let sprites_emissive = iso_draws.iter().any(|d| d.emissive > 1.0);
        if has_models {
            gfx.begin_model_pass(self.camera.scale.x);
        }
        let model_settings = RenderSettings {
            ambient: self.light_ambient,
            light_dir: self.light_dir,
            light_color: self.light_color,
            depth_span: [DEPTH_NEAR, DEPTH_FAR],
            ppm: PPM_TARGET,
            shadow: shadow_settings,
        };
        // Trace in render-list order (unchanged goldens), then draw: opaque
        // meshes front to back (larger view depth = nearer) so early depth
        // testing rejects hidden fragments before the lighting loop, then
        // the alpha-tested ones (their `discard` disables early-Z).
        let mut order: Vec<(bool, f32, &crate::model::ModelInstance)> = Vec::new();
        for draw in &model_draws {
            for inst in &draw.instances {
                if self.model_gpu.mesh(inst.mesh).is_none() {
                    continue;
                }
                if let Some(ref mut t) = self.trace {
                    t.push(golden::TraceItemParams {
                        order: draw.order,
                        kind: "Model",
                        name: &draw.name,
                        model: &inst.model,
                        camera_ignored: false,
                        texture: self.model_gpu.texture(inst.mesh),
                        frame: None,
                        // The team tint, only when it applies (untinted
                        // models trace exactly as before).
                        color: inst.team.map(|t| [t.tint[0], t.tint[1], t.tint[2], 1.0]),
                        depth: None,
                        normal: None,
                        screen: Some(golden::project_rect(&cam, &inst.model, false)),
                    });
                }
                order.push((inst.alpha_test, iso_view_depth(inst.model.w_axis.truncate()), inst));
            }
        }
        crate::model::sort_model_draw_order(&mut order);
        classic_gfx::perf::count("model_instances", order.len() as u64);
        if !order.is_empty() {
            let run = gfx.begin_model_draws(&cam, &iso_camera_matrix(), &model_settings);
            for (_, _, inst) in order {
                let Some(mesh) = self.model_gpu.mesh(inst.mesh) else { continue };
                gfx.draw_model_instance(
                    &run,
                    &inst.model,
                    mesh,
                    self.model_gpu.texture(inst.mesh),
                    &[0.8, 0.8, 0.8, 1.0],
                    inst.team.as_ref(),
                    &inst.emissive,
                    inst.alpha_test,
                    inst.ghost,
                );
            }
            gfx.end_model_draws(run);
        }
        if has_models {
            gfx.end_model_pass();
            classic_gfx::perf::phase("model_composite");
            gfx.composite_models(IsoSpritePass::Normal, classic_gfx::MODEL_GHOST_GROUP);
        }

        classic_gfx::perf::phase("sprites");
        // Phase 2: isometric normal passes — draw on top of terrain, writing
        // depth (depth-mapped sprites) and stencil ghost-group ids.  A single
        // `RenderSettings` (shared by both sprite passes) carries the light
        // preset; the world camera matrix rides on each `IsoDraw`.
        let sprite_settings = RenderSettings {
            ambient: self.light_ambient,
            light_dir: self.light_dir,
            light_color: self.light_color,
            depth_span: [DEPTH_NEAR, DEPTH_FAR],
            ppm: PPM_TARGET,
            shadow: shadow_settings,
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
                    depth: draw.depth_map.as_deref(),
                    normal: draw.normal_map.as_deref(),
                    screen: Some(golden::project_rect(&cam, &draw.model, false)),
                });
            }
        }
        let world_matrix = iso_camera_matrix();
        if !iso_draws.is_empty() {
            gfx.upload_sprite_instances(&sprite_frame.instances);
            gfx.upload_blend_thirds(&sprite_frame.thirds);
            // The ghost groups: each instance writes its group id where it
            // passes the normal pass's depth test, for the ghost pass to read.
            // Without the depth blit, re-plan with the groups in the stencil.
            if sprite_frame.any_ghost_group && !sprite_frame.stencil_groups {
                classic_gfx::perf::phase("sprite_ids");
                if gfx.begin_ghost_ids(sprite_frame.group_bounds) {
                    classic_gfx::perf::phase("sprite_ids_draw");
                    sprite_draws += gfx.draw_ghost_id_batches(
                        &cam,
                        &world_matrix,
                        &sprite_settings,
                        &sprite_frame.batches,
                    );
                } else {
                    sprite_frame =
                        crate::sprite_batches::plan_sprite_frame(&iso_draws, gfx, &cam, true);
                    gfx.upload_sprite_instances(&sprite_frame.instances);
                    gfx.upload_blend_thirds(&sprite_frame.thirds);
                }
            }
            classic_gfx::perf::phase("sprites");
            sprite_draws += gfx.draw_iso_sprite_batches(
                &cam,
                &world_matrix,
                &sprite_settings,
                &sprite_frame.batches,
                IsoSpritePass::Normal,
                &SELECTION_COLOR,
            );
            classic_gfx::perf::phase("sprite_ghosts");
            // Phase 3: isometric ghost passes — 40% alpha where behind the
            // depth buffer, skipping pixels the sprite's own ghost group
            // already occludes.
            sprite_draws += gfx.draw_iso_sprite_batches(
                &cam,
                &world_matrix,
                &sprite_settings,
                &sprite_frame.batches,
                IsoSpritePass::Ghost,
                &SELECTION_COLOR,
            );
        }
        classic_gfx::perf::count("sprite_instances", iso_draws.len() as u64);
        classic_gfx::perf::count("sprite_batches", sprite_frame.batches.len() as u64);
        classic_gfx::perf::count("sprite_draws", sprite_draws as u64);
        classic_gfx::perf::phase("model_ghost");
        // Phase 3b: model ghost — 40% alpha wherever a model is behind sprites or
        // terrain, skipping pixels its own composite covers (like sprites).
        if has_models {
            gfx.composite_models(IsoSpritePass::Ghost, classic_gfx::MODEL_GHOST_GROUP);
        }

        classic_gfx::perf::phase("particles_draw");
        // Phase 3c: particles + ribbons — depth-tested against the scene, no
        // depth write, premultiplied blend (see `particles`).
        let particle_settings = RenderSettings {
            ambient: self.light_ambient,
            light_dir: self.light_dir,
            light_color: self.light_color,
            depth_span: [DEPTH_NEAR, DEPTH_FAR],
            ppm: PPM_TARGET,
            shadow: None,
        };
        if !self.world_hidden {
            self.particles.draw(gfx, &cam, &particle_settings, self.trace.as_mut());
        }

        classic_gfx::perf::phase("bloom");
        // Bloom (before the UI): only on frames with something emissive on
        // screen, so scenes without effects stay pixel-identical.
        if self.particles.has_emissive() || models_emissive || sprites_emissive {
            if let Some(ref mut t) = self.trace {
                t.push(golden::TraceItemParams {
                    order: 0.0,
                    kind: "Bloom",
                    name: "bloom",
                    model: &Mat4::IDENTITY,
                    camera_ignored: true,
                    texture: None,
                    frame: None,
                    color: Some([
                        particles::BLOOM_THRESHOLD,
                        particles::BLOOM_INTENSITY,
                        particles::BLOOM_KNEE,
                        0.0,
                    ]),
                    depth: None,
                    normal: None,
                    screen: None,
                });
            }
            gfx.apply_bloom(
                particles::BLOOM_THRESHOLD,
                particles::BLOOM_KNEE,
                particles::BLOOM_INTENSITY,
            );
        }

        classic_gfx::perf::phase("ui");
        // Phase 4: UI + sprites + text (no depth test — draw-order layering).
        for (order, entity, kind) in &items {
            if matches!(kind, DrawKind::Tilemap | DrawKind::IsoSprite | DrawKind::Model) {
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
                            normal: None,
                            screen: Some(golden::project_rect(
                                &cam,
                                &sprite_model,
                                sprite.ignore_cam,
                            )),
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
                            normal: None,
                            screen: Some(golden::project_rect(&cam, &model, rect.ignore_cam)),
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
                            normal: None,
                            screen: Some(golden::project_rect(&cam, &model, true)),
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
                        let old = self.sdf_text_gpu.insert(
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
                        // The replaced glyph buffer would otherwise leak.
                        if let Some(old) = old {
                            old.glyph_buf.delete(&gfx.gl);
                        }
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
                            normal: None,
                            screen: Some(golden::project_rect(&cam, &model, sdf.ignore_cam)),
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

        classic_gfx::perf::phase("tail");
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

            // --- text layout map (deterministic, GPU-free; not part of the
            // line-by-line golden comparison) ---
            if config.golden_layout {
                let layout = golden::serialize_layout(&trace);
                match config.golden_mode.as_str() {
                    "update" => {
                        let _ = std::fs::create_dir_all(&baseline_dir);
                        let layout_path = baseline_dir.join("baseline.layout.txt");
                        if let Err(e) = std::fs::write(&layout_path, &layout) {
                            classic_core::cl_warn!(
                                classic_core::instrument::Chan::Golden,
                                "golden: failed to write {}: {e}",
                                layout_path.display()
                            );
                        } else {
                            classic_core::cl_info!(
                                classic_core::instrument::Chan::Golden,
                                "golden: wrote {}",
                                layout_path.display()
                            );
                        }
                    }
                    "check" => {
                        let artifact_dir = cwd.join("target/classic-test");
                        let _ = std::fs::create_dir_all(&artifact_dir);
                        let layout_path = artifact_dir.join("baseline.layout.txt");
                        let _ = std::fs::write(&layout_path, &layout);
                    }
                    _ => {}
                }
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

        // `CLASSIC_GOLDEN_PNG=<frames>`: dump this frame's pixels for review.
        if config.golden_png_frames.contains(&self.debug_frame) {
            if let Some(gfx) = self.gfx.as_ref() {
                let path = format!("{}/frame_{:05}.png", config.golden_dir, self.debug_frame);
                golden::capture_png(gfx, &path);
            }
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

        // A shown or hidden UI element changes its parent's layout (arrays
        // stack only enabled children) and must be placed where its parent
        // is now: re-lay the tree out.
        if self.world.get::<&classic_core::components::UiNode>(entity).is_ok() {
            if let Some(ui) = self.ui.as_mut() {
                ui.mark_dirty();
            }
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
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether a render-list item still draws under [`Engine::world_hidden`]: the
/// screen-space UI layer only (UI sprites, and rects / SDF text that ignore
/// the camera).  Terrain, iso sprites, models and world sprites / labels wait
/// for the scene.
fn draws_while_world_hidden(world: &hecs::World, e: hecs::Entity, kind: &DrawKind) -> bool {
    match kind {
        DrawKind::UiSprite => true,
        DrawKind::UiRect => world.get::<&RectRender>(e).is_ok_and(|r| r.ignore_cam),
        DrawKind::SdfText => world.get::<&SdfTextRender>(e).is_ok_and(|t| t.ignore_cam),
        DrawKind::Sprite | DrawKind::Tilemap | DrawKind::IsoSprite | DrawKind::Model => false,
    }
}
