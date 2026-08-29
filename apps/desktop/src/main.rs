use classic_rom::LoadedRoms;

/// Hydrate an engine from a resolved multi-ROM dependency DAG.
fn boot_engine(gl: std::rc::Rc<glow::Context>, loaded: &LoadedRoms) -> classic_engine::Engine {
    classic_demo::init_engine_multi(gl, loaded)
}

/// The known named ROMs and where their archives live on disk.
///
/// ROMs are not compiled in anymore: the `classic-roms` repo builds and
/// releases them, and `cargo xtask fetch-roms` stages them under
/// `roms/out/` (a gitignored local cache, overridable via `CLASSIC_ROM_DIR`).
fn rom_lookup(dir: String) -> impl Fn(&str) -> Option<String> {
    move |name: &str| {
        let file: String = match name {
            "demo" => "demo.rom".into(),
            "lunar" | "moon" => "lunar.rom".into(),
            "lrvtest" => "lrvtest.rom".into(),
            "basetest" => "basetest.rom".into(),
            "common" => "common.rom".into(),
            "compose" => "compose.rom".into(),
            _ => return None,
        };
        Some(format!("{dir}/{file}"))
    }
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
    let rom_dir = std::env::var("CLASSIC_ROM_DIR").unwrap_or_else(|_| "roms/out".to_string());
    let loaded =
        classic_platform::resolve_roms(&config.rom, &rom_lookup(rom_dir)).expect("resolve ROMs");

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
                let mut e = boot_engine(gl, &loaded);
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
            engine = Some(boot_engine(gl, &loaded));
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
