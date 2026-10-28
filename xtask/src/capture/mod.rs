//! `cargo xtask capture native|web`: the one way to look at the engine.
//!
//! Both modes run on the **hardware GPU** by default, step the simulation at
//! a **fixed dt**, write only the frames asked for, and finish with a JPEG
//! contact sheet (`sheet.jpg`, at most `--sheet-width` px wide) plus a
//! `capture.json` summary.  Each run prints the renderer string actually in
//! use and the capture's wall and CPU time, and refuses to run on a software
//! rasteriser (llvmpipe / SwiftShader) unless `--software` asked for one or
//! the machine has no GPU render node (`/dev/dri/renderD*`, e.g. CI).
//!
//! - `native` runs the release `classic-desktop` headless (surfaceless EGL)
//!   with `CLASSIC_FIXED_DT` + `CLASSIC_GOLDEN_PNG=<frames>`: synchronous
//!   workers, no wall-clock pacing, so it steps as fast as the GPU allows and
//!   is deterministic.  Pixels are read back only on the listed frames.
//! - `web` serves a `trunk build` dist (optionally fulfilling
//!   `classic-roms.com` requests from a local `--roms` dir), drives GPU
//!   headless Chromium over CDP through a scripted scenario (clicks, keys,
//!   waits, screenshots) with `?fixed_dt=` set and the frame-rate limit off,
//!   and records every console line, exception and error.
//!
//! See `classic-testing` §10 for the policy and recipes.

mod sheet;
mod web;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context};
use serde_json::json;

/// Renderer substrings that mean a software rasteriser.
const SOFTWARE_RENDERERS: &[&str] = &["llvmpipe", "softpipe", "swiftshader", "lavapipe"];

/// Options shared by both modes.
struct Common {
    out: Option<PathBuf>,
    width: u32,
    height: u32,
    dt: f64,
    rom: String,
    software: bool,
    sheet_width: u32,
    cols: Option<u32>,
    sheet: bool,
}

pub fn cmd_capture(args: &[String]) -> anyhow::Result<()> {
    match args.get(1).map(String::as_str) {
        Some("native") => native(&args[2..]),
        Some("web") => web::run(&args[2..]),
        _ => bail!(
            "usage: cargo xtask capture native|web [options]  (see `cargo xtask capture native --help`)"
        ),
    }
}

/// Parse one shared option at `args[*i]`; `Ok(false)` when it is not one.
fn parse_common(c: &mut Common, args: &[String], i: &mut usize) -> anyhow::Result<bool> {
    let a = args[*i].as_str();
    let mut val = || -> anyhow::Result<String> {
        *i += 1;
        args.get(*i).cloned().with_context(|| format!("{a} needs a value"))
    };
    match a {
        "--out" => c.out = Some(PathBuf::from(val()?)),
        "--size" => {
            let v = val()?;
            let (w, h) = v.split_once('x').context("--size takes WxH, e.g. 1280x720")?;
            c.width = w.parse()?;
            c.height = h.parse()?;
        }
        "--dt" => c.dt = val()?.parse()?,
        "--rom" => c.rom = val()?,
        "--software" => c.software = true,
        "--sheet-width" => c.sheet_width = val()?.parse()?,
        "--cols" => c.cols = Some(val()?.parse()?),
        "--no-sheet" => c.sheet = false,
        _ => return Ok(false),
    }
    Ok(true)
}

fn common_defaults(rom: &str) -> Common {
    Common {
        out: None,
        width: 1280,
        height: 720,
        dt: 1.0 / 60.0,
        rom: rom.to_string(),
        software: false,
        sheet_width: 1400,
        cols: None,
        sheet: true,
    }
}

const COMMON_HELP: &str = "\
  --out <dir>          output dir (default target/capture/<mode>-<unix time>)
  --size <WxH>         viewport (default 1280x720; use the smallest that answers the question)
  --dt <s>             fixed simulation step (default 1/60; scenario frame numbers assume theirs)
  --rom <spec>         rom:<name>, a .rom path, or a URL
  --software           force the software rasteriser (llvmpipe / SwiftShader): CI parity only
  --sheet-width <px>   contact-sheet width (default 1400)
  --cols <n>           contact-sheet columns (default: near-square, at most 4)
  --no-sheet           skip the contact sheet";

/// The output dir: `--out`, or a fresh `target/capture/<mode>-<stamp>`.
fn out_dir(c: &Common, root: &Path, mode: &str) -> anyhow::Result<PathBuf> {
    let out = match &c.out {
        Some(o) => o.clone(),
        None => {
            let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
            root.join(format!("target/capture/{mode}-{stamp}"))
        }
    };
    fs::create_dir_all(&out).with_context(|| format!("create {}", out.display()))?;
    fs::canonicalize(&out).context("canonicalize --out")
}

/// Whether this machine has a GPU render node to run on.
fn gpu_present() -> bool {
    fs::read_dir("/dev/dri")
        .map(|d| d.flatten().any(|e| e.file_name().to_string_lossy().starts_with("renderD")))
        .unwrap_or(false)
}

/// Decide hardware vs software and say why.  Returns `true` for software.
fn choose_software(c: &Common) -> bool {
    if c.software {
        eprintln!(
            "capture: --software: forcing the software rasteriser (CI parity; slow, all cores)"
        );
        true
    } else if !gpu_present() {
        eprintln!("capture: no GPU render node under /dev/dri: falling back to software");
        true
    } else {
        false
    }
}

/// Fail when a software renderer shows up although hardware was expected.
fn check_renderer(renderer: &str, software: bool) -> anyhow::Result<()> {
    let lower = renderer.to_ascii_lowercase();
    let is_soft = SOFTWARE_RENDERERS.iter().any(|s| lower.contains(s));
    if is_soft && !software {
        bail!(
            "renderer is a software rasteriser ({renderer}) but --software was not passed: \
             check /dev/dri permissions, LIBGL_ALWAYS_SOFTWARE and the Mesa driver path"
        );
    }
    Ok(())
}

/// Children CPU seconds (user + system) reaped so far by this process.
fn children_cpu_secs() -> f64 {
    let Ok(stat) = fs::read_to_string("/proc/self/stat") else { return 0.0 };
    // Fields after the `(comm)`: state is field 3; cutime/cstime are 16/17.
    let rest = stat.rsplit_once(") ").map(|(_, r)| r).unwrap_or("");
    let f: Vec<&str> = rest.split_whitespace().collect();
    let ticks = |n: usize| f.get(n - 3).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
    (ticks(16) + ticks(17)) / 100.0
}

/// CPU time of a whole process group, sampled from `/proc` every 200 ms
/// (last sample per pid).  Chromium's GPU / renderer / utility processes are
/// not all reaped into our `cutime`, so the web mode counts them this way.
pub(crate) struct GroupCpu {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    handle: std::thread::JoinHandle<f64>,
}

impl GroupCpu {
    pub(crate) fn start(pgid: u32) -> Self {
        use std::sync::atomic::{AtomicBool, Ordering};
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let handle = std::thread::spawn(move || {
            let mut seen: std::collections::HashMap<u32, u64> = Default::default();
            loop {
                for (pid, ticks) in group_ticks(pgid) {
                    seen.insert(pid, ticks);
                }
                if flag.load(Ordering::Relaxed) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            seen.values().sum::<u64>() as f64 / 100.0
        });
        Self { stop, handle }
    }

    /// Stop sampling (after one final pass) and return CPU seconds.
    pub(crate) fn finish(self) -> f64 {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        self.handle.join().unwrap_or(0.0)
    }
}

/// `(pid, utime + stime ticks)` of every live process in group `pgid`.
fn group_ticks(pgid: u32) -> Vec<(u32, u64)> {
    let Ok(dir) = fs::read_dir("/proc") else { return Vec::new() };
    dir.flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
        .filter_map(|pid| {
            let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
            let f: Vec<&str> = stat.rsplit_once(") ")?.1.split_whitespace().collect();
            // After `(comm)`: pgrp is field 5, utime/stime are fields 14/15.
            (f.get(2)?.parse::<u32>().ok()? == pgid)
                .then(|| {
                    Some((pid, f.get(11)?.parse::<u64>().ok()? + f.get(12)?.parse::<u64>().ok()?))
                })
                .flatten()
        })
        .collect()
}

/// Parse a frame list: `10,30,60`, ranges `221-230`, stepped `0-300:30`.
fn parse_frames(s: &str) -> anyhow::Result<Vec<u64>> {
    let mut out = Vec::new();
    for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (range, step) = match part.split_once(':') {
            Some((r, st)) => (r, st.parse::<u64>().context("frame step")?),
            None => (part, 1),
        };
        anyhow::ensure!(step > 0, "frame step must be > 0");
        match range.split_once('-') {
            Some((a, b)) => {
                let (a, b): (u64, u64) = (a.parse()?, b.parse()?);
                anyhow::ensure!(a <= b, "frame range {range} is backwards");
                out.extend((a..=b).step_by(step as usize));
            }
            None => out.push(range.parse().with_context(|| format!("frame `{range}`"))?),
        }
    }
    out.sort_unstable();
    out.dedup();
    anyhow::ensure!(!out.is_empty(), "empty frame list");
    Ok(out)
}

/// The finishing steps both modes share: contact sheet, summary, report.
fn finish(
    c: &Common,
    out: &Path,
    tiles: Vec<sheet::Tile>,
    summary: serde_json::Value,
) -> anyhow::Result<()> {
    let mut summary = summary;
    if c.sheet && !tiles.is_empty() {
        let path = out.join("sheet.jpg");
        let (w, h) = sheet::write_sheet(&tiles, &path, c.sheet_width, c.cols)?;
        println!("capture: sheet {} ({w}x{h}, {} frame(s))", path.display(), tiles.len());
        summary["sheet"] = json!(path);
    }
    fs::write(out.join("capture.json"), serde_json::to_string_pretty(&summary)? + "\n")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// native
// ---------------------------------------------------------------------------

const NATIVE_HELP: &str = "\
usage: cargo xtask capture native --frames <list> [options]
  --frames <list>      frames to write: 10,30,60  221-230  0-300:30
  --total <n>          frames to run (default: last listed frame + 1)
  --scenario <file>    CLASSIC_TEST scenario (CLASSIC_TEST=1 + CLASSIC_TEST_FILE)
  --rom-dir <dir>      ROM directory (default roms/out; a classic-roms roms/out works)
  --no-ui              CLASSIC_NO_UI=1
  --bin <path>         desktop binary (default: build --release -p classic-desktop)
  --env KEY=VALUE      extra environment (repeatable)";

fn native(args: &[String]) -> anyhow::Result<()> {
    let mut c = common_defaults("rom:demo");
    let (mut frames, mut total, mut scenario, mut rom_dir) = (None, None, None, None);
    let (mut no_ui, mut bin, mut env) = (false, None, Vec::<(String, String)>::new());
    let mut i = 0;
    while i < args.len() {
        if !parse_common(&mut c, args, &mut i)? {
            let a = args[i].as_str();
            let mut val = || -> anyhow::Result<String> {
                i += 1;
                args.get(i).cloned().with_context(|| format!("{a} needs a value"))
            };
            match a {
                "--frames" => frames = Some(parse_frames(&val()?)?),
                "--total" => total = Some(val()?.parse::<u64>()?),
                "--scenario" => scenario = Some(PathBuf::from(val()?)),
                "--rom-dir" => rom_dir = Some(PathBuf::from(val()?)),
                "--no-ui" => no_ui = true,
                "--bin" => bin = Some(PathBuf::from(val()?)),
                "--env" => {
                    let kv = val()?;
                    let (k, v) = kv.split_once('=').context("--env takes KEY=VALUE")?;
                    env.push((k.to_string(), v.to_string()));
                }
                "-h" | "--help" => {
                    println!("{NATIVE_HELP}\n{COMMON_HELP}");
                    return Ok(());
                }
                other => bail!("unknown option {other}\n{NATIVE_HELP}\n{COMMON_HELP}"),
            }
        }
        i += 1;
    }
    let frames = frames.context("--frames is required (only the frames you need are written)")?;
    let total = total.unwrap_or(frames[frames.len() - 1] + 1);
    let root = crate::repo_root()?;
    let out = out_dir(&c, &root, "native")?;
    let software = choose_software(&c);
    if env.iter().any(|(k, v)| k == "CLASSIC_WORKERS" && v == "threaded") {
        eprintln!(
            "capture: warning: CLASSIC_WORKERS=threaded paces every frame to --dt in wall \
             time (real time) and is not deterministic; captures should not use it"
        );
    }
    let bin = match bin {
        Some(b) => fs::canonicalize(&b).with_context(|| format!("--bin {}", b.display()))?,
        None => crate::bench::build_release(&root)?,
    };
    let rom_dir =
        fs::canonicalize(rom_dir.unwrap_or_else(|| root.join("roms/out"))).context("--rom-dir")?;
    let rom = if c.rom.starts_with("rom:") || c.rom.contains("://") {
        c.rom.clone()
    } else {
        fs::canonicalize(&c.rom).with_context(|| format!("--rom {}", c.rom))?.display().to_string()
    };
    // Stale review captures in a reused --out would land on the sheet.
    for e in fs::read_dir(&out)?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with("frame_") && name.ends_with(".png") {
            fs::remove_file(e.path())?;
        }
    }

    let list = frames.iter().map(u64::to_string).collect::<Vec<_>>().join(",");
    let mut cmd = Command::new(&bin);
    cmd.current_dir(&root)
        // Never inherit a renderer / pacing / profiler choice from the shell.
        .env_remove("LIBGL_ALWAYS_SOFTWARE")
        .env_remove("LP_NUM_THREADS")
        .env_remove("CLASSIC_WORKERS")
        .env_remove("CLASSIC_PERF")
        .env_remove("CLASSIC_PERF_GPU")
        .env_remove("CLASSIC_GOLDEN")
        .env("CLASSIC_HEADLESS", "1")
        .env("CLASSIC_ROM", &rom)
        .env("CLASSIC_ROM_DIR", &rom_dir)
        .env("CLASSIC_FIXED_DT", c.dt.to_string())
        .env("CLASSIC_FRAMES", total.to_string())
        .env("CLASSIC_WIDTH", c.width.to_string())
        .env("CLASSIC_HEIGHT", c.height.to_string())
        .env("CLASSIC_GOLDEN_PNG", &list)
        .env("CLASSIC_GOLDEN_DIR", &out)
        .env("RUST_LOG", std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()));
    if software {
        // llvmpipe's multithreaded rasteriser races on the sprite ghost pass
        // (CI pins it to one thread for the same reason).
        cmd.env("LIBGL_ALWAYS_SOFTWARE", "1").env("LP_NUM_THREADS", "0");
    }
    if let Some(s) = &scenario {
        let s = fs::canonicalize(s).with_context(|| format!("--scenario {}", s.display()))?;
        cmd.env("CLASSIC_TEST", "1").env("CLASSIC_TEST_FILE", s);
    }
    if no_ui {
        cmd.env("CLASSIC_NO_UI", "1");
    }
    for (k, v) in &env {
        cmd.env(k, v);
    }
    let log_path = out.join("run.log");
    let log = fs::File::create(&log_path)?;
    cmd.stdout(Stdio::from(log.try_clone()?)).stderr(Stdio::from(log));
    eprintln!(
        "capture: native {rom} dt={} {}x{} frames={list} ({total} run) -> {}",
        c.dt,
        c.width,
        c.height,
        out.display()
    );
    let cpu0 = children_cpu_secs();
    let t0 = Instant::now();
    let status = cmd.status().with_context(|| format!("run {}", bin.display()))?;
    let wall = t0.elapsed().as_secs_f64();
    let cpu = children_cpu_secs() - cpu0;

    let text = fs::read_to_string(&log_path).unwrap_or_default();
    let renderer = native_renderer(&text);
    let test = text
        .lines()
        .find_map(|l| l.split_once("=== CLASSIC_TEST COMPLETE: ").map(|(_, r)| r))
        .map(|r| r.trim_end_matches(" ===").to_string());
    let panics = text.lines().filter(|l| l.contains("panicked at")).count();
    let mut tiles: Vec<sheet::Tile> = frames
        .iter()
        .map(|f| out.join(format!("frame_{f:05}.png")))
        .filter(|p| p.exists())
        .map(|path| {
            let stem = path.file_stem().unwrap_or_default().to_string_lossy();
            let n: u64 = stem.trim_start_matches("frame_").parse().unwrap_or(0);
            let label = format!("f{n}");
            sheet::Tile { path, label }
        })
        .collect();
    tiles.sort_by(|a, b| a.path.cmp(&b.path));

    println!("capture: renderer: {}", renderer.as_deref().unwrap_or("<not logged>"));
    println!(
        "capture: wall {wall:.2} s, cpu {cpu:.2} s ({:.0}%), {}/{} frame(s) written{}",
        100.0 * cpu / wall.max(1e-9),
        tiles.len(),
        frames.len(),
        test.as_deref().map(|t| format!(", test: {t}")).unwrap_or_default()
    );
    let summary = json!({
        "mode": "native",
        "rom": rom,
        "renderer": renderer,
        "software": software,
        "dt": c.dt,
        "size": [c.width, c.height],
        "frames": frames,
        "total_frames": total,
        "wall_s": wall,
        "cpu_s": cpu,
        "test": test,
        "panics": panics,
        "exit": status.code(),
        "log": log_path,
    });
    finish(&c, &out, tiles, summary)?;
    if let Some(r) = &renderer {
        check_renderer(r, software)?;
    }
    if !status.success() {
        bail!("classic-desktop exited with {status}; see {}", log_path.display());
    }
    Ok(())
}

/// The renderer string from the headless platform's startup line
/// (`headless: GL renderer=<r> vendor=<v> version=<x.y>`); older binaries
/// only logged the vendor.
fn native_renderer(log: &str) -> Option<String> {
    let line = log.lines().find(|l| l.contains("headless: GL "))?;
    let rest = line.split_once("headless: GL ")?.1;
    match rest.strip_prefix("renderer=") {
        Some(r) => Some(r.split(" vendor=").next().unwrap_or(r).to_string()),
        None => Some(rest.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_lists_parse_ranges_and_steps() {
        assert_eq!(parse_frames("10,30,60").unwrap(), vec![10, 30, 60]);
        assert_eq!(parse_frames("221-224").unwrap(), vec![221, 222, 223, 224]);
        assert_eq!(parse_frames("0-90:30,45").unwrap(), vec![0, 30, 45, 60, 90]);
        assert_eq!(parse_frames("5,5,1").unwrap(), vec![1, 5]);
        assert!(parse_frames("9-1").is_err());
        assert!(parse_frames("").is_err());
        assert!(parse_frames("0-9:0").is_err());
    }

    #[test]
    fn renderer_comes_from_the_headless_line() {
        let log = "x\n[t INFO platform] [f000000] headless: GL renderer=AMD Radeon 680M \
                   (radeonsi, rembrandt) vendor=Mesa 26.2.3 version=3.2\n";
        assert_eq!(native_renderer(log).as_deref(), Some("AMD Radeon 680M (radeonsi, rembrandt)"));
        let old = "[t INFO platform] headless: GL vendor=Mesa 26.2.3 version=3.2";
        assert_eq!(native_renderer(old).as_deref(), Some("vendor=Mesa 26.2.3 version=3.2"));
        assert_eq!(native_renderer("nothing"), None);
    }

    #[test]
    fn software_renderers_are_refused_unless_asked_for() {
        assert!(check_renderer("llvmpipe (LLVM 19.1.7, 256 bits)", false).is_err());
        assert!(check_renderer("llvmpipe (LLVM 19.1.7, 256 bits)", true).is_ok());
        assert!(check_renderer("ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device", false).is_err());
        assert!(check_renderer("AMD Radeon 680M (radeonsi, rembrandt, ACO)", false).is_ok());
    }
}
