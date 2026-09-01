//! Web Basis Universal transcoder (P1.0/R3).
//!
//! `basis-universal` (the native codec) cannot link into a
//! `wasm32-unknown-unknown` crate, so the web path uses our own precompiled
//! transcoder: an Emscripten build of the Basis Universal transcoder (see
//! [`transcoder`], Apache 2.0 — `transcoder/NOTICE` + `transcoder/build.sh`),
//! exposing the full `transcoder_texture_format` set (including
//! `ETC2_EAC_R11`, closing the depth gap).
//!
//! The standalone wasm module is instantiated **synchronously on the main
//! thread** via its `bootstrap.js` glue (`WebAssembly.Module` +
//! `WebAssembly.Instance`), so the synchronous
//! [`Gfx::add_texture_basis`](crate::Gfx::add_texture_basis) load path can call
//! into it without an async refactor.

use glow::HasContext;
use js_sys::{Function, Reflect, Uint8Array};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use super::compressed::{CompressedFormat, Decoded};

const BOOTSTRAP_JS: &str = include_str!("transcoder/bootstrap.js");
const BASIS_TRANSCODER_WASM: &[u8] = include_bytes!("transcoder/basis_transcoder.wasm");

/// `basis_universal` `transcoder_texture_format` values (the C enum from
/// `basisu_transcoder.h`, matching the native `basis-universal` crate's
/// `TranscoderTextureFormat` discriminants).  The wasm exposes the full enum
/// (any of these values); the engine's fallback chain uses the subset below.
const TF_ETC2_RGBA: u32 = 1;
const TF_BC3_RGBA: u32 = 3;
const TF_BC4_R: u32 = 4;
const TF_BC7_RGBA: u32 = 6;
const TF_RGBA32: u32 = 13;
const TF_ETC2_EAC_R11: u32 = 20;
/// Exposed by the wasm for two-channel (tangent-space) normals; not part of the
/// current engine fallback chain.
#[allow(dead_code)]
const TF_ETC2_EAC_RG11: u32 = 21;

/// The compressed-format capabilities a WebGL 2 context advertises.
struct Caps {
    bptc: bool,
    rgtc: bool,
    s3tc: bool,
    etc2: bool,
}

fn web_caps(gl: &glow::Context) -> Caps {
    let ext = gl.supported_extensions();
    let caps = Caps {
        bptc: ext.contains("EXT_texture_compression_bptc"),
        rgtc: ext.contains("EXT_texture_compression_rgtc"),
        s3tc: ext.contains("WEBGL_compressed_texture_s3tc"),
        etc2: true, // ETC2/EAC is core in WebGL 2
    };
    log::debug!(
        "compressed(web): bptc={} rgtc={} s3tc={} etc2={}",
        caps.bptc,
        caps.rgtc,
        caps.s3tc,
        caps.etc2
    );
    caps
}

/// The compressed-target candidates for a [`CompressedFormat`], mirroring the
/// native `compressed.rs` chain (BPTC → S3TC → ETC2 for albedo/normal; RGTC →
/// ETC2_EAC_R11 for depth).
fn candidates(caps: &Caps, format: CompressedFormat) -> Vec<(bool, u32, u32)> {
    match format {
        CompressedFormat::Bc7Rgba => vec![
            (caps.bptc, TF_BC7_RGBA, glow::COMPRESSED_RGBA_BPTC_UNORM),
            (caps.s3tc, TF_BC3_RGBA, glow::COMPRESSED_RGBA_S3TC_DXT5_EXT),
            (caps.etc2, TF_ETC2_RGBA, glow::COMPRESSED_RGBA8_ETC2_EAC),
        ],
        CompressedFormat::Bc4R => vec![
            (caps.rgtc, TF_BC4_R, glow::COMPRESSED_RED_RGTC1),
            (caps.etc2, TF_ETC2_EAC_R11, glow::COMPRESSED_R11_EAC),
        ],
    }
}

/// A lazily-initialised handle to the synchronous wasm transcoder.
struct Transcoder {
    transcode_fn: Function,
}

impl Transcoder {
    fn new() -> Result<Self, JsValue> {
        // Evaluate our own glue (`bootstrap.js`), which instantiates the
        // standalone wasm synchronously and exposes `__classicBasisTranscoder`
        // on the global object.
        js_sys::eval(BOOTSTRAP_JS)?;

        let global = js_sys::global();
        let obj = Reflect::get(&global, &JsValue::from_str("__classicBasisTranscoder"))?;
        let init_fn: Function = Reflect::get(&obj, &JsValue::from_str("initialize"))?.dyn_into()?;
        let wasm = Uint8Array::from(BASIS_TRANSCODER_WASM);
        init_fn.call1(&JsValue::NULL, &wasm)?;
        let transcode_fn: Function =
            Reflect::get(&obj, &JsValue::from_str("transcode"))?.dyn_into()?;
        Ok(Self { transcode_fn })
    }

    /// Transcode a `.basis` payload to `format`, returning the texture
    /// dimensions and raw output bytes (compressed block data, or RGBA8 when
    /// `format` is [`TF_RGBA32`]).
    fn transcode(&self, bytes: &[u8], format: u32) -> Option<(u32, u32, Vec<u8>)> {
        let arg = Uint8Array::from(bytes);
        let result = self.transcode_fn.call2(&JsValue::NULL, &arg, &JsValue::from(format)).ok()?;
        if result.is_null() || result.is_undefined() {
            return None;
        }
        let width = Reflect::get(&result, &JsValue::from_str("width")).ok()?.as_f64()? as u32;
        let height = Reflect::get(&result, &JsValue::from_str("height")).ok()?.as_f64()? as u32;
        let data: Uint8Array =
            Reflect::get(&result, &JsValue::from_str("data")).ok()?.dyn_into().ok()?;
        Some((width, height, data.to_vec()))
    }
}

fn transcoder() -> Option<&'static Transcoder> {
    static CELL: std::sync::OnceLock<Result<Transcoder, JsValue>> = std::sync::OnceLock::new();
    match CELL.get_or_init(Transcoder::new) {
        Ok(tc) => Some(tc),
        Err(err) => {
            log::warn!("basis transcoder init failed: {err:?}");
            None
        }
    }
}

/// Transcode a `.basis` payload to the best compressed target the context
/// supports, mirroring the native fallback chain (BPTC → S3TC → ETC2 for
/// albedo/normal; RGTC → ETC2_EAC_R11 for depth).
pub fn transcode(gl: &glow::Context, bytes: &[u8], format: CompressedFormat) -> Option<Decoded> {
    let caps = web_caps(gl);
    let tc = transcoder()?;
    for (supported, target, gl_internal) in candidates(&caps, format) {
        if !supported {
            continue;
        }
        if let Some((w, h, data)) = tc.transcode(bytes, target) {
            return Some(Decoded { internal_format: gl_internal, width: w, height: h, data });
        }
    }
    None
}

/// Raw RGBA8 transcode (the final fallback).  For a 1-channel depth source the
/// transcoder replicates R into G/B/A, so sampling `.r` stays correct.
pub fn transcode_rgba8(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    transcoder()?.transcode(bytes, TF_RGBA32)
}
