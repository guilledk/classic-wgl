use classic_demo::DemoAssets;

/// Every embedded asset, resolved at compile time.
const ASSETS: DemoAssets<'static> = DemoAssets {
    manifest_json: include_str!("../../../public/manifest.json"),
    state_json: include_str!("../../../public/state.json"),
    state_lunar_json: include_str!("../../../public/state_lunar.json"),
    tileset_png: include_bytes!("../../../public/res/road_tileset.png"),
    sdf_atlas_png: include_bytes!("../../../public/res/dejavusans-sdf.png"),
    sdf_metrics_json: include_str!("../../../public/res/dejavusans-sdf.json"),
    semaphore01_png: include_bytes!("../../../public/res/semaphore01.png"),
    semaphore02_png: include_bytes!("../../../public/res/semaphore02.png"),
    house_png: include_bytes!("../../../public/res/house01.png"),
    cursor_png: include_bytes!("../../../public/res/cursor.png"),
    humanoid_png: include_bytes!("../../../public/res/humanoid.png"),
    cool_snek_png: include_bytes!("../../../public/res/cool_snek.png"),
    tree_png: include_bytes!("../../../public/res/tree.png"),
    editor_icons_png: include_bytes!("../../../public/res/editor_icons.png"),
    nav_tileset_png: include_bytes!("../../../public/res/nav_tileset.png"),
};

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    use classic_demo::Scene;
    use classic_platform::Platform;
    use std::cell::Cell;
    use std::rc::Rc;

    env_logger::init();
    classic_core::cl_info!(
        classic_core::instrument::Chan::Platform,
        "classic-wgl desktop starting"
    );

    let config = classic_engine::env_config::EnvConfig::get();
    let scene = Scene::parse(&config.scene);

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
                let mut e = classic_demo::init_engine(gl, &ASSETS, scene);
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
            engine = Some(classic_demo::init_engine(gl, &ASSETS, scene));
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
