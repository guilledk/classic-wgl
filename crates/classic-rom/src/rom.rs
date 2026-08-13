//! The `Rom` type: a self-contained, loadable scene bundle.
//!
//! A [`Rom`] binds a manifest, a resource set, and a serialized entity state
//! into one object that the engine can hydrate.  [`Rom::load`] reads it from a
//! [`RomArchive`]; [`Rom::pack`] serializes it back to a zip archive (the
//! canonical ROM container).

use std::io::Write;

use crate::archive::RomArchive;
use crate::manifest::RomManifest;
use crate::resource::ResourceSet;

/// A self-contained ROM: manifest + resources + entity state.
#[derive(Clone, Debug)]
pub struct Rom {
    /// Parsed manifest (drives shader/texture/font/animation loading).
    pub manifest: RomManifest,
    /// Raw `manifest.json` text, round-tripped verbatim by [`Rom::pack`].
    pub manifest_json: String,
    /// Name-keyed resource blobs (textures, fonts, code modules).
    pub resources: ResourceSet,
    /// The serialized entity graph (`state.json`).
    pub state: String,
}

impl Rom {
    /// Read a ROM from an archive.  Expects `manifest.json` and `state.json`
    /// at the archive root, and every manifest-declared resource inlined.
    pub fn load(archive: &RomArchive) -> anyhow::Result<Self> {
        let manifest_json = archive.read_string("manifest.json")?;
        let manifest: RomManifest = serde_json::from_str(&manifest_json)?;
        let resources = ResourceSet::from_archive(archive, &manifest)?;
        let state = archive.read_string("state.json")?;
        Ok(Self { manifest, manifest_json, resources, state })
    }

    /// Serialize the ROM to a zip archive.
    pub fn pack(&self) -> anyhow::Result<Vec<u8>> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        writer.start_file("manifest.json", opts)?;
        writer.write_all(self.manifest_json.as_bytes())?;
        writer.start_file("state.json", opts)?;
        writer.write_all(self.state.as_bytes())?;

        for entry in &self.manifest.manifest.textures {
            if let Some(bytes) = self.resources.textures().get(&entry.name) {
                writer.start_file(crate::rom_path(&entry.src), opts)?;
                writer.write_all(bytes)?;
            }
        }
        for entry in &self.manifest.manifest.sdf_fonts {
            if let Some(metrics) = self.resources.fonts().get(&entry.name) {
                writer.start_file(crate::rom_path(&entry.metrics), opts)?;
                writer.write_all(metrics)?;
            }
        }
        for entry in &self.manifest.code {
            if let Some(src) = self.resources.code().get(&entry.name) {
                writer.start_file(crate::rom_path(&entry.src), opts)?;
                writer.write_all(src)?;
            }
        }

        Ok(writer.finish()?.into_inner())
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
        "sdfFonts": [{"name": "dejavusans", "metrics": "/res/dejavusans-sdf.json"}],
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
}
