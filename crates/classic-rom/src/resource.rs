//! The resource set a ROM bundles: textures, fonts, code modules, and
//! per-animation metadata, keyed by name.
//!
//! A [`ResourceSet`] can be produced from a [`RomArchive`] (the shipped-ROM
//! path) or from an [`AssetLoader`] (the loose-files / embedded dev path),
//! driven by a [`RomManifest`].  Shader sources are resolved separately by a
//! named shader registry (Part 3 of the ROM plan), so `from_archive` /
//! `from_loader` populate textures, fonts, code modules, and animation
//! metadata only.

use std::collections::BTreeMap;

use crate::archive::RomArchive;
use crate::loader::AssetLoader;
use crate::manifest::RomManifest;

/// The seven categories of bundleable resource.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResourceKind {
    Texture,
    /// Per-texture depth map (grayscale `gl_FragDepth` mask), keyed by the
    /// *texture* name it belongs to.
    Depth,
    /// Per-texture normal map (RGB world-space normal, keyed by the *texture*
    /// name it belongs to).
    Normal,
    Font,
    Code,
    /// Per-animation renderer metadata (e.g. per-frame visual offsets).
    Animation,
    /// Raw binary grid (tile / nav / height) data.
    Grid,
    /// Wheeled-vehicle definition sidecar (`vehicle.json`).
    Vehicle,
    /// Packed-atlas frame table (`frames.json`) sidecar for a texture.
    Frames,
}

/// Name-keyed byte blobs, grouped by kind.
#[derive(Clone, Debug, Default)]
pub struct ResourceSet {
    textures: BTreeMap<String, Vec<u8>>,
    depths: BTreeMap<String, Vec<u8>>,
    normals: BTreeMap<String, Vec<u8>>,
    fonts: BTreeMap<String, Vec<u8>>,
    code: BTreeMap<String, Vec<u8>>,
    animations: BTreeMap<String, Vec<u8>>,
    grids: BTreeMap<String, Vec<u8>>,
    vehicles: BTreeMap<String, Vec<u8>>,
    frames: BTreeMap<String, Vec<u8>>,
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
        self.textures.len()
            + self.depths.len()
            + self.normals.len()
            + self.fonts.len()
            + self.code.len()
            + self.animations.len()
            + self.grids.len()
            + self.vehicles.len()
            + self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn textures(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.textures
    }

    pub fn depths(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.depths
    }

    pub fn normals(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.normals
    }

    pub fn fonts(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.fonts
    }

    pub fn code(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.code
    }

    pub fn animations(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.animations
    }

    pub fn grids(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.grids
    }

    pub fn vehicles(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.vehicles
    }

    pub fn frames(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.frames
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
            set.textures.insert(entry.name.clone(), load(crate::rom_path(&entry.src))?);
            if let Some(depth) = &entry.depth {
                set.depths.insert(entry.name.clone(), load(crate::rom_path(depth))?);
            }
            if let Some(normal) = &entry.normal {
                set.normals.insert(entry.name.clone(), load(crate::rom_path(normal))?);
            }
        }
        for entry in &manifest.manifest.sdf_fonts {
            set.fonts.insert(entry.name.clone(), load(crate::rom_path(&entry.metrics))?);
        }
        for entry in &manifest.code {
            set.code.insert(entry.name.clone(), load(crate::rom_path(&entry.src))?);
        }
        for entry in &manifest.manifest.animations {
            if let Some(metadata) = &entry.metadata {
                set.animations.insert(entry.name.clone(), load(crate::rom_path(metadata))?);
            }
        }
        for entry in &manifest.grids {
            set.grids.insert(entry.name.clone(), load(crate::rom_path(&entry.src))?);
        }
        for entry in &manifest.manifest.vehicles {
            set.vehicles.insert(entry.name.clone(), load(crate::rom_path(&entry.src))?);
        }
        for entry in &manifest.manifest.textures {
            if let Some(path) = &entry.frames {
                set.frames.insert(entry.name.clone(), load(crate::rom_path(path))?);
            }
        }
        Ok(set)
    }

    fn map(&self, kind: ResourceKind) -> &BTreeMap<String, Vec<u8>> {
        match kind {
            ResourceKind::Texture => &self.textures,
            ResourceKind::Depth => &self.depths,
            ResourceKind::Normal => &self.normals,
            ResourceKind::Font => &self.fonts,
            ResourceKind::Code => &self.code,
            ResourceKind::Animation => &self.animations,
            ResourceKind::Grid => &self.grids,
            ResourceKind::Vehicle => &self.vehicles,
            ResourceKind::Frames => &self.frames,
        }
    }

    fn map_mut(&mut self, kind: ResourceKind) -> &mut BTreeMap<String, Vec<u8>> {
        match kind {
            ResourceKind::Texture => &mut self.textures,
            ResourceKind::Depth => &mut self.depths,
            ResourceKind::Normal => &mut self.normals,
            ResourceKind::Font => &mut self.fonts,
            ResourceKind::Code => &mut self.code,
            ResourceKind::Animation => &mut self.animations,
            ResourceKind::Grid => &mut self.grids,
            ResourceKind::Vehicle => &mut self.vehicles,
            ResourceKind::Frames => &mut self.frames,
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
        "code": [{"name": "main", "src": "code/main.wasm"}],
        "shaders": [],
        "textures": [{"name": "humanoid", "src": "res/humanoid.png"}],
        "sdf_fonts": [{"name": "dejavusans", "metrics": "res/dejavusans-sdf.json"}],
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
            ("code/main.wasm", b"\0asm".as_slice()),
        ] {
            writer.start_file(name, opts).unwrap();
            writer.write_all(data).unwrap();
        }
        let bytes = writer.finish().unwrap().into_inner();
        RomArchive::from_bytes(&bytes).unwrap()
    }

    #[test]
    fn from_archive_populates_textures_fonts_code() {
        let set = ResourceSet::from_archive(&test_archive(), &test_manifest()).unwrap();
        assert_eq!(set.len(), 3);
        assert_eq!(set.get(ResourceKind::Texture, "humanoid"), Some(b"png".as_slice()));
        assert_eq!(set.get(ResourceKind::Font, "dejavusans"), Some(b"{}".as_slice()));
        assert_eq!(set.get(ResourceKind::Code, "main"), Some(b"\0asm".as_slice()));
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
            ("code/main.wasm", b"\0asm".as_slice()),
        ];
        let loader = EmbeddedAssetLoader::new(ASSETS);
        let set = ResourceSet::from_loader(&loader, &test_manifest()).unwrap();
        assert_eq!(set.len(), 3);
        assert_eq!(set.get(ResourceKind::Code, "main"), Some(b"\0asm".as_slice()));
    }

    #[test]
    fn from_archive_populates_animation_metadata() {
        let manifest: RomManifest = serde_json::from_str(
            r#"{
                "shaders": [],
                "textures": [{"name": "rocketLanding", "src": "res/landing_spritesheet.png"}],
                "animations": [{
                    "name": "rocketLanding",
                    "src": "rocketLanding",
                    "rate": 24,
                    "sequence": [0, 1],
                    "metadata": "animations/rocketLanding.json"
                }]
            }"#,
        )
        .unwrap();

        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, data) in [
            ("res/landing_spritesheet.png", b"png".as_slice()),
            ("animations/rocketLanding.json", b"{\"positions\":[]}".as_slice()),
        ] {
            writer.start_file(name, opts).unwrap();
            writer.write_all(data).unwrap();
        }
        let archive = RomArchive::from_bytes(&writer.finish().unwrap().into_inner()).unwrap();

        let set = ResourceSet::from_archive(&archive, &manifest).unwrap();
        assert_eq!(
            set.get(ResourceKind::Animation, "rocketLanding"),
            Some(b"{\"positions\":[]}".as_slice())
        );
    }

    #[test]
    fn from_archive_populates_grids() {
        let manifest: RomManifest = serde_json::from_str(
            r#"{
                "shaders": [],
                "textures": [],
                "animations": [],
                "grids": [{"name": "demo.tiles", "src": "data/demo.tiles.bin"}]
            }"#,
        )
        .unwrap();

        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        writer.start_file("data/demo.tiles.bin", opts).unwrap();
        writer.write_all(&[1, 0, 0, 0, 2, 0, 0, 0]).unwrap();
        let archive = RomArchive::from_bytes(&writer.finish().unwrap().into_inner()).unwrap();

        let set = ResourceSet::from_archive(&archive, &manifest).unwrap();
        assert_eq!(set.get(ResourceKind::Grid, "demo.tiles"), Some(&[1, 0, 0, 0, 2, 0, 0, 0][..]));
    }

    #[test]
    fn from_archive_populates_frames() {
        let manifest: RomManifest = serde_json::from_str(
            r#"{
                "shaders": [],
                "textures": [
                    { "name": "humanoid", "src": "res/humanoid.png", "frames": "res/humanoid.frames.json" }
                ],
                "animations": []
            }"#,
        )
        .unwrap();

        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        writer.start_file("res/humanoid.png", opts).unwrap();
        writer.write_all(b"png").unwrap();
        writer.start_file("res/humanoid.frames.json", opts).unwrap();
        writer.write_all(b"{\"version\":1,\"sheets\":[],\"frames\":{}}").unwrap();
        let archive = RomArchive::from_bytes(&writer.finish().unwrap().into_inner()).unwrap();

        let set = ResourceSet::from_archive(&archive, &manifest).unwrap();
        assert_eq!(
            set.get(ResourceKind::Frames, "humanoid"),
            Some(b"{\"version\":1,\"sheets\":[],\"frames\":{}}".as_slice())
        );
    }

    #[test]
    fn from_archive_populates_depth_maps() {
        let manifest: RomManifest = serde_json::from_str(
            r#"{
                "shaders": [],
                "textures": [
                    {"name": "lrvBody", "src": "res/lrv_body.png", "depth": "res/lrv_body_depth.png", "depth_range": 0.05},
                    {"name": "tree", "src": "res/tree.png"}
                ],
                "animations": []
            }"#,
        )
        .unwrap();

        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, data) in [
            ("res/lrv_body.png", b"body".as_slice()),
            ("res/lrv_body_depth.png", b"depth".as_slice()),
            ("res/tree.png", b"tree".as_slice()),
        ] {
            writer.start_file(name, opts).unwrap();
            writer.write_all(data).unwrap();
        }
        let archive = RomArchive::from_bytes(&writer.finish().unwrap().into_inner()).unwrap();

        let set = ResourceSet::from_archive(&archive, &manifest).unwrap();
        assert_eq!(set.get(ResourceKind::Depth, "lrvBody"), Some(b"depth".as_slice()));
        // Textures without a depth map get no entry.
        assert_eq!(set.get(ResourceKind::Depth, "tree"), None);
    }

    #[test]
    fn from_archive_populates_normal_maps() {
        let manifest: RomManifest = serde_json::from_str(
            r#"{
                "shaders": [],
                "textures": [
                    {"name": "lrvBody", "src": "res/lrv_body.png", "normal": "res/lrv_body_normal.png"},
                    {"name": "tree", "src": "res/tree.png"}
                ],
                "animations": []
            }"#,
        )
        .unwrap();

        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, data) in [
            ("res/lrv_body.png", b"body".as_slice()),
            ("res/lrv_body_normal.png", b"normal".as_slice()),
            ("res/tree.png", b"tree".as_slice()),
        ] {
            writer.start_file(name, opts).unwrap();
            writer.write_all(data).unwrap();
        }
        let archive = RomArchive::from_bytes(&writer.finish().unwrap().into_inner()).unwrap();

        let set = ResourceSet::from_archive(&archive, &manifest).unwrap();
        assert_eq!(set.get(ResourceKind::Normal, "lrvBody"), Some(b"normal".as_slice()));
        // Textures without a normal map get no entry.
        assert_eq!(set.get(ResourceKind::Normal, "tree"), None);
    }
}
