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

#[cfg(not(target_arch = "wasm32"))]
mod module_cache;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use classic_core::cl_error;
use classic_core::cl_info;
use classic_core::instrument::Chan;
use classic_engine::Engine;
use classic_guest::{create_runtime, GuestLimits, GuestRuntime};
use classic_rom::{BootEvent, BootSink, LoadedRom, LoadedRoms, Rom};

use crate::state::{DemoState, DemoStateRef};

/// Compiled native guest modules keyed by ROM resolver name, plus the optional
/// compiled Tier-3 worker module for the root ROM — the off-main-thread half of
/// guest init (see [`compile_guest_modules`]).  Both are empty/`None` on web,
/// where guests and the worker compile inline.
///
/// The fields are read on the native async path only (`init_guests` +
/// `install_worker`); on web the payload is always empty and both fields are
/// dead.
#[allow(dead_code)]
pub struct CompiledModules {
    modules: HashMap<String, classic_guest::CompiledModule>,
    worker: Option<classic_worker::CompiledWorker>,
}

impl CompiledModules {
    /// An empty payload — the inline-compile path (sync / headless / golden /
    /// web), where nothing was pre-compiled off-thread.
    pub fn new() -> Self {
        Self { modules: HashMap::new(), worker: None }
    }
}

impl Default for CompiledModules {
    fn default() -> Self {
        Self::new()
    }
}

/// Install a single ROM guest runtime: instantiate the ROM's compiled guest
/// module against the given namespace, run its optional one-shot `init` hook
/// synchronously (before the first frame), and register a per-frame `on_update`
/// closure that runs `update(dt)` and — once, after the first update — the
/// optional `start` hook.
pub fn init_guest(
    e: &mut Engine,
    state: &DemoStateRef,
    wasm: &[u8],
    limits: &GuestLimits,
    namespace: &str,
    rom: &str,
    sink: &dyn BootSink,
) {
    // The deterministic harness forces synchronous workers so frame output is
    // independent of background-thread scheduling.
    e.set_synchronous_workers(limits.synchronous_workers);
    sink.on_event(BootEvent::GuestCompiling { rom: rom.to_string() });
    match create_runtime(wasm, limits) {
        Ok(rt) => install_guest_runtime(e, state, rt, namespace, rom, sink),
        Err(err) => cl_error!(Chan::Guest, "init_guest: {err}"),
    }
}

/// Install a guest from a module already compiled off-thread.  The
/// `GuestCompiling` event was emitted on the background thread during
/// [`compile_guest_modules`]; only instantiation happens here (GL thread).
#[cfg(not(target_arch = "wasm32"))]
pub fn init_guest_compiled(
    e: &mut Engine,
    state: &DemoStateRef,
    module: &classic_guest::CompiledModule,
    limits: &GuestLimits,
    namespace: &str,
    rom: &str,
    sink: &dyn BootSink,
) {
    e.set_synchronous_workers(limits.synchronous_workers);
    match classic_guest::create_runtime_from_module(module, limits) {
        Ok(rt) => install_guest_runtime(e, state, rt, namespace, rom, sink),
        Err(err) => cl_error!(Chan::Guest, "init_guest: {err}"),
    }
}

/// The shared tail of guest install: emit `GuestInstantiated`, run the
/// one-shot `init` hook synchronously, and register the per-frame
/// `update`/`start` closure.
fn install_guest_runtime(
    e: &mut Engine,
    state: &DemoStateRef,
    mut rt: Box<dyn GuestRuntime>,
    namespace: &str,
    rom: &str,
    sink: &dyn BootSink,
) {
    sink.on_event(BootEvent::GuestInstantiated { rom: rom.to_string() });
    rt.set_namespace(namespace);
    if let Err(err) = rt.init(e) {
        cl_error!(Chan::Guest, "guest init failed: {err}");
    }
    let rt: Rc<RefCell<Box<dyn GuestRuntime>>> = Rc::new(RefCell::new(rt));
    state.borrow_mut().guests.push(rt.clone());
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

/// The per-ROM guest limits used by both the off-thread compile and the on-
/// thread instantiate, so the two halves agree on fuel/memory/sync config.
fn guest_limits(entry: &LoadedRom) -> GuestLimits {
    let env = classic_engine::env_config::EnvConfig::get();
    GuestLimits {
        trusted: entry.rom.manifest.trusted,
        // The deterministic harness (CLASSIC_TEST) and golden capture both
        // force synchronous workers so frame output is independent of
        // background-thread scheduling.
        synchronous_workers: env.test_active() || env.golden_active(),
        ..GuestLimits::default()
    }
}

/// Compile every guest module in the DAG off the main thread (native
/// wasmtime), keyed by ROM resolver name, plus the root ROM's Tier-3 worker
/// module.  Emits `GuestCompiling` per compiled foreground guest.  On web this
/// returns an empty payload (guests and the worker compile inline).
pub fn compile_guest_modules(loaded: &LoadedRoms, sink: &dyn BootSink) -> CompiledModules {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut modules = HashMap::new();
        for entry in &loaded.order {
            let Some(wasm) = entry.rom.resources.code().get("main") else { continue };
            let limits = guest_limits(entry);
            sink.on_event(BootEvent::GuestCompiling { rom: entry.name.clone() });
            match module_cache::load_or_compile(entry, wasm, &limits) {
                Ok(module) => {
                    modules.insert(entry.name.clone(), module);
                }
                Err(err) => cl_error!(Chan::Guest, "compile guest `{}`: {err}", entry.name),
            }
        }
        let worker = compile_worker_module(loaded);
        CompiledModules { modules, worker }
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (loaded, sink);
        CompiledModules::new()
    }
}

/// Compile the root ROM's Tier-3 worker module off-thread (native wasmtime).
/// Returns `None` when the root ships no worker wasm (or on compile error).
#[cfg(not(target_arch = "wasm32"))]
fn compile_worker_module(loaded: &LoadedRoms) -> Option<classic_worker::CompiledWorker> {
    let root = loaded.root_rom()?;
    let wasm = root.resources.code().get("worker")?;
    match classic_worker::CompiledWorker::compile(wasm) {
        Ok(compiled) => Some(compiled),
        Err(err) => {
            cl_error!(Chan::Guest, "compile worker guest `{}`: {err}", loaded.root);
            None
        }
    }
}

/// Install a guest for every ROM in the DAG that ships a `main` code module, in
/// topological order (deps first), so a dependent scene's guest `init` can
/// reference dependency entities at init and per-frame `update`s run deps
/// before dependents.
pub fn init_guests(
    e: &mut Engine,
    state: &DemoStateRef,
    loaded: &LoadedRoms,
    compiled: &CompiledModules,
    sink: &dyn BootSink,
) {
    // Web compiles guests inline, so the pre-compiled map is always empty there.
    #[cfg(target_arch = "wasm32")]
    let _ = compiled;

    for entry in &loaded.order {
        let Some(wasm) = entry.rom.resources.code().get("main") else { continue };
        let ns = e.rom_namespace(&entry.name);
        let limits = guest_limits(entry);
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(module) = compiled.modules.get(&entry.name) {
            init_guest_compiled(e, state, module, &limits, &ns, &entry.name, sink);
            continue;
        }
        init_guest(e, state, wasm, &limits, &ns, &entry.name, sink);
    }
}

/// Install the background guest worker (Tier 3) for the root ROM.  Uses the
/// off-thread-compiled worker module when present (the async native path);
/// otherwise compiles inline (the sync / headless / golden / web path).
#[cfg(not(target_arch = "wasm32"))]
fn install_worker(e: &mut Engine, loaded: &LoadedRoms, compiled: &CompiledModules, sync: bool) {
    let Some(root) = loaded.root_rom() else { return };
    let Some(worker_wasm) = root.resources.code().get("worker") else { return };
    let result = match &compiled.worker {
        Some(compiled_worker) => e.install_guest_worker_compiled(compiled_worker, sync),
        None => e.install_guest_worker(worker_wasm, sync),
    };
    if let Err(err) = result {
        cl_error!(Chan::Guest, "init_engine: install_guest_worker: {err}");
    }
}

/// Web variant: the worker compiles inline (browser-native wasm in a Worker, or
/// wasmi in sync mode), so there is never a pre-compiled module to install.
#[cfg(target_arch = "wasm32")]
fn install_worker(e: &mut Engine, loaded: &LoadedRoms, compiled: &CompiledModules, sync: bool) {
    let _ = compiled;
    let Some(root) = loaded.root_rom() else { return };
    let Some(worker_wasm) = root.resources.code().get("worker") else { return };
    if let Err(err) = e.install_guest_worker(worker_wasm, sync) {
        cl_error!(Chan::Guest, "init_engine: install_guest_worker: {err}");
    }
}

/// Full demo engine bootstrap for a loaded multi-ROM dependency DAG.
///
/// `load_roms` hydrates shaders, resources and the entity graph (deps before
/// dependents); each ROM's guest owns its own scene look, and the shared host
/// layer (editor HUD, widgets, lighting default, hooks, test runner) is
/// installed on top.
pub fn init_engine_multi(
    gl: Rc<glow::Context>,
    loaded: &LoadedRoms,
    sink: &dyn BootSink,
) -> Engine {
    let mut e = Engine::new();
    e.load_roms(gl, loaded, sink);
    finish_init_engine(&mut e, loaded, &CompiledModules::new(), sink);
    e
}

/// The shared post-load tail of [`init_engine_multi`] (and its async variant):
/// cursor/camera/animator prefabs, default lighting, the background guest
/// worker, the ROM guests, terrain commit, colliders, and the editor/HUD host
/// layer.  Public so an incremental caller (e.g. the web app interleaving
/// [`classic_engine::Engine::boot_step`]s across frames) can finish boot after
/// its plan drains.
pub fn finish_init_engine(
    e: &mut Engine,
    loaded: &LoadedRoms,
    compiled: &CompiledModules,
    sink: &dyn BootSink,
) {
    let state: DemoStateRef = Rc::new(RefCell::new(DemoState::default()));

    prefabs::init_cursor(e);
    prefabs::init_camera_wasd(e);
    prefabs::init_animator_system(e);

    // Default lighting (sunny) is applied before the guest installs, so a
    // guest that sets its own look (lunar) wins over the default.
    lighting::init_lighting(e, &state);

    // Install the background guest worker (Tier 3) *before* the foreground
    // guests run their `init` hook, so a generating guest can submit work from
    // `init`.  Worker code stays root-only for now (per-ROM workers deferred).
    {
        let env = classic_engine::env_config::EnvConfig::get();
        install_worker(e, loaded, compiled, env.test_active() || env.golden_active());
    }

    // ROM guest code.  Each guest owns its terrain — a generating guest
    // bulk-uploads the grids, a hand-authored guest commits its inline state —
    // and then owns its own view setup.
    init_guests(e, &state, loaded, compiled, sink);

    // Static scene (no guest): commit the ROM-authored grids so the tilemap
    // renders without a guest driving `commit_terrain`.
    let has_main = loaded.order.iter().any(|entry| entry.rom.resources.code().contains_key("main"));
    if !has_main {
        let height_scale = e
            .entity_by_role(classic_core::RoleKind::Tilemap)
            .and_then(|te| e.world.get::<&classic_core::components::Tilemap>(te).ok())
            .map_or(32.0, |tm| tm.tile_pixel_size[0] as f32);
        e.commit_terrain(height_scale);
    }

    // Footprint colliders sample the terrain heights, so they run after the
    // guest has committed the map.
    prefabs::init_footprint_colliders(e);

    // `CLASSIC_NO_UI` suppresses the whole editor/HUD/overlay layer so a capture
    // shows only the lit scene — the reference frame for lighting/shadow work,
    // where the SDF panel and HUD would otherwise occlude a third of the view.
    let host_features = loaded.root_rom().map(|r| r.manifest.host_features).unwrap_or(false);
    if host_features && !classic_engine::env_config::EnvConfig::get().no_ui {
        prefabs::init_debug_toggles(e, &state);
        editor::init_ui(e);
        editor::init_tool_buttons(e, &state);
        editor::init_height_widget(e, &state);
        editor::init_vehicle_widget(e);
        lighting::init_light_widget(e, &state);
        // The test-light widget is interactive-only (it spawns a pooled light
        // at the mouse), so it stays out of the deterministic headless/golden
        // render path.
        let env = classic_engine::env_config::EnvConfig::get();
        if !env.headless && !env.test_active() && !env.golden_active() {
            lighting::init_test_light_widget(e, &state);
        }
        editor::init_tile_palette(e, &state);
        editor::init_nav_palette(e, &state);
        e.init_nav_mesh_render();
        editor::init_editor_mode_control(e, &state);
        e.measure_all_ui_labels();
        hud::init_text_showcase(e, &state);
        hud::init_iso_coord_overlay(e, &state);

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
    testing::install(e, &state);

    let entrypoint = loaded.root_rom().map(|r| r.manifest.entrypoint.clone()).unwrap_or_default();
    cl_info!(Chan::Frame, "classic-demo initialized (entrypoint={})", entrypoint);
}

/// Full demo engine bootstrap for a single loaded ROM (the legacy path).  Wraps
/// the ROM in a one-entry [`LoadedRoms`] and delegates to
/// [`init_engine_multi`], so both paths share one boot sequence.
pub fn init_engine(gl: Rc<glow::Context>, rom: &Rom) -> Engine {
    let name = if rom.manifest.entrypoint.is_empty() {
        "root".to_string()
    } else {
        rom.manifest.entrypoint.clone()
    };
    let loaded = LoadedRoms {
        root: name.clone(),
        order: vec![classic_rom::LoadedRom {
            name,
            namespace: rom.manifest.namespace.clone(),
            rom: rom.clone(),
            sha256: None,
        }],
    };
    init_engine_multi(gl, &loaded, &classic_rom::NullBootSink)
}
