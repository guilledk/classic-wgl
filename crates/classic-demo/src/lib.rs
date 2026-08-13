//! classic-demo: Application-specific prefabs, editor tools and UI bootstrap.
//!
//! Provides [`init_engine`] — the single source of truth for the demo's
//! startup sequence (used by both native and web targets).
//!
//! The engine (`classic-engine`) is generic: it owns the world, camera, input,
//! gfx and tilemap/nav plumbing.  Everything here — the editor HUD, tool
//! widgets, light presets, debug overlays, the CLASSIC_TEST scenario and the
//! scenes — is demo content, built as free functions over `&mut Engine` +
//! shared `DemoState` and registered through the engine's hook surface.
//!
//! Two scenes are available, selected by name:
//!
//! - `demo` — the original hand-authored map loaded from `state.json`
//!   (tile/nav data inlined).
//! - `lunar` — a procedurally generated lunar surface (see
//!   `classic_core::terrain::lunar`).  Uses `state_lunar.json`, which reuses
//!   the same entity names so the whole editor toolchain keeps working.

pub mod editor;
pub mod hud;
pub mod lighting;
pub mod prefabs;
pub mod scenes;
pub mod state;
pub mod testing;

use std::cell::RefCell;
use std::rc::Rc;

use classic_core::cl_info;
use classic_core::instrument::Chan;
use classic_core::terrain::lunar::LunarParams;
use classic_engine::Engine;
use classic_rom::{AssetLoader, ResourceSet, RomManifest};

use crate::state::{DemoState, DemoStateRef};

/// Scene selector.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Scene {
    #[default]
    Demo,
    Lunar,
}

impl Scene {
    /// Parse a scene name from `CLASSIC_SCENE` / the `?scene=` query param.
    /// Anything unrecognised falls back to [`Scene::Demo`].
    pub fn parse(name: &str) -> Self {
        match name.trim().to_ascii_lowercase().as_str() {
            "lunar" | "moon" => Scene::Lunar,
            _ => Scene::Demo,
        }
    }
}

/// Full demo engine bootstrap for the named scene.
///
/// Resources are resolved through an [`AssetLoader`] (the apps supply an
/// embedded map at compile time): the `manifest.json` is parsed, a
/// [`ResourceSet`] is built from it, and the engine hydrates shaders,
/// textures, fonts and animations from that set.
pub fn init_engine(gl: Rc<glow::Context>, loader: &dyn AssetLoader, scene: Scene) -> Engine {
    let manifest: RomManifest =
        serde_json::from_str(&loader.load_string("/manifest.json").expect("load manifest.json"))
            .expect("parse manifest.json");
    let resources =
        ResourceSet::from_loader(loader, &manifest).expect("build resource set from manifest");

    let mut e = Engine::new();
    e.init_gfx(gl, &manifest, &resources);

    let state: DemoStateRef = Rc::new(RefCell::new(DemoState::default()));

    match scene {
        Scene::Demo => {
            let state_json = loader.load_string("/state.json").expect("load state.json");
            e.load_state(&state_json).expect("load state.json");
            e.init_tilemap("tilemap");
        }
        Scene::Lunar => {
            let state_json =
                loader.load_string("/state_lunar.json").expect("load state_lunar.json");
            e.load_state(&state_json).expect("load state_lunar.json");
            // Generates terrain, nav data and the tileset texture, and
            // installs all three.  Must precede `init_navigation`.
            scenes::lunar::init_lunar_terrain(&mut e, &state, LunarParams::default());
        }
    }

    prefabs::init_cursor(&mut e);
    prefabs::init_camera_wasd(&mut e);
    prefabs::init_animator_system(&mut e);
    prefabs::init_agent_system(&mut e);
    prefabs::init_footprint_colliders(&mut e);

    match scene {
        Scene::Demo => e.init_navigation(),
        // The generator already derived walkability from real terrain slope
        // and guaranteed every spawn is mutually reachable; re-deriving it
        // from the coarse height rule here would undo that.
        Scene::Lunar => {
            let nav =
                state.borrow().lunar.as_ref().map(|s| s.terrain.nav.clone()).unwrap_or_default();
            e.init_navigation_data(nav);
        }
    }

    prefabs::init_debug_toggles(&mut e, &state);
    editor::init_ui(&mut e);
    editor::init_tool_buttons(&mut e, &state);
    editor::init_height_widget(&mut e, &state);
    lighting::init_light_widget(&mut e, &state);
    scenes::lunar::init_lunar_widget(&mut e, &state);
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

    match scene {
        Scene::Demo => {
            let mut iso = classic_core::math::cartesian_to_iso_4().inverse();
            iso = glam::Mat4::from_scale(glam::Vec3::new(45.0, 45.0, 1.0)) * iso;
            let origin = iso.transform_point3(glam::Vec3::new(32.0, 13.0, 0.0));
            e.camera.position.x = origin.x;
            e.camera.position.y = origin.y;
            e.show_grid = true;
        }
        Scene::Lunar => {
            // Zoom out: at scale 1.0 a 45px tile fills the view with ~28 tiles,
            // which shows none of the terrain the generator produces.
            e.camera.scale = glam::Vec3::new(0.32, 0.32, 1.0);
            scenes::lunar::focus_camera_on_spawn(&mut e, &state);
            // Airless lighting: near-zero ambient and a hard low sun, which is
            // what makes the crater relief legible.
            lighting::apply_light_preset(&mut e, &state, "lunar");
            // The editor grid overlay fights the natural surface.
            e.show_grid = false;
        }
    }

    cl_info!(Chan::Frame, "classic-demo initialized ({scene:?} scene)");
    e
}
