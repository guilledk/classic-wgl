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
//!   cargo xtask release <major|minor|patch>    # bump version + freeze changelog
//!   cargo xtask release --version <X.Y.Z>      # set an explicit version
//!   cargo xtask check-version                  # fail when Cargo.toml/CHANGELOG.md drift

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
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
/// is not fetched here.)  `common` + `lunar-common` are the shared asset-only
/// dependency ROMs the shipped scenes resolve at boot.
const ROMS: &[(&str, &str)] = &[
    ("demo", "demo.rom"),
    ("lunar", "lunar.rom"),
    ("lrvtest", "lrvtest.rom"),
    ("basetest", "basetest.rom"),
    ("common", "common.rom"),
    ("lunar-common", "lunar-common.rom"),
];

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str).unwrap_or("fetch-roms") {
        "build-pathfinder" => return build_pathfinder(),
        "lock-roms" => return cmd_lock_roms(arg_value(&args, "--url")),
        "check-roms" => return cmd_check_roms(arg_value(&args, "--url")),
        "release" => return cmd_release(&args),
        "check-version" => return cmd_check_version(),
        "fetch-roms" | "all" => {}
        other => anyhow::bail!(
            "unknown command `{other}` (expected fetch-roms, lock-roms, check-roms, release, check-version, or build-pathfinder)"
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
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("xtask manifest dir has a repo-root parent")?
        .to_path_buf();
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

// ---------------------------------------------------------------------------
// Versioning (`release` / `check-version`)
// ---------------------------------------------------------------------------

/// The repository root (parent of the xtask crate manifest dir).
fn repo_root() -> anyhow::Result<PathBuf> {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|p| p.to_path_buf())
        .context("xtask manifest dir has a repo-root parent")
}

/// Parse the `major.minor.patch` numeric prefix of a semver string (ignoring
/// any `-prerelease` suffix).
fn parse_version(v: &str) -> anyhow::Result<(u64, u64, u64)> {
    let numeric = v.split('-').next().unwrap_or(v);
    let mut parts = numeric.split('.');
    let major = parts.next().context("version major")?.parse::<u64>()?;
    let minor = parts.next().context("version minor")?.parse::<u64>()?;
    let patch = parts.next().context("version patch")?.parse::<u64>()?;
    Ok((major, minor, patch))
}

/// Apply a semver bump (major/minor/patch), dropping any prerelease suffix.
fn bump_version(v: &str, kind: &str) -> anyhow::Result<String> {
    let (mut major, mut minor, mut patch) = parse_version(v)?;
    match kind {
        "major" => {
            major += 1;
            minor = 0;
            patch = 0;
        }
        "minor" => {
            minor += 1;
            patch = 0;
        }
        "patch" => patch += 1,
        other => anyhow::bail!("unknown bump `{other}` (expected major, minor, or patch)"),
    }
    Ok(format!("{major}.{minor}.{patch}"))
}

/// Read `[workspace.package] version` from the root `Cargo.toml`.
fn read_workspace_version(root: &Path) -> anyhow::Result<String> {
    let cargo = fs::read_to_string(root.join("Cargo.toml")).context("read Cargo.toml")?;
    let mut in_ws = false;
    for line in cargo.lines() {
        let t = line.trim();
        if t == "[workspace.package]" {
            in_ws = true;
            continue;
        }
        if in_ws && t.starts_with('[') {
            break;
        }
        if in_ws && t.starts_with("version") {
            let v = t.split_once('=').context("parse version line")?.1.trim();
            return Ok(v.trim_matches('"').to_string());
        }
    }
    anyhow::bail!("no [workspace.package] version found in Cargo.toml")
}

/// Replace the `[workspace.package] version` in the root `Cargo.toml`.
fn set_workspace_version(root: &Path, old: &str, new: &str) -> anyhow::Result<()> {
    let path = root.join("Cargo.toml");
    let cargo = fs::read_to_string(&path).context("read Cargo.toml")?;
    let needle = format!("version = \"{old}\"");
    if !cargo.contains(&needle) {
        anyhow::bail!("version `{old}` not found in Cargo.toml");
    }
    fs::write(&path, cargo.replacen(&needle, &format!("version = \"{new}\""), 1))
        .context("write Cargo.toml")
}

/// The first released version heading in `CHANGELOG.md` (skipping
/// `[Unreleased]`).
fn changelog_top_version(root: &Path) -> anyhow::Result<Option<String>> {
    let text = fs::read_to_string(root.join("CHANGELOG.md")).context("read CHANGELOG.md")?;
    for line in text.lines() {
        if let Some(rest) = line.trim().strip_prefix("## [") {
            if let Some(ver) = rest.split(']').next() {
                if ver == "Unreleased" {
                    continue;
                }
                return Ok(Some(ver.to_string()));
            }
        }
    }
    Ok(None)
}

/// Freeze the `## [Unreleased]` section into `## [<version>] - <date>`.
fn freeze_changelog(root: &Path, new_version: &str, date: &str) -> anyhow::Result<()> {
    let path = root.join("CHANGELOG.md");
    let text = fs::read_to_string(&path).context("read CHANGELOG.md")?;
    const MARKER: &str = "## [Unreleased]";
    if !text.contains(MARKER) {
        anyhow::bail!("CHANGELOG.md has no `## [Unreleased]` section");
    }
    let block = format!("## [Unreleased]\n\n## [{new_version}] - {date}");
    fs::write(&path, text.replacen(MARKER, &block, 1)).context("write CHANGELOG.md")
}

/// Today's date in `YYYY-MM-DD` (UTC).
fn today_date() -> anyhow::Result<String> {
    let out = Command::new("date").args(["-u", "+%Y-%m-%d"]).output().context("run date")?;
    if !out.status.success() {
        anyhow::bail!("`date` failed");
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// `cargo xtask check-version`: fail when `Cargo.toml` and `CHANGELOG.md`
/// disagree about the current version.
fn cmd_check_version() -> anyhow::Result<()> {
    let root = repo_root()?;
    let version = read_workspace_version(&root)?;
    let top = changelog_top_version(&root)?.context("CHANGELOG.md has no released version")?;
    if version != top {
        anyhow::bail!(
            "version drift: Cargo.toml `{version}` != CHANGELOG.md `{top}` \
             (run `cargo xtask release ...`)"
        );
    }
    println!("version `{version}` matches CHANGELOG.md");
    Ok(())
}

/// `cargo xtask release <major|minor|patch>` (or `--version X.Y.Z`): bump the
/// workspace version, freeze the changelog, and verify.  Prints the commit/tag
/// commands — it does not mutate git.
fn cmd_release(args: &[String]) -> anyhow::Result<()> {
    let root = repo_root()?;
    let current = read_workspace_version(&root)?;

    let new_version = if let Some(explicit) = arg_value(args, "--version") {
        explicit
    } else {
        let kind = args
            .iter()
            .skip(1)
            .find(|a| !a.starts_with('-'))
            .context("specify major, minor, patch, or --version X.Y.Z")?
            .clone();
        bump_version(&current, &kind)?
    };

    set_workspace_version(&root, &current, &new_version)?;
    freeze_changelog(&root, &new_version, &today_date()?)?;
    cmd_check_version()?;

    println!("bumped {current} -> {new_version}");
    println!("next:");
    println!("  git add Cargo.toml CHANGELOG.md");
    println!("  git commit -m \"release v{new_version}\"");
    println!("  git tag -a v{new_version} -m \"release v{new_version}\"");
    println!("  git push origin master --tags");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ignores_prerelease() {
        assert_eq!(parse_version("0.1.0-alpha.0").unwrap(), (0, 1, 0));
        assert_eq!(parse_version("1.2.3").unwrap(), (1, 2, 3));
    }

    #[test]
    fn bump_semver() {
        assert_eq!(bump_version("0.1.0-alpha.0", "patch").unwrap(), "0.1.1");
        assert_eq!(bump_version("0.1.0-alpha.0", "minor").unwrap(), "0.2.0");
        assert_eq!(bump_version("0.1.0-alpha.0", "major").unwrap(), "1.0.0");
        assert_eq!(bump_version("1.2.3", "patch").unwrap(), "1.2.4");
        assert!(bump_version("1.2.3", "wat").is_err());
    }

    #[test]
    fn changelog_top_version_skips_unreleased() {
        let dir = std::env::temp_dir().join("xtask_top_version");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("CHANGELOG.md");
        std::fs::write(&p, "## [Unreleased]\n\n## [0.1.0-alpha.0] - 2026-08-28\n").unwrap();
        assert_eq!(changelog_top_version(&dir).unwrap(), Some("0.1.0-alpha.0".into()));
        std::fs::remove_dir_all(&dir).ok();
    }
}
