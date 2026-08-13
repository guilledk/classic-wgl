//! The resource set a ROM bundles: textures, fonts, shaders, and scripts,
//! keyed by name.
//!
//! A [`ResourceSet`] can be produced from a [`RomArchive`] (the shipped-ROM
//! path) or from an [`AssetLoader`] (the loose-files / embedded dev path),
//! driven by a [`RomManifest`].  Shader sources are resolved separately by a
//! named shader registry (Part 3 of the ROM plan), so `from_archive` /
//! `from_loader` populate textures, fonts, and scripts only.

use std::collections::BTreeMap;

use crate::archive::RomArchive;
use crate::loader::AssetLoader;
use crate::manifest::RomManifest;

/// The four categories of bundleable resource.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResourceKind {
    Texture,
    Font,
    Shader,
    Script,
}

/// Name-keyed byte blobs, grouped by kind.
#[derive(Clone, Debug, Default)]
pub struct ResourceSet {
    textures: BTreeMap<String, Vec<u8>>,
    fonts: BTreeMap<String, Vec<u8>>,
    shaders: BTreeMap<String, Vec<u8>>,
    scripts: BTreeMap<String, Vec<u8>>,
}

impl ResourceSet {
    /// Look up a resource by kind + name.
    pub fn get(&self, kind: ResourceKind, name: &str) -> Option<&[u8]> {
        let map = self.map(kind);
        map.get(name).map(|v| v.as_slice())
    }

    /// Insert a resource blob.
    pub fn insert(&mut self, kind: ResourceKind, name: impl Into<String>, bytes: Vec<u8>) {
        self.map_mut(kind).insert(name.into(), bytes);
    }

    /// The total number of resources across all categories.
    pub fn len(&self) -> usize {
        self.textures.len() + self.fonts.len() + self.shaders.len() + self.scripts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn textures(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.textures
    }

    pub fn fonts(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.fonts
    }

    pub fn shaders(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.shaders
    }

    pub fn scripts(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.scripts
    }

    /// Build a resource set by reading manifest-declared resources out of a
    /// ROM archive.  A missing entry is an error (a self-contained ROM must
    /// bundle everything it declares).
    pub fn from_archive(archive: &RomArchive, manifest: &RomManifest) -> anyhow::Result<Self> {
        Self::build(manifest, |path| {
            archive
                .read(path)
                .map(|b| b.to_vec())
                .ok_or_else(|| anyhow::anyhow!("ROM entry not found: {path}"))
        })
    }

    /// Build a resource set by loading manifest-declared resources through an
    /// [`AssetLoader`] (loose files / embedded map).
    pub fn from_loader(loader: &dyn AssetLoader, manifest: &RomManifest) -> anyhow::Result<Self> {
        Self::build(manifest, |path| Ok(loader.load_bytes(path)?.to_vec()))
    }

    fn build(
        manifest: &RomManifest,
        mut load: impl FnMut(&str) -> anyhow::Result<Vec<u8>>,
    ) -> anyhow::Result<Self> {
        let mut set = Self::default();
        for entry in &manifest.manifest.textures {
            set.textures.insert(entry.name.clone(), load(&entry.src)?);
        }
        for entry in &manifest.manifest.sdf_fonts {
            set.fonts.insert(entry.name.clone(), load(&entry.metrics)?);
        }
        for entry in &manifest.scripts {
            set.scripts.insert(entry.name.clone(), load(&entry.src)?);
        }
        Ok(set)
    }

    fn map(&self, kind: ResourceKind) -> &BTreeMap<String, Vec<u8>> {
        match kind {
            ResourceKind::Texture => &self.textures,
            ResourceKind::Font => &self.fonts,
            ResourceKind::Shader => &self.shaders,
            ResourceKind::Script => &self.scripts,
        }
    }

    fn map_mut(&mut self, kind: ResourceKind) -> &mut BTreeMap<String, Vec<u8>> {
        match kind {
            ResourceKind::Texture => &mut self.textures,
            ResourceKind::Font => &mut self.fonts,
            ResourceKind::Shader => &mut self.shaders,
            ResourceKind::Script => &mut self.scripts,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    use crate::loader::EmbeddedAssetLoader;

    // NB: paths are written without a leading `/` here; canonicalising the
    // manifest's leading-slash convention to archive entries is a Part 4
    // concern (the ROM builder).
    const MANIFEST_JSON: &str = r#"{
        "format_version": 1,
        "entrypoint": "demo",
        "scripts": [{"name": "main", "src": "scripts/main.rhai"}],
        "shaders": [],
        "textures": [{"name": "humanoid", "src": "res/humanoid.png"}],
        "sdfFonts": [{"name": "dejavusans", "metrics": "res/dejavusans-sdf.json"}],
        "animations": []
    }"#;

    fn test_manifest() -> RomManifest {
        serde_json::from_str(MANIFEST_JSON).unwrap()
    }

    fn test_archive() -> RomArchive {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, data) in [
            ("res/humanoid.png", b"png".as_slice()),
            ("res/dejavusans-sdf.json", b"{}".as_slice()),
            ("scripts/main.rhai", b"fn update(ctx) {}".as_slice()),
        ] {
            writer.start_file(name, opts).unwrap();
            writer.write_all(data).unwrap();
        }
        let bytes = writer.finish().unwrap().into_inner();
        RomArchive::from_bytes(&bytes).unwrap()
    }

    #[test]
    fn from_archive_populates_textures_fonts_scripts() {
        let set = ResourceSet::from_archive(&test_archive(), &test_manifest()).unwrap();
        assert_eq!(set.len(), 3);
        assert_eq!(set.get(ResourceKind::Texture, "humanoid"), Some(b"png".as_slice()));
        assert_eq!(set.get(ResourceKind::Font, "dejavusans"), Some(b"{}".as_slice()));
        assert_eq!(set.get(ResourceKind::Script, "main"), Some(b"fn update(ctx) {}".as_slice()));
    }

    #[test]
    fn from_archive_errors_on_missing_entry() {
        let archive = test_archive();
        let mut manifest = test_manifest();
        manifest.manifest.textures[0].src = "res/missing.png".into();
        assert!(ResourceSet::from_archive(&archive, &manifest).is_err());
    }

    #[test]
    fn from_loader_populates() {
        static ASSETS: &[(&str, &[u8])] = &[
            ("res/humanoid.png", b"png".as_slice()),
            ("res/dejavusans-sdf.json", b"{}".as_slice()),
            ("scripts/main.rhai", b"fn update(ctx) {}".as_slice()),
        ];
        let loader = EmbeddedAssetLoader::new(ASSETS);
        let set = ResourceSet::from_loader(&loader, &test_manifest()).unwrap();
        assert_eq!(set.len(), 3);
        assert_eq!(set.get(ResourceKind::Script, "main"), Some(b"fn update(ctx) {}".as_slice()));
    }
}
