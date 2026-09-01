use wasm_bindgen::prelude::*;

/// Where each named ROM (`?rom=<name>`, empty => demo) is served from.
///
/// ROMs are built by the `classic-roms` repo and published to a Cloudflare R2
/// public bucket served at `classic-roms.com` (CORS-enabled).
#[cfg(target_arch = "wasm32")]
const ROM_URLS: &[(&str, &str)] = &[
    ("demo", "https://classic-roms.com/demo.rom"),
    ("lunar", "https://classic-roms.com/lunar.rom"),
    ("moon", "https://classic-roms.com/lunar.rom"),
    ("lrvtest", "https://classic-roms.com/lrvtest.rom"),
    ("basetest", "https://classic-roms.com/basetest.rom"),
];

/// The `roms.json` checksum index, used to content-address cached ROMs.
#[cfg(target_arch = "wasm32")]
const ROM_INDEX_URL: &str = "https://classic-roms.com/roms.json";

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

/// Resolve the `?rom=` selector to archive bytes, caching named ROMs in the
/// browser's Cache API (keyed by their `roms.json` sha256) so repeat loads
/// don't re-download.  Arbitrary URLs/paths fall back to a plain fetch.
#[cfg(target_arch = "wasm32")]
async fn resolve_web_rom(spec: &str) -> anyhow::Result<classic_rom::AssetBytes> {
    use classic_rom::RomSource;

    match classic_rom::parse_rom_spec(spec) {
        RomSource::Embedded(name) => {
            let url = ROM_URLS
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, url)| *url)
                .ok_or_else(|| anyhow::anyhow!("unknown ROM `{name}`"))?;
            // `moon` is a legacy alias for the `lunar` scene; its checksum
            // index entry is `lunar`.
            let index_key = if name == "moon" { "lunar" } else { &name };
            classic_platform::rom::resolve_named_rom_cached(index_key, url, ROM_INDEX_URL).await
        }
        _ => {
            classic_platform::resolve_rom_async(
                spec,
                &classic_platform::rom::static_lookup(ROM_URLS),
            )
            .await
        }
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
    let mut overlay = BootOverlay::new(&format!("downloading scene `{label}`…"));

    let rom_bytes = match resolve_web_rom(&spec).await {
        Ok(bytes) => bytes,
        Err(err) => {
            overlay.error(&format!("failed to load scene `{label}`:\n{err}"));
            return Err(err);
        }
    };
    overlay.clear();

    let archive = classic_rom::RomArchive::from_bytes(&rom_bytes).expect("open ROM archive");
    let rom = classic_rom::Rom::load(&archive).expect("load ROM");

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

    let mut engine: Option<classic_engine::Engine> = None;

    platform.run_loop(move |gl, input, vw, vh, delta, should_close| {
        if engine.is_none() {
            classic_core::cl_info!(
                classic_core::instrument::Chan::Platform,
                "web: initialising engine"
            );
            engine = Some(classic_demo::init_engine(gl, &rom));
        }
        if let Some(e) = engine.as_mut() {
            e.frame(input, vw, vh, delta);
            if e.test_should_close {
                *should_close = true;
            }
        }
    });

    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
async fn run() -> anyhow::Result<()> {
    let _ = "classic-web: use `trunk build` or `trunk serve` on wasm32";
    Ok(())
}
