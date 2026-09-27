//! The web Worker guest runtime (wasm only, untrusted guests).
//!
//! Untrusted guests run on the browser's native `WebAssembly` engine inside a
//! `Worker`, with host imports bridged synchronously to the main thread over a
//! `SharedArrayBuffer` + `Atomics` channel.  Because browser Wasm has no fuel
//! API, the main thread enforces a wall-clock budget per call and
//! `worker.terminate()`s the guest if it exceeds it (surfaced as
//! [`GuestError::FuelExhausted`]).
//!
//! The import surface and request format live in [`crate::worker_bridge`]
//! (generated from the ABI table); this module is only the transport.  The
//! Worker receives the SAB, the guest module bytes and the import descriptor in
//! its first message.  Requests and responses of any size stream through the
//! SAB's byte region in chunks: each non-final request chunk is acknowledged,
//! and each non-final response chunk waits for the Worker's acknowledgement.
//! A guest trap or link error in the Worker is reported back as
//! [`GuestError::Trap`].
//!
//! The SAB layout (offsets and flag indices) must match `worker.js` exactly.
//! The main thread busy-polls (`Atomics::wait` is disallowed on the main
//! thread); only the Worker blocks on `Atomics::wait`.

use std::cell::RefCell;
use std::rc::Rc;

use classic_engine::Engine;
use wasm_bindgen::JsValue;

use crate::runtime::{GuestError, GuestLimits, GuestRuntime};
use crate::sdk::GuestHost;
use crate::worker_bridge::{descriptor_json, WorkerCall, WorkerImports};

// SAB layout (shared with worker.js).
const FLAG_SLOTS: u32 = 16; // Int32Array
const NUM_OFFSET: u32 = FLAG_SLOTS * 4 * 2; // 128: 8-aligned for the Float64Array
const NUM_SLOTS: u32 = 40; // Float64Array: args 0..32, then F_DT, F_RET
const BUF_OFFSET: u32 = NUM_OFFSET + NUM_SLOTS * 8;
const SAB_SIZE: u32 = 1 << 20;
const BUF_BYTES: u32 = SAB_SIZE - BUF_OFFSET;

// Int32 flag indices.
const I_REQ_READY: u32 = 0;
const I_RESP_READY: u32 = 1;
const I_DONE: u32 = 2;
const I_GO: u32 = 3;
const I_COMMAND: u32 = 4;
const I_FAULT: u32 = 5;
const I_REQ_OP: u32 = 6;
const I_REQ_NUM_COUNT: u32 = 7;
const I_MSG: u32 = 8;
const I_CHUNK_LEN: u32 = 9;
const I_TOTAL_LEN: u32 = 10;
const I_READY: u32 = 11;

// Float64 slots.
const F_DT: u32 = 32;
const F_RET: u32 = 33;

// Commands.
const CMD_INIT: i32 = 0;
const CMD_UPDATE: i32 = 1;
const CMD_START: i32 = 2;

// Worker -> main message kinds.
const MSG_REQUEST: i32 = 0;
const MSG_RESPONSE_ACK: i32 = 1;

const WORKER_SRC: &str = include_str!("worker.js");

fn js_err(e: &JsValue) -> String {
    e.as_string().unwrap_or_else(|| format!("{e:?}"))
}

/// A request being received chunk by chunk.
struct PendingRequest {
    op: i32,
    nums: Vec<f64>,
    total: usize,
    payload: Vec<u8>,
}

/// Browser-native [`GuestRuntime`] running an untrusted guest in a Worker.
pub struct WorkerWasmRuntime {
    host: Rc<RefCell<GuestHost>>,
    imports: WorkerImports,
    worker: web_sys::Worker,
    flags: js_sys::Int32Array,
    nums: js_sys::Float64Array,
    buf: js_sys::Uint8Array,
    limits: GuestLimits,
    request: Option<PendingRequest>,
    response: Vec<u8>,
    response_sent: usize,
}

impl WorkerWasmRuntime {
    fn flag_load(&self, idx: u32) -> i32 {
        js_sys::Atomics::load(&self.flags, idx).unwrap_or(0)
    }

    fn flag_store(&self, idx: u32, val: i32) {
        let _ = js_sys::Atomics::store(&self.flags, idx, val);
    }

    /// Signal the Worker that a response (or acknowledgement) is ready.
    fn respond(&self) {
        self.flag_store(I_RESP_READY, 1);
        let _ = js_sys::Atomics::notify(&self.flags, I_RESP_READY);
    }

    /// Service one Worker message: a request chunk or a response-chunk ack.
    fn service(&mut self) {
        let msg = self.flag_load(I_MSG);
        let chunk = (self.flag_load(I_CHUNK_LEN).max(0) as u32).min(BUF_BYTES);
        self.flag_store(I_REQ_READY, 0);

        if msg == MSG_RESPONSE_ACK {
            self.send_response_chunk();
            return;
        }
        debug_assert_eq!(msg, MSG_REQUEST);

        let mut request = self.request.take().unwrap_or_else(|| {
            let count = (self.flag_load(I_REQ_NUM_COUNT).max(0) as u32).min(F_DT);
            PendingRequest {
                op: self.flag_load(I_REQ_OP),
                nums: (0..count).map(|i| self.nums.get_index(i)).collect(),
                total: self.flag_load(I_TOTAL_LEN).max(0) as usize,
                payload: Vec::new(),
            }
        });
        let start = request.payload.len();
        request.payload.resize(start + chunk as usize, 0);
        self.buf.subarray(0, chunk).copy_to(&mut request.payload[start..]);

        if request.payload.len() < request.total && chunk > 0 {
            self.request = Some(request);
            self.respond();
            return;
        }

        let mut call = WorkerCall::new(request.nums, &request.payload);
        let ret = self.imports.dispatch(request.op, &mut self.host.borrow_mut(), &mut call);
        self.nums.set_index(F_RET, ret);
        self.response = call.out.unwrap_or_default();
        self.response_sent = 0;
        self.send_response_chunk();
    }

    /// Stream the next chunk of the current response to the Worker.
    fn send_response_chunk(&mut self) {
        let remaining = &self.response[self.response_sent..];
        let n = remaining.len().min(BUF_BYTES as usize);
        self.buf.subarray(0, n as u32).copy_from(&remaining[..n]);
        self.response_sent += n;
        self.flag_store(I_CHUNK_LEN, n as i32);
        self.flag_store(I_TOTAL_LEN, self.response.len() as i32);
        self.respond();
    }

    /// Run one guest entry point (init/update/start) with the wall-clock
    /// watchdog: service host imports until the worker signals done, or
    /// terminate it on budget exhaustion.
    fn run(&mut self, engine: &mut Engine, cmd: i32, dt: f64) -> Result<(), GuestError> {
        self.host.borrow_mut().set_engine(engine);
        self.nums.set_index(F_DT, dt);
        self.flag_store(I_DONE, 0);
        self.flag_store(I_COMMAND, cmd);
        self.flag_store(I_GO, 1);
        let _ = js_sys::Atomics::notify(&self.flags, I_GO);

        let deadline = js_sys::Date::now() + self.limits.max_frame_millis as f64;
        loop {
            if self.flag_load(I_DONE) != 0 {
                if self.flag_load(I_FAULT) != 0 {
                    let len = (self.flag_load(I_CHUNK_LEN).max(0) as u32).min(BUF_BYTES);
                    let msg =
                        String::from_utf8_lossy(&self.buf.subarray(0, len).to_vec()).into_owned();
                    return Err(GuestError::Trap(msg));
                }
                return Ok(());
            }
            if self.flag_load(I_REQ_READY) != 0 {
                self.service();
                continue;
            }
            if js_sys::Date::now() > deadline {
                self.worker.terminate();
                return Err(GuestError::FuelExhausted);
            }
        }
    }
}

impl GuestRuntime for WorkerWasmRuntime {
    fn new(wasm: &[u8], limits: &GuestLimits) -> Result<Self, GuestError> {
        if !classic_worker::sab_available() {
            return Err(GuestError::Instantiate(
                "SharedArrayBuffer unavailable (needs cross-origin isolation)".into(),
            ));
        }

        let sab = js_sys::SharedArrayBuffer::new(SAB_SIZE);
        let flags = js_sys::Int32Array::new_with_byte_offset_and_length(&sab, 0, FLAG_SLOTS);
        let nums =
            js_sys::Float64Array::new_with_byte_offset_and_length(&sab, NUM_OFFSET, NUM_SLOTS);
        let buf = js_sys::Uint8Array::new_with_byte_offset_and_length(&sab, BUF_OFFSET, BUF_BYTES);

        let worker = classic_worker::spawn_web_worker(WORKER_SRC, None)
            .map_err(|e| GuestError::Instantiate(js_err(&e)))?;

        let init = js_sys::Object::new();
        let set = |key: &str, value: &JsValue| {
            js_sys::Reflect::set(&init, &JsValue::from_str(key), value)
                .map(|_| ())
                .map_err(|e| GuestError::Instantiate(js_err(&e)))
        };
        let module = js_sys::Uint8Array::from(wasm);
        set("sab", &sab)?;
        set("wasm", &module)?;
        set("imports", &JsValue::from_str(&descriptor_json()))?;
        classic_worker::post_transfer(&worker, &init, &[&module.buffer()])
            .map_err(|e| GuestError::Instantiate(js_err(&e)))?;

        Ok(Self {
            host: Rc::new(RefCell::new(GuestHost::new())),
            imports: WorkerImports::new(),
            worker,
            flags,
            nums,
            buf,
            limits: limits.clone(),
            request: None,
            response: Vec::new(),
            response_sent: 0,
        })
    }

    fn init(&mut self, engine: &mut Engine) -> Result<(), GuestError> {
        self.run(engine, CMD_INIT, 0.0)
    }

    fn update(&mut self, engine: &mut Engine, dt: f64) -> Result<(), GuestError> {
        self.run(engine, CMD_UPDATE, dt)
    }

    fn start(&mut self, engine: &mut Engine) -> Result<(), GuestError> {
        self.run(engine, CMD_START, 0.0)
    }

    /// Ready once the `Worker` has booted and tried to instantiate the module
    /// (a link error still counts: it is then reported by the next call).
    fn is_ready(&self) -> bool {
        self.flag_load(I_READY) != 0
    }

    fn set_namespace(&mut self, namespace: &str) {
        self.host.borrow_mut().set_namespace(namespace);
    }
}
