//! ROM resolution: materialise a `CLASSIC_ROM` / `?rom=` selector into bytes.
//!
//! ROMs are no longer compiled into the binaries.  They are built and published
//! from the separate `classic-roms` repo, and each app references them by
//! location: a CDN URL on web, a cached path (or URL) on native.
//!
//! [`resolve_rom_source`] is the shared front-end: it parses the selector via
//! [`classic_rom::parse_rom_spec`] and dispatches named ROMs (`rom:<name>`, or
//! the empty default) through a caller-supplied `name -> location` registry.
//! Materialising the actual bytes is platform-specific: [`resolve_rom`] on
//! native (`fs::read` / blocking `ureq`), [`resolve_rom_async`] on web
//! (async `fetch`).

use classic_rom::{AssetBytes, BootEvent, BootSink, Rom, RomSource};

/// Build a lookup closure from a static `name -> URL/path` table.
///
/// Convenience for apps with a fixed registry (the web app's CDN URLs).
/// Desktop builds its own runtime closure over `roms/out/` instead.
pub fn static_lookup<'a>(table: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |name: &str| table.iter().find(|(n, _)| *n == name).map(|(_, l)| (*l).to_string())
}

/// Resolve the source (not the bytes) for a ROM selector.
///
/// Named ROMs (`rom:<name>`, or the empty default `demo`) are dispatched
/// through `index`, which maps a name to a `http(s)://` URL or a filesystem
/// path (a bare value is a path on native and a page-relative URL on web).
/// Non-`rom:` selectors pass through unchanged.
pub fn resolve_rom_source(
    spec: &str,
    index: &dyn Fn(&str) -> Option<String>,
) -> anyhow::Result<RomSource> {
    match classic_rom::parse_rom_spec(spec) {
        RomSource::Embedded(name) => {
            let location = index(&name)
                .ok_or_else(|| anyhow::anyhow!("unknown ROM `{name}` (not in the app registry)"))?;
            match classic_rom::parse_rom_spec(&location) {
                s @ (RomSource::Url(_) | RomSource::Path(_)) => Ok(s),
                RomSource::Embedded(other) => anyhow::bail!(
                    "ROM `{name}` registry entry must be a URL or path, got `rom:{other}`"
                ),
                RomSource::Data(_) => unreachable!("registry values are never data: URIs"),
            }
        }
        other => Ok(other),
    }
}

/// Materialise an already-resolved source on native: read a file or fetch a
/// URL with blocking `ureq`.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_rom_bytes(source: &RomSource) -> anyhow::Result<AssetBytes> {
    match source {
        RomSource::Url(url) => fetch_url(url).map(AssetBytes::Owned),
        RomSource::Path(path) => {
            let bytes = std::fs::read(path)
                .map_err(|e| anyhow::anyhow!("failed to read ROM {}: {e}", path.display()))?;
            Ok(AssetBytes::Owned(bytes))
        }
        RomSource::Data(bytes) => Ok(AssetBytes::Owned(bytes.clone())),
        RomSource::Embedded(_) => unreachable!("resolve_rom_source resolves names first"),
    }
}

/// Resolve a ROM selector to bytes on native.
#[cfg(not(target_arch = "wasm32"))]
pub fn resolve_rom(
    spec: &str,
    index: &dyn Fn(&str) -> Option<String>,
) -> anyhow::Result<AssetBytes> {
    let source = resolve_rom_source(spec, index)?;
    load_rom_bytes(&source)
}

/// Resolve a ROM selector to a multi-ROM dependency DAG on native.
///
/// The root selector may be a named ROM (`rom:<name>`), whose `deps` are
/// recursively resolved through `index`, or a direct URL/path/data source
/// whose manifest `deps` are then resolved the same way.  ROMs are
/// cycle-checked, de-duplicated, and returned in topological order (deps
/// before dependents).  Boot progress is streamed to `sink`.
#[cfg(not(target_arch = "wasm32"))]
pub fn resolve_roms(
    spec: &str,
    index: &dyn Fn(&str) -> Option<String>,
    sink: &dyn BootSink,
) -> anyhow::Result<classic_rom::LoadedRoms> {
    sink.on_event(BootEvent::ResolveStarted { spec: spec.to_string() });

    let named_load = |name: &str| {
        let source = resolve_rom_source(&format!("rom:{name}"), index)?;
        let bytes = load_rom_bytes(&source)?;
        sink.on_event(BootEvent::RomFetched { name: name.to_string(), bytes: bytes.len() });
        rom_from_bytes(&bytes, name, sink)
    };

    match classic_rom::parse_rom_spec(spec) {
        RomSource::Embedded(name) => classic_rom::LoadedRoms::resolve(&name, named_load),
        other => {
            let root_bytes = load_rom_bytes(&other)?;
            sink.on_event(BootEvent::RomFetched {
                name: "root".to_string(),
                bytes: root_bytes.len(),
            });
            let root_rom = rom_from_bytes(&root_bytes, "root", sink)?;
            let root_name = rom_name(&root_rom);
            let mut root_loaded = false;
            classic_rom::LoadedRoms::resolve(&root_name, |name| {
                if name == root_name && !root_loaded {
                    root_loaded = true;
                    Ok(root_rom.clone())
                } else {
                    named_load(name)
                }
            })
        }
    }
}

/// Parse a fully-loaded [`Rom`] from archive bytes, emitting
/// `RomDecompressed` (after archive open) and `RomParsed` (from [`Rom::load`])
/// to `sink` under the given `name`.
fn rom_from_bytes(bytes: &[u8], name: &str, sink: &dyn BootSink) -> anyhow::Result<Rom> {
    let archive = classic_rom::RomArchive::from_bytes(bytes)?;
    sink.on_event(BootEvent::RomDecompressed { name: name.to_string(), entries: archive.len() });
    Rom::load(&archive, sink)
}

/// The resolver name for a directly-loaded (non-`rom:`) root ROM: its
/// manifest entrypoint, falling back to `"root"`.
fn rom_name(rom: &Rom) -> String {
    if rom.manifest.entrypoint.is_empty() {
        "root".to_string()
    } else {
        rom.manifest.entrypoint.clone()
    }
}

/// Fetch a URL as raw bytes (blocking `ureq`, native).
#[cfg(not(target_arch = "wasm32"))]
fn fetch_url(url: &str) -> anyhow::Result<Vec<u8>> {
    use std::io::Read;

    let resp = ureq::get(url).call().map_err(|e| anyhow::anyhow!("fetch {url}: {e}"))?;
    let mut reader = resp.into_reader();
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf).map_err(|e| anyhow::anyhow!("read {url}: {e}"))?;
    Ok(buf)
}

/// Fetch the bytes for an already-resolved ROM source on web.
///
/// `RomSource::Path` values are treated as URLs relative to the page origin
/// (there is no filesystem on wasm).
#[cfg(target_arch = "wasm32")]
pub async fn load_rom_bytes_async(source: RomSource) -> anyhow::Result<AssetBytes> {
    match source {
        RomSource::Url(url) => fetch_url_async(&url).await.map(AssetBytes::Owned),
        RomSource::Path(path) => {
            let url = path.to_string_lossy().into_owned();
            fetch_url_async(&url).await.map(AssetBytes::Owned)
        }
        RomSource::Data(bytes) => Ok(AssetBytes::Owned(bytes)),
        RomSource::Embedded(_) => unreachable!("resolve_rom_source resolves names first"),
    }
}

/// Resolve a ROM selector to bytes on web (async `fetch`).
#[cfg(target_arch = "wasm32")]
pub async fn resolve_rom_async(
    spec: &str,
    index: &dyn Fn(&str) -> Option<String>,
) -> anyhow::Result<AssetBytes> {
    let source = resolve_rom_source(spec, index)?;
    load_rom_bytes_async(source).await
}

/// Load a named ROM to a parsed [`Rom`] via the name -> location index (web),
/// caching the archive in the browser Cache API keyed by the `sha256` published
/// in `index_url` (the `roms.json` checksum index) so repeat page loads serve
/// the ROM locally.
#[cfg(target_arch = "wasm32")]
async fn load_named_rom_async(
    name: &str,
    index: &dyn Fn(&str) -> Option<String>,
    index_url: &str,
    sink: &dyn BootSink,
) -> anyhow::Result<Rom> {
    let source = resolve_rom_source(&format!("rom:{name}"), index)?;
    let bytes = match source {
        RomSource::Url(url) => resolve_named_rom_cached(name, &url, index_url).await?,
        RomSource::Path(path) => {
            resolve_named_rom_cached(name, &path.to_string_lossy(), index_url).await?
        }
        RomSource::Data(bytes) => AssetBytes::Owned(bytes),
        RomSource::Embedded(_) => unreachable!("resolve_rom_source resolves names first"),
    };
    sink.on_event(BootEvent::RomFetched { name: name.to_string(), bytes: bytes.len() });
    rom_from_bytes(&bytes, name, sink)
}

/// Resolve a ROM selector to a multi-ROM dependency DAG on web.
///
/// Mirrors the native [`resolve_roms`]: named roots resolve their `deps`
/// recursively through `index` (each ROM fetched + Cache-API-cached keyed by
/// the `sha256` published in `index_url`), direct URL/path roots contribute
/// their manifest `deps`.  Cycle-checked, de-duplicated, topologically ordered.
/// Boot progress is streamed to `sink`.
#[cfg(target_arch = "wasm32")]
pub async fn resolve_roms_async(
    spec: &str,
    index: &dyn Fn(&str) -> Option<String>,
    index_url: &str,
    sink: &dyn BootSink,
) -> anyhow::Result<classic_rom::LoadedRoms> {
    sink.on_event(BootEvent::ResolveStarted { spec: spec.to_string() });

    match classic_rom::parse_rom_spec(spec) {
        RomSource::Embedded(name) => {
            classic_rom::LoadedRoms::resolve_async(&name, |n| {
                let index_url = index_url.to_string();
                async move { load_named_rom_async(&n, index, &index_url, sink).await }
            })
            .await
        }
        other => {
            let root_bytes = load_rom_bytes_async(other).await?;
            sink.on_event(BootEvent::RomFetched {
                name: "root".to_string(),
                bytes: root_bytes.len(),
            });
            let root_rom = rom_from_bytes(&root_bytes, "root", sink)?;
            let root_name = rom_name(&root_rom);
            let mut root_loaded = false;
            classic_rom::LoadedRoms::resolve_async(&root_name, |name| {
                let is_root = name == root_name && !root_loaded;
                if is_root {
                    root_loaded = true;
                }
                let root_rom = root_rom.clone();
                let index_url = index_url.to_string();
                async move {
                    if is_root {
                        Ok(root_rom)
                    } else {
                        load_named_rom_async(&name, index, &index_url, sink).await
                    }
                }
            })
            .await
        }
    }
}

/// Fetch a URL as raw bytes via the async `fetch` API (web).
#[cfg(target_arch = "wasm32")]
async fn fetch_url_async(url: &str) -> anyhow::Result<Vec<u8>> {
    let resp = fetch_response(url).await?;
    response_bytes(&resp).await
}

/// Fetch a URL and return the `web_sys::Response` (checking the HTTP status).
#[cfg(target_arch = "wasm32")]
async fn fetch_response(url: &str) -> anyhow::Result<web_sys::Response> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    let window = web_sys::window().ok_or_else(|| anyhow::anyhow!("no window"))?;
    let promise = window.fetch_with_str(url);
    let response =
        JsFuture::from(promise).await.map_err(|e| anyhow::anyhow!("fetch {url}: {e:?}"))?;
    let response: web_sys::Response = response
        .dyn_into()
        .map_err(|_| anyhow::anyhow!("fetch {url}: response was not a Response"))?;
    if !response.ok() {
        anyhow::bail!("fetch {url}: HTTP {}", response.status());
    }
    Ok(response)
}

/// Read a `web_sys::Response` body as raw bytes.
#[cfg(target_arch = "wasm32")]
async fn response_bytes(response: &web_sys::Response) -> anyhow::Result<Vec<u8>> {
    use wasm_bindgen_futures::JsFuture;

    let buffer = JsFuture::from(
        response.array_buffer().map_err(|e| anyhow::anyhow!("array_buffer failed: {e:?}"))?,
    )
    .await
    .map_err(|e| anyhow::anyhow!("read body: {e:?}"))?;
    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}

/// Resolve a *named* ROM to bytes, caching the downloaded archive in the
/// browser's Cache API keyed by the content `sha256` published in `roms.json`.
///
/// The (tiny) checksum index is fetched fresh on every call; if the cache
/// already holds a copy with a matching hash it is reused verbatim, otherwise
/// the archive is fetched and stored.  Repeat page loads therefore serve the
/// ROM locally (no re-download) while still picking up newly published ROMs.
#[cfg(target_arch = "wasm32")]
pub async fn resolve_named_rom_cached(
    index_key: &str,
    url: &str,
    index_url: &str,
) -> anyhow::Result<AssetBytes> {
    let sha = rom_sha256(index_key, index_url).await?;
    let key = format!("{url}?v={sha}");

    if let Some(cache) = rom_cache().await {
        if let Some(resp) = cache_match(&cache, &key).await? {
            return response_bytes(&resp).await.map(AssetBytes::Owned);
        }
    }

    let resp = fetch_response(url).await?;
    if let Some(cache) = rom_cache().await {
        // `Cache::put` consumes the body, so stash a teed copy in the cache
        // (via the JS `Response.clone`) and read the original below.
        if let Ok(cached) = web_sys::Response::clone(&resp) {
            let _ = cache_put(&cache, &key, &cached).await;
        }
    }
    response_bytes(&resp).await.map(AssetBytes::Owned)
}

/// Fetch the `roms.json` checksum index and return the `sha256` for `name`.
#[cfg(target_arch = "wasm32")]
async fn rom_sha256(name: &str, index_url: &str) -> anyhow::Result<String> {
    let url = format!("{index_url}?t={}", js_sys::Date::now() as u64);
    let bytes = fetch_url_async(&url).await?;
    let index: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| anyhow::anyhow!("parse {index_url}: {e}"))?;
    index
        .get(name)
        .and_then(|e| e.get("sha256"))
        .and_then(|s| s.as_str())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("{index_url} has no sha256 entry for `{name}`"))
}

/// Open (or create) the `classic-roms` Cache API cache, if available.
#[cfg(target_arch = "wasm32")]
async fn rom_cache() -> Option<web_sys::Cache> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    let window = web_sys::window()?;
    let caches = window.caches().ok()?;
    let promise = caches.open("classic-roms");
    let cache = JsFuture::from(promise).await.ok()?;
    cache.dyn_into::<web_sys::Cache>().ok()
}

/// Look up `key` in a Cache API cache (None on a miss).
#[cfg(target_arch = "wasm32")]
async fn cache_match(
    cache: &web_sys::Cache,
    key: &str,
) -> anyhow::Result<Option<web_sys::Response>> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    let promise = cache.match_with_str(key);
    let value = JsFuture::from(promise).await.map_err(|e| anyhow::anyhow!("cache.match: {e:?}"))?;
    if value.is_undefined() {
        return Ok(None);
    }
    let resp = value
        .dyn_into::<web_sys::Response>()
        .map_err(|_| anyhow::anyhow!("cache.match: entry was not a Response"))?;
    Ok(Some(resp))
}

/// Store `response` under `key` in a Cache API cache.
#[cfg(target_arch = "wasm32")]
async fn cache_put(cache: &web_sys::Cache, key: &str, response: &web_sys::Response) {
    use wasm_bindgen_futures::JsFuture;

    let promise = cache.put_with_str(key, response);
    let _ = JsFuture::from(promise).await;
}

/// A [`BootSink`] that logs each event to the `boot` instrument channel.
/// Opted into via `CLASSIC_BOOT_LOG` / `CLASSIC_LOADER=console`; the no-op
/// [`classic_rom::NullBootSink`] is the default.
#[derive(Clone, Copy, Debug, Default)]
pub struct LogBootSink;

impl BootSink for LogBootSink {
    fn on_event(&self, event: BootEvent) {
        match &event {
            BootEvent::BootFailed { .. } => {
                classic_core::cl_error!(
                    classic_core::instrument::Chan::Boot,
                    "{}",
                    event.describe()
                );
            }
            _ => {
                classic_core::cl_info!(
                    classic_core::instrument::Chan::Boot,
                    "{}",
                    event.describe()
                );
            }
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use classic_rom::{ResourceSet, RomManifest};

    fn make_rom(name: &str, deps: &[&str]) -> classic_rom::Rom {
        let deps_json = deps.iter().map(|d| format!("\"{d}\"")).collect::<Vec<_>>().join(",");
        let manifest_json = format!(
            r#"{{"format_version":1,"entrypoint":"{name}","deps":[{deps_json}],
                "shaders":[],"textures":[],"animations":[]}}"#
        );
        let manifest: RomManifest = serde_json::from_str(&manifest_json).unwrap();
        classic_rom::Rom {
            manifest,
            manifest_json,
            resources: ResourceSet::default(),
            state: "{\"entities\":{}}".into(),
        }
    }

    fn write_rom(dir: &std::path::Path, name: &str, rom: &classic_rom::Rom) -> String {
        let path = dir.join(format!("{name}.rom"));
        std::fs::write(&path, rom.pack_zip().unwrap()).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn resolve_roms_streams_boot_events_for_a_dag() {
        let dir = std::env::temp_dir().join(format!("classic-rom-boot-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let common = make_rom("common", &[]);
        let scene = make_rom("scene", &["common"]);
        let common_path = write_rom(&dir, "common", &common);
        let scene_path = write_rom(&dir, "scene", &scene);

        let index = |name: &str| -> Option<String> {
            match name {
                "scene" => Some(scene_path.clone()),
                "common" => Some(common_path.clone()),
                _ => None,
            }
        };

        let sink = classic_rom::VecBootSink::new();
        let loaded = resolve_roms("rom:scene", &index, &sink).unwrap();

        // The resolved DAG is topological: deps precede the dependent.
        let order: Vec<&str> = loaded.order.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(order, vec!["common", "scene"]);

        let events = sink.events();
        // Resolve starts first, and both ROMs are parsed.
        assert!(matches!(events.first(), Some(BootEvent::ResolveStarted { .. })));
        let parsed: std::collections::BTreeSet<&str> = events
            .iter()
            .filter_map(|e| match e {
                BootEvent::RomParsed { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(parsed, std::collections::BTreeSet::from(["common", "scene"]));

        std::fs::remove_dir_all(&dir).ok();
    }
}
