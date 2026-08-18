//! ROM archive reading.
//!
//! A [`RomArchive`] opens a self-contained container (zip, tar.gz, or
//! tar.zst), detects its format from magic bytes, and decompresses every
//! entry into an in-memory `BTreeMap<path, bytes>`.  This makes `list()` and
//! `read()` uniform across formats and O(1) on the caller side.
//!
//! Streaming / partial reads for very large terrain archives are explicitly
//! deferred (see the ROM plan) — the whole archive is materialised on open.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use anyhow::Context;

use crate::format::{detect_format, RomFormat};

/// A decompressed, in-memory view of a ROM archive.
#[derive(Clone, Debug, Default)]
pub struct RomArchive {
    files: BTreeMap<String, Vec<u8>>,
}

impl RomArchive {
    /// Open a ROM archive from disk, detecting its container format from the
    /// file's leading magic bytes.
    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let mut file = File::open(path.as_ref())
            .with_context(|| format!("open ROM archive {}", path.as_ref().display()))?;

        let mut magic = [0u8; 4];
        let n = file.read(&mut magic)?;
        file.seek(SeekFrom::Start(0))?;

        let format = detect_format(&magic[..n]).ok_or_else(|| {
            anyhow::anyhow!("unknown ROM archive format: {}", path.as_ref().display())
        })?;
        Self::from_reader(file, format)
    }

    /// Open a ROM archive from a byte buffer, detecting the format from the
    /// leading magic bytes.
    pub fn from_bytes(data: &[u8]) -> anyhow::Result<Self> {
        let format = detect_format(data)
            .ok_or_else(|| anyhow::anyhow!("unknown ROM archive format (no magic match)"))?;
        Self::from_reader(std::io::Cursor::new(data), format)
    }

    /// Decompress a ROM archive from a seekable reader of a known format.
    pub fn from_reader<R: Read + Seek>(reader: R, format: RomFormat) -> anyhow::Result<Self> {
        let files = match format {
            RomFormat::Zip => read_zip(reader)?,
            RomFormat::TarGz => read_tar(flate2::read::GzDecoder::new(reader))?,
            RomFormat::TarZst => read_tar(ruzstd::decoding::StreamingDecoder::new(reader)?)?,
        };
        Ok(Self { files })
    }

    /// The paths of every entry in the archive, sorted.
    pub fn list(&self) -> Vec<&str> {
        self.files.keys().map(|s| s.as_str()).collect()
    }

    /// Read a single entry by its archive-internal path.
    pub fn read(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(|v| v.as_slice())
    }

    /// Read a single entry and interpret it as UTF-8.
    pub fn read_string(&self, path: &str) -> anyhow::Result<String> {
        let bytes =
            self.read(path).ok_or_else(|| anyhow::anyhow!("ROM entry not found: {path}"))?;
        Ok(String::from_utf8(bytes.to_vec())?)
    }

    /// The number of entries in the archive.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// True when the archive holds no entries.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

fn read_zip<R: Read + Seek>(reader: R) -> anyhow::Result<BTreeMap<String, Vec<u8>>> {
    let mut archive = zip::ZipArchive::new(reader).context("open zip archive")?;
    let mut files = BTreeMap::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).context("read zip entry")?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).context("decompress zip entry")?;
        files.insert(name, buf);
    }
    Ok(files)
}

fn read_tar<R: Read>(reader: R) -> anyhow::Result<BTreeMap<String, Vec<u8>>> {
    let mut archive = tar::Archive::new(reader);
    let entries = archive.entries().context("open tar archive")?;
    let mut files = BTreeMap::new();
    for entry in entries {
        let mut entry = entry.context("read tar entry")?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry.path().context("tar entry path")?.into_owned();
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).context("decompress tar entry")?;
        files.insert(path.to_string_lossy().to_string(), buf);
    }
    Ok(files)
}
