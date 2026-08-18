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
    let buffer = JsFuture::from(
        response
            .array_buffer()
            .map_err(|e| anyhow::anyhow!("fetch {url}: array_buffer failed: {e:?}"))?,
    )
    .await
    .map_err(|e| anyhow::anyhow!("read {url}: {e:?}"))?;
    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}
