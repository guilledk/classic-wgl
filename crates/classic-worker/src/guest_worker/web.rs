//! Background guest worker (web backend): a second `.wasm` instance running
//! pure guest entry points.
//!
//! Two modes share one API:
//! - **Worker** (default): the guest wasm runs in a dedicated `web_sys::Worker`
//!   (`guest_worker.js`), so heavy entries (e.g. the lunar map generator)
//!   execute off the render thread and on the browser's native wasm JIT.
//! - **Sync**: the wasmi runtime stays on the render thread, owned by a
//!   synchronous [`JobQueue`], and each entry runs inline at `spawn_task` time
//!   (the `synchronous_workers` mode used by the deterministic test/golden
//!   harness).
//!
//! The Worker surfaces only the `task_arg`/`task_return` imports the shipped
//! `lunar-worker` guest uses; the wider reduced import surface (mutating
//! imports trap) is provided by the sync `wasmi` backend and remains the
//! fallback for any guest that needs it.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use classic_core::pathfinder::NavSnapshot;
use wasm_bindgen::prelude::*;
use wasmi::{Caller, Config, Engine as WasmiEngine, Instance, Linker, Module, Store};

use super::install_worker_imports;
use super::{TaskId, WorkerHost};
use crate::jobs::{Job, JobQueue};

const WORKER_SRC: &str = include_str!("guest_worker.js");

/// A completed task result (see the native backend).
pub type TaskResult = Result<Vec<u8>, String>;

/// Placeholder for the compiled-worker type on web, where the worker guest runs
/// browser-native wasm in a `web_sys::Worker` (no off-thread cranelift compile),
/// so the `CompiledModules` payload stays target-independent.
pub struct CompiledWorker {
    _priv: (),
}

/// The engine-facing handle to the background guest worker.
pub struct GuestWorker {
    mode: Mode,
    results: Rc<RefCell<HashMap<TaskId, TaskResult>>>,
}

enum Mode {
    Worker(web_sys::Worker),
    Sync(Box<JobQueue<GuestJob>>),
}

/// One background task: run the guest export `entry` against `arg`.
struct GuestJob {
    entry: String,
    arg: Vec<u8>,
}

impl Job for GuestJob {
    type State = Runtime;
    type Output = TaskResult;

    fn run(self, runtime: &mut Runtime) -> TaskResult {
        runtime.run(&self.entry, self.arg)
    }
}

/// The wasmi runtime pieces (owned here; web is single-threaded).
struct Runtime {
    store: Store<WorkerHost>,
    instance: Instance,
}

impl Runtime {
    fn run(&mut self, entry: &str, arg: Vec<u8>) -> TaskResult {
        self.store.data_mut().set_arg(arg);
        self.store.data_mut().take_result();

        let func = self
            .instance
            .get_typed_func::<(), ()>(&self.store, entry)
            .map_err(|e| format!("worker guest missing export '{entry}': {e}"))?;

        func.call(&mut self.store, ())
            .map_err(|e| format!("worker guest '{entry}' trapped: {e}"))?;

        Ok(self.store.data_mut().take_result())
    }
}

impl GuestWorker {
    /// Compile and instantiate the worker guest.  When `synchronous` is true the
    /// wasmi runtime runs entries inline on the render thread; otherwise the
    /// guest wasm runs in a dedicated `Worker`.
    pub fn new(wasm: &[u8], nav: Arc<NavSnapshot>, synchronous: bool) -> Result<Self, String> {
        let results = Rc::new(RefCell::new(HashMap::new()));

        if synchronous {
            let runtime = build_runtime(wasm, nav)?;
            return Ok(Self {
                mode: Mode::Sync(Box::new(JobQueue::synchronous(runtime))),
                results,
            });
        }

        // Install the result handler.
        let on_message = {
            let results = results.clone();
            Box::new(move |data: JsValue| {
                let id = js_sys::Reflect::get(&data, &JsValue::from_str("id"))
                    .ok()
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0) as TaskId;
                let kind = js_sys::Reflect::get(&data, &JsValue::from_str("type"))
                    .ok()
                    .and_then(|v| v.as_string());
                let result = match kind.as_deref() {
                    Some("result") => {
                        let bytes = js_sys::Reflect::get(&data, &JsValue::from_str("result")).ok();
                        match bytes {
                            Some(b) if b.is_object() => Ok(js_sys::Uint8Array::new(&b).to_vec()),
                            _ => Ok(Vec::new()),
                        }
                    }
                    Some("error") => {
                        let msg = js_sys::Reflect::get(&data, &JsValue::from_str("message"))
                            .ok()
                            .and_then(|v| v.as_string())
                            .unwrap_or_else(|| "worker guest trapped".to_string());
                        Err(msg)
                    }
                    _ => return,
                };
                results.borrow_mut().insert(id, result);
            })
        };
        let worker = crate::spawn_web_worker(WORKER_SRC, Some(on_message))
            .map_err(|e| format!("failed to spawn guest worker: {e:?}"))?;

        // Hand the guest wasm bytes to the worker (it instantiates them and
        // queues any run messages until ready).
        {
            let wasm = js_sys::Uint8Array::from(wasm);
            let init = js_sys::Object::new();
            let _ =
                js_sys::Reflect::set(&init, &JsValue::from_str("type"), &JsValue::from_str("init"));
            let _ = js_sys::Reflect::set(&init, &JsValue::from_str("wasm"), &wasm);
            let _ = worker.post_message(&init);
        }

        Ok(Self { mode: Mode::Worker(worker), results })
    }

    /// Replace the nav snapshot used by the worker.  A no-op in Worker mode —
    /// the reduced surface surfaced there (`task_arg`/`task_return`) does not
    /// touch the nav snapshot.
    pub fn set_nav(&mut self, nav: Arc<NavSnapshot>) {
        if let Mode::Sync(queue) = &mut self.mode {
            queue.update(move |runtime| runtime.store.data_mut().set_nav(nav));
        }
    }

    /// Run a task and buffer its result.  Posts to the Worker (non-blocking) in
    /// Worker mode; runs inline in sync mode.
    pub fn spawn_task(&mut self, id: TaskId, entry: &str, arg: Vec<u8>) {
        match &mut self.mode {
            Mode::Sync(queue) => {
                queue.submit(id, GuestJob { entry: entry.to_string(), arg });
            }
            Mode::Worker(worker) => {
                let arg = js_sys::Uint8Array::from(arg.as_slice());
                let msg = js_sys::Object::new();
                let _ = js_sys::Reflect::set(
                    &msg,
                    &JsValue::from_str("type"),
                    &JsValue::from_str("run"),
                );
                let _ = js_sys::Reflect::set(
                    &msg,
                    &JsValue::from_str("id"),
                    &JsValue::from_f64(id as f64),
                );
                let _ = js_sys::Reflect::set(
                    &msg,
                    &JsValue::from_str("entry"),
                    &JsValue::from_str(entry),
                );
                let _ = js_sys::Reflect::set(&msg, &JsValue::from_str("arg"), &arg);
                let _ = worker.post_message(&msg);
            }
        }
    }

    /// Poll a previously submitted task.  Non-blocking; `None` while the Worker
    /// has not yet delivered a result.
    pub fn poll_task(&mut self, id: TaskId) -> Option<TaskResult> {
        match &mut self.mode {
            Mode::Sync(queue) => queue.poll(id),
            Mode::Worker(_) => self.results.borrow_mut().remove(&id),
        }
    }

    /// Web has no blocking join; determinism is handled by the synchronous
    /// fallback (`synchronous_workers`), not this worker.
    pub fn join(&self) {}
}

impl Drop for GuestWorker {
    fn drop(&mut self) {
        if let Mode::Worker(worker) = &self.mode {
            worker.terminate();
        }
    }
}

/// Produce the trap error for a mutating import.
fn trap(name: &str) -> wasmi::Error {
    wasmi::Error::new(format!("mutating host import '{name}' is not allowed in a worker guest"))
}

/// Register the reduced worker import surface (see `install_worker_imports!`).
fn install_imports(linker: &mut Linker<WorkerHost>) -> Result<(), wasmi::Error> {
    install_worker_imports!(
        linker,
        WorkerHost,
        wasmi::Error,
        trap,
        read_str,
        read_bytes,
        write_bytes
    )
}

/// Build the wasmi store + instance for the worker guest (reduced surface).
fn build_runtime(wasm: &[u8], nav: Arc<NavSnapshot>) -> Result<Runtime, String> {
    let engine = WasmiEngine::new(&Config::default());
    let module = Module::new(&engine, wasm).map_err(|e| format!("worker guest compile: {e}"))?;

    let mut linker = Linker::<WorkerHost>::new(&engine);
    install_imports(&mut linker).map_err(|e| format!("worker guest import link: {e}"))?;

    let mut store = Store::new(&engine, WorkerHost::new(nav));
    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(|e| format!("worker guest instantiate: {e}"))?
        .start(&mut store)
        .map_err(|e| format!("worker guest start: {e}"))?;

    Ok(Runtime { store, instance })
}

/// Read a UTF-8 string from the worker guest's linear memory (wasmi backend).
fn read_str(caller: &mut Caller<'_, WorkerHost>, ptr: i32, len: i32) -> String {
    let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) else {
        return String::new();
    };
    classic_core::abi::read_str_from(mem.data(&*caller), ptr, len)
}

/// Read raw bytes from the worker guest's linear memory (wasmi backend).
fn read_bytes(caller: &mut Caller<'_, WorkerHost>, ptr: i32, len: i32) -> Vec<u8> {
    let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) else {
        return Vec::new();
    };
    classic_core::abi::read_bytes_from(mem.data(&*caller), ptr, len)
}

/// Write bytes into the worker guest's linear memory (wasmi backend).
fn write_bytes(caller: &mut Caller<'_, WorkerHost>, ptr: i32, bytes: &[u8]) -> i32 {
    let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) else {
        return -1;
    };
    classic_core::abi::write_bytes_to(mem.data_mut(caller), ptr, bytes)
}
