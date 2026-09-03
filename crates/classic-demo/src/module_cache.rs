//! On-disk cache of compiled `wasmtime::Module`s, keyed by the published ROM
//! sha256, so repeat desktop launches skip cranelift.
//!
//! Only `trusted` ROMs with a known sha256 are cached; untrusted ROMs and the
//! web/legacy paths compile inline every launch.  The cache image is prefixed
//! with a magic + version header so a stale or incompatible serialized module
//! (wasmtime upgrade, config change) is treated as a miss and recompiled.
//! Writes are best-effort: a cache that can't be read or written never fails
//! the boot.

use std::path::{Path, PathBuf};

use classic_guest::{CompiledModule, GuestError, GuestLimits};
use classic_rom::LoadedRom;

const MAGIC: &[u8; 5] = b"CWMOD";
const VERSION: u32 = 1;

/// Load a compiled guest module from the on-disk cache, or compile it and
/// store it for the next launch.
pub fn load_or_compile(
    entry: &LoadedRom,
    wasm: &[u8],
    limits: &GuestLimits,
) -> Result<CompiledModule, GuestError> {
    let Some(sha) = cache_key(entry) else {
        return classic_guest::compile_module(wasm, limits);
    };
    let Some(dir) = module_cache_dir() else {
        return classic_guest::compile_module(wasm, limits);
    };
    let path = dir.join(format!("{sha}.module"));

    if let Some(module) = read_cached(&path, limits) {
        return Ok(module);
    }

    let module = classic_guest::compile_module(wasm, limits)?;
    write_cached(&path, &module);
    Ok(module)
}

/// The cache key for a ROM entry: its published sha256, but only for trusted
/// ROMs (untrusted guests must compile fresh every launch).
fn cache_key(entry: &LoadedRom) -> Option<&str> {
    if entry.rom.manifest.trusted {
        entry.sha256.as_deref()
    } else {
        None
    }
}

/// The compiled-module cache directory: `$CLASSIC_MODULE_CACHE_DIR`, else
/// `$XDG_CACHE_HOME/classic-wgl/modules`, else `$HOME/.cache/classic-wgl/modules`.
fn module_cache_dir() -> Option<PathBuf> {
    let base = std::env::var_os("CLASSIC_MODULE_CACHE_DIR").map(PathBuf::from).or_else(|| {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
    })?;
    Some(base.join("classic-wgl").join("modules"))
}

/// Read and deserialize a cached module, returning `None` on any miss/mismatch
/// (missing file, bad magic, stale version, incompatible wasmtime build).
fn read_cached(path: &Path, limits: &GuestLimits) -> Option<CompiledModule> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() < MAGIC.len() + 4 || &bytes[..MAGIC.len()] != MAGIC {
        return None;
    }
    let version = u32::from_le_bytes(bytes[MAGIC.len()..MAGIC.len() + 4].try_into().ok()?);
    if version != VERSION {
        return None;
    }
    CompiledModule::deserialize(&bytes[MAGIC.len() + 4..], limits).ok()
}

/// Serialize `module` and write it to `path`, best-effort.
fn write_cached(path: &Path, module: &CompiledModule) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(serialized) = module.serialize() else { return };
    let mut buf = Vec::with_capacity(MAGIC.len() + 4 + serialized.len());
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&VERSION.to_le_bytes());
    buf.extend_from_slice(&serialized);
    let _ = std::fs::write(path, buf);
}
