//! classic-demo: Application-specific prefabs, editor tools and UI bootstrap.
//!
//! Provides [`init_engine`] — the single source of truth for the demo's
//! startup sequence (used by both native and web targets).  It takes a loaded
//! [`classic_rom::Rom`]; the engine hydrates shaders/resources/state, and the
//! scene assemblers (`scenes::demo` / `scenes::lunar`, selected by the ROM's
//! `entrypoint`) add their terrain/nav/view setup on top.
//!
//! The engine (`classic-engine`) is generic: it owns the world, camera, input,
//! gfx and tilemap/nav plumbing.  Everything here — the editor HUD, tool
//! widgets, light presets, debug overlays, the CLASSIC_TEST scenario and the
//! scenes — is demo content, built as free functions over `&mut Engine` +
//! shared `DemoState` and registered through the engine's hook surface.
//!
//! Two scenes ship as ROMs:
//!
//! - `demo` — the original hand-authored map (tile/nav data inlined).
//! - `lunar` — a procedurally generated lunar surface (see
//!   `classic_core::terrain::lunar`).  Reuses the same entity names so the
//!   whole editor toolchain keeps working.

pub mod editor;
pub mod hud;
pub mod lighting;
pub mod prefabs;
pub mod scenes;
pub mod state;
pub mod testing;

use std::cell::RefCell;
use std::rc::Rc;

use classic_core::cl_error;
use classic_core::cl_info;
use classic_core::instrument::Chan;
use classic_core::terrain::lunar::LunarParams;
use classic_engine::Engine;
use classic_guest::{GuestLimits, GuestRuntime, WasmiRuntime};
use classic_rom::Rom;

use crate::state::{DemoState, DemoStateRef};

/// True when the ROM's `entrypoint` names the procedurally generated lunar
/// scene (anything else — including the default empty entrypoint — is the
/// hand-authored demo scene).
fn is_lunar(rom: &Rom) -> bool {
    matches!(rom.manifest.entrypoint.as_str(), "lunar" | "moon")
}

/// Install the ROM guest runtime: instantiate the ROM's compiled guest module
/// and register a per-frame `on_update` closure that runs `update(dt)`.
pub fn init_guest(e: &mut Engine, state: &DemoStateRef, wasm: &[u8], limits: &GuestLimits) {
    match WasmiRuntime::new(wasm, limits) {
        Ok(rt) => {
            let rt: Rc<RefCell<Box<dyn GuestRuntime>>> = Rc::new(RefCell::new(Box::new(rt)));
            state.borrow_mut().guest = Some(rt.clone());
            e.on_update(move |engine| {
                let dt = engine.time.delta as f64;
                if let Err(err) = rt.borrow_mut().update(engine, dt) {
                    cl_error!(Chan::Guest, "guest update failed: {err}");
                }
            });
        }
        Err(err) => cl_error!(Chan::Guest, "init_guest: {err}"),
    }
}

/// Full demo engine bootstrap for a loaded ROM.
///
/// `load_rom` hydrates shaders, resources and the entity graph; the scene
/// assemblers (`scenes::demo` / `scenes::lunar`) add their terrain/nav/view
/// setup, and the shared host layer (editor HUD, widgets, lighting, hooks,
/// test runner) is installed on top.
pub fn init_engine(gl: Rc<glow::Context>, rom: &Rom) -> Engine {
    let mut e = Engine::new();
    e.load_rom(gl, rom);

    let state: DemoStateRef = Rc::new(RefCell::new(DemoState::default()));
    let lunar = is_lunar(rom);

    if lunar {
        // Generates terrain, nav data and the tileset texture, and installs
        // all three.  Must precede `hydrate_nav`.
        scenes::lunar::init_lunar_terrain(&mut e, &state, LunarParams::default());
    } else {
        scenes::demo::hydrate_terrain(&mut e);
    }

    prefabs::init_cursor(&mut e);
    prefabs::init_camera_wasd(&mut e);
    prefabs::init_animator_system(&mut e);
    prefabs::init_footprint_colliders(&mut e);

    if lunar {
        scenes::lunar::hydrate_nav(&mut e, &state);
    } else {
        scenes::demo::hydrate_nav(&mut e);
    }

    if rom.manifest.host_features {
        prefabs::init_debug_toggles(&mut e, &state);
        editor::init_ui(&mut e);
        editor::init_tool_buttons(&mut e, &state);
        editor::init_height_widget(&mut e, &state);
        lighting::init_light_widget(&mut e, &state);
        editor::init_tile_palette(&mut e, &state);
        editor::init_nav_palette(&mut e, &state);
        e.init_nav_mesh_render();
        editor::init_editor_mode_control(&mut e, &state);
        e.measure_all_ui_labels();
        lighting::init_lighting(&mut e, &state);
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
        testing::install(&mut e, &state);
    }

    // ROM guest code (a no-op module in the shipped demo/lunar ROMs; see the
    // WASM guest plan).  The module ships inside the ROM archive.
    if let Some(wasm) = rom.resources.code().get("main") {
        let limits = GuestLimits { trusted: rom.manifest.trusted, ..GuestLimits::default() };
        init_guest(&mut e, &state, wasm, &limits);
    }

    if lunar {
        scenes::lunar::setup_view(&mut e, &state);
    } else {
        scenes::demo::setup_view(&mut e);
    }

    cl_info!(Chan::Frame, "classic-demo initialized (entrypoint={})", rom.manifest.entrypoint);
    e
}
