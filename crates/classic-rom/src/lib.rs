//! classic-rom: self-contained "ROM" archive reading.
//!
//! A ROM is a playable artifact bundled into a single archive (zip, tar.gz,
//! or tar.zst) holding entity state, resources, and scripts.  This crate
//! provides the archive reader ([`RomArchive`]), container-format detection
//! ([`RomFormat`], [`detect_format`]), the asset-loading abstraction
//! ([`AssetLoader`]), and the manifest / resource model
//! ([`RomManifest`], [`ResourceSet`]) that the engine consumes to hydrate a
//! world.

pub mod archive;
pub mod format;
pub mod loader;
pub mod manifest;
pub mod resource;

pub use archive::RomArchive;
pub use format::{detect_format, RomFormat};
#[cfg(not(target_arch = "wasm32"))]
pub use loader::FsAssetLoader;
pub use loader::{AssetBytes, AssetLoader, EmbeddedAssetLoader};
pub use manifest::{RomManifest, ScriptManifestEntry};
pub use resource::{ResourceKind, ResourceSet};
