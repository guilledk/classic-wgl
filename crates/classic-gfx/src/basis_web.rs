//! Web Basis Universal transcoder (P1.0/R3).
//!
//! `basis-universal` (the native codec) cannot link into a
//! `wasm32-unknown-unknown` crate, so the web path uses our own precompiled
//! transcoder: an Emscripten build of the Basis Universal transcoder (see
//! [`transcoder`], Apache 2.0 — `transcoder/NOTICE` + `transcoder/build.sh`),
//! exposing the full `transcoder_texture_format` set (including
//! `ETC2_EAC_R11`, closing the depth gap).
//!
//! There are two backends:
//! - **Worker** (default): the transcode runs in a dedicated web `Worker`
//!   (`transcoder_worker.js`), with the wasm instantiated asynchronously; the
//!   main thread only uploads the decoded payload.
//! - **Sync** (fallback): the wasm is instantiated synchronously on the main
//!   thread (`WebAssembly.Module` + `WebAssembly.Instance` via `bootstrap.js`),
//!   used when the worker cannot start.

use glow::HasContext;
use js_sys::{Function, Reflect, Uint8Array};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use super::compressed::{CompressedFormat, Decoded};

const BOOTSTRAP_JS: &str = include_str!("transcoder/bootstrap.js");
const TRANSCODER_WORKER_JS: &str = include_str!("transcoder/transcoder_worker.js");
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

/// A lazily-initialised handle to the synchronous wasm transcoder (the
/// main-thread fallback).
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

/// The web-Worker backend (the fast path): a `Worker` running the wasm, with a
/// per-request promise the worker resolves via `postMessage`.
struct TranscoderWorker {
    worker: web_sys::Worker,
    next_id: Rc<Cell<u64>>,
    pending: Rc<RefCell<HashMap<u64, Function>>>,
}

impl TranscoderWorker {
    fn new(wasm: &[u8]) -> Result<Self, JsValue> {
        let next_id = Rc::new(Cell::new(0u64));
        let pending: Rc<RefCell<HashMap<u64, Function>>> = Rc::new(RefCell::new(HashMap::new()));

        // Build the worker from an inline source Blob (mirrors the pathfinder /
        // guest worker pattern in `classic-worker`).
        let blob_parts = js_sys::Array::of1(&JsValue::from_str(TRANSCODER_WORKER_JS));
        let blob = web_sys::Blob::new_with_str_sequence(blob_parts.as_ref())
            .map_err(|e| js_sys::Error::new(&format!("basis worker blob: {e:?}")))?;
        let url = web_sys::Url::create_object_url_with_blob(&blob)
            .map_err(|e| js_sys::Error::new(&format!("basis worker url: {e:?}")))?;
        let worker = web_sys::Worker::new(&url)
            .map_err(|e| js_sys::Error::new(&format!("basis worker spawn: {e:?}")))?;

        // Resolve the promise for a completed transcode (keyed by request id).
        {
            let pending = pending.clone();
            let onmessage = Closure::wrap(Box::new(move |event: JsValue| {
                let data = js_sys::Reflect::get(&event, &JsValue::from_str("data"))
                    .unwrap_or(JsValue::NULL);
                let id = js_sys::Reflect::get(&data, &JsValue::from_str("id"))
                    .ok()
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0) as u64;
                if let Some(resolve) = pending.borrow_mut().remove(&id) {
                    let _ = resolve.call1(&JsValue::NULL, &data);
                }
            }) as Box<dyn FnMut(JsValue)>);
            worker.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
            onmessage.forget();
        }

        // Hand the wasm bytes to the worker (it instantiates them async and
        // queues any transcode messages until ready).
        {
            let wasm = js_sys::Uint8Array::from(wasm);
            let init = js_sys::Object::new();
            Reflect::set(&init, &JsValue::from_str("type"), &JsValue::from_str("init"))?;
            Reflect::set(&init, &JsValue::from_str("wasm"), &wasm)?;
            worker.post_message(&init)?;
        }

        Ok(Self { worker, next_id, pending })
    }

    /// Enqueue a transcode and return the promise the worker will resolve.
    fn request(&self, bytes: &[u8], format: u32) -> Result<js_sys::Promise, JsValue> {
        let id = self.next_id.get();
        self.next_id.set(id + 1);

        let mut resolve = None;
        let promise = js_sys::Promise::new(&mut |res, _rej| resolve = Some(res));

        self.pending.borrow_mut().insert(id, resolve.unwrap());

        let msg = js_sys::Object::new();
        Reflect::set(&msg, &JsValue::from_str("type"), &JsValue::from_str("transcode"))?;
        Reflect::set(&msg, &JsValue::from_str("id"), &JsValue::from_f64(id as f64))?;
        Reflect::set(&msg, &JsValue::from_str("bytes"), &Uint8Array::from(bytes))?;
        Reflect::set(&msg, &JsValue::from_str("format"), &JsValue::from(format))?;
        self.worker.post_message(&msg)?;
        Ok(promise)
    }
}

thread_local! {
    static WORKER: RefCell<Option<TranscoderWorker>> = RefCell::new(None);
}

/// Lazily spawn (once) the transcode worker and run `f` against it.
fn with_worker<R>(f: impl FnOnce(&TranscoderWorker) -> R) -> Option<R> {
    WORKER.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            match TranscoderWorker::new(BASIS_TRANSCODER_WASM) {
                Ok(worker) => *slot = Some(worker),
                Err(err) => {
                    log::warn!("basis transcode worker unavailable: {err:?}");
                    return None;
                }
            }
        }
        slot.as_ref().map(f)
    })
}

/// Parse a worker `result` message (`{ ok, width, height, data }`).
fn parse_result(result: &JsValue) -> Option<(u32, u32, Vec<u8>)> {
    let ok = Reflect::get(result, &JsValue::from_str("ok")).ok()?.as_bool()?;
    if !ok {
        return None;
    }
    let width = Reflect::get(result, &JsValue::from_str("width")).ok()?.as_f64()? as u32;
    let height = Reflect::get(result, &JsValue::from_str("height")).ok()?.as_f64()? as u32;
    let data: Uint8Array =
        Reflect::get(result, &JsValue::from_str("data")).ok()?.dyn_into().ok()?;
    Some((width, height, data.to_vec()))
}

/// Transcode to `format` via the worker, falling back to the synchronous
/// main-thread transcoder when the worker cannot start (or a request errors).
async fn transcode_target(bytes: &[u8], format: u32) -> Option<(u32, u32, Vec<u8>)> {
    if let Some(promise) = with_worker(|worker| worker.request(bytes, format)).and_then(|r| r.ok())
    {
        if let Ok(result) = wasm_bindgen_futures::JsFuture::from(promise).await {
            return parse_result(&result);
        }
    }
    // Worker path failed — fall back to the synchronous main-thread transcoder.
    transcoder()?.transcode(bytes, format)
}

/// Transcode a `.basis` payload to the best compressed target the context
/// supports, mirroring the native fallback chain.  Uses the worker (async);
/// the caller awaits before uploading.
pub async fn transcode_async(
    gl: &glow::Context,
    bytes: &[u8],
    format: CompressedFormat,
) -> Option<Decoded> {
    let caps = web_caps(gl);
    for (supported, target, gl_internal) in candidates(&caps, format) {
        if !supported {
            continue;
        }
        if let Some((w, h, data)) = transcode_target(bytes, target).await {
            return Some(Decoded { internal_format: gl_internal, width: w, height: h, data });
        }
    }
    None
}

/// Raw RGBA8 transcode via the worker (the final fallback).  For a 1-channel
/// depth source the transcoder replicates R into G/B/A, so sampling `.r` stays
/// correct.
pub async fn transcode_rgba8_async(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    transcode_target(bytes, TF_RGBA32).await
}

/// Synchronous transcode (main-thread fallback; used when the worker cannot
/// start, and by the deterministic/sync load path).
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

/// Raw RGBA8 transcode (synchronous fallback).
pub fn transcode_rgba8(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    transcoder()?.transcode(bytes, TF_RGBA32)
}
