use classic_platform::EmbeddedAssetLoader;

/// Every embedded asset, resolved at compile time and keyed by manifest path.
static ASSETS: &[(&str, &[u8])] = &[
    ("/manifest.json", include_str!("../../../public/manifest.json").as_bytes()),
    ("/state.json", include_str!("../../../public/state.json").as_bytes()),
    ("/state_lunar.json", include_str!("../../../public/state_lunar.json").as_bytes()),
    (
        "/res/dejavusans-sdf.json",
        include_str!("../../../public/res/dejavusans-sdf.json").as_bytes(),
    ),
    ("/res/skynet_logo.png", include_bytes!("../../../public/res/skynet_logo.png")),
    ("/res/cool_snek.png", include_bytes!("../../../public/res/cool_snek.png")),
    ("/res/road_tileset.png", include_bytes!("../../../public/res/road_tileset.png")),
    ("/res/nav_tileset.png", include_bytes!("../../../public/res/nav_tileset.png")),
    ("/res/cursor.png", include_bytes!("../../../public/res/cursor.png")),
    ("/res/font.png", include_bytes!("../../../public/res/font.png")),
    ("/res/dejavusans-sdf.png", include_bytes!("../../../public/res/dejavusans-sdf.png")),
    ("/res/editor_icons.png", include_bytes!("../../../public/res/editor_icons.png")),
    ("/res/humanoid.png", include_bytes!("../../../public/res/humanoid.png")),
    ("/res/semaphore01.png", include_bytes!("../../../public/res/semaphore01.png")),
    ("/res/semaphore02.png", include_bytes!("../../../public/res/semaphore02.png")),
    ("/res/tree.png", include_bytes!("../../../public/res/tree.png")),
    ("/res/house01.png", include_bytes!("../../../public/res/house01.png")),
];

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
                let loader = EmbeddedAssetLoader::new(ASSETS);
                let mut e = classic_demo::init_engine(gl, &loader, scene);
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
            let loader = EmbeddedAssetLoader::new(ASSETS);
            engine = Some(classic_demo::init_engine(gl, &loader, scene));
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
