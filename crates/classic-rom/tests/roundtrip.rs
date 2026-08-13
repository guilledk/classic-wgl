//! Round-trip tests for `RomArchive` across the three supported container
//! formats.  Zip and tar.gz fixtures are written in-process; the tar.zst
//! fixture is a pre-compressed byte blob (ruzstd is decode-only) decoded from
//! an embedded base64 constant.

use std::io::{Cursor, Write};

use classic_rom::{RomArchive, RomFormat};

fn sample_files() -> Vec<(&'static str, &'static [u8])> {
    vec![
        ("manifest.json", b"{\"format_version\":1}".as_slice()),
        ("res/textures/a.png", b"\x89PNG-fake".as_slice()),
        ("scripts/main.rhai", b"fn update(ctx) {}".as_slice()),
    ]
}

fn build_zip() -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, data) in sample_files() {
        writer.start_file(name, opts).unwrap();
        writer.write_all(data).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn build_tar_gz() -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let gz = flate2::write::GzEncoder::new(&mut buf, flate2::Compression::default());
        let mut tar = tar::Builder::new(gz);
        for (name, data) in sample_files() {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, name, data).unwrap();
        }
        tar.finish().unwrap();
    }
    buf
}

// `tar -cf - manifest.txt state.txt sub/n.txt | zstd -q` fixture with three
// small entries.  See the `classic-rom` crate for why this is embedded.
const ZSTD_FIXTURE_B64: &str = "KLUv/QRYpQQA0gYWGaCnOQa2CKqJuNn/Guh/v9JLdpNMFzXCMQW5mss4roCgpU0I3qHnplUXiGIJoiRKZcHKUpsJLoWl1eYXox9390Fk0KAx0PncHeH99znH/3sTC1jNgWGlQxQA/YvSAQhg3v+sajKgIkCYBynBG+T5C5WMAXUABA+QyZAA5EmDB+p3AqoAEQVAGoANDDvzqmkCX8cjZYo=";

fn zstd_fixture() -> Vec<u8> {
    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, ZSTD_FIXTURE_B64).unwrap()
}

fn assert_sample_contents(archive: &RomArchive) {
    for (name, data) in sample_files() {
        assert_eq!(archive.read(name), Some(data), "entry {name} round-trips");
    }
}

#[test]
fn zip_roundtrip() {
    let bytes = build_zip();
    let archive = RomArchive::from_bytes(&bytes).unwrap();
    assert_eq!(archive.len(), 3);
    assert_sample_contents(&archive);
}

#[test]
fn tar_gz_roundtrip() {
    let bytes = build_tar_gz();
    let archive = RomArchive::from_bytes(&bytes).unwrap();
    assert_eq!(archive.len(), 3);
    assert_sample_contents(&archive);
}

#[test]
fn tar_zst_roundtrip() {
    let bytes = zstd_fixture();
    let archive = RomArchive::from_bytes(&bytes).unwrap();
    assert_eq!(archive.len(), 3);
    assert_eq!(archive.read("manifest.txt"), Some("hello world\n".as_bytes()));
    assert_eq!(archive.read("state.txt"), Some("data\n".as_bytes()));
    assert_eq!(archive.read("sub/n.txt"), Some("nested\n".as_bytes()));
}

#[test]
fn list_is_sorted_and_complete() {
    let archive = RomArchive::from_bytes(&build_tar_gz()).unwrap();
    let names = archive.list();
    assert_eq!(names, vec!["manifest.json", "res/textures/a.png", "scripts/main.rhai"]);
}

#[test]
fn read_missing_entry_returns_none() {
    let archive = RomArchive::from_bytes(&build_zip()).unwrap();
    assert!(archive.read("does/not/exist").is_none());
}

#[test]
fn from_reader_with_explicit_format() {
    let bytes = build_tar_gz();
    let archive = RomArchive::from_reader(Cursor::new(bytes), RomFormat::TarGz).unwrap();
    assert_sample_contents(&archive);
}

#[test]
fn unknown_format_is_rejected() {
    let err = RomArchive::from_bytes(b"this is not an archive at all").unwrap_err();
    assert!(err.to_string().contains("unknown ROM archive format"));
}
