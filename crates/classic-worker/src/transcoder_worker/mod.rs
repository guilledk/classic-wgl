//! Web Basis Universal transcode `Worker` (web only).
//!
//! Runs a Basis Universal transcoder wasm (`classic-gfx` owns and passes in
//! `basis_transcoder.wasm`) in a dedicated `Worker` (`transcoder_worker.js`),
//! so the CPU transcode happens off the main thread.  Each request gets a
//! promise the worker's reply resolves; the GL upload (and the synchronous
//! main-thread fallback) stay in `classic-gfx`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use js_sys::{Function, Reflect, Uint8Array};
use wasm_bindgen::{JsCast, JsValue};

const TRANSCODER_WORKER_JS: &str = include_str!("transcoder_worker.js");

/// A `Worker` running the transcoder wasm, with a per-request promise the
/// worker resolves via `postMessage`.
pub struct TranscoderWorker {
    worker: web_sys::Worker,
    next_id: Rc<Cell<u64>>,
    pending: Rc<RefCell<HashMap<u64, Function>>>,
}

impl TranscoderWorker {
    /// Spawn the worker and hand it the transcoder `wasm` (it instantiates the
    /// module asynchronously and queues any requests that arrive first).
    pub fn new(wasm: &[u8]) -> Result<Self, JsValue> {
        let next_id = Rc::new(Cell::new(0u64));
        let pending: Rc<RefCell<HashMap<u64, Function>>> = Rc::new(RefCell::new(HashMap::new()));

        // Resolve the promise for a completed transcode (keyed by request id).
        let on_message = {
            let pending = pending.clone();
            Box::new(move |data: JsValue| {
                let id = Reflect::get(&data, &JsValue::from_str("id"))
                    .ok()
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0) as u64;
                if let Some(resolve) = pending.borrow_mut().remove(&id) {
                    let _ = resolve.call1(&JsValue::NULL, &data);
                }
            })
        };
        let worker = crate::spawn_web_worker(TRANSCODER_WORKER_JS, Some(on_message))
            .map_err(|e| js_sys::Error::new(&format!("basis worker spawn: {e:?}")))?;

        let wasm = Uint8Array::from(wasm);
        let init = js_sys::Object::new();
        Reflect::set(&init, &JsValue::from_str("type"), &JsValue::from_str("init"))?;
        Reflect::set(&init, &JsValue::from_str("wasm"), &wasm)?;
        crate::post_transfer(&worker, &init, &[&wasm.buffer()])?;

        Ok(Self { worker, next_id, pending })
    }

    /// Enqueue a transcode of `bytes` to the basis_universal
    /// `transcoder_texture_format` `format`, returning the promise the worker
    /// resolves with its reply (read it with [`parse_result`]).
    pub fn request(&self, bytes: &[u8], format: u32) -> Result<js_sys::Promise, JsValue> {
        let id = self.next_id.get();
        self.next_id.set(id + 1);

        let mut resolve = None;
        let promise = js_sys::Promise::new(&mut |res, _rej| resolve = Some(res));
        self.pending.borrow_mut().insert(id, resolve.unwrap());

        let msg = js_sys::Object::new();
        Reflect::set(&msg, &JsValue::from_str("type"), &JsValue::from_str("transcode"))?;
        Reflect::set(&msg, &JsValue::from_str("id"), &JsValue::from_f64(id as f64))?;
        let bytes = Uint8Array::from(bytes);
        Reflect::set(&msg, &JsValue::from_str("bytes"), &bytes)?;
        Reflect::set(&msg, &JsValue::from_str("format"), &JsValue::from(format))?;
        crate::post_transfer(&self.worker, &msg, &[&bytes.buffer()])?;
        Ok(promise)
    }
}

/// Parse a worker reply (`{ ok, width, height, data }`) into the texture
/// dimensions and output bytes; `None` when the transcode failed.
pub fn parse_result(result: &JsValue) -> Option<(u32, u32, Vec<u8>)> {
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
