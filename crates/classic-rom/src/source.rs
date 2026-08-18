//! ROM selection: parsing the `CLASSIC_ROM` / `?rom=` selector into a
//! [`RomSource`].
//!
//! The selector is a URI-scheme grammar: embedded ROMs are namespaced under
//! `rom:`, `http(s)://` is a network fetch, `file:` / a bare value is a local
//! path, and `data:` is reserved for future inline payloads.  This module is
//! pure — it only classifies the string; actually materialising bytes (reading
//! a file, fetching a URL, or looking up an embedded ROM) is the platform
//! layer's job (`classic_platform::resolve_rom`).

use std::path::PathBuf;

/// Where a ROM archive comes from, as parsed from a selector string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RomSource {
    /// A named ROM embedded at compile time (the `rom:<name>` namespace).
    Embedded(String),
    /// An `http://` / `https://` URL to fetch.
    Url(String),
    /// A local filesystem path (`file:` or a bare value).
    Path(PathBuf),
    /// A `data:` URI (parsed but not yet materialised by the platform layer).
    Data(Vec<u8>),
}

/// The name of the shipped default ROM (used when the selector is empty).
pub const DEFAULT_ROM: &str = "demo";

/// Parse a ROM selector string into a [`RomSource`].
///
/// Grammar:
/// - empty → [`RomSource::Embedded`]`("demo")`
/// - `rom:<name>` → [`RomSource::Embedded`]`(name)`
/// - `http://` / `https://` → [`RomSource::Url`]
/// - `file://` → [`RomSource::Path`] (the `file://` prefix stripped)
/// - `data:` → [`RomSource::Data`] (raw bytes after the comma)
/// - anything else → [`RomSource::Path`] (a bare, un-prefixed path)
pub fn parse_rom_spec(spec: &str) -> RomSource {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return RomSource::Embedded(DEFAULT_ROM.to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("rom:") {
        return RomSource::Embedded(rest.trim().to_string());
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return RomSource::Url(trimmed.to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("file://") {
        return RomSource::Path(PathBuf::from(rest));
    }
    if let Some(rest) = trimmed.strip_prefix("data:") {
        // Split at the first comma: `data:<meta>,<payload>`.
        let payload = rest.split_once(',').map(|(_, d)| d).unwrap_or(rest);
        return RomSource::Data(payload.as_bytes().to_vec());
    }
    RomSource::Path(PathBuf::from(trimmed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_spec_defaults_to_demo() {
        assert_eq!(parse_rom_spec(""), RomSource::Embedded("demo".into()));
        assert_eq!(parse_rom_spec("   "), RomSource::Embedded("demo".into()));
    }

    #[test]
    fn embedded_namespace() {
        assert_eq!(parse_rom_spec("rom:lunar"), RomSource::Embedded("lunar".into()));
        assert_eq!(parse_rom_spec("rom:moon"), RomSource::Embedded("moon".into()));
        assert_eq!(parse_rom_spec(" rom:demo "), RomSource::Embedded("demo".into()));
    }

    #[test]
    fn http_urls() {
        assert_eq!(
            parse_rom_spec("http://example.com/x.rom"),
            RomSource::Url("http://example.com/x.rom".into())
        );
        assert_eq!(
            parse_rom_spec("https://example.com/x.rom"),
            RomSource::Url("https://example.com/x.rom".into())
        );
    }

    #[test]
    fn file_and_bare_paths() {
        assert_eq!(
            parse_rom_spec("file:///tmp/demo.rom"),
            RomSource::Path(PathBuf::from("/tmp/demo.rom"))
        );
        assert_eq!(parse_rom_spec("lunar.rom"), RomSource::Path(PathBuf::from("lunar.rom")));
        assert_eq!(
            parse_rom_spec("./roms/lunar.rom"),
            RomSource::Path(PathBuf::from("./roms/lunar.rom"))
        );
    }

    #[test]
    fn data_uri_payload() {
        assert_eq!(
            parse_rom_spec("data:application/x-tar,hello"),
            RomSource::Data(b"hello".to_vec())
        );
    }
}
