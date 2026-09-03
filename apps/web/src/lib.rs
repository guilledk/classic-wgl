use wasm_bindgen::prelude::*;

/// Where each named ROM (`?rom=<name>`, empty => demo) is served from.
///
/// ROMs are built by the `classic-roms` repo and published to a Cloudflare R2
/// public bucket served at `classic-roms.com` (CORS-enabled).  `common` and
/// `lunar-common` are the shared asset-only dependency ROMs the shipped scenes
/// resolve at boot.
#[cfg(target_arch = "wasm32")]
const ROM_URLS: &[(&str, &str)] = &[
    ("demo", "https://classic-roms.com/demo.rom"),
    ("lunar", "https://classic-roms.com/lunar.rom"),
    ("moon", "https://classic-roms.com/lunar.rom"),
    ("lrvtest", "https://classic-roms.com/lrvtest.rom"),
    ("basetest", "https://classic-roms.com/basetest.rom"),
    ("common", "https://classic-roms.com/common.rom"),
    ("lunar-common", "https://classic-roms.com/lunar-common.rom"),
];

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

    // Fetches the (now small) app wasm boots instantly; the scene ROM is
    // streamed in before the engine starts.
    wasm_bindgen_futures::spawn_local(async {
        if let Err(err) = run().await {
            log::error!("boot failed: {err:#}");
        }
    });
}

/// Resolve the `?rom=` selector to a multi-ROM dependency DAG: the named root
/// plus its `common`/`lunar-common` deps, each fetched from the CDN through the
/// name -> location registry and Cache-API-cached keyed by the `sha256`
/// published in `roms.json`.  Arbitrary URLs/paths are resolved the same way
/// (their manifest `deps` are fetched through the registry).
#[cfg(target_arch = "wasm32")]
async fn resolve_web_roms(
    spec: &str,
    sink: &dyn classic_rom::BootSink,
) -> anyhow::Result<classic_rom::LoadedRoms> {
    classic_platform::resolve_roms_async(
        spec,
        &classic_platform::rom::static_lookup(ROM_URLS),
        "https://classic-roms.com/roms.json",
        sink,
    )
    .await
}

/// The boot loading-screen mode for this session, from the `?loader=` URL
/// parameter (default `visual`).
#[cfg(target_arch = "wasm32")]
fn loader_mode() -> classic_engine::env_config::LoaderMode {
    match query_param("loader").as_deref() {
        Some("console") => classic_engine::env_config::LoaderMode::Console,
        Some("off") => classic_engine::env_config::LoaderMode::Off,
        _ => classic_engine::env_config::LoaderMode::Visual,
    }
}

/// Choose the boot sink for this session, plus the visual loader (when the
/// loader mode is `visual`).
#[cfg(target_arch = "wasm32")]
fn boot_sink() -> (
    std::sync::Arc<dyn classic_rom::BootSink>,
    Option<std::sync::Arc<classic_engine::boot_loader::VisualBootSink>>,
) {
    match loader_mode() {
        classic_engine::env_config::LoaderMode::Visual => {
            let loader = std::sync::Arc::new(classic_engine::boot_loader::VisualBootSink::new());
            // `?boot_log=` also mirrors the stream to the console.
            if query_param("boot_log").is_some() {
                let tee = std::sync::Arc::new(classic_rom::TeeBootSink::new(vec![
                    loader.clone(),
                    std::sync::Arc::new(classic_platform::LogBootSink),
                ]));
                (tee, Some(loader))
            } else {
                (loader.clone(), Some(loader))
            }
        }
        classic_engine::env_config::LoaderMode::Console => {
            (std::sync::Arc::new(classic_platform::LogBootSink), None)
        }
        classic_engine::env_config::LoaderMode::Off => {
            if query_param("boot_log").is_some() {
                (std::sync::Arc::new(classic_platform::LogBootSink), None)
            } else {
                (std::sync::Arc::new(classic_rom::NullBootSink), None)
            }
        }
    }
}
/// Per-frame boot budget: run boot steps for at most this long before yielding
/// to the browser, so no single animation frame blocks past ~16 ms.
#[cfg(target_arch = "wasm32")]
const BOOT_BUDGET_MILLIS: u128 = 12;

/// Resolve after the next `requestAnimationFrame` tick — yield one frame so the
/// browser paints (and the loading screen animates) between hydration slices.
#[cfg(target_arch = "wasm32")]
fn next_frame() -> impl std::future::Future<Output = ()> {
    async {
        let promise = js_sys::Promise::new(&mut |resolve, _reject| {
            let resolve = resolve.clone();
            let cb = wasm_bindgen::closure::Closure::once(move || {
                let _ = resolve.call0(&JsValue::UNDEFINED);
            });
            let window = web_sys::window().expect("no window");
            let _ = window.request_animation_frame(cb.as_ref().unchecked_ref());
            cb.forget();
        });
        let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
    }
}

#[cfg(target_arch = "wasm32")]
async fn run() -> anyhow::Result<()> {
    // `?classic_log=` configures channel logging; `?rom=` selects the ROM
    // to boot (`rom:<name>`, a full URL, or a relative path).
    if let Some(spec) = query_param("classic_log") {
        classic_core::instrument::init(&spec);
    }
    let spec = query_param("rom").unwrap_or_default();
    let (sink, loader) = boot_sink();

    use classic_platform::web::WebPlatform;
    use classic_platform::Platform;

    // Canvas-first: create the WebGL2 context *before* fetching the ROM, so the
    // GL loading screen can draw while the archive streams in (mirrors the
    // desktop window-first boot).  There is no DOM overlay anymore.
    let platform = match WebPlatform::new("glCanvas") {
        Ok(p) => p,
        Err(e) => {
            log::error!("WebGL2 init failed: {e:?}");
            return Err(anyhow::anyhow!("WebPlatform::new: {e:?}"));
        }
    };
    let gl = platform.gl();
    let (vw, vh) = platform.viewport();

    // Set up the GL layer + embedded font up front so the loader renders from
    // frame 0, then render one initial frame before the (awaited) fetch.
    let mut engine = classic_engine::Engine::new();
    engine.init_gfx(gl.clone());
    // Install the loading-screen UI from frame 0 (visual loader only), sync
    // it, and render through the normal frame pipeline.
    if let Some(loader) = &loader {
        loader.install(&mut engine);
        loader.sync(&mut engine, vw, vh);
        engine.frame(&mut classic_platform::InputState::new(), vw, vh, 0.0);
    }

    let boot_start = std::time::Instant::now();
    let loaded = match resolve_web_roms(&spec, sink.as_ref()).await {
        Ok(loaded) => loaded,
        Err(err) => {
            sink.on_event(classic_rom::BootEvent::BootFailed {
                phase: "resolve",
                error: format!("{err:#}"),
            });
            return Err(err);
        }
    };
    // Esc aborts the load: stop hydrating and leave the loader hanging.
    if platform.input().borrow().was_key_pressed("Escape") {
        log::info!("boot aborted (Esc)");
        return Ok(());
    }
    if let Some(loader) = &loader {
        loader.set_dag(&loaded);
        loader.sync(&mut engine, vw, vh);
        engine.frame(&mut classic_platform::InputState::new(), vw, vh, 0.0);
    }

    // Hydrate the engine incrementally: build the boot plan, then drain a
    // time-budgeted slice of steps per animation frame (drawing the loader each
    // frame) so the browser stays responsive while the large atlases decode.
    // The `.basis` transcode stays in its dedicated Worker (awaited after the
    // plan drains).
    let mut plan = engine.begin_boot_gfx(gl, &loaded, sink.as_ref());
    while !plan.is_done() {
        if platform.input().borrow().was_key_pressed("Escape") {
            log::info!("boot aborted (Esc)");
            return Ok(());
        }
        let frame_start = std::time::Instant::now();
        loop {
            if plan.is_done() {
                break;
            }
            engine.boot_step(&mut plan, 1);
            if frame_start.elapsed().as_millis() >= BOOT_BUDGET_MILLIS {
                break;
            }
        }
        if let Some(loader) = &loader {
            loader.sync(&mut engine, vw, vh);
            engine.frame(&mut classic_platform::InputState::new(), vw, vh, 0.0);
        }
        next_frame().await;
    }
    if platform.input().borrow().was_key_pressed("Escape") {
        log::info!("boot aborted (Esc)");
        return Ok(());
    }
    engine.upload_pending_basis(&mut plan, sink.as_ref()).await;
    if let Some(loader) = &loader {
        loader.sync(&mut engine, vw, vh);
        engine.frame(&mut classic_platform::InputState::new(), vw, vh, 0.0);
    }
    if let Some(loader) = &loader {
        loader.uninstall(&mut engine);
    }
    classic_demo::finish_init_engine(
        &mut engine,
        &loaded,
        &classic_demo::CompiledModules::new(),
        sink.as_ref(),
    );
    sink.on_event(classic_rom::BootEvent::BootComplete { elapsed: boot_start.elapsed() });

    platform.run_loop(move |_gl, input, vw, vh, delta, should_close| {
        engine.frame(input, vw, vh, delta);
        if engine.test_should_close {
            *should_close = true;
        }
    });

    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
async fn run() -> anyhow::Result<()> {
    let _ = "classic-web: use `trunk build` or `trunk serve` on wasm32";
    Ok(())
}
