//! ROM manifest model.
//!
//! A ROM's `manifest.json` extends the engine's existing [`Manifest`]
//! (shaders/textures/fonts/animations) with the fields a self-contained ROM
//! needs: a format version, an entrypoint, and the bundled script list.

use classic_core::types::Manifest;

/// A single bundled script in a ROM manifest.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct ScriptManifestEntry {
    pub name: String,
    pub src: String,
}

/// The ROM manifest: an engine [`Manifest`] plus ROM-specific metadata.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct RomManifest {
    #[serde(flatten)]
    pub manifest: Manifest,
    pub format_version: u32,
    pub entrypoint: String,
    #[serde(default)]
    pub scripts: Vec<ScriptManifestEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rom_manifest_with_flattened_base() {
        // NB: `Manifest` currently uses snake_case serde names (`sdf_fonts`),
        // so the base manifest fields here use snake_case; Part 3 of the ROM
        // plan adds `rename_all = "camelCase"` when sdf_fonts becomes
        // authoritative for loading.
        let json = r#"{
            "format_version": 1,
            "entrypoint": "demo",
            "scripts": [{"name": "main", "src": "scripts/main.rhai"}],
            "shaders": [],
            "textures": [{"name": "humanoid", "src": "/res/humanoid.png"}],
            "sdf_fonts": [{"name": "dejavusans", "metrics": "/res/dejavusans-sdf.json"}],
            "animations": []
        }"#;
        let m: RomManifest = serde_json::from_str(json).unwrap();
        assert_eq!(m.format_version, 1);
        assert_eq!(m.entrypoint, "demo");
        assert_eq!(m.scripts.len(), 1);
        assert_eq!(m.scripts[0].name, "main");
        assert_eq!(m.manifest.textures.len(), 1);
        assert_eq!(m.manifest.textures[0].name, "humanoid");
        assert_eq!(m.manifest.sdf_fonts[0].name, "dejavusans");
    }
}
