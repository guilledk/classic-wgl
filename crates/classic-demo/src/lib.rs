//! classic-demo: Application-specific prefabs, editor tools and UI bootstrap.
//!
//! Provides [`init_engine`] — the single source of truth for the demo's
//! startup sequence (used by both native and web targets).  It takes a loaded
//! [`classic_rom::Rom`]; the engine hydrates shaders/resources/state, and each
//! ROM's guest owns its own scene look (camera framing, light, grid).
//!
//! The engine (`classic-engine`) is generic: it owns the world, camera, input,
//! gfx and tilemap/nav plumbing.  Everything here — the editor HUD, tool
//! widgets, light presets, debug overlays, the CLASSIC_TEST scenario — is demo
//! content, built as free functions over `&mut Engine` + shared `DemoState`
//! and registered through the engine's hook surface.
//!
//! Two scenes ship as ROMs:
//!
//! - `demo` — the original hand-authored map (tile/nav data inlined).
//! - `lunar` — a procedurally generated lunar surface (see the `lunar-guest`
//!   ROM guest, which owns the map algorithm).  Reuses the same entity names
//!   so the whole editor toolchain keeps working.

pub mod editor;
pub mod hud;
pub mod lighting;
pub mod prefabs;
pub mod render_order;
pub mod state;
pub mod testing;

use std::cell::RefCell;
use std::rc::Rc;

use classic_core::cl_error;
use classic_core::cl_info;
use classic_core::instrument::Chan;
use classic_engine::Engine;
use classic_guest::{create_runtime, GuestLimits, GuestRuntime};
use classic_rom::Rom;

use crate::state::{DemoState, DemoStateRef};

/// Install the ROM guest runtime: instantiate the ROM's compiled guest module,
/// run its optional one-shot `init` hook synchronously (before the first
/// frame), and register a per-frame `on_update` closure that runs `update(dt)`
/// and — once, after the first update — the optional `start` hook.
pub fn init_guest(e: &mut Engine, state: &DemoStateRef, wasm: &[u8], limits: &GuestLimits) {
    // The deterministic harness forces synchronous workers so frame output is
    // independent of background-thread scheduling.
    e.set_synchronous_workers(limits.synchronous_workers);
    match create_runtime(wasm, limits) {
        Ok(mut rt) => {
            if let Err(err) = rt.init(e) {
                cl_error!(Chan::Guest, "guest init failed: {err}");
            }
            let rt: Rc<RefCell<Box<dyn GuestRuntime>>> = Rc::new(RefCell::new(rt));
            state.borrow_mut().guest = Some(rt.clone());
            let mut started = false;
            e.on_update(move |engine| {
                let dt = engine.time.delta as f64;
                let mut guest = rt.borrow_mut();
                if let Err(err) = guest.update(engine, dt) {
                    cl_error!(Chan::Guest, "guest update failed: {err}");
                }
                if !started {
                    started = true;
                    if let Err(err) = guest.start(engine) {
                        cl_error!(Chan::Guest, "guest start failed: {err}");
                    }
                }
            });
        }
        Err(err) => cl_error!(Chan::Guest, "init_guest: {err}"),
    }
}

/// Full demo engine bootstrap for a loaded ROM.
///
/// `load_rom` hydrates shaders, resources and the entity graph; the ROM guest
/// owns the scene look (terrain hydration/generation, camera framing, light,
/// grid), and the shared host layer (editor HUD, widgets, lighting default,
/// hooks, test runner) is installed on top.
pub fn init_engine(gl: Rc<glow::Context>, rom: &Rom) -> Engine {
    let mut e = Engine::new();
    e.load_rom(gl, rom);

    let state: DemoStateRef = Rc::new(RefCell::new(DemoState::default()));

    prefabs::init_cursor(&mut e);
    prefabs::init_camera_wasd(&mut e);
    prefabs::init_animator_system(&mut e);

    // Default lighting (sunny) is applied before the guest installs, so a
    // guest that sets its own look (lunar) wins over the default.
    lighting::init_lighting(&mut e, &state);

    // Install the background guest worker (Tier 3) *before* the foreground
    // guest runs its `init` hook, so the lunar guest can submit generation work
    // from `init` (and apply it synchronously under the golden harness).
    if let Some(worker_wasm) = rom.resources.code().get("worker") {
        let env = classic_engine::env_config::EnvConfig::get();
        if let Err(err) =
            e.install_guest_worker(worker_wasm, env.test_active() || env.golden_active())
        {
            cl_error!(Chan::Guest, "init_engine: install_guest_worker: {err}");
        }
    }

    // ROM guest code.  Each guest owns its terrain — the lunar guest generates
    // + bulk-uploads the grids, the demo guest commits its hand-authored inline
    // state — and then owns its own view setup.
    if let Some(wasm) = rom.resources.code().get("main") {
        let env = classic_engine::env_config::EnvConfig::get();
        let limits = GuestLimits {
            trusted: rom.manifest.trusted,
            // The deterministic harness (CLASSIC_TEST) and golden capture both
            // force synchronous workers so frame output is independent of
            // background-thread scheduling.
            synchronous_workers: env.test_active() || env.golden_active(),
            ..GuestLimits::default()
        };
        init_guest(&mut e, &state, wasm, &limits);
    } else {
        // Static scene (no guest): commit the ROM-authored grids so the
        // tilemap renders without a guest driving `commit_terrain`.
        let height_scale = e
            .entity_by_role(classic_core::RoleKind::Tilemap)
            .and_then(|te| e.world.get::<&classic_core::components::Tilemap>(te).ok())
            .map_or(32.0, |tm| tm.tile_pixel_size[0] as f32);
        e.commit_terrain(height_scale);
    }

    // Footprint colliders sample the terrain heights, so they run after the
    // guest has committed the map.
    prefabs::init_footprint_colliders(&mut e);

    if rom.manifest.host_features {
        prefabs::init_debug_toggles(&mut e, &state);
        editor::init_ui(&mut e);
        editor::init_tool_buttons(&mut e, &state);
        editor::init_height_widget(&mut e, &state);
        editor::init_vehicle_widget(&mut e);
        lighting::init_light_widget(&mut e, &state);
        // The test-light widget is interactive-only (it spawns a pooled light
        // at the mouse), so it stays out of the deterministic headless/golden
        // render path.
        let env = classic_engine::env_config::EnvConfig::get();
        if !env.headless && !env.test_active() && !env.golden_active() {
            lighting::init_test_light_widget(&mut e, &state);
        }
        editor::init_tile_palette(&mut e, &state);
        editor::init_nav_palette(&mut e, &state);
        e.init_nav_mesh_render();
        editor::init_editor_mode_control(&mut e, &state);
        e.measure_all_ui_labels();
        hud::init_text_showcase(&mut e, &state);
        hud::init_iso_coord_overlay(&mut e, &state);

        // Engine hooks — the demo owns this behaviour, registered as callbacks.
        {
            let s = state.clone();
            e.on_pre_update(move |engine| hud::route_text_scroll(engine, &s));
        }
        {
            let s = state.clone();
            e.on_selection_end(move |engine| editor::apply_editor_selection(engine, &s));
        }
        {
            let s = state.clone();
            e.add_overlay(move |engine| hud::draw_debug_overlay(engine, &s));
        }
        {
            e.add_overlay(hud::draw_rts_rubber_band);
        }
    }
    testing::install(&mut e, &state);

    cl_info!(Chan::Frame, "classic-demo initialized (entrypoint={})", rom.manifest.entrypoint);
    e
}
