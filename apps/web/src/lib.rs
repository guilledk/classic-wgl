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

        const ASSETS: classic_demo::DemoAssets<'static> = classic_demo::DemoAssets {
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

        use classic_platform::web::WebPlatform;
        use classic_platform::Platform;

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
                engine = Some(classic_demo::init_engine(gl, &ASSETS, scene));
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
