//! `xtask`: classic-wgl's slim task runner.
//!
//! ROM archives are no longer built here.  The `classic-roms` repo authors and
//! builds them, then publishes them to a Cloudflare R2 public bucket served at
//! `https://classic-roms.com/`; the only thing this tool needs to do is stage a
//! local copy so the native app, dev loop and golden tests can boot offline.
//!
//! Usage:
//!   cargo xtask fetch-roms                     # download roms into roms/out/
//!   cargo xtask fetch-roms --url <rom-base>    # override the ROM base URL
//!   cargo xtask fetch-roms --skip-verify       # proceed without a roms.json index
//!   cargo xtask                                # alias for fetch-roms
//!   cargo xtask build-pathfinder               # compile + stage pathfinder.wasm
//!   cargo xtask lock-roms [--url <rom-base>]   # pin the published checksums to the lockfile
//!   cargo xtask check-roms [--url <rom-base>]  # fail fast when the published ROMs drift

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use anyhow::Context;
use sha2::{Digest, Sha256};

/// Default base URL for the published ROMs (the R2 bucket behind
/// `classic-roms.com`).
const DEFAULT_ROM_BASE: &str = "https://classic-roms.com";

/// Committed content-hash pin: the exact `roms.json` checksums the golden
/// baselines under `tests/golden/` were generated against.  `check-roms`
/// refuses to run against a bucket that has drifted from this lock, so a
/// golden failure is never a confusing atlas/geometry diff — it is an explicit
/// "republish or re-pin" signal.
const LOCK_PATH: &str = "tests/golden/roms.lock.json";

/// The ROMs shipped by `classic-roms`.  (`moon` is a resolve-time alias for
/// `lunar` only — the desktop/web `rom_lookup`/`static_lookup` handle it — so it
/// is not fetched here.)
const ROMS: &[(&str, &str)] =
    &[("demo", "demo.rom"), ("lunar", "lunar.rom"), ("lrvtest", "lrvtest.rom")];

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str).unwrap_or("fetch-roms") {
        "build-pathfinder" => return build_pathfinder(),
        "lock-roms" => return cmd_lock_roms(arg_value(&args, "--url")),
        "check-roms" => return cmd_check_roms(arg_value(&args, "--url")),
        "fetch-roms" | "all" => {}
        other => anyhow::bail!(
            "unknown command `{other}` (expected fetch-roms, lock-roms, check-roms, or build-pathfinder)"
        ),
    }

    let mut url = DEFAULT_ROM_BASE.to_string();
    let mut skip_verify = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--url" => {
                i += 1;
                url = args.get(i).context("--url needs a value")?.clone();
            }
            "--skip-verify" => skip_verify = true,
            cmd if cmd == "fetch-roms" || cmd == "all" => {}
            other => {
                anyhow::bail!("unknown argument `{other}` (expected fetch-roms, --url <base>, or --skip-verify)")
            }
        }
        i += 1;
    }

    let out_dir = PathBuf::from("roms/out");
    fs::create_dir_all(&out_dir).with_context(|| "create roms/out")?;

    // Fetch the checksum index first (roms.json), then the archives.  A
    // missing index is a hard error unless `--skip-verify` is passed: booting
    // a ROM without verifying it against the published checksums is a silent
    // staleness hazard.
    let index = match fetch(&format!("{url}/roms.json")) {
        Ok(bytes) => Some(bytes),
        Err(e) if skip_verify => {
            eprintln!(
                "note: no roms.json index at base ({e:#}); --skip-verify, proceeding unverified"
            );
            None
        }
        Err(e) => {
            anyhow::bail!(
                "no roms.json index at base ({e:#}); refusing to fetch unverified ROMs \
                 (pass --skip-verify to override)"
            );
        }
    };

    for &(name, file) in ROMS {
        let bytes = fetch(&format!("{url}/{file}")).with_context(|| format!("download {file}"))?;
        if let Some(index) = &index {
            verify(name, index, &bytes)?;
        }
        let dst = out_dir.join(file);
        fs::write(&dst, &bytes).with_context(|| format!("write {}", dst.display()))?;
        println!("wrote {} ({:.1} MiB)", dst.display(), bytes.len() as f64 / (1024.0 * 1024.0));
    }

    println!("roms ready under roms/out/; boot with CLASSIC_ROM=rom:demo (etc.)");
    Ok(())
}

/// Build the web `pathfinder.wasm` module (the Rust pathfinder compiled to
/// wasm) and stage it next to `web.rs` so the web `Worker` can instantiate it.
/// Must run before any `--target wasm32-unknown-unknown` build/check.
fn build_pathfinder() -> anyhow::Result<()> {
    let root = std::env::current_dir().context("current dir")?;
    let status = Command::new("cargo")
        .args([
            "build",
            "--target",
            "wasm32-unknown-unknown",
            "-p",
            "classic-pathfinder-wasm",
            "--release",
        ])
        .current_dir(&root)
        .status()
        .context("build classic-pathfinder-wasm")?;
    if !status.success() {
        anyhow::bail!("classic-pathfinder-wasm build failed");
    }
    let wasm = root.join("target/wasm32-unknown-unknown/release/classic_pathfinder_wasm.wasm");
    let dst = root.join("crates/classic-worker/src/pathfinder_worker/pathfinder.wasm");
    fs::copy(&wasm, &dst)
        .with_context(|| format!("copy {} -> {}", wasm.display(), dst.display()))?;
    println!("staged {}", dst.display());
    Ok(())
}

/// Blocking GET of `url` into raw bytes (ureq).
fn fetch(url: &str) -> anyhow::Result<Vec<u8>> {
    let resp = ureq::get(url)
        .timeout(Duration::from_secs(300))
        .call()
        .map_err(|e| anyhow::anyhow!("GET {url}: {e}"))?;
    let mut out = Vec::new();
    resp.into_reader().read_to_end(&mut out).map_err(|e| anyhow::anyhow!("read {url}: {e}"))?;
    Ok(out)
}

/// Verify `bytes` against the `<name>` entry of a `roms.json` index.
///
/// The index format is `{ "<name>": { "size": <int>, "sha256": "<hex>" } }`.
fn verify(name: &str, index_json: &[u8], bytes: &[u8]) -> anyhow::Result<()> {
    let index: serde_json::Value =
        serde_json::from_slice(index_json).with_context(|| "parse roms.json index")?;
    let entry = index.get(name).with_context(|| format!("roms.json has no entry for `{name}`"))?;
    if let Some(size) = entry.get("size").and_then(|s| s.as_u64()) {
        if size as usize != bytes.len() {
            anyhow::bail!("`{name}` size mismatch: index={size}, downloaded={}", bytes.len());
        }
    }
    if let Some(expected) = entry.get("sha256").and_then(|s| s.as_str()) {
        let digest = hex(&Sha256::digest(bytes));
        if !digest.eq_ignore_ascii_case(expected) {
            anyhow::bail!("`{name}` sha256 mismatch: index={expected}, downloaded={digest}");
        }
    }
    Ok(())
}

/// Hex-encode a SHA-256 digest.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Return the value following `--flag` in `args`, if present.
fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned()
}

/// `cargo xtask lock-roms`: pin the currently-published `roms.json` checksums
/// to `tests/golden/roms.lock.json`.  Run this alongside `CLASSIC_GOLDEN=update`
/// so the lockfile and the regenerated baselines stay in lockstep.
fn cmd_lock_roms(url: Option<String>) -> anyhow::Result<()> {
    let base = url.unwrap_or_else(|| DEFAULT_ROM_BASE.to_string());
    let index = fetch(&format!("{base}/roms.json")).context("fetch roms.json to lock")?;
    // Validate the index shape before committing it to the repo.
    let parsed: serde_json::Value =
        serde_json::from_slice(&index).context("parse roms.json index")?;
    for name in ["demo", "lunar", "lrvtest"] {
        let entry =
            parsed.get(name).with_context(|| format!("roms.json has no entry for `{name}`"))?;
        entry
            .get("size")
            .and_then(|s| s.as_u64())
            .with_context(|| format!("`{name}` missing size"))?;
        entry
            .get("sha256")
            .and_then(|s| s.as_str())
            .with_context(|| format!("`{name}` missing sha256"))?;
    }
    fs::write(LOCK_PATH, &index).with_context(|| format!("write {LOCK_PATH}"))?;
    println!("locked published ROM checksums to {LOCK_PATH}");
    Ok(())
}

/// `cargo xtask check-roms`: fail fast when the published ROMs no longer match
/// the committed lockfile — i.e. the bucket changed since the golden baselines
/// were generated.  The CI golden job runs this before `fetch-roms` so a drift
/// is a clear, distinct signal rather than an atlas/geometry golden diff.
fn cmd_check_roms(url: Option<String>) -> anyhow::Result<()> {
    let base = url.unwrap_or_else(|| DEFAULT_ROM_BASE.to_string());
    let lock: serde_json::Value =
        serde_json::from_slice(&fs::read(LOCK_PATH).with_context(|| {
            format!("read {LOCK_PATH} (run `cargo xtask lock-roms` to create it)")
        })?)
        .context("parse lockfile")?;
    let published: serde_json::Value = serde_json::from_slice(
        &fetch(&format!("{base}/roms.json")).context("fetch published roms.json")?,
    )
    .context("parse published roms.json")?;

    let mut drifted = false;
    for name in ["demo", "lunar", "lrvtest"] {
        let expected = lock.get(name).with_context(|| format!("lockfile has no `{name}` entry"))?;
        let actual = published
            .get(name)
            .with_context(|| format!("published roms.json has no `{name}` entry"))?;
        let (esha, asha) = (sha(expected), sha(actual));
        let (esize, asize) = (size(expected), size(actual));
        if esha != asha || esize != asize {
            drifted = true;
            eprintln!(
                "ROM drift: `{name}` sha256={asha} size={asize} (lock: sha256={esha} size={esize})"
            );
        }
    }
    if drifted {
        anyhow::bail!(
            "published ROMs drifted from {LOCK_PATH}: the bucket changed since the golden \
             baselines were generated.  Republish the correct ROMs, or run \
             `CLASSIC_GOLDEN=update` together with `cargo xtask lock-roms` to re-pin."
        );
    }
    println!("roms match lockfile {LOCK_PATH}");
    Ok(())
}

/// `sha256` field of a `roms.json` entry, or a sentinel when absent.
fn sha(entry: &serde_json::Value) -> String {
    entry.get("sha256").and_then(|s| s.as_str()).unwrap_or("<none>").to_string()
}

/// `size` field of a `roms.json` entry, or 0 when absent.
fn size(entry: &serde_json::Value) -> u64 {
    entry.get("size").and_then(|s| s.as_u64()).unwrap_or(0)
}
