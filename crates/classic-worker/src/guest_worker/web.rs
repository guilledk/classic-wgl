//! Background guest worker (web backend): a second `.wasm` instance running
//! pure guest entry points, backed by wasmi.
//!
//! The browser has no `std::thread`, and a real async `Worker`-bridged wasm
//! runtime is not yet implemented.  For now the web
//! backend runs entries **synchronously** on the render thread, so `spawn_task`
//! runs the entry inline and `poll_task` returns the result immediately.  The
//! reduced import surface (mutating imports trap) is identical to native.

use std::collections::HashMap;
use std::sync::Arc;

use classic_core::pathfinder::NavSnapshot;
use wasmi::{Caller, Config, Engine as WasmiEngine, Instance, Linker, Module, Store};

use super::install_worker_imports;
use super::{TaskId, WorkerHost};

/// A completed task result (see the native backend).
pub type TaskResult = Result<Vec<u8>, String>;

/// The engine-facing handle to the background guest worker.
pub struct GuestWorker {
    runtime: Runtime,
    results: HashMap<TaskId, TaskResult>,
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
    /// Compile and instantiate the worker guest.  The `synchronous` flag is
    /// accepted for API parity with the native backend but ignored — the web
    /// backend always runs entries inline (a real async web `Worker` is
    /// not yet implemented).
    pub fn new(wasm: &[u8], nav: Arc<NavSnapshot>, _synchronous: bool) -> Result<Self, String> {
        let runtime = build_runtime(wasm, nav)?;
        Ok(Self { runtime, results: HashMap::new() })
    }

    /// Replace the nav snapshot used by the worker.
    pub fn set_nav(&mut self, nav: Arc<NavSnapshot>) {
        self.runtime.store.data_mut().set_nav(nav);
    }

    /// Run a task synchronously and buffer its result.
    pub fn spawn_task(&mut self, id: TaskId, entry: &str, arg: Vec<u8>) {
        let result = self.runtime.run(entry, arg);
        self.results.insert(id, result);
    }

    /// Poll a previously submitted task.  Synchronous on web, so the result is
    /// always available immediately.
    pub fn poll_task(&mut self, id: TaskId) -> Option<TaskResult> {
        self.results.remove(&id)
    }

    /// No-op on web (synchronous).
    pub fn join(&self) {}
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
