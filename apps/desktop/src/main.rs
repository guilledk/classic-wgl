const STATE_JSON: &str = include_str!("../../../public/state.json");
const MANIFEST_JSON: &str = include_str!("../../../public/manifest.json");
const TILESET_PNG: &[u8] = include_bytes!("../../../public/res/road_tileset.png");
const MAP_DATA: &str = include_str!("../../../public/map001.txt");
const NAV_DATA: &str = include_str!("../../../public/map001.nav.txt");
const SDF_ATLAS_PNG: &[u8] = include_bytes!("../../../public/res/dejavusans-sdf.png");
const SDF_METRICS_JSON: &str = include_str!("../../../public/res/dejavusans-sdf.json");
const SEMAPHORE01_PNG: &[u8] = include_bytes!("../../../public/res/semaphore01.png");
const SEMAPHORE02_PNG: &[u8] = include_bytes!("../../../public/res/semaphore02.png");
const HOUSE01_PNG: &[u8] = include_bytes!("../../../public/res/house01.png");
const CURSOR_PNG: &[u8] = include_bytes!("../../../public/res/cursor.png");
const HUMANOID_PNG: &[u8] = include_bytes!("../../../public/res/humanoid.png");
const COOL_SNEK_PNG: &[u8] = include_bytes!("../../../public/res/cool_snek.png");
const TREE_PNG: &[u8] = include_bytes!("../../../public/res/tree.png");
const EDITOR_ICONS_PNG: &[u8] = include_bytes!("../../../public/res/editor_icons.png");
const NAV_TILESET_PNG: &[u8] = include_bytes!("../../../public/res/nav_tileset.png");

use std::rc::Rc;

fn init_engine(gl: Rc<glow::Context>) -> classic_engine::Engine {
    let mut e = classic_engine::Engine::new();
    e.init_gfx(gl, MANIFEST_JSON);
    e.load_state(STATE_JSON).expect("load state.json");
    e.init_tilemap("tilemap", TILESET_PNG, MAP_DATA);
    e.load_sdf_font("dejavusans-sdf", SDF_METRICS_JSON, SDF_ATLAS_PNG);
    e.load_texture_png("semaphore01", SEMAPHORE01_PNG);
    e.load_texture_png("semaphore02", SEMAPHORE02_PNG);
    e.load_texture_png("house", HOUSE01_PNG);
    e.load_texture_png("cursor", CURSOR_PNG);
    e.load_texture_png("humanoid", HUMANOID_PNG);
    e.load_texture_png("coolSnake", COOL_SNEK_PNG);
    e.load_texture_png("tree", TREE_PNG);
    e.load_texture_png("editorIcons", EDITOR_ICONS_PNG);
    e.load_texture_png("navTileset", NAV_TILESET_PNG);
    e.init_cursor();
    e.init_camera_wasd();
    e.init_animator_system();
    e.init_agent_system();
    e.init_footprint_colliders();
    e.init_navigation(NAV_DATA);
    e.init_debug_toggles();
    e.init_ui();
    e.init_tool_buttons();
    e.init_height_widget();
    e.init_light_widget();
    e.init_tile_palette();
    e.init_nav_palette();
    e.init_nav_mesh_render();
    e.init_editor_mode_control();
    e.measure_all_ui_labels();
    e.init_lighting();
    e.init_text_showcase();
    e.init_iso_coord_overlay();

    let mut iso = classic_core::math::cartesian_to_iso_4().inverse();
    iso = glam::Mat4::from_scale(glam::Vec3::new(45.0, 45.0, 1.0)) * iso;
    let origin = iso.transform_point3(glam::Vec3::new(32.0, 13.0, 0.0));
    e.camera.position.x = origin.x;
    e.camera.position.y = origin.y;
    e.show_grid = true;

    e
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    use classic_platform::Platform;
    use std::cell::Cell;
    use std::rc::Rc;

    env_logger::init();
    classic_core::cl_info!(
        classic_core::instrument::Chan::Platform,
        "classic-wgl desktop starting"
    );

    let config = classic_engine::env_config::EnvConfig::get();

    if config.headless {
        let w = config.forced_width.unwrap_or(1280.0) as u32;
        let h = config.forced_height.unwrap_or(720.0) as u32;
        let platform =
            classic_platform::headless::HeadlessPlatform::new(w, h).expect("headless platform");
        let mut engine: Option<classic_engine::Engine> = None;
        let test_failed = Rc::new(Cell::new(false));
        let tf = test_failed.clone();

        platform.run_loop(move |gl, input, vw, vh, delta, should_close| {
            if engine.is_none() {
                classic_core::cl_info!(
                    classic_core::instrument::Chan::Platform,
                    "headless: initialising engine"
                );
                let mut e = init_engine(gl);
                if let Some(gfx) = e.gfx.as_mut() {
                    gfx.set_render_target(vw as u32, vh as u32);
                }
                engine = Some(e);
            }
            if let Some(e) = engine.as_mut() {
                e.frame(input, vw, vh, delta);
                if e.test_should_close {
                    *should_close = true;
                }
                if e.test_failed {
                    tf.set(true);
                }
            }
        });

        if test_failed.get() {
            std::process::exit(1);
        }
        return;
    }

    // -- Normal windowed native path -----------------------------------------

    let max_frames: Option<u64> = std::env::var("CLASSIC_FRAMES").ok().and_then(|v| v.parse().ok());
    let mut frame_count: u64 = 0;

    let platform = classic_platform::native::NativePlatform::new();
    let mut engine: Option<classic_engine::Engine> = None;
    let test_failed = Rc::new(Cell::new(false));
    let tf = test_failed.clone();

    platform.run_loop(move |gl, input, vw, vh, _delta, should_close| {
        if engine.is_none() {
            engine = Some(init_engine(gl));
        }
        if let Some(e) = engine.as_mut() {
            if let Some(limit) = max_frames {
                if frame_count >= limit {
                    classic_core::cl_info!(
                        classic_core::instrument::Chan::Platform,
                        "CLASSIC_FRAMES={limit} reached, exiting"
                    );
                    *should_close = true;
                    return;
                }
                frame_count += 1;
            }
            e.frame(input, vw, vh, _delta);
            if e.test_should_close {
                *should_close = true;
            }
            if e.test_failed {
                tf.set(true);
            }
        }
    });

    if test_failed.get() {
        std::process::exit(1);
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {}
