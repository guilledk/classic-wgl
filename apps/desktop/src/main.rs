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
                let mut e = classic_demo::init_engine(
                    gl,
                    MANIFEST_JSON,
                    STATE_JSON,
                    TILESET_PNG,
                    MAP_DATA,
                    NAV_DATA,
                    SDF_ATLAS_PNG,
                    SDF_METRICS_JSON,
                    SEMAPHORE01_PNG,
                    SEMAPHORE02_PNG,
                    HOUSE01_PNG,
                    CURSOR_PNG,
                    HUMANOID_PNG,
                    COOL_SNEK_PNG,
                    TREE_PNG,
                    EDITOR_ICONS_PNG,
                    NAV_TILESET_PNG,
                );
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
            engine = Some(classic_demo::init_engine(
                gl,
                MANIFEST_JSON,
                STATE_JSON,
                TILESET_PNG,
                MAP_DATA,
                NAV_DATA,
                SDF_ATLAS_PNG,
                SDF_METRICS_JSON,
                SEMAPHORE01_PNG,
                SEMAPHORE02_PNG,
                HOUSE01_PNG,
                CURSOR_PNG,
                HUMANOID_PNG,
                COOL_SNEK_PNG,
                TREE_PNG,
                EDITOR_ICONS_PNG,
                NAV_TILESET_PNG,
            ));
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