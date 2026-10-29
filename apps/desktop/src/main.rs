use classic_engine::boot::{BootFrame, InterleavedBoot};
use classic_rom::{BootEvent, BootSink, NullBootSink};

/// The demo's post-load boot hook, boxed for a boot driver.
fn demo_finish() -> Box<dyn classic_engine::boot::BootFinish> {
    Box::new(classic_demo::DemoFinish::default())
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
///
/// `lunar-buildings` (the lunar buildings: landing pad, crane; a dependency
/// of `lunar` and `padtest`) is listed before it is published: it boots from
/// a locally built copy in `CLASSIC_ROM_DIR`, and only reaches for the CDN
/// (failing loudly until the landing window publishes it) when absent.  It
/// joins `fetch-roms` / `roms.lock.json` when it is published.
fn rom_lookup(dir: String) -> impl Fn(&str) -> Option<String> {
    move |name: &str| {
        let file: String = match name {
            "demo" => "demo.rom".into(),
            "lunar" | "moon" => "lunar.rom".into(),
            "lrvtest" => "lrvtest.rom".into(),
            "lighttest" => "lighttest.rom".into(),
            "fxtest" => "fxtest.rom".into(),
            "common" => "common.rom".into(),
            "lunar-common" => "lunar-common.rom".into(),
            "lunar-buildings" => "lunar-buildings.rom".into(),
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
        // `CLASSIC_WIDTH`/`HEIGHT` are the drawable's physical pixels; at
        // `CLASSIC_PIXEL_RATIO` r the frames lay out `w / r` x `h / r` points.
        let platform = classic_platform::headless::HeadlessPlatform::new(w, h)
            .expect("headless platform")
            .with_pixel_ratio(config.pixel_ratio_override().unwrap_or(1.0));
        let mut loaded = Some(loaded);
        let mut engine: Option<classic_engine::Engine> = None;
        let test_failed = Rc::new(Cell::new(false));
        let tf = test_failed.clone();

        platform.run_loop(move |gl, input, view, delta, should_close| {
            if let Some(loaded) = loaded.take() {
                classic_core::cl_info!(
                    classic_core::instrument::Chan::Platform,
                    "headless: initialising engine"
                );
                let mut e =
                    classic_engine::boot::run_sync(gl, loaded, demo_finish(), sink.as_ref());
                sink.on_event(BootEvent::BootComplete { elapsed: boot_start.elapsed() });
                sampler.take();
                if let Some(gfx) = e.gfx.as_mut() {
                    let (pw, ph) = view.physical();
                    gfx.set_render_target(pw, ph);
                }
                engine = Some(e);
            }
            if let Some(e) = engine.as_mut() {
                let t0 = std::time::Instant::now();
                e.frame_sized(input, view, delta);
                // Threaded workers advance in wall time: pace the frames to
                // the fixed step so they see the frame rate a player does.
                if config.workers_threaded {
                    if let Some(dt) = config.fixed_dt {
                        let step = std::time::Duration::from_secs_f32(dt);
                        if let Some(rest) = step.checked_sub(t0.elapsed()) {
                            std::thread::sleep(rest);
                        }
                    }
                }
                if e.test_should_close {
                    *should_close = true;
                }
                if e.test_failed {
                    tf.set(true);
                }
            }
        });

        // Flush the frame profiler (`CLASSIC_PERF`) before any exit.
        classic_gfx::perf::finish();
        if test_failed.get() {
            std::process::exit(1);
        }
        return;
    }

    // -- Windowed native path: create the window first, then boot off-thread --

    // The boot thread resolves the ROMs and prepares the pipeline (decode,
    // basis transcode, guest compile) while this thread renders the loader.
    let spec = config.rom.clone();
    let lookup = rom_lookup(rom_dir);
    let mut boot = InterleavedBoot::threaded(
        loader,
        move |sink: &dyn BootSink| classic_platform::resolve_roms(&spec, &lookup, sink),
        demo_finish(),
    )
    .expect("failed to spawn boot thread");

    let max_frames: Option<u64> = std::env::var("CLASSIC_FRAMES").ok().and_then(|v| v.parse().ok());
    let mut frame_count: u64 = 0;

    // The window follows the display's scale factor (logical points on a
    // HiDPI screen) unless `CLASSIC_PIXEL_RATIO` pins it.
    let platform = classic_platform::native::NativePlatform::new()
        .with_pixel_ratio(config.pixel_ratio_override());
    let mut engine: Option<classic_engine::Engine> = None;
    let mut booted = false;
    let test_failed = Rc::new(Cell::new(false));
    let tf = test_failed.clone();

    platform.run_loop(move |gl, input, view, delta, should_close| {
        // Esc aborts the in-flight load and stops the process (the detached
        // boot thread dies with it).
        if !booted && input.was_key_pressed("Escape") {
            classic_core::cl_info!(classic_core::instrument::Chan::Platform, "boot aborted (Esc)");
            *should_close = true;
            return;
        }

        // First frame: the GL layer + loading screen, drawable before boot.
        let e = engine.get_or_insert_with(|| boot.engine(gl.clone()));

        // One time-budgeted slice of the boot per frame, then the loader (pre-
        // boot) or the engine (post-boot).
        if !booted {
            match boot.poll(e, &gl, sink.as_ref()) {
                BootFrame::Loading => {}
                BootFrame::Booted => {
                    sink.on_event(BootEvent::BootComplete { elapsed: boot_start.elapsed() });
                    sampler.take();
                    booted = true;
                }
                BootFrame::Failed(err) => {
                    eprintln!("resolve ROMs: {err}");
                    *should_close = true;
                    return;
                }
            }
        }
        if !booted {
            boot.draw_loader(e, input, view, delta);
            return;
        }
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
        e.frame_sized(input, view, delta);
        if e.test_should_close {
            *should_close = true;
        }
        if e.test_failed {
            tf.set(true);
        }
    });

    classic_gfx::perf::finish();
    if test_failed.get() {
        std::process::exit(1);
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(test)]
mod tests {
    use super::rom_lookup;

    #[test]
    fn rom_lookup_prefers_a_local_rom_then_the_cdn() {
        let dir = std::env::temp_dir().join(format!("classic-rom-lookup-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let lookup = rom_lookup(dir.to_string_lossy().into_owned());
        // Not built locally (unpublished `lunar-buildings` included): the CDN.
        assert_eq!(
            lookup("lunar-buildings").as_deref(),
            Some("https://classic-roms.com/lunar-buildings.rom")
        );
        // A local build in `CLASSIC_ROM_DIR` wins.
        std::fs::write(dir.join("lunar-buildings.rom"), b"rom").unwrap();
        let local = dir.join("lunar-buildings.rom").to_string_lossy().into_owned();
        assert_eq!(lookup("lunar-buildings"), Some(local));
        assert_eq!(lookup("moon"), lookup("lunar"), "moon aliases lunar");
        assert_eq!(lookup("padtest"), None, "unknown names are not ROM names");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
