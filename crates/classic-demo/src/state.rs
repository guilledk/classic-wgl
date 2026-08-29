//! Demo-owned state: editor tool state and the widget entity handles that
//! previously lived as fields on `Engine`.
//!
//! Splitting these out of `Engine` is what makes the engine demo-neutral:
//! `Engine` owns the world, camera, input, gfx and generic tilemap/nav
//! plumbing; `DemoState` owns the editor UI and the per-scene entities it
//! builds.  Widgets share one `Rc<RefCell<DemoState>>` (the established
//! shared-state pattern) so their `on_update` closures can read and write it
//! without touching `Engine`.

use std::cell::RefCell;
use std::rc::Rc;

use classic_gfx::GlBuffer;
use hecs::Entity;

/// Editor tool state — aggregated so widget closures can share a single
/// `Rc<RefCell<EditorState>>` instead of 11 individual `Rc<Cell>` / `Rc<RefCell>`.
#[derive(Clone, Debug)]
pub struct EditorState {
    pub target: String,
    pub tile: u32,
    pub nav_tile: u32,
    pub height: i32,
    pub height_scale: i32,
    pub height_mode: String,
    pub panel_menu_open: bool,
    pub agent_selected: bool,
    pub debug_footprints: bool,
    pub light_preset: String,
    pub light_azimuth: f32,
    pub light_elevation: f32,
    /// Toggle a persistent test point light (placed at the mouse's iso tile).
    pub test_light: bool,
}

impl Default for EditorState {
    fn default() -> Self {
        Self {
            target: "none".into(),
            tile: 0,
            nav_tile: 0,
            height: 0,
            height_scale: 1,
            height_mode: "blend".into(),
            panel_menu_open: false,
            agent_selected: false,
            debug_footprints: false,
            light_preset: "sunny".into(),
            light_azimuth: 45.0,
            light_elevation: 45.0,
            test_light: false,
        }
    }
}

/// Shared demo application state.
pub struct DemoState {
    pub editor: EditorState,
    pub tile_palette_e: Option<Entity>,
    pub nav_palette_e: Option<Entity>,
    pub height_widget_e: Option<Entity>,
    pub light_widget_e: Option<Entity>,
    pub text_showcase_e: Option<Entity>,
    pub text_demo_content_h: f32,
    pub menu_panel_e: Option<Entity>,
    pub iso_compass_buf: Option<GlBuffer>,
    pub iso_coord_x_e: Option<Entity>,
    pub iso_coord_y_e: Option<Entity>,
    pub iso_coord_z_e: Option<Entity>,
    /// Handle of the demo's persistent test point light (spawned by the
    /// light-widget test toggle), so it can be released/updated on toggle off.
    pub test_light_handle: Option<u32>,
    /// Draw each active light as an X marker + a vertical Z line from the
    /// terrain surface to the light (KeyL).
    pub debug_lights: bool,
    /// The ROM guest runtime (installed by `init_guest`).
    pub guest: Option<Rc<RefCell<Box<dyn classic_guest::GuestRuntime>>>>,
}

impl Default for DemoState {
    fn default() -> Self {
        Self {
            editor: EditorState::default(),
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
            test_light_handle: None,
            debug_lights: false,
            guest: None,
        }
    }
}

/// Convenience alias for the shared-state handle the widgets use.
pub type DemoStateRef = Rc<RefCell<DemoState>>;
