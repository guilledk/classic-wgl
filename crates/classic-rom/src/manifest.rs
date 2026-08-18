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

/// A bundled binary grid resource (tile / nav / height data) in a ROM
/// manifest.  The payload is raw little-endian numbers; the element type is
/// implied by the consumer (tiles/nav are `u32`, heights are `f32`).
#[derive(Clone, Debug, serde::Deserialize)]
pub struct GridEntry {
    pub name: String,
    pub src: String,
}

/// The default ROM state entry name.
pub const DEFAULT_STATE_ENTRY: &str = "state.json";

fn default_state_entry() -> String {
    DEFAULT_STATE_ENTRY.to_string()
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
    /// Namespace prefix for this ROM's entities (empty = global, un-namespaced
    /// names).  Groundwork for multi-ROM loading: when non-empty, entity names
    /// are qualified as `"{namespace}::{name}"`.
    #[serde(default)]
    pub namespace: String,
    /// Archive entry holding the serialized entity state (default `state.json`).
    #[serde(default = "default_state_entry")]
    pub state: String,
    #[serde(default)]
    pub code: Vec<CodeEntry>,
    /// Bundled binary grid resources (tile / nav / height data), referenced by
    /// name from the entity state.
    #[serde(default)]
    pub grids: Vec<GridEntry>,
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
            "sdf_fonts": [{"name": "dejavusans", "metrics": "/res/dejavusans-sdf.json"}],
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
