//! ROM resolution: materialise a `CLASSIC_ROM` / `?rom=` selector into the
//! archive bytes to boot.
//!
//! [`resolve_rom`] is the unified entry point shared by the desktop and web
//! apps.  It parses the selector via [`classic_rom::parse_rom_spec`] and
//! dispatches: embedded names resolve against the caller's compile-time
//! registry, `http(s)://` URLs are fetched (blocking `ureq` on native, a
//! synchronous `XmlHttpRequest` on web), and paths are read from disk on
//! native (on web a bare value is a URL relative to the page origin).

use classic_rom::{AssetBytes, RomSource};

/// Resolve a ROM selector string to its archive bytes.
///
/// `embedded` is the app's compile-time registry of named ROMs (e.g.
/// `("demo", include_bytes!(...))`), matched case-sensitively against the
/// `rom:<name>` namespace (empty selector defaults to `rom:demo`).
pub fn resolve_rom(
    spec: &str,
    embedded: &'static [(&'static str, &'static [u8])],
) -> anyhow::Result<AssetBytes> {
    let source = classic_rom::parse_rom_spec(spec);
    match source {
        RomSource::Embedded(name) => embedded
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, bytes)| AssetBytes::Borrowed(bytes))
            .ok_or_else(|| anyhow::anyhow!("unknown embedded ROM: {name}")),
        RomSource::Url(url) => fetch_url(&url).map(AssetBytes::Owned),
        RomSource::Path(path) => {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let bytes = std::fs::read(&path)
                    .map_err(|e| anyhow::anyhow!("failed to read ROM {}: {e}", path.display()))?;
                Ok(AssetBytes::Owned(bytes))
            }
            #[cfg(target_arch = "wasm32")]
            {
                fetch_url(&path.to_string_lossy()).map(AssetBytes::Owned)
            }
        }
        RomSource::Data(_) => anyhow::bail!("data: ROM sources are not yet supported"),
    }
}

/// Fetch a URL as raw bytes: blocking `ureq` on native, a synchronous XHR on
/// web (matching the synchronous web bootstrap).
#[cfg(not(target_arch = "wasm32"))]
fn fetch_url(url: &str) -> anyhow::Result<Vec<u8>> {
    use std::io::Read;

    let resp = ureq::get(url).call().map_err(|e| anyhow::anyhow!("fetch {url}: {e}"))?;
    let mut reader = resp.into_reader();
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf).map_err(|e| anyhow::anyhow!("read {url}: {e}"))?;
    Ok(buf)
}

/// Fetch a URL as raw bytes via a synchronous `XmlHttpRequest`.
#[cfg(target_arch = "wasm32")]
fn fetch_url(url: &str) -> anyhow::Result<Vec<u8>> {
    let xhr =
        web_sys::XmlHttpRequest::new().map_err(|e| anyhow::anyhow!("XmlHttpRequest: {e:?}"))?;
    xhr.open_with_async("GET", url, false).map_err(|e| anyhow::anyhow!("XHR open: {e:?}"))?;
    xhr.set_response_type(web_sys::XmlHttpRequestResponseType::Arraybuffer);
    xhr.send().map_err(|e| anyhow::anyhow!("XHR send: {e:?}"))?;
    let status = xhr.status().map_err(|e| anyhow::anyhow!("XHR status: {e:?}"))?;
    if status < 200 || status >= 300 {
        anyhow::bail!("fetch {url}: HTTP {status}");
    }
    let buffer = xhr.response().map_err(|e| anyhow::anyhow!("XHR response: {e:?}"))?;
    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}
