//! Background guest worker (native backend): a second `.wasm` instance running
//! pure guest entry points.
//!
//! Two modes share one API:
//! - **Threaded** (default): the runtime is built once on the creating thread
//!   (so build errors surface synchronously), then moved onto a dedicated
//!   `std::thread` that owns it for its lifetime.  The engine submits a
//!   `Run { entry, arg }` and polls for the result; a `Flush` barrier (the
//!   determinism hook) rides the same FIFO channel as `Run`, so it completes
//!   only after all earlier tasks.
//! - **Sync**: the runtime stays on the calling thread and each entry runs
//!   inline at `spawn_task` time (the `synchronous_workers` fallback used by
//!   the deterministic test/golden harness).

use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;

use classic_core::pathfinder::NavSnapshot;
use wasmtime::{Caller, Engine as WasmtimeEngine, Instance, Linker, Module, Store};

use super::install_worker_imports;
use super::{TaskId, WorkerHost};

enum Command {
    Run { id: TaskId, entry: String, arg: Vec<u8> },
    SetNav(Arc<NavSnapshot>),
    Flush(mpsc::Sender<()>),
    Shutdown,
}

/// A completed task result: the guest's returned bytes, or the error that
/// trapped/panicked while running its entry point.
pub type TaskResult = Result<Vec<u8>, String>;

/// A compiled worker module (`Send + Sync`), the off-main-thread half of worker
/// init.  Compile it on a background thread, then instantiate it on the GL
/// thread with [`GuestWorker::new_compiled`] — which must use the *same* engine
/// that compiled it.
pub struct CompiledWorker {
    pub(crate) engine: WasmtimeEngine,
    pub(crate) module: Module,
}

impl CompiledWorker {
    /// Compile the worker guest from its `.wasm` bytes (native wasmtime).
    /// Off-thread-safe: no GL, no engine references, and the result is `Send`.
    pub fn compile(wasm: &[u8]) -> Result<Self, String> {
        let engine = WasmtimeEngine::new(&wasmtime::Config::new())
            .map_err(|e| format!("worker guest engine: {e}"))?;
        let module =
            Module::new(&engine, wasm).map_err(|e| format!("worker guest compile: {e}"))?;
        Ok(Self { engine, module })
    }
}

/// The engine-facing handle to the background guest worker.
pub struct GuestWorker {
    mode: Mode,
    results: HashMap<TaskId, TaskResult>,
}

enum Mode {
    Threaded { tx: mpsc::Sender<Command>, rx: mpsc::Receiver<(TaskId, TaskResult)> },
    Sync(Runtime),
}

/// The wasmtime runtime pieces (owned by the worker thread, or the calling
/// thread in sync mode).
struct Runtime {
    store: Store<WorkerHost>,
    instance: Instance,
}

impl Runtime {
    /// Run the named guest export (signature `fn entry() -> ()`) against the
    /// current task argument, returning the guest's result bytes.
    fn run(&mut self, entry: &str, arg: Vec<u8>) -> TaskResult {
        self.store.data_mut().set_arg(arg);
        self.store.data_mut().take_result();

        let func = self
            .instance
            .get_typed_func::<(), ()>(&mut self.store, entry)
            .map_err(|e| format!("worker guest missing export '{entry}': {e}"))?;

        func.call(&mut self.store, ())
            .map_err(|e| format!("worker guest '{entry}' trapped: {e}"))?;

        Ok(self.store.data_mut().take_result())
    }
}

impl GuestWorker {
    /// Compile and instantiate the worker guest.  When `synchronous` is true the
    /// runtime runs entries inline on the calling thread; otherwise it runs on a
    /// dedicated thread.
    pub fn new(wasm: &[u8], nav: Arc<NavSnapshot>, synchronous: bool) -> Result<Self, String> {
        let compiled = CompiledWorker::compile(wasm)?;
        Self::new_compiled(&compiled, nav, synchronous)
    }

    /// Instantiate a worker from a module already compiled off-thread.  Uses the
    /// same engine that compiled the module (see [`CompiledWorker`]).  When
    /// `synchronous` is true the runtime runs entries inline on the calling
    /// thread; otherwise it runs on a dedicated thread.
    pub fn new_compiled(
        compiled: &CompiledWorker,
        nav: Arc<NavSnapshot>,
        synchronous: bool,
    ) -> Result<Self, String> {
        if synchronous {
            let runtime = instantiate(&compiled.engine, &compiled.module, Arc::clone(&nav))?;
            return Ok(Self { mode: Mode::Sync(runtime), results: HashMap::new() });
        }

        let runtime = instantiate(&compiled.engine, &compiled.module, Arc::clone(&nav))?;
        let (tx, worker_rx) = mpsc::channel::<Command>();
        let (worker_tx, rx) = mpsc::channel::<(TaskId, TaskResult)>();

        thread::Builder::new()
            .name("classic-worker-guest".to_string())
            .spawn(move || {
                let mut runtime = runtime;
                while let Ok(command) = worker_rx.recv() {
                    match command {
                        Command::Run { id, entry, arg } => {
                            let result = runtime.run(&entry, arg);
                            let _ = worker_tx.send((id, result));
                        }
                        Command::SetNav(nav) => runtime.store.data_mut().set_nav(nav),
                        Command::Flush(ack) => {
                            let _ = ack.send(());
                        }
                        Command::Shutdown => break,
                    }
                }
            })
            .map_err(|e| format!("failed to spawn worker guest thread: {e}"))?;

        Ok(Self { mode: Mode::Threaded { tx, rx }, results: HashMap::new() })
    }

    /// Replace the nav snapshot shared with the worker.
    pub fn set_nav(&mut self, nav: Arc<NavSnapshot>) {
        match &mut self.mode {
            Mode::Threaded { tx, .. } => {
                let _ = tx.send(Command::SetNav(nav));
            }
            Mode::Sync(runtime) => runtime.store.data_mut().set_nav(nav),
        }
    }

    /// Submit a task under a caller-chosen id.  Non-blocking in threaded mode;
    /// runs inline in sync mode.
    pub fn spawn_task(&mut self, id: TaskId, entry: &str, arg: Vec<u8>) {
        match &mut self.mode {
            Mode::Threaded { tx, .. } => {
                let _ = tx.send(Command::Run { id, entry: entry.to_string(), arg });
            }
            Mode::Sync(runtime) => {
                let result = runtime.run(entry, arg);
                self.results.insert(id, result);
            }
        }
    }

    /// Poll a previously submitted task (non-blocking).  `None` while pending.
    pub fn poll_task(&mut self, id: TaskId) -> Option<TaskResult> {
        if let Mode::Threaded { rx, .. } = &self.mode {
            while let Ok((id, result)) = rx.try_recv() {
                self.results.insert(id, result);
            }
        }
        self.results.remove(&id)
    }

    /// Block until every previously submitted task has completed (no-op in sync
    /// mode, where tasks run inline).
    pub fn join(&self) {
        if let Mode::Threaded { tx, .. } = &self.mode {
            let (ack_tx, ack_rx) = mpsc::channel();
            let _ = tx.send(Command::Flush(ack_tx));
            let _ = ack_rx.recv();
        }
    }
}

impl Drop for GuestWorker {
    fn drop(&mut self) {
        if let Mode::Threaded { tx, .. } = &self.mode {
            let _ = tx.send(Command::Shutdown);
        }
    }
}

/// Produce the trap error for a mutating import.
fn trap(name: &str) -> wasmtime::Error {
    wasmtime::Error::msg(format!("mutating host import '{name}' is not allowed in a worker guest"))
}

/// Register the reduced worker import surface (see `install_worker_imports!`).
fn install_imports(linker: &mut Linker<WorkerHost>) -> Result<(), wasmtime::Error> {
    install_worker_imports!(
        linker,
        WorkerHost,
        wasmtime::Error,
        trap,
        read_str,
        read_bytes,
        write_bytes
    )
}

/// Build the wasmtime store + instance for a compiled worker module (the
/// instantiate half of worker init; the cranelift compile already happened in
/// [`CompiledWorker::compile`]).
fn instantiate(
    engine: &WasmtimeEngine,
    module: &Module,
    nav: Arc<NavSnapshot>,
) -> Result<Runtime, String> {
    let mut linker = Linker::<WorkerHost>::new(engine);
    install_imports(&mut linker).map_err(|e| format!("worker guest import link: {e}"))?;

    let mut store = Store::new(engine, WorkerHost::new(nav));
    let instance = linker
        .instantiate(&mut store, module)
        .map_err(|e| format!("worker guest instantiate: {e}"))?;

    Ok(Runtime { store, instance })
}

/// Read a UTF-8 string from the worker guest's linear memory (wasmtime backend).
fn read_str(caller: &mut Caller<'_, WorkerHost>, ptr: i32, len: i32) -> String {
    let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) else {
        return String::new();
    };
    classic_core::abi::read_str_from(mem.data(&*caller), ptr, len)
}

/// Read raw bytes from the worker guest's linear memory (wasmtime backend).
fn read_bytes(caller: &mut Caller<'_, WorkerHost>, ptr: i32, len: i32) -> Vec<u8> {
    let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) else {
        return Vec::new();
    };
    classic_core::abi::read_bytes_from(mem.data(&*caller), ptr, len)
}

/// Write bytes into the worker guest's linear memory (wasmtime backend).
fn write_bytes(caller: &mut Caller<'_, WorkerHost>, ptr: i32, bytes: &[u8]) -> i32 {
    let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) else {
        return -1;
    };
    classic_core::abi::write_bytes_to(mem.data_mut(caller), ptr, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn open_nav() -> Arc<NavSnapshot> {
        Arc::new(NavSnapshot::new(2, 2, vec![1; 4]))
    }

    fn poll_until(worker: &mut GuestWorker, id: TaskId) -> TaskResult {
        for _ in 0..1000 {
            match worker.poll_task(id) {
                Some(result) => return result,
                None => thread::sleep(Duration::from_millis(1)),
            }
        }
        Err("timed out".to_string())
    }

    #[test]
    fn runs_entry_and_returns_result() {
        let wasm = wat::parse_str(
            r#"(module
                (import "env" "task_arg" (func $task_arg (param i32 i32) (result i32)))
                (import "env" "task_return" (func $task_return (param i32 i32)))
                (memory (export "memory") 1)
                (data (i32.const 0) "OK")
                (func (export "entry")
                    (call $task_return (i32.const 0) (i32.const 2))))"#,
        )
        .unwrap();

        let mut worker = GuestWorker::new(&wasm, open_nav(), false).unwrap();
        worker.spawn_task(1, "entry", vec![1, 2, 3]);
        assert_eq!(poll_until(&mut worker, 1), Ok(b"OK".to_vec()));
    }

    #[test]
    fn sync_mode_runs_inline() {
        let wasm = wat::parse_str(
            r#"(module
                (import "env" "task_return" (func $task_return (param i32 i32)))
                (memory (export "memory") 1)
                (data (i32.const 0) "OK")
                (func (export "entry")
                    (call $task_return (i32.const 0) (i32.const 2))))"#,
        )
        .unwrap();

        let mut worker = GuestWorker::new(&wasm, open_nav(), true).unwrap();
        worker.spawn_task(1, "entry", Vec::new());
        assert_eq!(worker.poll_task(1), Some(Ok(b"OK".to_vec())));
    }

    #[test]
    fn field_kernel_roundtrip() {
        let wasm = wat::parse_str(
            r#"(module
                (import "env" "alloc_field" (func $alloc (param i32 i32 i32 i32 i32) (result i32)))
                (import "env" "write_field" (func $write (param i32 i32 i32 i32) (result i32)))
                (import "env" "map_scalar" (func $map_scalar (param i32 i32 i32 f64) (result i32)))
                (import "env" "read_field" (func $read (param i32 i32 i32 i32) (result i32)))
                (import "env" "task_return" (func $task_return (param i32 i32)))
                (memory (export "memory") 1)
                (data (i32.const 0) "f")
                (data (i32.const 16) "\00\00\80\3f\00\00\00\40")
                (func (export "entry")
                    (drop (call $alloc (i32.const 0) (i32.const 1) (i32.const 2) (i32.const 1) (i32.const 0)))
                    (drop (call $write (i32.const 0) (i32.const 1) (i32.const 16) (i32.const 8)))
                    (drop (call $map_scalar (i32.const 2) (i32.const 0) (i32.const 1) (f64.const 2.0)))
                    (drop (call $read (i32.const 0) (i32.const 1) (i32.const 32) (i32.const 8)))
                    (call $task_return (i32.const 32) (i32.const 8))))"#,
        )
        .unwrap();

        let mut worker = GuestWorker::new(&wasm, open_nav(), false).unwrap();
        worker.spawn_task(1, "entry", Vec::new());
        // [1.0, 2.0] * 2.0 = [2.0, 4.0] as little-endian f32.
        let expected = [2.0f32.to_le_bytes(), 4.0f32.to_le_bytes()].concat();
        assert_eq!(poll_until(&mut worker, 1), Ok(expected));
    }

    #[test]
    fn mutating_import_traps() {
        let wasm = wat::parse_str(
            r#"(module
                (import "env" "spawn" (func $spawn (param i32 i32) (result i32)))
                (memory (export "memory") 1)
                (func (export "entry")
                    (drop (call $spawn (i32.const 0) (i32.const 0)))))"#,
        )
        .unwrap();

        let mut worker = GuestWorker::new(&wasm, open_nav(), false).unwrap();
        worker.spawn_task(1, "entry", Vec::new());
        match poll_until(&mut worker, 1) {
            Err(msg) => assert!(msg.contains("trapped"), "unexpected error: {msg}"),
            Ok(_) => panic!("mutating import should have trapped"),
        }
    }

    #[test]
    fn join_barrier_waits_for_inflight() {
        let wasm = wat::parse_str(
            r#"(module
                (import "env" "task_return" (func $task_return (param i32 i32)))
                (memory (export "memory") 1)
                (data (i32.const 0) "Z")
                (func (export "entry")
                    (call $task_return (i32.const 0) (i32.const 1))))"#,
        )
        .unwrap();

        let mut worker = GuestWorker::new(&wasm, open_nav(), false).unwrap();
        worker.spawn_task(1, "entry", Vec::new());
        worker.join();
        assert_eq!(worker.poll_task(1), Some(Ok(b"Z".to_vec())));
    }
}
