//! GPU-compressed texture storage (Phase 1 — KTX2/Basis Universal).
//!
//! classic-assets emits a Basis Universal `.basis` payload per shared-atlas
//! sheet (encoded by classic-roms' `xtask encode`); at load, the engine
//! transcodes that payload to the GPU's native compressed format and uploads
//! it via `compressed_tex_image_2d`.
//!
//! Native uses the `basis-universal` crate (a C++ codec) and probes the context
//! for the best supported compressed target, preferring (in order):
//!
//!   albedo/normal  → BC7 (BPTC) → BC3 (S3TC DXT5) → ETC2_RGBA
//!   depth          → BC4 (RGTC) → ETC2_EAC_R11
//!
//! with a raw RGBA8 transcode as the final fallback.  The web path uses a
//! separate precompiled transcoder `.wasm` (three.js `basis_transcoder`) — see
//! `basis_web`.

#[cfg(not(target_arch = "wasm32"))]
use glow::HasContext;

/// The GPU transcode target named by a manifest `format` field.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CompressedFormat {
    /// 8 bpp BC7 (BPTC) — albedo + 3-channel world-space normals.
    Bc7Rgba,
    /// 4 bpp BC4 (RGTC single-channel) — grayscale depth maps.
    Bc4R,
}

impl CompressedFormat {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "BC7_RGBA" => Some(Self::Bc7Rgba),
            "BC4_R" => Some(Self::Bc4R),
            _ => None,
        }
    }
}

/// A transcoded texture payload, ready for a `compressed_tex_image_2d` upload.
pub struct Decoded {
    pub internal_format: u32,
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// The compressed-format capabilities a context advertises.
#[cfg(not(target_arch = "wasm32"))]
struct Caps {
    bptc: bool,
    rgtc: bool,
    s3tc: bool,
    etc2: bool,
}

#[cfg(not(target_arch = "wasm32"))]
fn caps(gl: &glow::Context) -> Caps {
    let version = gl.version();
    let desktop = !version.is_embedded;
    let ext = gl.supported_extensions();
    log::debug!(
        "compressed: gl {version:?} desktop={desktop} bptc={} rgtc={} s3tc={} etc2={}",
        ext.contains("GL_ARB_texture_compression_bptc")
            || ext.contains("EXT_texture_compression_bptc"),
        ext.contains("GL_ARB_texture_compression_rgtc")
            || ext.contains("EXT_texture_compression_rgtc"),
        ext.contains("GL_EXT_texture_compression_s3tc")
            || ext.contains("EXT_texture_compression_s3tc"),
        !desktop || version.major >= 3, // ETC2 is core GLES 3.0+ (and desktop 4.3+)
    );
    Caps {
        bptc: (desktop && (version.major > 4 || (version.major == 4 && version.minor >= 2)))
            || ext.contains("GL_ARB_texture_compression_bptc")
            || ext.contains("EXT_texture_compression_bptc"),
        rgtc: (desktop && version.major >= 3)
            || ext.contains("GL_ARB_texture_compression_rgtc")
            || ext.contains("EXT_texture_compression_rgtc"),
        s3tc: ext.contains("GL_EXT_texture_compression_s3tc")
            || ext.contains("EXT_texture_compression_s3tc"),
        // ETC2 is core in GLES 3.0+, and in desktop GL 4.3+ (GL_ARB_ES3_compatibility).
        etc2: !desktop
            || version.major > 4
            || (version.major == 4 && version.minor >= 3)
            || ext.contains("GL_ARB_ES3_compatibility")
            || ext.contains("GL_OES_compressed_ETC2_RGB8_texture"),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn transcode_to(
    bytes: &[u8],
    target: basis_universal::TranscoderTextureFormat,
) -> Option<(u32, u32, Vec<u8>)> {
    use basis_universal::{TranscodeParameters, Transcoder};
    let mut tc = Transcoder::new();
    if !tc.validate_header(bytes) {
        return None;
    }
    tc.prepare_transcoding(bytes).ok()?;
    let desc = tc.image_level_description(bytes, 0, 0)?;
    let data = tc
        .transcode_image_level(
            bytes,
            target,
            TranscodeParameters { image_index: 0, level_index: 0, ..Default::default() },
        )
        .ok()?;
    Some((desc.original_width, desc.original_height, data))
}

/// Transcode a `.basis` payload to the best compressed target the context
/// supports.  Returns `None` when no compressed target is available (the caller
/// falls back to [`transcode_rgba8`]).
#[cfg(not(target_arch = "wasm32"))]
pub fn transcode(gl: &glow::Context, bytes: &[u8], format: CompressedFormat) -> Option<Decoded> {
    use basis_universal::TranscoderTextureFormat as T;
    let caps = caps(gl);
    let candidates: &[(bool, T, u32)] = match format {
        CompressedFormat::Bc7Rgba => &[
            (caps.bptc, T::BC7_RGBA, glow::COMPRESSED_RGBA_BPTC_UNORM),
            (caps.s3tc, T::BC3_RGBA, glow::COMPRESSED_RGBA_S3TC_DXT5_EXT),
            (caps.etc2, T::ETC2_RGBA, glow::COMPRESSED_RGBA8_ETC2_EAC),
        ],
        CompressedFormat::Bc4R => &[
            (caps.rgtc, T::BC4_R, glow::COMPRESSED_RED_RGTC1),
            (caps.etc2, T::ETC2_EAC_R11, glow::COMPRESSED_R11_EAC),
        ],
    };
    for (supported, target, gl_internal) in candidates {
        if !supported {
            continue;
        }
        if let Some((w, h, data)) = transcode_to(bytes, *target) {
            return Some(Decoded { internal_format: *gl_internal, width: w, height: h, data });
        }
    }
    None
}

/// Raw RGBA8 transcode (the final fallback).  For a 1-channel depth source, the
/// basis transcoder replicates R into G/B, so sampling `.r` is still correct.
#[cfg(not(target_arch = "wasm32"))]
pub fn transcode_rgba8(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    use basis_universal::TranscoderTextureFormat as T;
    transcode_to(bytes, T::RGBA32)
}

// Web: the compressed path uses a separate precompiled transcoder `.wasm`
// (P1.0/R2) — the vendored three.js `basis_transcoder` build, instantiated
// synchronously on the main thread (see `basis_web`).
#[cfg(target_arch = "wasm32")]
pub use crate::basis_web::{transcode, transcode_rgba8};
