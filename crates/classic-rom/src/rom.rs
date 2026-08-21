//! The `Rom` type: a self-contained, loadable scene bundle.
//!
//! A [`Rom`] binds a manifest, a resource set, and a serialized entity state
//! into one object that the engine can hydrate.  [`Rom::load`] reads it from a
//! [`RomArchive`]; [`Rom::pack`] serializes it back to a zstd-compressed tar
//! (the canonical container, falling back to deflate zip on wasm).

use std::io::Write;

use crate::archive::RomArchive;
use crate::manifest::RomManifest;
use crate::resource::ResourceSet;

/// The bootstrap archive entry: the ROM manifest (always read first, so its
/// entry name must be well-known).
pub const MANIFEST_ENTRY: &str = "manifest.json";

/// A self-contained ROM: manifest + resources + entity state.
#[derive(Clone, Debug)]
pub struct Rom {
    /// Parsed manifest (drives shader/texture/font/animation loading).
    pub manifest: RomManifest,
    /// Raw `manifest.json` text, round-tripped verbatim by [`Rom::pack`].
    pub manifest_json: String,
    /// Name-keyed resource blobs (textures, fonts, code modules).
    pub resources: ResourceSet,
    /// The serialized entity graph (the manifest's `state` entry).
    pub state: String,
}

impl Rom {
    /// Read a ROM from an archive.  Expects the manifest at [`MANIFEST_ENTRY`]
    /// and the entity state at the manifest-declared `state` entry, plus every
    /// manifest-declared resource inlined.
    pub fn load(archive: &RomArchive) -> anyhow::Result<Self> {
        let manifest_json = archive.read_string(MANIFEST_ENTRY)?;
        let manifest: RomManifest = serde_json::from_str(&manifest_json)?;
        let resources = ResourceSet::from_archive(archive, &manifest)?;
        let state = archive.read_string(&manifest.state)?;
        Ok(Self { manifest, manifest_json, resources, state })
    }

    /// Serialize the ROM to its canonical container: a zstd-compressed tar
    /// (`tar.zst`) on native targets, falling back to a deflate zip on wasm32
    /// (which has no zstd encoder).  [`RomArchive`] reads either.
    pub fn pack(&self) -> anyhow::Result<Vec<u8>> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.pack_tar_zst()
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.pack_zip()
        }
    }

    /// Serialize the ROM to a deflate zip archive (wasm-compatible fallback and
    /// the historical container format).
    pub fn pack_zip(&self) -> anyhow::Result<Vec<u8>> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (path, bytes) in self.entries() {
            writer.start_file(path, opts)?;
            writer.write_all(&bytes)?;
        }
        Ok(writer.finish()?.into_inner())
    }

    /// Serialize the ROM to a zstd-compressed tar archive.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn pack_tar_zst(&self) -> anyhow::Result<Vec<u8>> {
        let mut tar = tar::Builder::new(Vec::new());
        for (path, bytes) in self.entries() {
            let mut header = tar::Header::new_gnu();
            header.set_mode(0o644);
            header.set_size(bytes.len() as u64);
            tar.append_data(&mut header, &path, bytes.as_slice())?;
        }
        let tar_bytes = tar.into_inner()?;
        Ok(zstd::bulk::compress(&tar_bytes, 19)?)
    }

    /// The flat list of `(path, bytes)` archive entries, in manifest-first order.
    fn entries(&self) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        out.push((MANIFEST_ENTRY.to_string(), self.manifest_json.as_bytes().to_vec()));
        out.push((self.manifest.state.clone(), self.state.as_bytes().to_vec()));

        for entry in &self.manifest.manifest.textures {
            if let Some(bytes) = self.resources.textures().get(&entry.name) {
                out.push((crate::rom_path(&entry.src).to_string(), bytes.clone()));
            }
            if let (Some(path), Some(bytes)) =
                (&entry.frames, self.resources.frames().get(&entry.name))
            {
                out.push((crate::rom_path(path).to_string(), bytes.clone()));
            }
        }
        for entry in &self.manifest.manifest.textures {
            if let Some(path) = &entry.depth {
                if let Some(bytes) = self.resources.depths().get(&entry.name) {
                    out.push((crate::rom_path(path).to_string(), bytes.clone()));
                }
            }
            if let Some(path) = &entry.normal {
                if let Some(bytes) = self.resources.normals().get(&entry.name) {
                    out.push((crate::rom_path(path).to_string(), bytes.clone()));
                }
            }
        }
        for entry in &self.manifest.manifest.sdf_fonts {
            if let Some(metrics) = self.resources.fonts().get(&entry.name) {
                out.push((crate::rom_path(&entry.metrics).to_string(), metrics.clone()));
            }
        }
        for entry in &self.manifest.code {
            if let Some(src) = self.resources.code().get(&entry.name) {
                out.push((crate::rom_path(&entry.src).to_string(), src.clone()));
            }
        }
        for entry in &self.manifest.manifest.animations {
            if let Some(metadata) = self.resources.animations().get(&entry.name) {
                if let Some(path) = &entry.metadata {
                    out.push((crate::rom_path(path).to_string(), metadata.clone()));
                }
            }
        }
        for entry in &self.manifest.grids {
            if let Some(bytes) = self.resources.grids().get(&entry.name) {
                out.push((crate::rom_path(&entry.src).to_string(), bytes.clone()));
            }
        }
        for entry in &self.manifest.manifest.vehicles {
            if let Some(bytes) = self.resources.vehicles().get(&entry.name) {
                out.push((crate::rom_path(&entry.src).to_string(), bytes.clone()));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource::ResourceKind;

    const MANIFEST_JSON: &str = r#"{
        "format_version": 1,
        "entrypoint": "demo",
        "code": [{"name": "main", "src": "/code/main.wasm"}],
        "shaders": [],
        "textures": [{"name": "humanoid", "src": "/res/humanoid.png"}],
        "sdf_fonts": [{"name": "dejavusans", "metrics": "/res/dejavusans-sdf.json"}],
        "animations": []
    }"#;

    fn test_rom() -> Rom {
        let manifest: RomManifest = serde_json::from_str(MANIFEST_JSON).unwrap();
        let mut resources = ResourceSet::default();
        resources.insert(ResourceKind::Texture, "humanoid", b"png-bytes".to_vec());
        resources.insert(ResourceKind::Font, "dejavusans", b"{\"name\":\"dejavusans\"}".to_vec());
        resources.insert(ResourceKind::Code, "main", b"\0asm".to_vec());

        Rom {
            manifest,
            manifest_json: MANIFEST_JSON.into(),
            resources,
            state: "{\"entities\":{}}".into(),
        }
    }

    #[test]
    fn pack_and_load_round_trips() {
        let rom = test_rom();
        let bytes = rom.pack().unwrap();
        let archive = RomArchive::from_bytes(&bytes).unwrap();
        let loaded = Rom::load(&archive).unwrap();

        assert_eq!(loaded.state, rom.state);
        assert_eq!(loaded.manifest.entrypoint, "demo");
        assert_eq!(
            loaded.resources.get(ResourceKind::Texture, "humanoid"),
            Some(b"png-bytes".as_slice())
        );
        assert_eq!(loaded.resources.get(ResourceKind::Code, "main"), Some(b"\0asm".as_slice()));
    }

    #[test]
    fn pack_manifest_round_trips_verbatim() {
        let rom = test_rom();
        let bytes = rom.pack().unwrap();
        let archive = RomArchive::from_bytes(&bytes).unwrap();
        assert_eq!(archive.read_string("manifest.json").unwrap(), rom.manifest_json);
    }

    #[test]
    fn pack_emits_zstd_magic() {
        let rom = test_rom();
        let bytes = rom.pack().unwrap();
        // zstd frames begin `28 b5 2f fd`.
        assert_eq!(&bytes[0..4], &[0x28, 0xB5, 0x2F, 0xFD]);
    }

    #[test]
    fn pack_zip_round_trips() {
        let rom = test_rom();
        let bytes = rom.pack_zip().unwrap();
        let archive = RomArchive::from_bytes(&bytes).unwrap();
        let loaded = Rom::load(&archive).unwrap();
        assert_eq!(loaded.state, rom.state);
        assert_eq!(
            loaded.resources.get(ResourceKind::Texture, "humanoid"),
            Some(b"png-bytes".as_slice())
        );
    }

    #[test]
    fn load_rejects_missing_state() {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        writer.start_file("manifest.json", opts).unwrap();
        writer.write_all(MANIFEST_JSON.as_bytes()).unwrap();
        let bytes = writer.finish().unwrap().into_inner();

        let archive = RomArchive::from_bytes(&bytes).unwrap();
        assert!(Rom::load(&archive).is_err());
    }

    #[test]
    fn pack_and_load_round_trips_depth_maps() {
        let manifest_json = r#"{
            "format_version": 1,
            "entrypoint": "demo",
            "shaders": [],
            "textures": [
                {"name": "lrvBody", "src": "/res/lrv_body.png", "depth": "/res/lrv_body_depth.png", "depth_range": 0.05},
                {"name": "tree", "src": "/res/tree.png"}
            ],
            "animations": []
        }"#;
        let manifest: RomManifest = serde_json::from_str(manifest_json).unwrap();
        let mut resources = ResourceSet::default();
        resources.insert(ResourceKind::Texture, "lrvBody", b"body".to_vec());
        resources.insert(ResourceKind::Depth, "lrvBody", b"depth".to_vec());
        resources.insert(ResourceKind::Texture, "tree", b"tree".to_vec());
        let rom = Rom {
            manifest,
            manifest_json: manifest_json.into(),
            resources,
            state: "{\"entities\":{}}".into(),
        };

        let bytes = rom.pack().unwrap();
        let archive = RomArchive::from_bytes(&bytes).unwrap();
        let loaded = Rom::load(&archive).unwrap();

        assert_eq!(loaded.resources.get(ResourceKind::Depth, "lrvBody"), Some(b"depth".as_slice()));
        assert_eq!(loaded.manifest.manifest.textures[0].depth_range, 0.05);
        assert_eq!(loaded.resources.get(ResourceKind::Depth, "tree"), None);
    }

    #[test]
    fn pack_and_load_round_trips_normal_maps() {
        let manifest_json = r#"{
            "format_version": 1,
            "entrypoint": "demo",
            "shaders": [],
            "textures": [
                {"name": "lrvBody", "src": "/res/lrv_body.png", "normal": "/res/lrv_body_normal.png"},
                {"name": "tree", "src": "/res/tree.png"}
            ],
            "animations": []
        }"#;
        let manifest: RomManifest = serde_json::from_str(manifest_json).unwrap();
        let mut resources = ResourceSet::default();
        resources.insert(ResourceKind::Texture, "lrvBody", b"body".to_vec());
        resources.insert(ResourceKind::Normal, "lrvBody", b"normal".to_vec());
        resources.insert(ResourceKind::Texture, "tree", b"tree".to_vec());
        let rom = Rom {
            manifest,
            manifest_json: manifest_json.into(),
            resources,
            state: "{\"entities\":{}}".into(),
        };

        let bytes = rom.pack().unwrap();
        let archive = RomArchive::from_bytes(&bytes).unwrap();
        let loaded = Rom::load(&archive).unwrap();

        assert_eq!(
            loaded.resources.get(ResourceKind::Normal, "lrvBody"),
            Some(b"normal".as_slice())
        );
        assert_eq!(loaded.resources.get(ResourceKind::Normal, "tree"), None);
    }
}
