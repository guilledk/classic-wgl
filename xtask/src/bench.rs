//! `cargo xtask bench` / `cargo xtask bench-compare`: run a ROM headless under
//! the frame profiler (`CLASSIC_PERF`) and report it per benchmark section.
//!
//! A bench-aware ROM's guest reads `bench_mode()` and marks its sections with
//! `bench_section(name)` / `bench_end()`; the markers land in the profiler's
//! JSON lines next to every frame.  `bench` runs the release desktop binary
//! `--warmup` times (discarded) plus `--runs` times with GPU timers off (wall
//! and CPU numbers), then once with them on (GPU numbers), waiting for an idle
//! machine before each run, and folds the runs into one report (schema
//! `classic-bench` v1).  `bench-compare` prints per-section deltas between two
//! reports against a noise floor.
//!
//! A ROM without bench sections still works with `--frames N` (every frame is
//! then section `main`), optionally driven by a `CLASSIC_TEST` scenario
//! (`--scenario <file>`).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// Report schema name + version.
const SCHEMA: &str = "classic-bench";
const VERSION: u64 = 1;

/// The section of frames before any `bench_section` marker.
const MAIN_SECTION: &str = "main";

struct Opts {
    rom: PathBuf,
    short: bool,
    runs: usize,
    warmup: usize,
    out: Option<PathBuf>,
    width: u32,
    height: u32,
    fixed_dt: f64,
    frames: Option<u64>,
    scenario: Option<PathBuf>,
    skip: usize,
    gpu: bool,
    idle_gate: bool,
    bin: Option<PathBuf>,
    timeout: Duration,
    env: Vec<(String, String)>,
}

fn parse_opts(args: &[String]) -> anyhow::Result<Opts> {
    let mut o = Opts {
        rom: PathBuf::from("roms/out/bench.rom"),
        short: false,
        runs: 3,
        warmup: 1,
        out: None,
        width: 3840,
        height: 2160,
        fixed_dt: 1.0 / 60.0,
        frames: None,
        scenario: None,
        skip: 10,
        gpu: true,
        idle_gate: true,
        bin: None,
        timeout: Duration::from_secs(900),
        env: Vec::new(),
    };
    let mut i = 1;
    let val = |i: &mut usize, flag: &str| -> anyhow::Result<String> {
        *i += 1;
        args.get(*i).cloned().with_context(|| format!("{flag} needs a value"))
    };
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--rom" => o.rom = val(&mut i, a)?.into(),
            "--short" => o.short = true,
            "--runs" => o.runs = val(&mut i, a)?.parse().context("--runs")?,
            "--warmup" => o.warmup = val(&mut i, a)?.parse().context("--warmup")?,
            "--out" => o.out = Some(val(&mut i, a)?.into()),
            "--width" => o.width = val(&mut i, a)?.parse().context("--width")?,
            "--height" => o.height = val(&mut i, a)?.parse().context("--height")?,
            "--fixed-dt" => o.fixed_dt = val(&mut i, a)?.parse().context("--fixed-dt")?,
            "--frames" => o.frames = Some(val(&mut i, a)?.parse().context("--frames")?),
            "--scenario" => o.scenario = Some(val(&mut i, a)?.into()),
            "--skip" => o.skip = val(&mut i, a)?.parse().context("--skip")?,
            "--no-gpu" => o.gpu = false,
            "--no-idle-gate" => o.idle_gate = false,
            "--bin" => o.bin = Some(val(&mut i, a)?.into()),
            "--timeout" => {
                o.timeout = Duration::from_secs(val(&mut i, a)?.parse().context("--timeout")?)
            }
            "--env" => {
                let kv = val(&mut i, a)?;
                let (k, v) = kv.split_once('=').context("--env takes KEY=VALUE")?;
                o.env.push((k.to_string(), v.to_string()));
            }
            other => bail!(
                "unknown bench argument `{other}` (expected --rom, --short, --runs, --warmup, \
                 --out, --width, --height, --fixed-dt, --frames, --scenario, --skip, --no-gpu, \
                 --no-idle-gate, --bin, --timeout, --env)"
            ),
        }
        i += 1;
    }
    if o.runs == 0 {
        bail!("--runs must be at least 1");
    }
    Ok(o)
}

/// `cargo xtask bench [...]`.
pub fn cmd_bench(args: &[String]) -> anyhow::Result<()> {
    let o = parse_opts(args)?;
    let root = super::repo_root()?;
    let rom = fs::canonicalize(&o.rom)
        .with_context(|| format!("ROM {} not found (pass --rom <path>)", o.rom.display()))?;
    let rom_sha = super::hex(&Sha256::digest(fs::read(&rom)?));
    let bin = match &o.bin {
        Some(b) => fs::canonicalize(b).with_context(|| format!("--bin {}", b.display()))?,
        None => build_release(&root)?,
    };
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let out =
        o.out.clone().unwrap_or_else(|| root.join(format!("target/bench/bench-{stamp}.json")));
    let stem = out.with_extension("");
    if let Some(dir) = out.parent() {
        fs::create_dir_all(dir)?;
    }

    // Warmup runs (discarded), measured runs, then the GPU-timer run.
    let mut plan: Vec<(String, bool)> = Vec::new();
    plan.extend((0..o.warmup).map(|i| (format!("w{i}"), false)));
    plan.extend((0..o.runs).map(|i| (format!("r{i}"), false)));
    if o.gpu {
        plan.push(("gpu".to_string(), true));
    }
    let mut runs: Vec<Run> = Vec::new();
    let mut gpu_run: Option<Run> = None;
    for (tag, gpu) in &plan {
        if o.idle_gate {
            idle_gate(Duration::from_secs(1500))?;
        }
        let perf = PathBuf::from(format!("{}.{tag}.jsonl", stem.display()));
        let log = PathBuf::from(format!("{}.{tag}.log", stem.display()));
        eprintln!(
            "bench: run {tag}{} -> {}",
            if *gpu { " (GPU timers)" } else { "" },
            perf.display()
        );
        run_once(&o, &bin, &rom, &perf, &log, *gpu)?;
        let run = Run::load(&perf)?;
        if *gpu {
            gpu_run = Some(run);
        } else if !tag.starts_with('w') {
            runs.push(run);
        }
    }

    let report = build_report(&o, &root, &rom, &rom_sha, &runs, gpu_run.as_ref())?;
    fs::write(&out, serde_json::to_string_pretty(&report)? + "\n")?;
    print_report(&report);
    println!("report: {}", out.display());
    Ok(())
}

/// Build the release desktop binary; returns its path.
fn build_release(root: &Path) -> anyhow::Result<PathBuf> {
    let status = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .current_dir(root)
        .args(["build", "--release", "-p", "classic-desktop"])
        .status()
        .context("cargo build")?;
    if !status.success() {
        bail!("cargo build --release -p classic-desktop failed");
    }
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"));
    let target = if target.is_absolute() { target } else { root.join(target) };
    Ok(target.join("release/classic-desktop"))
}

fn run_once(
    o: &Opts,
    bin: &Path,
    rom: &Path,
    perf: &Path,
    log: &Path,
    gpu: bool,
) -> anyhow::Result<()> {
    let mut cmd = Command::new(bin);
    cmd.env_remove("LIBGL_ALWAYS_SOFTWARE")
        .env("CLASSIC_HEADLESS", "1")
        .env("CLASSIC_WIDTH", o.width.to_string())
        .env("CLASSIC_HEIGHT", o.height.to_string())
        .env("CLASSIC_ROM", rom)
        .env("CLASSIC_ROM_DIR", rom.parent().unwrap_or(Path::new(".")))
        .env("CLASSIC_FIXED_DT", o.fixed_dt.to_string())
        .env("CLASSIC_BENCH", if o.short { "short" } else { "full" })
        .env("CLASSIC_PERF", perf)
        .env("CLASSIC_PERF_GPU", if gpu { "1" } else { "0" })
        .env("RUST_LOG", std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into()));
    if let Some(n) = o.frames {
        cmd.env("CLASSIC_FRAMES", n.to_string());
    } else {
        cmd.env_remove("CLASSIC_FRAMES");
    }
    if let Some(s) = &o.scenario {
        cmd.env("CLASSIC_TEST", "1").env("CLASSIC_TEST_FILE", fs::canonicalize(s)?);
    }
    for (k, v) in &o.env {
        cmd.env(k, v);
    }
    let file = fs::File::create(log)?;
    cmd.stdout(Stdio::from(file.try_clone()?)).stderr(Stdio::from(file));
    let mut child = cmd.spawn().with_context(|| format!("spawn {}", bin.display()))?;
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                bail!("{} exited with {status}; see {}", bin.display(), log.display());
            }
            return Ok(());
        }
        if start.elapsed() > o.timeout {
            let _ = child.kill();
            bail!(
                "run exceeded {}s (a ROM without `bench_end` needs --frames); see {}",
                o.timeout.as_secs(),
                log.display()
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// One frame of a run.
struct Frame {
    wall: f64,
    cpu: f64,
    pace: f64,
    phases: Vec<(String, f64)>,
    counters: Vec<(String, f64)>,
    gpu: Option<(f64, Vec<(String, f64)>)>,
}

/// One run's frames, grouped by section in order.
struct Run {
    renderer: String,
    sections: Vec<Section>,
}

struct Section {
    name: String,
    params: Value,
    frames: Vec<Frame>,
}

fn pairs(v: &Value) -> Vec<(String, f64)> {
    v.as_object()
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.as_f64().unwrap_or(0.0))).collect())
        .unwrap_or_default()
}

impl Run {
    fn load(path: &Path) -> anyhow::Result<Self> {
        let text = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("parse {}", path.display()))
    }

    fn parse(text: &str) -> anyhow::Result<Self> {
        let mut renderer = String::new();
        // (start frame, name, params); `None` name = ended.
        let mut marks: Vec<(u64, Option<String>, Value)> = Vec::new();
        let mut frames: BTreeMap<u64, Frame> = BTreeMap::new();
        let mut gpus: BTreeMap<u64, (f64, Vec<(String, f64)>)> = BTreeMap::new();
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let v: Value = serde_json::from_str(line)?;
            let f = v["f"].as_u64().unwrap_or(0);
            match v["kind"].as_str().unwrap_or("") {
                "header" => renderer = v["renderer"].as_str().unwrap_or("").to_string(),
                "section" => marks.push((
                    f,
                    Some(v["name"].as_str().unwrap_or("?").to_string()),
                    v["params"].clone(),
                )),
                "section_end" => marks.push((f, None, Value::Null)),
                "frame" => {
                    frames.insert(
                        f,
                        Frame {
                            wall: v["wall"].as_f64().unwrap_or(0.0),
                            cpu: v["cpu"].as_f64().unwrap_or(0.0),
                            pace: v["pace"].as_f64().unwrap_or(0.0),
                            phases: pairs(&v["phases"]),
                            counters: pairs(&v["counters"]),
                            gpu: None,
                        },
                    );
                }
                "gpu" => {
                    gpus.insert(f, (v["total"].as_f64().unwrap_or(0.0), pairs(&v["phases"])));
                }
                _ => {}
            }
        }
        if frames.is_empty() {
            bail!("no frame lines (was CLASSIC_PERF honoured?)");
        }
        for (f, g) in gpus {
            if let Some(fr) = frames.get_mut(&f) {
                fr.gpu = Some(g);
            }
        }
        let mut sections: Vec<Section> = Vec::new();
        let mut cur: Option<usize> = None;
        let mut marks = marks.into_iter().peekable();
        if marks.peek().is_none_or(|m| m.0 > 0) {
            sections.push(Section {
                name: MAIN_SECTION.into(),
                params: json!({}),
                frames: Vec::new(),
            });
            cur = Some(0);
        }
        for (f, frame) in frames {
            while marks.peek().is_some_and(|m| m.0 <= f) {
                let (_, name, params) = marks.next().expect("peeked");
                cur = name.map(|name| {
                    sections.push(Section { name, params, frames: Vec::new() });
                    sections.len() - 1
                });
            }
            if let Some(i) = cur {
                sections[i].frames.push(frame);
            }
        }
        Ok(Self { renderer, sections })
    }
}

/// Mean and nearest-rank order statistics.
fn stats(v: &[f64]) -> Value {
    if v.is_empty() {
        return json!({"n": 0});
    }
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let rank = |p: f64| s[((p * s.len() as f64).ceil() as usize).clamp(1, s.len()) - 1];
    json!({
        "n": s.len(),
        "mean": round(s.iter().sum::<f64>() / s.len() as f64),
        "median": round(rank(0.5)),
        "p95": round(rank(0.95)),
        "p99": round(rank(0.99)),
        "max": round(s[s.len() - 1]),
    })
}

fn round(v: f64) -> f64 {
    (v * 1e4).round() / 1e4
}

fn mean(v: &[f64]) -> f64 {
    if v.is_empty() {
        0.0
    } else {
        v.iter().sum::<f64>() / v.len() as f64
    }
}

/// Per-frame means of named values, in first-seen order.
fn means<'a>(rows: impl Iterator<Item = &'a [(String, f64)]>, n: usize) -> Value {
    let mut acc: Vec<(String, f64)> = Vec::new();
    for row in rows {
        for (k, v) in row {
            match acc.iter_mut().find(|(a, _)| a == k) {
                Some(e) => e.1 += v,
                None => acc.push((k.clone(), *v)),
            }
        }
    }
    let mut m = serde_json::Map::new();
    for (k, v) in acc {
        m.insert(k, json!(round(v / n.max(1) as f64)));
    }
    Value::Object(m)
}

fn build_report(
    o: &Opts,
    root: &Path,
    rom: &Path,
    rom_sha: &str,
    runs: &[Run],
    gpu_run: Option<&Run>,
) -> anyhow::Result<Value> {
    let git = |args: &[&str]| -> String {
        Command::new("git")
            .current_dir(root)
            .args(args)
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    };
    let rev = git(&["rev-parse", "HEAD"]);
    let dirty = !git(&["status", "--porcelain", "--untracked-files=no"]).is_empty();

    // Section order: first appearance across the measured runs.
    let mut names: Vec<String> = Vec::new();
    for r in runs {
        for s in &r.sections {
            if !names.contains(&s.name) {
                names.push(s.name.clone());
            }
        }
    }
    let mut sections = Vec::new();
    for name in &names {
        let mut frames: Vec<&Frame> = Vec::new();
        let mut run_wall = Vec::new();
        let mut run_cpu = Vec::new();
        let mut params = Value::Null;
        for r in runs {
            for s in r.sections.iter().filter(|s| &s.name == name) {
                let fr: Vec<&Frame> = s.frames.iter().skip(o.skip).collect();
                run_wall.push(round(mean(&fr.iter().map(|f| f.wall).collect::<Vec<_>>())));
                run_cpu.push(round(mean(&fr.iter().map(|f| f.cpu).collect::<Vec<_>>())));
                if params.is_null() {
                    params = s.params.clone();
                }
                frames.extend(fr);
            }
        }
        let col = |f: fn(&Frame) -> f64| frames.iter().map(|fr| f(fr)).collect::<Vec<_>>();
        let gpu_frames: Vec<&Frame> = gpu_run
            .map(|g| {
                g.sections
                    .iter()
                    .filter(|s| &s.name == name)
                    .flat_map(|s| s.frames.iter().skip(o.skip))
                    .filter(|f| f.gpu.is_some())
                    .collect()
            })
            .unwrap_or_default();
        let gpu_total: Vec<f64> =
            gpu_frames.iter().map(|f| f.gpu.as_ref().expect("gpu").0).collect();
        sections.push(json!({
            "name": name,
            "params": params,
            "frames": frames.len(),
            "wall": stats(&col(|f| f.wall)),
            "cpu": stats(&col(|f| f.cpu)),
            "pace": stats(&col(|f| f.pace)),
            "gpu": stats(&gpu_total),
            "runs": {"wall_mean": run_wall, "cpu_mean": run_cpu},
            "phases_cpu": means(frames.iter().map(|f| f.phases.as_slice()), frames.len()),
            "phases_gpu": means(
                gpu_frames.iter().map(|f| f.gpu.as_ref().expect("gpu").1.as_slice()),
                gpu_frames.len()
            ),
            "counters": means(frames.iter().map(|f| f.counters.as_slice()), frames.len()),
        }));
    }
    Ok(json!({
        "schema": SCHEMA,
        "version": VERSION,
        "engine_rev": rev,
        "engine_dirty": dirty,
        "rom": {"path": rom, "sha256": rom_sha},
        "gl_renderer": runs.first().map(|r| r.renderer.clone()).unwrap_or_default(),
        "resolution": [o.width, o.height],
        "dt": o.fixed_dt,
        "mode": if o.short { "short" } else { "full" },
        "runs": runs.len(),
        "gpu_run": gpu_run.is_some(),
        "skip_frames": o.skip,
        "sections": sections,
    }))
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or(f64::NAN)
}

fn print_report(r: &Value) {
    println!(
        "{} @ {} ({}), {}x{}, {} runs",
        r["rom"]["path"].as_str().unwrap_or("?"),
        &r["engine_rev"].as_str().unwrap_or("?")
            [..r["engine_rev"].as_str().map_or(0, |s| s.len().min(10))],
        r["gl_renderer"].as_str().unwrap_or("?"),
        r["resolution"][0],
        r["resolution"][1],
        r["runs"]
    );
    println!(
        "{:<24} {:>6} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8}",
        "section", "frames", "wall", "median", "p95", "cpu", "pace", "gpu"
    );
    for s in r["sections"].as_array().into_iter().flatten() {
        println!(
            "{:<24} {:>6} {:>8.2} {:>8.2} {:>8.2} {:>8.2} {:>8.2} {:>8.2}",
            s["name"].as_str().unwrap_or("?"),
            s["frames"],
            num(&s["wall"]["mean"]),
            num(&s["wall"]["median"]),
            num(&s["wall"]["p95"]),
            num(&s["cpu"]["mean"]),
            num(&s["pace"]["mean"]),
            num(&s["gpu"]["mean"]),
        );
    }
}

/// Block until the machine is idle: no `classic-desktop` running, no process
/// above 50% of a core over 2 s, and the GPU under 20% busy (when the driver
/// exposes `gpu_busy_percent`).
fn idle_gate(timeout: Duration) -> anyhow::Result<()> {
    if !cfg!(target_os = "linux") {
        eprintln!("bench: idle gate needs /proc; skipped");
        return Ok(());
    }
    let start = Instant::now();
    loop {
        let before = proc_ticks();
        std::thread::sleep(Duration::from_secs(2));
        let after = proc_ticks();
        let hz = 100.0;
        let mut top = (0.0f64, String::new());
        for (pid, (comm, t1)) in &after {
            if let Some((_, t0)) = before.get(pid) {
                let pct = (t1.saturating_sub(*t0)) as f64 / (2.0 * hz) * 100.0;
                if pct > top.0 {
                    top = (pct, comm.clone());
                }
            }
        }
        let running = after.values().any(|(c, _)| c == "classic-desktop");
        let gpu = gpu_busy();
        if !running && top.0 < 50.0 && gpu < 20 {
            return Ok(());
        }
        eprintln!(
            "bench: waiting for idle (top {:.0}% {}, gpu {gpu}%, classic-desktop running: {running})",
            top.0, top.1
        );
        if start.elapsed() > timeout {
            bail!("machine never went idle ({}s)", timeout.as_secs());
        }
        std::thread::sleep(Duration::from_secs(3));
    }
}

/// `pid -> (comm, utime + stime ticks)` for every process.
fn proc_ticks() -> BTreeMap<u32, (String, u64)> {
    let mut out = BTreeMap::new();
    let Ok(dir) = fs::read_dir("/proc") else { return out };
    for e in dir.flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        let Ok(stat) = fs::read_to_string(e.path().join("stat")) else { continue };
        let (Some(l), Some(r)) = (stat.find('('), stat.rfind(')')) else { continue };
        let comm = stat[l + 1..r].to_string();
        let fields: Vec<&str> = stat[r + 1..].split_whitespace().collect();
        // Fields after `)` start at `state` (field 3); utime/stime are 14/15.
        let t = |i: usize| fields.get(i).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
        out.insert(pid, (comm, t(11) + t(12)));
    }
    out
}

/// The busiest GPU's `gpu_busy_percent` (0 when no driver exposes it).
fn gpu_busy() -> u32 {
    let Ok(dir) = fs::read_dir("/sys/class/drm") else { return 0 };
    dir.flatten()
        .filter_map(|e| fs::read_to_string(e.path().join("device/gpu_busy_percent")).ok())
        .filter_map(|s| s.trim().parse().ok())
        .max()
        .unwrap_or(0)
}

/// `cargo xtask bench-compare <a.json> <b.json> [--noise-pct P]`.
pub fn cmd_bench_compare(args: &[String]) -> anyhow::Result<()> {
    let mut files = Vec::new();
    let mut noise_pct = 2.0;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--noise-pct" => {
                i += 1;
                noise_pct = args.get(i).context("--noise-pct needs a value")?.parse()?;
            }
            f => files.push(f.to_string()),
        }
        i += 1;
    }
    let [a, b] = files.as_slice() else {
        bail!("usage: cargo xtask bench-compare <a.json> <b.json> [--noise-pct P]");
    };
    let load = |p: &str| -> anyhow::Result<Value> {
        let v: Value =
            serde_json::from_str(&fs::read_to_string(p).with_context(|| p.to_string())?)?;
        if v["schema"] != SCHEMA || v["version"].as_u64() != Some(VERSION) {
            bail!("{p}: not a {SCHEMA} v{VERSION} report");
        }
        Ok(v)
    };
    let (a, b) = (load(a)?, load(b)?);
    print!("{}", compare(&a, &b, noise_pct));
    Ok(())
}

/// The run-to-run standard deviation of a section's per-run means.
fn run_sd(s: &Value, key: &str) -> f64 {
    let v: Vec<f64> =
        s["runs"][key].as_array().into_iter().flatten().filter_map(Value::as_f64).collect();
    if v.len() < 2 {
        return 0.0;
    }
    let m = mean(&v);
    (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (v.len() - 1) as f64).sqrt()
}

/// The comparison table (one row per metric per shared section).  A delta
/// counts only past the noise floor: the larger of `noise_pct` of the base,
/// 0.02 ms and twice the larger run-to-run deviation (wall/CPU means).
fn compare(a: &Value, b: &Value, noise_pct: f64) -> String {
    let mut out = String::new();
    let rev =
        |r: &Value| r["engine_rev"].as_str().unwrap_or("?").chars().take(10).collect::<String>();
    out += &format!(
        "{} -> {}   {} {}x{}\n",
        rev(a),
        rev(b),
        a["gl_renderer"].as_str().unwrap_or("?"),
        a["resolution"][0],
        a["resolution"][1]
    );
    if a["rom"]["sha256"] != b["rom"]["sha256"] {
        out += "WARNING: different ROMs\n";
    }
    if a["resolution"] != b["resolution"] || a["gl_renderer"] != b["gl_renderer"] {
        out += "WARNING: different resolution or GL renderer\n";
    }
    let empty = vec![];
    let bs = b["sections"].as_array().unwrap_or(&empty);
    for sa in a["sections"].as_array().unwrap_or(&empty) {
        let name = sa["name"].as_str().unwrap_or("?");
        let Some(sb) = bs.iter().find(|s| s["name"] == sa["name"]) else {
            out += &format!("{name}: only in a\n");
            continue;
        };
        out += &format!("{name} ({} -> {} frames)\n", sa["frames"], sb["frames"]);
        for (metric, stat, sd_key) in [
            ("wall", "mean", Some("wall_mean")),
            ("wall", "median", None),
            ("wall", "p95", None),
            ("cpu", "mean", Some("cpu_mean")),
            ("cpu", "p95", None),
            ("gpu", "mean", None),
        ] {
            let (va, vb) = (num(&sa[metric][stat]), num(&sb[metric][stat]));
            if va.is_nan() || vb.is_nan() {
                continue;
            }
            let sd = sd_key.map_or(0.0, |k| run_sd(sa, k).max(run_sd(sb, k)));
            let floor = (va.abs() * noise_pct / 100.0).max(0.02).max(2.0 * sd);
            let d = vb - va;
            let verdict = if d.abs() <= floor {
                "within noise"
            } else if d < 0.0 {
                "faster"
            } else {
                "slower"
            };
            let pct = if va != 0.0 { d / va * 100.0 } else { 0.0 };
            out += &format!(
                "  {:<11} {:>8.2} -> {:>8.2}  {:>+7.2} ms {:>+6.1}%  (floor {:.2})  {verdict}\n",
                format!("{metric}.{stat}"),
                va,
                vb,
                d,
                pct,
                floor
            );
        }
    }
    for sb in bs {
        if !a["sections"].as_array().unwrap_or(&empty).iter().any(|s| s["name"] == sb["name"]) {
            out += &format!("{}: only in b\n", sb["name"].as_str().unwrap_or("?"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINES: &str = r#"{"perf":1,"kind":"header","renderer":"R","gpu":false}
{"kind":"frame","f":0,"wall":0,"cpu":5,"pace":0,"phases":{"update":1},"counters":{}}
{"kind":"section","f":1,"name":"a","params":{"zoom":1.0}}
{"kind":"frame","f":1,"wall":10,"cpu":6,"pace":1,"phases":{"update":2},"counters":{"lights":4}}
{"kind":"frame","f":2,"wall":12,"cpu":8,"pace":1,"phases":{"update":4},"counters":{"lights":6}}
{"kind":"gpu","f":1,"total":9,"phases":{"models":9}}
{"kind":"section_end","f":3}
{"kind":"frame","f":3,"wall":30,"cpu":30,"pace":0,"phases":{},"counters":{}}
"#;

    #[test]
    fn frames_land_in_their_sections() {
        let r = Run::parse(LINES).unwrap();
        assert_eq!(r.renderer, "R");
        let names: Vec<_> = r.sections.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["main", "a"]);
        assert_eq!(r.sections[0].frames.len(), 1);
        // Frame 3 is past `section_end`: in no section.
        assert_eq!(r.sections[1].frames.len(), 2);
        assert_eq!(r.sections[1].params["zoom"], 1.0);
        assert_eq!(r.sections[1].frames[0].gpu.as_ref().unwrap().0, 9.0);
    }

    #[test]
    fn report_aggregates_runs_and_compares() {
        let runs = vec![Run::parse(LINES).unwrap(), Run::parse(LINES).unwrap()];
        let mut o = parse_opts(&["bench".into()]).unwrap();
        o.skip = 0;
        let r = build_report(&o, Path::new("."), Path::new("x.rom"), "00", &runs, Some(&runs[0]))
            .unwrap();
        let a = &r["sections"][1];
        assert_eq!(a["name"], "a");
        assert_eq!(a["frames"], 4);
        assert_eq!(a["wall"]["mean"], 11.0);
        assert_eq!(a["cpu"]["median"], 6.0);
        assert_eq!(a["gpu"]["n"], 1);
        assert_eq!(a["phases_cpu"]["update"], 3.0);
        assert_eq!(a["counters"]["lights"], 5.0);
        assert_eq!(a["runs"]["wall_mean"], json!([11.0, 11.0]));
        // Identical reports: every row is within noise.
        let c = compare(&r, &r, 2.0);
        assert!(c.contains("wall.mean") && !c.contains("faster") && !c.contains("slower"), "{c}");
        // A 10% faster wall mean in `b` shows.
        let mut b = r.clone();
        b["sections"][1]["wall"]["mean"] = json!(9.9);
        assert!(compare(&r, &b, 2.0).contains("faster"));
    }

    #[test]
    fn stats_are_nearest_rank() {
        let v: Vec<f64> = (1..=100).map(f64::from).collect();
        let s = stats(&v);
        assert_eq!((s["median"].as_f64(), s["p95"].as_f64()), (Some(50.0), Some(95.0)));
        assert_eq!(stats(&[])["n"], 0);
    }
}
