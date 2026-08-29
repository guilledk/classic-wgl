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
    /// Names of ROMs this ROM depends on, forming an arbitrary multi-ROM DAG.
    /// Resolved through the same name -> location index as the root ROM and
    /// loaded before their dependents (topological order).  Deps contribute
    /// resources, entities, and (in the full multi-ROM path) guest code.
    #[serde(default)]
    pub deps: Vec<String>,
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
    /// Semantic version of the ROM content, stamped by `classic-roms` (see its
    /// per-ROM `version` in `scene.json`).  Distinct from `format_version`,
    /// which is the manifest schema contract.  `None` for ROMs packed before
    /// per-ROM versioning existed.
    #[serde(default)]
    pub version: Option<String>,
    /// Item catalog: ROM-namespaced item definitions, interned by the host
    /// into a per-ROM [`classic_core::inventory::ItemRegistry`] at load.
    #[serde(default)]
    pub items: Vec<classic_core::inventory::ItemDef>,
    /// Inventory types: named per-class capacity multipliers used by the host
    /// inventory mechanics.
    #[serde(default)]
    pub inventory_types: Vec<classic_core::inventory::InventoryType>,
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

    #[test]
    fn version_is_optional() {
        let without = r#"{"shaders": [], "textures": [], "animations": []}"#;
        let m: RomManifest = serde_json::from_str(without).unwrap();
        assert_eq!(m.version, None);

        let with = r#"{"shaders": [], "textures": [], "animations": [], "version": "0.2.0"}"#;
        let m: RomManifest = serde_json::from_str(with).unwrap();
        assert_eq!(m.version.as_deref(), Some("0.2.0"));
    }

    #[test]
    fn items_and_inventory_types_deserialize_and_default_empty() {
        let json = r#"{
            "shaders": [],
            "textures": [],
            "animations": [],
            "items": [
                {"name": "shipping_container", "class": "container",
                 "stack_rule": {"rule": "unit", "max_per_stack": 1}}
            ],
            "inventory_types": [
                {"name": "cargo_bay", "capacity_mult": [["container", 1.0]]}
            ]
        }"#;
        let m: RomManifest = serde_json::from_str(json).unwrap();
        assert_eq!(m.items.len(), 1);
        assert_eq!(m.items[0].name, "shipping_container");
        assert_eq!(m.inventory_types.len(), 1);
        assert_eq!(m.inventory_types[0].name, "cargo_bay");

        // Absent items/inventory_types default to empty.
        let empty: RomManifest =
            serde_json::from_str(r#"{"shaders":[],"textures":[],"animations":[]}"#).unwrap();
        assert!(empty.items.is_empty());
        assert!(empty.inventory_types.is_empty());
    }

    #[test]
    fn deps_deserialize_and_default_empty() {
        let with = r#"{
            "shaders": [],
            "textures": [],
            "animations": [],
            "namespace": "lunar",
            "deps": ["common", "vehicles"]
        }"#;
        let m: RomManifest = serde_json::from_str(with).unwrap();
        assert_eq!(m.namespace, "lunar");
        assert_eq!(m.deps, vec!["common".to_string(), "vehicles".to_string()]);

        let empty: RomManifest =
            serde_json::from_str(r#"{"shaders":[],"textures":[],"animations":[]}"#).unwrap();
        assert!(empty.deps.is_empty());
    }
}
