use wasm_bindgen::prelude::*;

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

    // Read ?classic_log= query param for channel configuration on web.
    if let Some(window) = web_sys::window() {
        let search = window.location().search().unwrap_or_default();
        if let Some(qp) = search.strip_prefix("?classic_log=").or_else(|| {
            search.strip_prefix("?").and_then(|s| {
                s.split('&')
                    .find(|p| p.starts_with("classic_log="))
                    .map(|p| &p["classic_log=".len()..])
            })
        }) {
            classic_core::instrument::init(qp);
        }
    }

    classic_core::cl_info!(classic_core::instrument::Chan::Platform, "classic-web starting");

    #[cfg(target_arch = "wasm32")]
    {
        const MANIFEST_JSON: &str = include_str!("../../../public/manifest.json");
        const STATE_JSON: &str = include_str!("../../../public/state.json");
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