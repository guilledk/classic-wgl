//! classic-rom: self-contained "ROM" archive reading.
//!
//! A ROM is a playable artifact bundled into a single archive (zip, tar.gz,
//! or tar.zst) holding entity state, resources, and scripts.  This crate
//! provides the archive reader ([`RomArchive`]), container-format detection
//! ([`RomFormat`], [`detect_format`]), and — once wired — the manifest /
//! resource model that [`classic_engine`] consumes to hydrate a world.

pub mod archive;
pub mod format;

pub use archive::RomArchive;
pub use format::{detect_format, RomFormat};
