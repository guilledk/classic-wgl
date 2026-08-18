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

use classic_rom::{AssetBytes, RomSource};

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
