//! Asset loading abstraction.
//!
//! The [`AssetLoader`] trait abstracts over where raw resource bytes come
//! from (filesystem, compile-time embedded data, an archive, or a web
//! `fetch`).  It lives here — in the resource-foundation crate — so that
//! [`crate::resource::ResourceSet`] can build from *either* a [`RomArchive`]
//! or an [`AssetLoader`] without pulling in a platform/GL dependency.

/// Raw bytes returned by an [`AssetLoader`], either owned or borrowed from a
/// `'static` compile-time buffer.
pub enum AssetBytes {
    Owned(Vec<u8>),
    Borrowed(&'static [u8]),
}

impl std::ops::Deref for AssetBytes {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        match self {
            AssetBytes::Owned(v) => v,
            AssetBytes::Borrowed(b) => b,
        }
    }
}

/// Loads raw byte blobs by path.
pub trait AssetLoader {
    /// Load a raw byte blob from a path (e.g. `/res/sprite.png` or
    /// `/manifest.json`).
    fn load_bytes(&self, path: &str) -> anyhow::Result<AssetBytes>;

    /// Load a UTF-8 string from a path.
    fn load_string(&self, path: &str) -> anyhow::Result<String> {
        let b = self.load_bytes(path)?;
        Ok(String::from_utf8(b.to_vec())?)
    }
}

/// An [`AssetLoader`] backed by compile-time `include_bytes!`/`include_str!`
/// data.  Paths are matched exactly against the supplied `(path, bytes)`
/// table and returned as borrowed slices (no allocation).
///
/// Works on every target (native and wasm); this is what the release apps
/// use to ship assets in the binary.
pub struct EmbeddedAssetLoader {
    entries: &'static [(&'static str, &'static [u8])],
}

impl EmbeddedAssetLoader {
    pub fn new(entries: &'static [(&'static str, &'static [u8])]) -> Self {
        Self { entries }
    }
}

impl AssetLoader for EmbeddedAssetLoader {
    fn load_bytes(&self, path: &str) -> anyhow::Result<AssetBytes> {
        self.entries
            .iter()
            .find(|(p, _)| *p == path)
            .map(|(_, bytes)| AssetBytes::Borrowed(bytes))
            .ok_or_else(|| anyhow::anyhow!("embedded asset not found: {path}"))
    }
}

/// An [`AssetLoader`] that reads files from disk, rooted at a directory.
///
/// Native-only (`std::fs::read`); the wasm target has no synchronous
/// filesystem.
#[cfg(not(target_arch = "wasm32"))]
pub struct FsAssetLoader {
    root: std::path::PathBuf,
}

#[cfg(not(target_arch = "wasm32"))]
impl FsAssetLoader {
    pub fn new(root: impl Into<std::path::PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl AssetLoader for FsAssetLoader {
    fn load_bytes(&self, path: &str) -> anyhow::Result<AssetBytes> {
        let full = self.root.join(path.trim_start_matches('/'));
        let bytes = std::fs::read(&full)
            .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", full.display()))?;
        Ok(AssetBytes::Owned(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_loader_returns_borrowed_bytes() {
        static ASSETS: &[(&str, &[u8])] = &[("/res/sprite.png", b"png-bytes")];
        let loader = EmbeddedAssetLoader::new(ASSETS);

        match loader.load_bytes("/res/sprite.png").unwrap() {
            AssetBytes::Borrowed(b) => assert_eq!(b, b"png-bytes"),
            AssetBytes::Owned(_) => panic!("expected borrowed bytes"),
        }
    }

    #[test]
    fn embedded_loader_errors_on_missing_path() {
        let loader = EmbeddedAssetLoader::new(&[]);
        assert!(loader.load_bytes("/res/missing.png").is_err());
    }

    #[test]
    fn embedded_loader_load_string() {
        static ASSETS: &[(&str, &[u8])] = &[("/manifest.json", b"{\"a\":1}")];
        let loader = EmbeddedAssetLoader::new(ASSETS);
        assert_eq!(loader.load_string("/manifest.json").unwrap(), "{\"a\":1}");
    }
}
