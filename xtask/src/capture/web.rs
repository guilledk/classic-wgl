//! `cargo xtask capture web`: a scripted GPU headless-Chromium run of the web
//! build over the Chrome DevTools Protocol.
//!
//! The dist is served by a small built-in HTTP server (with the COOP/COEP
//! headers `trunk serve` sends, so `SharedArrayBuffer` paths match the dev
//! loop).  With `--roms <dir>`, every `https://classic-roms.com/<file>`
//! request whose file exists in `<dir>` is fulfilled from it (CDP `Fetch`),
//! so an unpublished ROM set — a classic-roms `roms/out`, `roms.json`
//! included — boots exactly as it would from the CDN.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, Context};
use base64::Engine as _;
use serde_json::{json, Value};
use tungstenite::{Message, WebSocket};

use super::{sheet, Common};

/// The Chromium flags validated on Mesa radv/radeonsi (AMD Radeon 680M):
/// ANGLE on Vulkan, GPU blocklist ignored, and no vsync / frame-rate cap so a
/// `?fixed_dt=` run steps as fast as the GPU allows.
const GPU_FLAGS: &[&str] =
    &["--enable-gpu", "--ignore-gpu-blocklist", "--use-angle=vulkan", "--enable-features=Vulkan"];
/// `--uncapped`: no vsync, no frame-rate cap.
const UNCAPPED_FLAGS: &[&str] = &["--disable-gpu-vsync", "--disable-frame-rate-limit"];
/// The software fallback (`--software`, or no GPU render node).
const SOFTWARE_FLAGS: &[&str] = &["--use-angle=swiftshader", "--enable-unsafe-swiftshader"];

/// A page-side rAF counter, installed before any page script runs, so a
/// script can wait for N rendered frames instead of N seconds.
const FRAME_COUNTER_JS: &str = "(() => { const s = window.__classicCapture = { frames: 0 }; \
     const t = () => { s.frames++; requestAnimationFrame(t); }; requestAnimationFrame(t); })();";

/// The WebGL2 renderer string as the page sees it.
const RENDERER_JS: &str =
    "(() => { const gl = document.createElement('canvas').getContext('webgl2'); \
     if (!gl) return 'no webgl2'; const e = gl.getExtension('WEBGL_debug_renderer_info'); \
     return e ? gl.getParameter(e.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER); })()";

/// The script run when none is given: wait for boot, settle, one shot.
const DEFAULT_SCRIPT: &[&str] =
    &["wait-log classic-demo initialized 180", "wait-frames 30", "shot boot"];

const WEB_HELP: &str = "\
usage: cargo xtask capture web [options]
  --dist <dir>         a `trunk build` output (default: trunk build --release into target/capture-web-dist)
  --roms <dir>         fulfil classic-roms.com/<file> from <dir> when present (e.g. classic-roms roms/out)
  --script <file>      scenario script, one step per line (# comments)
  --step <step>        one script step (repeatable; appended after --script)
  --query <k=v>        extra URL query parameter (repeatable), e.g. loader=visual
  --timeout <s>        overall budget (default 300)
  --chromium <path>    Chromium binary (default: chromium on PATH)
  --uncapped           no vsync / frame-rate cap: max sim throughput, but screenshots lag the sim
                       (use for smoke runs without shots; capped 60 fps + a coarse --dt otherwise)
script steps:
  wait-log <text>[|<text>...] [timeout_s]   wait for a console line containing any text
  wait-frames <n>                           wait for n rendered frames (rAF)
  sleep <s>
  move <x> <y> | click <x> <y> [right] | wheel <x> <y> <dy>
  key <key> [code] [hold_frames]
  shot <name>                               screenshot -> <out>/<name>.png
  eval <js>";

struct WebOpts {
    dist: Option<PathBuf>,
    roms: Option<PathBuf>,
    steps: Vec<String>,
    query: Vec<String>,
    timeout: Duration,
    chromium: Option<PathBuf>,
    uncapped: bool,
}

pub fn run(args: &[String]) -> anyhow::Result<()> {
    let mut c = super::common_defaults("rom:lunar");
    let mut o = WebOpts {
        dist: None,
        roms: None,
        steps: Vec::new(),
        query: Vec::new(),
        timeout: Duration::from_secs(300),
        chromium: None,
        uncapped: false,
    };
    let mut script: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        if !super::parse_common(&mut c, args, &mut i)? {
            let a = args[i].as_str();
            let mut val = || -> anyhow::Result<String> {
                i += 1;
                args.get(i).cloned().with_context(|| format!("{a} needs a value"))
            };
            match a {
                "--dist" => o.dist = Some(PathBuf::from(val()?)),
                "--roms" => o.roms = Some(PathBuf::from(val()?)),
                "--script" => script = Some(PathBuf::from(val()?)),
                "--step" => o.steps.push(val()?),
                "--query" => o.query.push(val()?),
                "--timeout" => o.timeout = Duration::from_secs_f64(val()?.parse()?),
                "--chromium" => o.chromium = Some(PathBuf::from(val()?)),
                "--uncapped" => o.uncapped = true,
                "-h" | "--help" => {
                    println!("{WEB_HELP}\n{}", super::COMMON_HELP);
                    return Ok(());
                }
                other => bail!("unknown option {other}\n{WEB_HELP}\n{}", super::COMMON_HELP),
            }
        }
        i += 1;
    }
    let mut steps: Vec<String> = match &script {
        Some(p) => fs::read_to_string(p)
            .with_context(|| format!("--script {}", p.display()))?
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(String::from)
            .collect(),
        None => Vec::new(),
    };
    steps.append(&mut o.steps);
    if steps.is_empty() {
        steps = DEFAULT_SCRIPT.iter().map(|s| s.to_string()).collect();
    }

    let root = crate::repo_root()?;
    let out = super::out_dir(&c, &root, "web")?;
    let software = super::choose_software(&c);
    let dist = match &o.dist {
        Some(d) => fs::canonicalize(d).with_context(|| format!("--dist {}", d.display()))?,
        None => build_dist(&root)?,
    };
    anyhow::ensure!(dist.join("index.html").exists(), "{} has no index.html", dist.display());
    let roms = match &o.roms {
        Some(r) => Some(fs::canonicalize(r).with_context(|| format!("--roms {}", r.display()))?),
        None => None,
    };

    let listener = TcpListener::bind("127.0.0.1:0").context("bind the dist server")?;
    let port = listener.local_addr()?.port();
    let (served, served_roms) = (dist.clone(), roms.clone());
    std::thread::spawn(move || serve(listener, served, served_roms));
    let url = page_url(port, &c, &o.query);

    let chromium = match &o.chromium {
        Some(p) => p.clone(),
        None => which("chromium")
            .or_else(|| which("chromium-browser"))
            .context("no chromium on PATH (run inside `nix develop`)")?,
    };
    let profile = out.join(".chromium-profile");
    let _ = fs::remove_dir_all(&profile);
    fs::create_dir_all(&profile)?;
    let mut flags: Vec<String> = vec![
        "--headless=new".into(),
        "--no-sandbox".into(),
        "--no-first-run".into(),
        "--remote-debugging-port=0".into(),
        format!("--user-data-dir={}", profile.display()),
        format!("--window-size={},{}", c.width, c.height),
        "--disk-cache-size=1".into(),
    ];
    flags.extend((if software { SOFTWARE_FLAGS } else { GPU_FLAGS }).iter().map(|s| s.to_string()));
    if o.uncapped {
        flags.extend(UNCAPPED_FLAGS.iter().map(|s| s.to_string()));
    }
    flags.push("about:blank".into());
    eprintln!("capture: web {url} -> {}", out.display());
    eprintln!("capture: chromium {}", flags.join(" "));

    let t0 = Instant::now();
    let chromium_log = fs::File::create(out.join("chromium.log"))?;
    let mut child = Command::new(&chromium)
        .args(&flags)
        // Its own process group, so every Chromium process is CPU-accounted.
        .process_group(0)
        .stdout(Stdio::null())
        .stderr(Stdio::from(chromium_log))
        .spawn()
        .with_context(|| format!("spawn {}", chromium.display()))?;
    let group_cpu = super::GroupCpu::start(child.id());

    let (mut run, err) =
        drive(&mut child, &profile, &out, &url, &steps, roms.as_deref(), &c, &o, t0);
    // A clean Browser.close lets Chromium reap its GPU/renderer processes,
    // so their CPU time is counted below.
    if let Some(cdp) = run.cdp.as_mut() {
        let _ = cdp.send("Browser.close", json!({}));
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait()?.is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    if child.try_wait()?.is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let wall = t0.elapsed().as_secs_f64();
    let cpu = group_cpu.finish();
    let _ = fs::remove_dir_all(&profile);

    fs::write(out.join("console.log"), run.lines.join("\n") + "\n")?;
    let errors: Vec<&String> = run
        .lines
        .iter()
        .filter(|l| {
            let body = l.split_once("] ").map(|(_, b)| b).unwrap_or(l);
            body.starts_with("error:")
                || body.starts_with("exception:")
                || body.contains("boot failed")
                || body.contains("panicked at")
        })
        .filter(|l| !l.contains("favicon.ico"))
        .collect();
    let renderer = run.renderer.clone();
    println!("capture: renderer: {}", renderer.as_deref().unwrap_or("<unknown>"));
    println!(
        "capture: wall {wall:.2} s, cpu {cpu:.2} s ({:.0}%), {} shot(s), {} console line(s), {} error(s)",
        100.0 * cpu / wall.max(1e-9),
        run.shots.len(),
        run.lines.len(),
        errors.len()
    );
    for e in errors.iter().take(10) {
        println!("capture:   {e}");
    }
    let tiles: Vec<sheet::Tile> = run
        .shots
        .iter()
        .map(|(name, path)| sheet::Tile { path: path.clone(), label: name.clone() })
        .collect();
    let summary = json!({
        "mode": "web",
        "url": url,
        "renderer": renderer,
        "software": software,
        "chromium_flags": flags,
        "dt": c.dt,
        "size": [c.width, c.height],
        "steps": steps,
        "shots": run.shots.iter().map(|(_, p)| p).collect::<Vec<_>>(),
        "wall_s": wall,
        "cpu_s": cpu,
        "errors": errors,
        "console": out.join("console.log"),
    });
    super::finish(&c, &out, tiles, summary)?;
    if let Some(e) = err {
        return Err(e);
    }
    if let Some(r) = &renderer {
        super::check_renderer(r, software)?;
    }
    if !errors.is_empty() {
        bail!("{} console error(s); see {}", errors.len(), out.join("console.log").display());
    }
    Ok(())
}

/// What a drive produced, kept even when it failed part-way.
#[derive(Default)]
struct Run {
    cdp: Option<Cdp>,
    lines: Vec<String>,
    shots: Vec<(String, PathBuf)>,
    renderer: Option<String>,
}

#[allow(clippy::too_many_arguments)]
fn drive(
    child: &mut Child,
    profile: &Path,
    out: &Path,
    url: &str,
    steps: &[String],
    roms: Option<&Path>,
    c: &Common,
    o: &WebOpts,
    t0: Instant,
) -> (Run, Option<anyhow::Error>) {
    let mut run = Run::default();
    let deadline = t0 + o.timeout;
    let cdp = match connect(child, profile, roms, t0) {
        Ok(cdp) => cdp,
        Err(e) => return (run, Some(e)),
    };
    run.cdp = Some(cdp);
    let r = (|| -> anyhow::Result<()> {
        let cdp = run.cdp.as_mut().unwrap();
        for d in ["Runtime.enable", "Log.enable", "Page.enable"] {
            cdp.call(d, json!({}))?;
        }
        if roms.is_some() {
            cdp.call(
                "Fetch.enable",
                json!({ "patterns": [{ "urlPattern": "https://classic-roms.com/*" }] }),
            )?;
        }
        cdp.call("Page.addScriptToEvaluateOnNewDocument", json!({ "source": FRAME_COUNTER_JS }))?;
        cdp.call(
            "Emulation.setDeviceMetricsOverride",
            json!({ "width": c.width, "height": c.height, "deviceScaleFactor": 1, "mobile": false }),
        )?;
        cdp.call("Page.navigate", json!({ "url": url }))?;
        let renderer = cdp.eval(RENDERER_JS)?;
        let renderer = renderer.as_str().unwrap_or("?").to_string();
        run.renderer = Some(renderer);
        let mut cursor = 0usize;
        for step in steps {
            anyhow::ensure!(Instant::now() < deadline, "timed out before step `{step}`");
            cdp.record(format!("step: {step}"));
            run_step(cdp, step, out, &mut run.shots, &mut cursor, deadline)
                .with_context(|| format!("step `{step}`"))?;
        }
        Ok(())
    })();
    if let Some(cdp) = run.cdp.as_mut() {
        run.lines = std::mem::take(&mut cdp.lines);
    }
    (run, r.err())
}

fn run_step(
    cdp: &mut Cdp,
    step: &str,
    out: &Path,
    shots: &mut Vec<(String, PathBuf)>,
    cursor: &mut usize,
    deadline: Instant,
) -> anyhow::Result<()> {
    let (cmd, rest) = step.split_once(char::is_whitespace).unwrap_or((step, ""));
    let rest = rest.trim();
    let a: Vec<&str> = rest.split_whitespace().collect();
    let f = |n: usize| -> anyhow::Result<f64> {
        a.get(n)
            .with_context(|| format!("missing argument {}", n + 1))?
            .parse::<f64>()
            .map_err(Into::into)
    };
    match cmd {
        "wait-log" => {
            // The last token is a timeout when it parses as a number.
            let (text, secs) = match rest.rsplit_once(' ') {
                Some((t, s)) if s.parse::<f64>().is_ok() => (t, s.parse::<f64>()?),
                _ => (rest, 120.0),
            };
            let wanted: Vec<&str> = text.split('|').map(str::trim).collect();
            let until = (Instant::now() + Duration::from_secs_f64(secs)).min(deadline);
            loop {
                if let Some(n) = cdp.lines[*cursor..]
                    .iter()
                    .position(|l| wanted.iter().any(|w| l.contains(w)) && !l.contains("] step: "))
                {
                    *cursor += n + 1;
                    return Ok(());
                }
                if let Some(l) = cdp
                    .lines
                    .iter()
                    .find(|l| l.contains("boot failed") || l.contains("panicked at"))
                {
                    bail!("the page failed while waiting: {l}");
                }
                anyhow::ensure!(
                    Instant::now() < until,
                    "no console line matching `{text}` within {secs} s"
                );
                cdp.pump(Instant::now() + Duration::from_millis(50))?;
            }
        }
        "wait-frames" => cdp.wait_frames(f(0)? as u64, deadline)?,
        "sleep" => cdp.pump(Instant::now() + Duration::from_secs_f64(f(0)?))?,
        "move" => cdp.mouse("mouseMoved", f(0)?, f(1)?, "none", 0)?,
        "click" => {
            let (x, y) = (f(0)?, f(1)?);
            let button = if a.get(2) == Some(&"right") { "right" } else { "left" };
            let buttons = if button == "right" { 2 } else { 1 };
            cdp.mouse("mouseMoved", x, y, "none", 0)?;
            cdp.wait_frames(2, deadline)?;
            cdp.mouse("mousePressed", x, y, button, buttons)?;
            cdp.wait_frames(2, deadline)?;
            cdp.mouse("mouseReleased", x, y, button, 0)?;
        }
        "wheel" => {
            cdp.call(
                "Input.dispatchMouseEvent",
                json!({ "type": "mouseWheel", "x": f(0)?, "y": f(1)?, "deltaX": 0, "deltaY": f(2)? }),
            )?;
        }
        "key" => {
            let key = *a.first().context("key needs a key name")?;
            let code = a.get(1).copied().unwrap_or(key);
            let hold = a.get(2).map(|h| h.parse::<u64>()).transpose()?.unwrap_or(2);
            cdp.call(
                "Input.dispatchKeyEvent",
                json!({ "type": "keyDown", "key": key, "code": code }),
            )?;
            cdp.wait_frames(hold, deadline)?;
            cdp.call(
                "Input.dispatchKeyEvent",
                json!({ "type": "keyUp", "key": key, "code": code }),
            )?;
        }
        "shot" => {
            let name = a.first().copied().unwrap_or("shot");
            anyhow::ensure!(
                name.chars().all(|ch| ch.is_ascii_alphanumeric() || "_-.".contains(ch)),
                "shot names are [A-Za-z0-9_.-]"
            );
            let r = cdp.call("Page.captureScreenshot", json!({ "format": "png" }))?;
            let data = r["data"].as_str().context("screenshot without data")?;
            let path = out.join(format!("{name}.png"));
            fs::write(&path, base64::engine::general_purpose::STANDARD.decode(data)?)?;
            shots.push((name.to_string(), path));
        }
        "eval" => {
            let v = cdp.eval(rest)?;
            cdp.record(format!("eval: {v}"));
        }
        other => bail!("unknown step `{other}`"),
    }
    Ok(())
}

/// The page URL: the ROM, the fixed step, the console loader (boot events in
/// the console; the visual loader is opt-in via `--query loader=visual`),
/// then the caller's extra parameters.
fn page_url(port: u16, c: &Common, query: &[String]) -> String {
    let mut params: Vec<(String, String)> = vec![
        ("rom".into(), c.rom.clone()),
        ("fixed_dt".into(), c.dt.to_string()),
        ("loader".into(), "console".into()),
    ];
    for q in query {
        let (k, v) = q.split_once('=').unwrap_or((q, ""));
        params.retain(|(pk, _)| pk != k);
        params.push((k.to_string(), v.to_string()));
    }
    let qs: Vec<String> = params
        .iter()
        .map(|(k, v)| if v.is_empty() { k.clone() } else { format!("{k}={v}") })
        .collect();
    format!("http://127.0.0.1:{port}/?{}", qs.join("&"))
}

/// `trunk build --release` the web app into `target/capture-web-dist`.
fn build_dist(root: &Path) -> anyhow::Result<PathBuf> {
    let dist = root.join("target/capture-web-dist");
    eprintln!("capture: trunk build --release -> {}", dist.display());
    let status = Command::new("trunk")
        .current_dir(root)
        .args(["build", "apps/web/index.html", "--release", "--dist"])
        .arg(&dist)
        .status()
        .context("run trunk (inside `nix develop`)")?;
    anyhow::ensure!(status.success(), "trunk build failed");
    Ok(dist)
}

fn which(bin: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")?
        .to_string_lossy()
        .split(':')
        .map(|d| Path::new(d).join(bin))
        .find(|p| p.is_file())
}

// ---------------------------------------------------------------------------
// The dist server
// ---------------------------------------------------------------------------

/// Serve `dist`, plus `roms` (when given) under `/roms/`, so `--rom roms/<name>.rom`
/// boots a local ROM by relative URL.
fn serve(listener: TcpListener, dist: PathBuf, roms: Option<PathBuf>) {
    for stream in listener.incoming().flatten() {
        let (dist, roms) = (dist.clone(), roms.clone());
        std::thread::spawn(move || {
            let _ = serve_one(stream, &dist, roms.as_deref());
        });
    }
}

fn serve_one(mut stream: TcpStream, dist: &Path, roms: Option<&Path>) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    loop {
        let mut request = String::new();
        if reader.read_line(&mut request)? == 0 {
            return Ok(());
        }
        // Drain the headers; keep-alive is honoured (HTTP/1.1 default).
        let mut close = false;
        loop {
            let mut h = String::new();
            if reader.read_line(&mut h)? == 0 || h == "\r\n" || h == "\n" {
                break;
            }
            if h.to_ascii_lowercase().starts_with("connection: close") {
                close = true;
            }
        }
        let mut parts = request.split_whitespace();
        let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or("/"));
        let path = target.split(['?', '#']).next().unwrap_or("/");
        let rel = path.trim_start_matches('/');
        let rel = if rel.is_empty() { "index.html" } else { rel };
        let file = (!rel.split('/').any(|s| s == ".."))
            .then(|| {
                let in_roms = rel.strip_prefix("roms/").zip(roms).map(|(f, d)| d.join(f));
                in_roms.filter(|p| p.is_file()).unwrap_or_else(|| dist.join(rel))
            })
            .filter(|p| p.is_file());
        let (status, body, ctype) = match file {
            Some(p) => ("200 OK", fs::read(&p)?, content_type(&p)),
            None => ("404 Not Found", b"not found".to_vec(), "text/plain"),
        };
        let head = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\n\
             Cross-Origin-Opener-Policy: same-origin\r\nCross-Origin-Embedder-Policy: require-corp\r\n\
             Cache-Control: no-store\r\n\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes())?;
        if method != "HEAD" {
            stream.write_all(&body)?;
        }
        if close {
            return Ok(());
        }
    }
}

fn content_type(p: &Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript",
        "wasm" => "application/wasm",
        "json" => "application/json",
        "css" => "text/css",
        "png" => "image/png",
        _ => "application/octet-stream",
    }
}

// ---------------------------------------------------------------------------
// CDP
// ---------------------------------------------------------------------------

struct Cdp {
    ws: WebSocket<TcpStream>,
    next_id: u64,
    replies: HashMap<u64, Value>,
    lines: Vec<String>,
    roms: Option<PathBuf>,
    t0: Instant,
}

/// Wait for Chromium's `DevToolsActivePort`, then attach to its page target.
fn connect(
    child: &mut Child,
    profile: &Path,
    roms: Option<&Path>,
    t0: Instant,
) -> anyhow::Result<Cdp> {
    let port_file = profile.join("DevToolsActivePort");
    let until = Instant::now() + Duration::from_secs(30);
    let port: u16 = loop {
        if let Some(status) = child.try_wait()? {
            bail!("chromium exited during startup ({status}); see chromium.log");
        }
        if let Ok(s) = fs::read_to_string(&port_file) {
            if let Some(Ok(p)) = s.lines().next().map(str::parse) {
                break p;
            }
        }
        anyhow::ensure!(Instant::now() < until, "chromium never wrote DevToolsActivePort");
        std::thread::sleep(Duration::from_millis(50));
    };
    let ws_url = loop {
        let targets: Value = ureq::get(&format!("http://127.0.0.1:{port}/json/list"))
            .call()
            .ok()
            .and_then(|r| r.into_string().ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(Value::Null);
        if let Some(u) = targets.as_array().and_then(|ts| {
            ts.iter().find(|t| t["type"] == "page").and_then(|t| t["webSocketDebuggerUrl"].as_str())
        }) {
            break u.to_string();
        }
        anyhow::ensure!(Instant::now() < until, "no page target on the DevTools port");
        std::thread::sleep(Duration::from_millis(50));
    };
    let stream = TcpStream::connect(("127.0.0.1", port))?;
    stream.set_nodelay(true)?;
    let config = tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(512 << 20))
        .max_frame_size(Some(512 << 20));
    let (ws, _) = tungstenite::client::client_with_config(ws_url.as_str(), stream, Some(config))
        .map_err(|e| anyhow::anyhow!("DevTools websocket handshake: {e}"))?;
    ws.get_ref().set_read_timeout(Some(Duration::from_millis(20)))?;
    Ok(Cdp {
        ws,
        next_id: 0,
        replies: HashMap::new(),
        lines: Vec::new(),
        roms: roms.map(Path::to_path_buf),
        t0,
    })
}

impl Cdp {
    fn record(&mut self, line: String) {
        self.lines.push(format!("[{:8.2}] {line}", self.t0.elapsed().as_secs_f64()));
    }

    fn send(&mut self, method: &str, params: Value) -> anyhow::Result<u64> {
        self.next_id += 1;
        let msg = json!({ "id": self.next_id, "method": method, "params": params });
        self.ws.send(Message::text(msg.to_string())).context("DevTools send")?;
        Ok(self.next_id)
    }

    /// Send and wait for the reply (events keep being handled meanwhile).
    fn call(&mut self, method: &str, params: Value) -> anyhow::Result<Value> {
        let id = self.send(method, params)?;
        let until = Instant::now() + Duration::from_secs(120);
        loop {
            if let Some(r) = self.replies.remove(&id) {
                if let Some(e) = r.get("error") {
                    bail!("{method}: {e}");
                }
                return Ok(r.get("result").cloned().unwrap_or(Value::Null));
            }
            anyhow::ensure!(Instant::now() < until, "{method}: no reply in 120 s");
            self.read_one()?;
        }
    }

    fn eval(&mut self, expr: &str) -> anyhow::Result<Value> {
        let r = self.call(
            "Runtime.evaluate",
            json!({ "expression": expr, "returnByValue": true, "awaitPromise": true }),
        )?;
        if let Some(e) = r.get("exceptionDetails") {
            bail!("eval `{expr}` threw: {e}");
        }
        Ok(r["result"]["value"].clone())
    }

    fn wait_frames(&mut self, n: u64, deadline: Instant) -> anyhow::Result<()> {
        let frames = |cdp: &mut Self| -> anyhow::Result<u64> {
            Ok(cdp
                .eval("window.__classicCapture ? window.__classicCapture.frames : 0")?
                .as_u64()
                .unwrap_or(0))
        };
        let target = frames(self)? + n;
        while frames(self)? < target {
            anyhow::ensure!(Instant::now() < deadline, "timed out waiting for {n} frame(s)");
            self.pump(Instant::now() + Duration::from_millis(10))?;
        }
        Ok(())
    }

    fn mouse(
        &mut self,
        kind: &str,
        x: f64,
        y: f64,
        button: &str,
        buttons: u32,
    ) -> anyhow::Result<()> {
        self.call(
            "Input.dispatchMouseEvent",
            json!({ "type": kind, "x": x, "y": y, "button": button, "buttons": buttons, "clickCount": 1 }),
        )?;
        Ok(())
    }

    /// Handle incoming messages until `until`.
    fn pump(&mut self, until: Instant) -> anyhow::Result<()> {
        while Instant::now() < until {
            self.read_one()?;
        }
        Ok(())
    }

    /// Read and dispatch at most one message (returns after the socket's
    /// short read timeout when there is none).
    fn read_one(&mut self) -> anyhow::Result<()> {
        let msg = match self.ws.read() {
            Ok(m) => m,
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                return Ok(())
            }
            Err(e) => return Err(e).context("DevTools read"),
        };
        let Message::Text(text) = msg else { return Ok(()) };
        let v: Value = serde_json::from_str(text.as_str())?;
        if let Some(id) = v.get("id").and_then(Value::as_u64) {
            self.replies.insert(id, v);
            return Ok(());
        }
        let p = &v["params"];
        match v["method"].as_str().unwrap_or("") {
            "Runtime.consoleAPICalled" => {
                let text: Vec<String> = p["args"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|x| match &x["value"] {
                                Value::String(s) => s.clone(),
                                Value::Null => x["description"].as_str().unwrap_or("").to_string(),
                                other => other.to_string(),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let kind = p["type"].as_str().unwrap_or("log").to_string();
                self.record(format!("{kind}: {}", text.join(" ")));
            }
            "Runtime.exceptionThrown" => {
                let d = &p["exceptionDetails"];
                let desc =
                    d["exception"]["description"].as_str().or(d["text"].as_str()).unwrap_or("?");
                self.record(format!("exception: {desc}"));
            }
            "Log.entryAdded" => {
                let e = &p["entry"];
                let level = e["level"].as_str().unwrap_or("");
                let kind = if level == "error" { "error" } else { "browser" };
                self.record(format!(
                    "{kind}: {} {}",
                    e["text"].as_str().unwrap_or(""),
                    e["url"].as_str().unwrap_or("")
                ));
            }
            "Fetch.requestPaused" => self.fulfil(p)?,
            _ => {}
        }
        Ok(())
    }

    /// Serve a paused `classic-roms.com` request from `--roms` when the file
    /// is there; let it through to the network otherwise.
    fn fulfil(&mut self, p: &Value) -> anyhow::Result<()> {
        let id = p["requestId"].as_str().unwrap_or("").to_string();
        let url = p["request"]["url"].as_str().unwrap_or("");
        let file = url.split(['?', '#']).next().unwrap_or("").rsplit('/').next().unwrap_or("");
        let local = self
            .roms
            .as_ref()
            .filter(|_| !file.is_empty() && !file.contains(".."))
            .map(|d| d.join(file))
            .filter(|f| f.is_file());
        match local {
            Some(f) => {
                let body = fs::read(&f)?;
                self.record(format!("net: local {url} <- {} ({} bytes)", f.display(), body.len()));
                let ctype = if file.ends_with(".json") {
                    "application/json"
                } else {
                    "application/octet-stream"
                };
                self.send(
                    "Fetch.fulfillRequest",
                    json!({
                        "requestId": id,
                        "responseCode": 200,
                        "responseHeaders": [
                            { "name": "Content-Type", "value": ctype },
                            { "name": "Access-Control-Allow-Origin", "value": "*" },
                            { "name": "Cross-Origin-Resource-Policy", "value": "cross-origin" },
                            { "name": "Cache-Control", "value": "no-store" },
                        ],
                        "body": base64::engine::general_purpose::STANDARD.encode(body),
                    }),
                )?;
            }
            None => {
                self.record(format!("net: cdn {url}"));
                self.send("Fetch.continueRequest", json!({ "requestId": id }))?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn url_carries_rom_fixed_dt_and_overrides() {
        let mut c = crate::capture::common_defaults("rom:lunar");
        c.dt = 0.05;
        assert_eq!(
            page_url(8080, &c, &[]),
            "http://127.0.0.1:8080/?rom=rom:lunar&fixed_dt=0.05&loader=console"
        );
        let url = page_url(1, &c, &["loader=visual".into(), "perf".into()]);
        assert_eq!(url, "http://127.0.0.1:1/?rom=rom:lunar&fixed_dt=0.05&loader=visual&perf");
    }

    #[test]
    fn dist_server_serves_files_with_isolation_headers() {
        let dir = std::env::temp_dir().join(format!("xtask-serve-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("index.html"), "<html>hi</html>").unwrap();
        fs::write(dir.join("app.wasm"), [0u8, 97, 115, 109]).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let d = dir.clone();
        std::thread::spawn(move || serve(listener, d, None));
        let get = |path: &str| {
            let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            write!(s, "GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").unwrap();
            let mut r = String::new();
            s.read_to_string(&mut r).unwrap_or_default();
            r
        };
        let index = get("/?rom=rom:lunar");
        assert!(index.starts_with("HTTP/1.1 200"), "{index}");
        assert!(index.contains("Cross-Origin-Embedder-Policy: require-corp"));
        assert!(index.ends_with("<html>hi</html>"));
        assert!(get("/app.wasm").contains("Content-Type: application/wasm"));
        assert!(get("/../etc/passwd").starts_with("HTTP/1.1 404"));
        fs::remove_dir_all(&dir).unwrap();
    }
}
