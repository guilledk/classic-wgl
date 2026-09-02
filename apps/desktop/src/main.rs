use classic_rom::{BootEvent, BootSink, LoadedRoms, NullBootSink};
use std::collections::HashMap;
use std::sync::mpsc;

/// The off-thread boot result: a resolved DAG, its decoded textures, and its
/// compiled guest modules.  All three are owned + `Send`, so the background
/// boot thread hands the whole payload to the GL thread in one message.
struct DecodedAssets {
    loaded: LoadedRoms,
    decoded: HashMap<String, classic_engine::boot::DecodedTexture>,
    compiled: classic_demo::CompiledModules,
}

/// Messages streamed from the background boot thread to the GL run loop.
enum BootMsg {
    Event(BootEvent),
    Assets(Box<DecodedAssets>),
    Failed(String),
}

/// A [`BootSink`] that forwards the background thread's events over `mpsc` so
/// the main/GL thread observes them (logging, and later the loading screen).
struct ChannelBootSink {
    tx: mpsc::Sender<BootMsg>,
}

impl BootSink for ChannelBootSink {
    fn on_event(&self, event: BootEvent) {
        let _ = self.tx.send(BootMsg::Event(event));
    }
}

/// Run the CPU-bound boot stages on the background thread: ROM resolve +
/// archive decompress + parse, texture decode, and wasmtime `Module` compile.
fn boot_assets(
    spec: &str,
    lookup: &dyn Fn(&str) -> Option<String>,
    sink: &dyn BootSink,
) -> anyhow::Result<DecodedAssets> {
    let loaded = classic_platform::resolve_roms(spec, lookup, sink)?;

    // Decode every texture to owned pixels off-thread (no GL here).  A
    // throwaway engine only supplies the namespace/plan logic of `begin_boot`.
    let decoded = {
        let e = classic_engine::Engine::new();
        let mut plan = e.begin_boot(&loaded, sink);
        classic_engine::boot::decode_plan(&mut plan)
    };

    let compiled = classic_demo::compile_guest_modules(&loaded, sink);

    Ok(DecodedAssets { loaded, decoded, compiled })
}

/// Hydrate an engine from a resolved multi-ROM dependency DAG.
fn boot_engine(
    gl: std::rc::Rc<glow::Context>,
    loaded: &LoadedRoms,
    sink: &dyn BootSink,
) -> classic_engine::Engine {
    classic_demo::init_engine_multi(gl, loaded, sink)
}

/// Choose the boot sink for this process: a logging sink when the loader is
/// enabled or `CLASSIC_BOOT_LOG` is set, otherwise the no-op sink.
fn boot_sink() -> Box<dyn BootSink> {
    let env = classic_engine::env_config::EnvConfig::get();
    if env.boot_log || env.loader_mode != classic_engine::env_config::LoaderMode::Off {
        Box::new(classic_platform::LogBootSink)
    } else {
        Box::new(NullBootSink)
    }
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
            "lunar-common" => "lunar-common.rom".into(),
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
    let boot_start = std::time::Instant::now();
    let sink = boot_sink();

    if config.headless {
        // Headless / golden / test: resolve synchronously *before* the loop and
        // boot on the first frame, unchanged from the golden-neutral path.
        let loaded = match classic_platform::resolve_roms(
            &config.rom,
            &rom_lookup(rom_dir),
            sink.as_ref(),
        ) {
            Ok(loaded) => loaded,
            Err(err) => {
                sink.on_event(BootEvent::BootFailed {
                    phase: "resolve",
                    error: format!("{err:#}"),
                });
                eprintln!("resolve ROMs: {err:#}");
                std::process::exit(1);
            }
        };

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
                let mut e = boot_engine(gl, &loaded, sink.as_ref());
                sink.on_event(BootEvent::BootComplete { elapsed: boot_start.elapsed() });
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

    // -- Windowed native path: create the window first, then boot off-thread --

    let (tx, rx) = mpsc::channel::<BootMsg>();
    let bg_spec = config.rom.clone();
    let bg_lookup = rom_lookup(rom_dir);
    std::thread::spawn(move || {
        let bg_sink = ChannelBootSink { tx: tx.clone() };
        match boot_assets(&bg_spec, &bg_lookup, &bg_sink) {
            Ok(assets) => {
                let _ = tx.send(BootMsg::Assets(Box::new(assets)));
            }
            Err(err) => {
                bg_sink.on_event(BootEvent::BootFailed {
                    phase: "resolve",
                    error: format!("{err:#}"),
                });
                let _ = tx.send(BootMsg::Failed(format!("{err:#}")));
            }
        }
    });

    let max_frames: Option<u64> = std::env::var("CLASSIC_FRAMES").ok().and_then(|v| v.parse().ok());
    let mut frame_count: u64 = 0;

    let platform = classic_platform::native::NativePlatform::new();
    let mut engine: Option<classic_engine::Engine> = None;
    let mut assets: Option<Box<DecodedAssets>> = None;
    let test_failed = Rc::new(Cell::new(false));
    let tf = test_failed.clone();

    platform.run_loop(move |gl, input, vw, vh, delta, should_close| {
        // Drain the boot channel: forward background events to the process sink,
        // and pick up the decoded assets once the CPU boot stages finish.
        loop {
            match rx.try_recv() {
                Ok(BootMsg::Event(ev)) => sink.on_event(ev),
                Ok(BootMsg::Assets(a)) => assets = Some(a),
                Ok(BootMsg::Failed(err)) => {
                    eprintln!("resolve ROMs: {err}");
                    *should_close = true;
                    return;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    if engine.is_none() && assets.is_none() {
                        eprintln!("boot thread exited without producing assets");
                        *should_close = true;
                        return;
                    }
                    break;
                }
            }
        }

        if engine.is_none() {
            // The GL-side boot runs on this thread, on the first frame after
            // the CPU stages deliver their payload.
            if let Some(a) = assets.take() {
                let e = classic_demo::init_engine_multi_decoded(
                    gl,
                    &a.loaded,
                    a.decoded,
                    &a.compiled,
                    sink.as_ref(),
                );
                sink.on_event(BootEvent::BootComplete { elapsed: boot_start.elapsed() });
                engine = Some(e);
            } else {
                // Still resolving/decoding/compiling; nothing to render yet.
                return;
            }
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
}

#[cfg(target_arch = "wasm32")]
fn main() {}
