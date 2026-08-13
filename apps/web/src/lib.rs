use wasm_bindgen::prelude::*;

/// Read a query-string parameter from the page URL.
#[cfg(target_arch = "wasm32")]
fn query_param(name: &str) -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    let search = search.strip_prefix('?').unwrap_or(&search);
    let prefix = format!("{name}=");
    search.split('&').find_map(|p| p.strip_prefix(&prefix).map(|v| v.to_string()))
}

#[wasm_bindgen(start)]
pub fn main() {
    console_error_panic_hook::set_once();

    // Install a simple console logger for wasm (no env_logger on web).
    struct WebLogger;
    impl log::Log for WebLogger {
        fn enabled(&self, _: &log::Metadata) -> bool {
            true
        }
        fn log(&self, record: &log::Record) {
            let msg = format!(
                "[{target} {lvl}] {args}",
                target = record.target(),
                lvl = record.level(),
                args = record.args(),
            );
            match record.level() {
                log::Level::Error => web_sys::console::error_1(&msg.into()),
                log::Level::Warn => web_sys::console::warn_1(&msg.into()),
                log::Level::Info => web_sys::console::info_1(&msg.into()),
                _ => web_sys::console::log_1(&msg.into()),
            }
        }
        fn flush(&self) {}
    }
    log::set_logger(&WebLogger).ok();
    log::set_max_level(log::LevelFilter::Trace);

    classic_core::cl_info!(classic_core::instrument::Chan::Platform, "classic-web starting");

    #[cfg(target_arch = "wasm32")]
    {
        // `?classic_log=` configures channel logging; `?scene=` picks the
        // scene (the native `CLASSIC_SCENE` env var has no web equivalent).
        if let Some(spec) = query_param("classic_log") {
            classic_core::instrument::init(&spec);
        }
        let scene = classic_demo::Scene::parse(&query_param("scene").unwrap_or_default());

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

        use classic_platform::web::WebPlatform;
        use classic_platform::{EmbeddedAssetLoader, Platform};

        let platform = match WebPlatform::new("glCanvas") {
            Ok(p) => p,
            Err(e) => {
                web_sys::console::error_1(&e);
                return;
            }
        };

        let mut engine: Option<classic_engine::Engine> = None;

        platform.run_loop(move |gl, input, vw, vh, _delta, should_close| {
            if engine.is_none() {
                let loader = EmbeddedAssetLoader::new(ASSETS);
                engine = Some(classic_demo::init_engine(gl, &loader, scene));
            }
            if let Some(e) = engine.as_mut() {
                e.frame(input, vw, vh, _delta);
                if e.test_should_close {
                    *should_close = true;
                }
            }
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = "classic-web: use `trunk build` or `trunk serve` on wasm32";
    }
}
