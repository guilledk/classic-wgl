//! ROM manifest model.
//!
//! A ROM's `manifest.json` extends the engine's existing [`Manifest`]
//! (shaders/textures/fonts/animations) with the fields a self-contained ROM
//! needs: a format version, an entrypoint, and the bundled code module list.

use classic_core::types::Manifest;

/// A single bundled code (WASM) module in a ROM manifest.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct CodeEntry {
    pub name: String,
    pub src: String,
}

/// The ROM manifest: an engine [`Manifest`] plus ROM-specific metadata.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct RomManifest {
    #[serde(flatten)]
    pub manifest: Manifest,
    #[serde(default = "default_format_version")]
    pub format_version: u32,
    #[serde(default)]
    pub entrypoint: String,
    #[serde(default)]
    pub code: Vec<CodeEntry>,
    /// Whether this ROM ships the host toolchain (editor HUD, widgets, debug
    /// overlays, test runner).  Bare gameplay ROMs leave this false and skip
    /// the editor; the demo/lunar ROMs opt in.
    #[serde(default)]
    pub host_features: bool,
    /// Whether the ROM's guest code is trusted.  Untrusted guests (default)
    /// run sandboxed with fuel metering + memory caps; trusted ROMs skip the
    /// slow path (e.g. browser WebAssembly on web).
    #[serde(default)]
    pub trusted: bool,
}

fn default_format_version() -> u32 {
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rom_manifest_with_flattened_base() {
        let json = r#"{
            "format_version": 1,
            "entrypoint": "demo",
            "code": [{"name": "main", "src": "code/main.wasm"}],
            "shaders": [],
            "textures": [{"name": "humanoid", "src": "/res/humanoid.png"}],
            "sdfFonts": [{"name": "dejavusans", "metrics": "/res/dejavusans-sdf.json"}],
            "animations": []
        }"#;
        let m: RomManifest = serde_json::from_str(json).unwrap();
        assert_eq!(m.format_version, 1);
        assert_eq!(m.entrypoint, "demo");
        assert_eq!(m.code.len(), 1);
        assert_eq!(m.code[0].name, "main");
        assert_eq!(m.manifest.textures.len(), 1);
        assert_eq!(m.manifest.textures[0].name, "humanoid");
        assert_eq!(m.manifest.sdf_fonts[0].name, "dejavusans");
    }

    #[test]
    fn host_features_defaults_false() {
        let json = r#"{
            "shaders": [],
            "textures": [],
            "animations": []
        }"#;
        let m: RomManifest = serde_json::from_str(json).unwrap();
        assert!(!m.host_features);
        assert!(!m.trusted);
    }
}
