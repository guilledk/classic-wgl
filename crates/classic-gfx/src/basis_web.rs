//! Web Basis Universal transcoder (P1.0/R2).
//!
//! `basis-universal` (the native codec) cannot link into a
//! `wasm32-unknown-unknown` crate, so the web path uses a separate precompiled
//! transcoder: the three.js `basis_transcoder.{js,wasm}` build (vendored under
//! [`transcoder`], MIT/Apache — see `transcoder/NOTICE`).
//!
//! The Emscripten module is instantiated **synchronously on the main thread**
//! via its `instantiateWasm` hook (`WebAssembly.Module` + `WebAssembly.Instance`
//! are synchronous, unlike `instantiateStreaming`), so the synchronous
//! [`Gfx::add_texture_basis`](crate::Gfx::add_texture_basis) load path can call
//! into it without an async refactor.

use glow::HasContext;
use js_sys::{Function, Reflect, Uint8Array};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use super::compressed::{CompressedFormat, Decoded};

const BASIS_TRANSCODER_JS: &str = include_str!("transcoder/basis_transcoder.js");
const BOOTSTRAP_JS: &str = include_str!("transcoder/bootstrap.js");
const BASIS_TRANSCODER_WASM: &[u8] = include_bytes!("transcoder/basis_transcoder.wasm");

/// `basis_universal` `transcoder_texture_format` values (the three.js r162
/// `TranscoderFormat` enum).  The r162 build exposes 17 targets; notably it
/// does **not** expose ETC2_EAC_R11, so a depth sheet without RGTC support
/// falls back to the raw RGBA8 transcode.
const TF_ETC2_RGBA: u32 = 1;
const TF_BC3_RGBA: u32 = 3;
const TF_BC4_R: u32 = 4;
const TF_BC7_M5: u32 = 7;
const TF_RGBA32: u32 = 13;

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

/// A lazily-initialised handle to the synchronous wasm transcoder.
struct Transcoder {
    transcode_fn: Function,
}

impl Transcoder {
    fn new() -> Result<Self, JsValue> {
        // Evaluate the vendored UMD (defines the top-level `BASIS` factory)
        // followed by the bootstrap, which instantiates it synchronously and
        // exposes `__classicBasisTranscoder` on the global object.
        let glue = format!("{BASIS_TRANSCODER_JS}\n{BOOTSTRAP_JS}");
        js_sys::eval(&glue)?;

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
/// albedo/normal; RGTC → raw for depth, since the r162 wasm lacks ETC2_R11).
pub fn transcode(gl: &glow::Context, bytes: &[u8], format: CompressedFormat) -> Option<Decoded> {
    let caps = web_caps(gl);
    let candidates: &[(bool, u32, u32)] = match format {
        CompressedFormat::Bc7Rgba => &[
            (caps.bptc, TF_BC7_M5, glow::COMPRESSED_RGBA_BPTC_UNORM),
            (caps.s3tc, TF_BC3_RGBA, glow::COMPRESSED_RGBA_S3TC_DXT5_EXT),
            (caps.etc2, TF_ETC2_RGBA, glow::COMPRESSED_RGBA8_ETC2_EAC),
        ],
        CompressedFormat::Bc4R => &[(caps.rgtc, TF_BC4_R, glow::COMPRESSED_RED_RGTC1)],
    };
    let tc = transcoder()?;
    for (supported, target, gl_internal) in candidates {
        if !supported {
            continue;
        }
        if let Some((w, h, data)) = tc.transcode(bytes, *target) {
            return Some(Decoded { internal_format: *gl_internal, width: w, height: h, data });
        }
    }
    None
}

/// Raw RGBA8 transcode (the final fallback).  For a 1-channel depth source the
/// transcoder replicates R into G/B/A, so sampling `.r` stays correct.
pub fn transcode_rgba8(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    transcoder()?.transcode(bytes, TF_RGBA32)
}
