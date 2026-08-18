//! The guest runtime: sandboxed execution of ROM guest `.wasm` modules.

use classic_engine::Engine;
use wasmi::{Caller, Config, Engine as WasmEngine, Linker, Module, Store};

use crate::abi;
use crate::sdk::GuestHost;

/// Per-frame guest resource limits.
#[derive(Clone, Debug)]
pub struct GuestLimits {
    /// Fuel (operation) budget per `update` call; enforced when `!trusted`.
    pub fuel_per_frame: u64,
    /// Maximum guest linear-memory size in bytes.
    pub max_memory_bytes: usize,
    /// Trusted guests skip fuel metering and use the fast path.
    pub trusted: bool,
    /// Wall-clock budget (milliseconds) per `update` for the web Worker
    /// backend (browser Wasm has no fuel API); exceeding it terminates the
    /// worker.  Ignored by the wasmi/wasmtime backends.
    pub max_frame_millis: u64,
}

impl Default for GuestLimits {
    fn default() -> Self {
        Self {
            fuel_per_frame: 1_000_000,
            max_memory_bytes: 64 * 1024 * 1024,
            trusted: false,
            max_frame_millis: 50,
        }
    }
}

/// Errors surfaced while compiling or running a guest module.
#[derive(Debug)]
pub enum GuestError {
    Compile(String),
    Instantiate(String),
    MissingExport(String),
    FuelExhausted,
    Trap(String),
}

impl std::fmt::Display for GuestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GuestError::Compile(e) => write!(f, "guest compile failed: {e}"),
            GuestError::Instantiate(e) => write!(f, "guest instantiate failed: {e}"),
            GuestError::MissingExport(e) => write!(f, "guest missing export: {e}"),
            GuestError::FuelExhausted => write!(f, "guest exceeded its fuel budget"),
            GuestError::Trap(e) => write!(f, "guest trapped: {e}"),
        }
    }
}

impl std::error::Error for GuestError {}

/// A loaded, runnable ROM guest.
pub trait GuestRuntime {
    /// Compile and instantiate a guest module from its `.wasm` bytes.
    fn new(wasm: &[u8], limits: &GuestLimits) -> Result<Self, GuestError>
    where
        Self: Sized;

    /// Run the guest's optional `init()` once, before the first frame.  The
    /// default is a no-op for guests that do not export `init`.
    fn init(&mut self, _engine: &mut Engine) -> Result<(), GuestError> {
        Ok(())
    }

    /// Run the guest's `update(dt)` once against the engine.
    fn update(&mut self, engine: &mut Engine, dt: f64) -> Result<(), GuestError>;

    /// Run the guest's optional `start()` once, after the first `update`.  The
    /// default is a no-op for guests that do not export `start`.
    fn start(&mut self, _engine: &mut Engine) -> Result<(), GuestError> {
        Ok(())
    }
}

/// wasmi store data: the shared [`GuestHost`] engine bridge plus wasmi's
/// resource limiter (memory cap).
struct WasmiHost {
    guest: GuestHost,
    limits: wasmi::StoreLimits,
}

impl WasmiHost {
    fn new(limits: wasmi::StoreLimits) -> Self {
        Self { guest: GuestHost::new(), limits }
    }

    pub(crate) fn guest_mut(&mut self) -> &mut GuestHost {
        &mut self.guest
    }

    fn resource_limiter(&mut self) -> &mut dyn wasmi::ResourceLimiter {
        &mut self.limits
    }
}

/// wasmi-backed [`GuestRuntime`] (native and wasm targets).
pub struct WasmiRuntime {
    store: Store<WasmiHost>,
    init: Option<wasmi::TypedFunc<(), ()>>,
    update: wasmi::TypedFunc<(f64,), ()>,
    start: Option<wasmi::TypedFunc<(), ()>>,
    limits: GuestLimits,
}

impl WasmiRuntime {
    fn build_engine(limits: &GuestLimits) -> WasmEngine {
        let mut config = Config::default();
        // Fuel metering is the untrusted path's CPU guard; trusted guests
        // skip it (no per-operation overhead).
        config.consume_fuel(!limits.trusted);
        WasmEngine::new(&config)
    }

    fn install_imports(linker: &mut Linker<WasmiHost>) -> Result<(), wasmi::Error> {
        use crate::imports::install_host_imports;
        install_host_imports!(
            linker,
            WasmiHost,
            read_str,
            write_str,
            write_bytes,
            write_f64_pair,
            write_f64_triple
        )
    }
}

impl GuestRuntime for WasmiRuntime {
    fn new(wasm: &[u8], limits: &GuestLimits) -> Result<Self, GuestError> {
        let engine = Self::build_engine(limits);
        let module = Module::new(&engine, wasm).map_err(|e| GuestError::Compile(e.to_string()))?;

        let store_limits = wasmi::StoreLimitsBuilder::new()
            .memory_size(limits.max_memory_bytes)
            .trap_on_grow_failure(true)
            .build();
        let mut store = Store::new(&engine, WasmiHost::new(store_limits));
        store.limiter(|host: &mut WasmiHost| host.resource_limiter());

        let mut linker = Linker::new(&engine);
        Self::install_imports(&mut linker).map_err(|e| GuestError::Instantiate(e.to_string()))?;

        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|e| GuestError::Instantiate(e.to_string()))?
            .start(&mut store)
            .map_err(|e| GuestError::Instantiate(e.to_string()))?;

        let init = instance.get_typed_func::<(), ()>(&store, abi::INIT_EXPORT).ok();
        let update = instance
            .get_typed_func::<(f64,), ()>(&store, abi::UPDATE_EXPORT)
            .map_err(|_| GuestError::MissingExport(abi::UPDATE_EXPORT.to_string()))?;
        let start = instance.get_typed_func::<(), ()>(&store, abi::START_EXPORT).ok();

        Ok(Self { store, init, update, start, limits: limits.clone() })
    }

    fn init(&mut self, engine: &mut Engine) -> Result<(), GuestError> {
        let Some(init) = self.init else { return Ok(()) };
        self.store.data_mut().guest_mut().set_engine(engine);
        self.set_fuel_budget()?;
        init.call(&mut self.store, ()).map_err(Self::map_call_error)
    }

    fn update(&mut self, engine: &mut Engine, dt: f64) -> Result<(), GuestError> {
        self.store.data_mut().guest_mut().set_engine(engine);
        self.set_fuel_budget()?;
        self.update.call(&mut self.store, (dt,)).map_err(Self::map_call_error)
    }

    fn start(&mut self, engine: &mut Engine) -> Result<(), GuestError> {
        let Some(start) = self.start else { return Ok(()) };
        self.store.data_mut().guest_mut().set_engine(engine);
        self.set_fuel_budget()?;
        start.call(&mut self.store, ()).map_err(Self::map_call_error)
    }
}

impl WasmiRuntime {
    /// Reset the store's fuel budget for the next guest entry point (no-op for
    /// trusted guests).
    fn set_fuel_budget(&mut self) -> Result<(), GuestError> {
        if !self.limits.trusted {
            self.store
                .set_fuel(self.limits.fuel_per_frame)
                .map_err(|e| GuestError::Trap(e.to_string()))?;
        }
        Ok(())
    }

    /// Map a wasmi call error, distinguishing fuel exhaustion from a trap.
    fn map_call_error(e: wasmi::Error) -> GuestError {
        if e.as_trap_code() == Some(wasmi::core::TrapCode::OutOfFuel) {
            GuestError::FuelExhausted
        } else {
            GuestError::Trap(e.to_string())
        }
    }
}

/// Read a UTF-8 string from the guest's linear memory (wasmi backend).
fn read_str(caller: &mut Caller<'_, WasmiHost>, ptr: i32, len: i32) -> String {
    let Some(mem) = caller.get_export(abi::MEMORY_EXPORT).and_then(|e| e.into_memory()) else {
        return String::new();
    };
    abi::read_str_from(mem.data(&*caller), ptr, len)
}

/// Write bytes into the guest's linear memory (wasmi backend).
fn write_bytes(caller: &mut Caller<'_, WasmiHost>, ptr: i32, bytes: &[u8]) -> i32 {
    let Some(mem) = caller.get_export(abi::MEMORY_EXPORT).and_then(|e| e.into_memory()) else {
        return -1;
    };
    abi::write_bytes_to(mem.data_mut(caller), ptr, bytes)
}

/// Write a UTF-8 string into the guest's linear memory (wasmi backend).
fn write_str(caller: &mut Caller<'_, WasmiHost>, ptr: i32, s: &str) -> i32 {
    write_bytes(caller, ptr, s.as_bytes())
}

/// Write two `f64`s into the guest's linear memory (wasmi backend).
fn write_f64_pair(caller: &mut Caller<'_, WasmiHost>, ptr: i32, a: f64, b: f64) -> i32 {
    let Some(mem) = caller.get_export(abi::MEMORY_EXPORT).and_then(|e| e.into_memory()) else {
        return -1;
    };
    abi::write_f64_pair_to(mem.data_mut(caller), ptr, a, b)
}

/// Write three `f64`s into the guest's linear memory (wasmi backend).
fn write_f64_triple(caller: &mut Caller<'_, WasmiHost>, ptr: i32, a: f64, b: f64, c: f64) -> i32 {
    let Some(mem) = caller.get_export(abi::MEMORY_EXPORT).and_then(|e| e.into_memory()) else {
        return -1;
    };
    abi::write_f64_triple_to(mem.data_mut(caller), ptr, a, b, c)
}
