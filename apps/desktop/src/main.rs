use classic_rom::{BootEvent, BootSink, LoadedRoms, NullBootSink};
use std::collections::HashMap;
use std::sync::mpsc;

/// The off-thread boot result: a resolved DAG, its decoded textures + basis
/// sheets, and its compiled guest modules.  All owned + `Send`, so the
/// background boot thread hands the whole payload to the GL thread in one
/// message.
struct DecodedAssets {
    loaded: LoadedRoms,
    decoded: HashMap<String, classic_engine::boot::DecodedTexture>,
    basis: Vec<Option<classic_gfx::DecodedBasis>>,
    compiled: classic_demo::CompiledModules,
}

/// Owned incremental-boot state, moved out of [`DecodedAssets`] on the GL
/// thread so the plan can be drained a chunk at a time (interleaved with
/// loading-screen frames) instead of one blocking `load_roms_decoded` call.
#[cfg(not(target_arch = "wasm32"))]
struct NativeBoot {
    loaded: LoadedRoms,
    decoded: HashMap<String, classic_engine::boot::DecodedTexture>,
    basis: Vec<Option<classic_gfx::DecodedBasis>>,
    compiled: classic_demo::CompiledModules,
    cursor: usize,
    basis_cursor: usize,
    plan_done: bool,
    basis_done: bool,
}

/// Per-frame boot budget (native): run boot steps for at most this long before
/// yielding to the run loop so the loading screen keeps animating.
const BOOT_BUDGET_MILLIS: u128 = 12;

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
/// archive decompress + parse, texture decode, basis transcode (parallel), and
/// wasmtime `Module` compile.
fn boot_assets(
    spec: &str,
    lookup: &dyn Fn(&str) -> Option<String>,
    caps: classic_gfx::Caps,
    sink: &dyn BootSink,
) -> anyhow::Result<DecodedAssets> {
    let loaded = classic_platform::resolve_roms(spec, lookup, sink)?;

    // Decode every texture + transcode every `.basis` sheet off-thread (no GL
    // here).  `decode_assets` fans out across the loader thread pool.
    let (decoded, basis) = classic_engine::boot::decode_assets(&loaded, caps, sink);

    let compiled = classic_demo::compile_guest_modules(&loaded, sink);

    Ok(DecodedAssets { loaded, decoded, basis, compiled })
}

/// Hydrate an engine from a resolved multi-ROM dependency DAG.
fn boot_engine(
    gl: std::rc::Rc<glow::Context>,
    loaded: &LoadedRoms,
    sink: &dyn BootSink,
) -> classic_engine::Engine {
    classic_demo::init_engine_multi(gl, loaded, sink)
}

/// Choose the boot sink for this process, plus the visual loader (when the
/// effective loader mode is `visual`).
///
/// - `visual` → a [`VisualBootSink`] (GL loading screen) + no log sink.
/// - `console` (or `CLASSIC_BOOT_LOG`) → the [`LogBootSink`].
/// - `off` → the no-op sink.
///
/// The effective mode is forced to `off` for headless/golden/test (see
/// [`classic_engine::env_config::EnvConfig::effective_loader_mode`]).
fn boot_sink() -> (
    std::sync::Arc<dyn BootSink>,
    Option<std::sync::Arc<classic_engine::boot_loader::VisualBootSink>>,
) {
    let env = classic_engine::env_config::EnvConfig::get();
    match env.effective_loader_mode() {
        classic_engine::env_config::LoaderMode::Visual => {
            let loader = std::sync::Arc::new(classic_engine::boot_loader::VisualBootSink::new());
            // `CLASSIC_BOOT_LOG` also mirrors the stream to the console.
            if env.boot_log {
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
            if env.boot_log {
                (std::sync::Arc::new(classic_platform::LogBootSink), None)
            } else {
                (std::sync::Arc::new(NullBootSink), None)
            }
        }
    }
}

/// The known named ROMs and where their archives live on disk (with a CDN
/// fallback).
///
/// ROMs are not compiled in anymore: the `classic-roms` repo builds and
/// releases them, and `cargo xtask fetch-roms` stages them under
/// `roms/out/` (a gitignored local cache, overridable via `CLASSIC_ROM_DIR`).
/// If a ROM isn't staged locally it is streamed from the CDN instead, so a
/// fresh checkout downloads the ROM the same way the web build does.
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
        let local = format!("{dir}/{file}");
        if std::path::Path::new(&local).exists() {
            Some(local)
        } else {
            Some(format!("https://classic-roms.com/{file}"))
        }
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
    let (sink, loader) = boot_sink();

    // Periodic process CPU/mem sampling during boot (feeds the loader header /
    // boot log); skipped when the sink would just discard it.
    let sample_usage = loader.is_some()
        || config.boot_log
        || config.effective_loader_mode() == classic_engine::env_config::LoaderMode::Console;
    let mut sampler = if sample_usage {
        Some(classic_platform::ResourceUsageSampler::start(
            sink.clone(),
            std::time::Duration::from_millis(200),
        ))
    } else {
        None
    };

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
                sampler.take();
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

    // The background thread needs the GL compressed-format capabilities for
    // basis transcode, which are only queryable once the window/GL context
    // exists (frame 0).  It blocks on a small channel until the GL thread
    // sends them.
    let (caps_tx, caps_rx) = mpsc::channel::<classic_gfx::Caps>();
    let (tx, rx) = mpsc::channel::<BootMsg>();
    let bg_spec = config.rom.clone();
    let bg_lookup = rom_lookup(rom_dir);
    std::thread::spawn(move || {
        let Ok(caps) = caps_rx.recv() else { return };
        let bg_sink = ChannelBootSink { tx: tx.clone() };
        match boot_assets(&bg_spec, &bg_lookup, caps, &bg_sink) {
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
    let mut boot: Option<NativeBoot> = None;
    let mut booted = false;
    let mut caps_sent = false;
    let test_failed = Rc::new(Cell::new(false));
    let tf = test_failed.clone();

    platform.run_loop(move |gl, input, vw, vh, delta, should_close| {
        // Esc aborts the in-flight load and stops the process (the detached
        // boot thread dies with it).
        if !booted && input.was_key_pressed("Escape") {
            classic_core::cl_info!(classic_core::instrument::Chan::Platform, "boot aborted (Esc)");
            *should_close = true;
            return;
        }

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
                    if !booted && assets.is_none() && boot.is_none() {
                        eprintln!("boot thread exited without producing assets");
                        *should_close = true;
                        return;
                    }
                    break;
                }
            }
        }

        // Move the CPU-side payload into owned incremental-boot state (once).
        if let Some(a) = assets.take() {
            if let Some(l) = &loader {
                l.set_dag(&a.loaded);
            }
            boot = Some(NativeBoot {
                loaded: a.loaded,
                decoded: a.decoded,
                basis: a.basis,
                compiled: a.compiled,
                cursor: 0,
                basis_cursor: 0,
                plan_done: false,
                basis_done: false,
            });
        }

        // First frame: set up the GL layer + embedded font so the loading
        // screen can draw before the boot payload arrives, and hand the
        // compressed-format caps to the background thread.
        if engine.is_none() {
            let mut e = classic_engine::Engine::new();
            e.init_gfx(gl.clone());
            // Install the loading-screen UI from frame 0 (visual loader only).
            if let Some(l) = &loader {
                l.install(&mut e);
            }
            engine = Some(e);
        }
        if !caps_sent {
            caps_sent = true;
            let _ = caps_tx.send(classic_gfx::Caps::query(&gl));
        }

        // Run one chunk of hydration per frame (interleaved with the loader).
        if !booted {
            if let (Some(e), Some(b)) = (engine.as_mut(), boot.as_mut()) {
                if !b.plan_done {
                    // Drain plan steps under a time budget, rebuilding the plan
                    // each frame so it never needs to live across the frame.
                    let frame_start = std::time::Instant::now();
                    let (cursor, decoded, done) = {
                        let mut plan = e.begin_boot_gfx(gl.clone(), &b.loaded, sink.as_ref());
                        plan.set_cursor(b.cursor);
                        plan.set_decoded(std::mem::take(&mut b.decoded));
                        loop {
                            if plan.is_done() {
                                break;
                            }
                            e.boot_step_predecoded(&mut plan, 1);
                            if frame_start.elapsed().as_millis() >= BOOT_BUDGET_MILLIS {
                                break;
                            }
                        }
                        let done = plan.is_done();
                        (plan.cursor(), plan.take_decoded(), done)
                    };
                    b.cursor = cursor;
                    b.decoded = decoded;
                    b.plan_done = done;
                } else if !b.basis_done {
                    // One pre-decoded `.basis` upload per frame (chunked).
                    let done = {
                        let mut plan = e.begin_boot_gfx(gl.clone(), &b.loaded, sink.as_ref());
                        e.upload_basis_predecoded_at(
                            &mut plan,
                            b.basis_cursor,
                            &b.basis,
                            sink.as_ref(),
                        )
                    };
                    b.basis_cursor += 1;
                    b.basis_done = done;
                } else {
                    if let Some(l) = &loader {
                        l.uninstall(e);
                    }
                    classic_demo::finish_init_engine(e, &b.loaded, &b.compiled, sink.as_ref());
                    sink.on_event(BootEvent::BootComplete { elapsed: boot_start.elapsed() });
                    sampler.take();
                    booted = true;
                }
            }
        }

        // Render the loading screen (pre-boot) or the engine (post-boot).
        if let Some(e) = engine.as_mut() {
            if booted {
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
            } else if let Some(l) = &loader {
                // Sync the loader's UI entities, then render them through the
                // normal frame pipeline.
                l.sync(e, vw, vh);
                e.frame(input, vw, vh, delta);
            }
        }
    });

    if test_failed.get() {
        std::process::exit(1);
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {}
