//! Archive container format detection.

/// The archive container formats a ROM may be packaged in.
///
/// `tar.xz` is intentionally absent for now: it requires a native `liblzma`
/// (`xz2`), which is not available on all hosts and adds no value to the
/// initial checkpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RomFormat {
    /// `PK..` ZIP container, deflate-compressed entries.
    Zip,
    /// `.tar` inside a gzip stream.
    TarGz,
    /// `.tar` inside a zstd stream.
    TarZst,
}

/// Detect the archive format from the leading magic bytes of a file.
pub fn detect_format(bytes: &[u8]) -> Option<RomFormat> {
    // zstd frames begin `28 b5 2f fd` — check first (most distinctive).
    if bytes.len() >= 4 && bytes[0..4] == [0x28, 0xB5, 0x2F, 0xFD] {
        return Some(RomFormat::TarZst);
    }
    // gzip streams begin `1f 8b`.
    if bytes.len() >= 2 && bytes[0..2] == [0x1F, 0x8B] {
        return Some(RomFormat::TarGz);
    }
    // ZIP local file header `PK\x03\x04`, or empty archive EOCD `PK\x05\x06`.
    if bytes.len() >= 4
        && (bytes[0..4] == [0x50, 0x4B, 0x03, 0x04] || bytes[0..4] == [0x50, 0x4B, 0x05, 0x06])
    {
        return Some(RomFormat::Zip);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_zstd() {
        assert_eq!(detect_format(&[0x28, 0xB5, 0x2F, 0xFD, 0x00]), Some(RomFormat::TarZst));
    }

    #[test]
    fn detects_gzip() {
        assert_eq!(detect_format(&[0x1F, 0x8B, 0x08]), Some(RomFormat::TarGz));
    }

    #[test]
    fn detects_zip() {
        assert_eq!(detect_format(&[0x50, 0x4B, 0x03, 0x04]), Some(RomFormat::Zip));
    }

    #[test]
    fn detects_empty_zip() {
        assert_eq!(detect_format(&[0x50, 0x4B, 0x05, 0x06]), Some(RomFormat::Zip));
    }

    #[test]
    fn rejects_unknown() {
        assert_eq!(detect_format(&[0x00, 0x01, 0x02, 0x03]), None);
    }

    #[test]
    fn rejects_short_input() {
        assert_eq!(detect_format(&[0x1F]), None);
    }
}
