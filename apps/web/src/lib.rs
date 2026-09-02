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

/// A simple DOM overlay (outside the canvas so it renders while the wasm/ROM
/// bytes are streaming in).  `clear_overlay` removes it; the error variant
/// keeps it on screen with a reload button.
#[cfg(target_arch = "wasm32")]
struct BootOverlay {
    el: web_sys::HtmlDivElement,
}

#[cfg(target_arch = "wasm32")]
impl BootOverlay {
    fn new(message: &str) -> Self {
        let document = web_sys::window().and_then(|w| w.document()).expect("no document");
        let el: web_sys::HtmlDivElement =
            document.create_element("div").unwrap().dyn_into().unwrap();
        el.set_text_content(Some(message));
        let style = el.style();
        style.set_property("position", "fixed").unwrap();
        style.set_property("inset", "0").unwrap();
        style.set_property("display", "flex").ok();
        style.set_property("align-items", "center").ok();
        style.set_property("justify-content", "center").ok();
        style.set_property("background", "#000").unwrap();
        style.set_property("color", "#cfc8f0").unwrap();
        style.set_property("font-family", "monospace").unwrap();
        style.set_property("font-size", "18px").unwrap();
        style.set_property("z-index", "1000").unwrap();
        document.body().unwrap().append_child(el.as_ref()).unwrap();
        Self { el }
    }

    fn clear(&mut self) {
        if let Some(parent) = self.el.parent_node() {
            parent.remove_child(self.el.as_ref()).ok();
        }
    }

    fn set(&self, message: &str) {
        self.el.set_text_content(Some(message));
    }

    fn error(&self, message: &str) {
        self.el.set_text_content(Some(message));
    }
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

/// Choose the boot sink for this session: a logging sink when the loader is
/// enabled or `CLASSIC_BOOT_LOG` is set, otherwise the no-op sink.
#[cfg(target_arch = "wasm32")]
fn boot_sink() -> Box<dyn classic_rom::BootSink> {
    if query_param("classic_loader").is_some() || query_param("boot_log").is_some() {
        Box::new(classic_platform::LogBootSink)
    } else {
        Box::new(classic_rom::NullBootSink)
    }
}

/// Per-frame boot budget: run boot steps for at most this long before yielding
/// to the browser, so no single animation frame blocks past ~16 ms.
#[cfg(target_arch = "wasm32")]
const BOOT_BUDGET_MILLIS: u128 = 12;

/// Resolve after the next `requestAnimationFrame` tick — yield one frame so the
/// browser paints (and the boot overlay animates) between hydration slices.
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
    let label = query_param("rom").unwrap_or_else(|| "demo".to_string());
    let mut overlay = BootOverlay::new(&format!("loading scene `{label}`…"));

    let boot_start = std::time::Instant::now();
    let sink = boot_sink();
    let loaded = match resolve_web_roms(&spec, sink.as_ref()).await {
        Ok(loaded) => loaded,
        Err(err) => {
            sink.on_event(classic_rom::BootEvent::BootFailed {
                phase: "resolve",
                error: format!("{err:#}"),
            });
            overlay.error(&format!("failed to load scene `{label}`:\n{err}"));
            return Err(err);
        }
    };
    overlay.set("decoding resources…");

    use classic_platform::web::WebPlatform;
    use classic_platform::Platform;

    let platform = match WebPlatform::new("glCanvas") {
        Ok(p) => p,
        Err(e) => {
            let overlay = BootOverlay::new("WebGL2 unavailable");
            overlay.error(&format!("WebGL2 init failed: {e:?}"));
            return Err(anyhow::anyhow!("WebPlatform::new: {e:?}"));
        }
    };

    // Hydrate the engine incrementally: compile shaders + build the boot plan
    // up front, then drain a time-budgeted slice of steps per animation frame
    // so the browser stays responsive (and the overlay animates) while the
    // large atlases decode.  The `.basis` transcode stays in its dedicated
    // Worker (awaited after the plan drains).
    let gl = platform.gl();
    let mut engine = classic_engine::Engine::new();
    let mut plan = engine.begin_boot_gfx(gl, &loaded, sink.as_ref());
    while !plan.is_done() {
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
        next_frame().await;
    }
    engine.upload_pending_basis(&mut plan, sink.as_ref()).await;
    classic_demo::finish_init_engine(
        &mut engine,
        &loaded,
        &classic_demo::CompiledModules::new(),
        sink.as_ref(),
    );
    sink.on_event(classic_rom::BootEvent::BootComplete { elapsed: boot_start.elapsed() });
    overlay.clear();

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
