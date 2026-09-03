//! The wasmtime-backed guest runtime (native only).

use classic_engine::Engine;
use wasmtime::{
    Caller, Config, Engine as WasmtimeEngine, Linker, Module, Store, StoreLimits,
    StoreLimitsBuilder, TypedFunc,
};

use crate::abi;
use crate::runtime::{GuestError, GuestLimits, GuestRuntime};
use crate::sdk::GuestHost;

/// wasmtime store data: the shared [`GuestHost`] engine bridge plus wasmtime's
/// resource limiter (memory cap).
struct WasmtimeHost {
    guest: GuestHost,
    limits: StoreLimits,
}

impl WasmtimeHost {
    fn new(limits: StoreLimits) -> Self {
        Self { guest: GuestHost::new(), limits }
    }

    pub(crate) fn guest_mut(&mut self) -> &mut GuestHost {
        &mut self.guest
    }

    fn resource_limiter(&mut self) -> &mut dyn wasmtime::ResourceLimiter {
        &mut self.limits
    }
}

/// A compiled native guest module (`Send + Sync`), the off-main-thread half of
/// guest init.  Compile it on a background thread, then instantiate it on the
/// GL thread with [`WasmtimeRuntime::from_module`].
pub struct CompiledModule {
    pub(crate) engine: WasmtimeEngine,
    pub(crate) module: Module,
}

impl CompiledModule {
    /// Compile a guest module from its `.wasm` bytes (native wasmtime).
    /// Off-thread-safe: no GL, no engine references, and the result is `Send`.
    pub fn compile(wasm: &[u8], limits: &GuestLimits) -> Result<Self, GuestError> {
        let engine = WasmtimeRuntime::build_engine(limits)?;
        let module = Module::new(&engine, wasm).map_err(|e| GuestError::Compile(e.to_string()))?;
        Ok(Self { engine, module })
    }

    /// Serialize the compiled module for an on-disk cache (native wasmtime).
    /// The caller is responsible for versioning/invalidating the cache image.
    pub fn serialize(&self) -> Result<Vec<u8>, GuestError> {
        self.module.serialize().map_err(|e| GuestError::Compile(e.to_string()))
    }

    /// Rebuild a compiled module from a previously-serialized image, recreating
    /// the engine from `limits` so its config (fuel metering, cranelift
    /// settings) matches the one the module was originally compiled with.
    ///
    /// # Safety
    ///
    /// `serialized` must be a module image produced by [`Self::serialize`] from
    /// a compatible wasmtime build.  Deserializing arbitrary or corrupt bytes is
    /// memory-unsafe.  The boot layer guards this with a cache magic + version
    /// header and only ever feeds it bytes it wrote itself.
    pub fn deserialize(serialized: &[u8], limits: &GuestLimits) -> Result<Self, GuestError> {
        let engine = WasmtimeRuntime::build_engine(limits)?;
        // SAFETY: see the safety note above — `serialized` is a self-written,
        // version-checked module image for a compatible wasmtime build.
        let module = unsafe { Module::deserialize(&engine, serialized) }
            .map_err(|e| GuestError::Compile(e.to_string()))?;
        Ok(Self { engine, module })
    }
}

/// wasmtime-backed [`GuestRuntime`] (native target only).
pub struct WasmtimeRuntime {
    store: Store<WasmtimeHost>,
    init: Option<TypedFunc<(), ()>>,
    update: TypedFunc<(f64,), ()>,
    start: Option<TypedFunc<(), ()>>,
    limits: GuestLimits,
}

impl WasmtimeRuntime {
    fn build_engine(limits: &GuestLimits) -> Result<WasmtimeEngine, GuestError> {
        let mut config = Config::new();
        // Fuel metering is the untrusted path's CPU guard; trusted guests
        // skip it (no per-operation overhead).
        config.consume_fuel(!limits.trusted);
        WasmtimeEngine::new(&config).map_err(|e| GuestError::Instantiate(e.to_string()))
    }

    fn install_imports(linker: &mut Linker<WasmtimeHost>) -> Result<(), wasmtime::Error> {
        use crate::imports::install_host_imports;
        install_host_imports!(
            linker,
            WasmtimeHost,
            read_str,
            read_bytes,
            write_str,
            write_bytes,
            write_f64_pair,
            write_f64_triple
        )
    }
}

impl GuestRuntime for WasmtimeRuntime {
    fn new(wasm: &[u8], limits: &GuestLimits) -> Result<Self, GuestError> {
        let compiled = CompiledModule::compile(wasm, limits)?;
        Self::from_module(&compiled, limits)
    }

    fn init(&mut self, engine: &mut Engine) -> Result<(), GuestError> {
        if self.init.is_none() {
            return Ok(());
        }
        self.store.data_mut().guest_mut().set_engine(engine);
        self.set_fuel_budget()?;
        self.init.as_ref().unwrap().call(&mut self.store, ()).map_err(Self::map_call_error)
    }

    fn update(&mut self, engine: &mut Engine, dt: f64) -> Result<(), GuestError> {
        self.store.data_mut().guest_mut().set_engine(engine);
        self.set_fuel_budget()?;
        self.update.call(&mut self.store, (dt,)).map_err(Self::map_call_error)
    }

    fn start(&mut self, engine: &mut Engine) -> Result<(), GuestError> {
        if self.start.is_none() {
            return Ok(());
        }
        self.store.data_mut().guest_mut().set_engine(engine);
        self.set_fuel_budget()?;
        self.start.as_ref().unwrap().call(&mut self.store, ()).map_err(Self::map_call_error)
    }

    fn set_namespace(&mut self, namespace: &str) {
        self.store.data_mut().guest_mut().set_namespace(namespace);
    }
}

impl WasmtimeRuntime {
    /// Instantiate a runtime from a [`CompiledModule`] already compiled
    /// off-thread.  Runs on the GL/main thread: it builds the store, linker and
    /// instance against the pre-compiled module (the expensive cranelift
    /// compile already happened in [`CompiledModule::compile`]).
    pub fn from_module(
        compiled: &CompiledModule,
        limits: &GuestLimits,
    ) -> Result<Self, GuestError> {
        let store_limits = StoreLimitsBuilder::new()
            .memory_size(limits.max_memory_bytes)
            .trap_on_grow_failure(true)
            .build();
        let mut store = Store::new(&compiled.engine, WasmtimeHost::new(store_limits));
        store.limiter(|host: &mut WasmtimeHost| host.resource_limiter());

        let mut linker = Linker::new(&compiled.engine);
        Self::install_imports(&mut linker).map_err(|e| GuestError::Instantiate(e.to_string()))?;

        let instance = linker
            .instantiate(&mut store, &compiled.module)
            .map_err(|e| GuestError::Instantiate(e.to_string()))?;

        let init = instance.get_typed_func::<(), ()>(&mut store, abi::INIT_EXPORT).ok();
        let update = instance
            .get_typed_func::<(f64,), ()>(&mut store, abi::UPDATE_EXPORT)
            .map_err(|_| GuestError::MissingExport(abi::UPDATE_EXPORT.to_string()))?;
        let start = instance.get_typed_func::<(), ()>(&mut store, abi::START_EXPORT).ok();

        Ok(Self { store, init, update, start, limits: limits.clone() })
    }
}

impl WasmtimeRuntime {
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

    /// Map a wasmtime call error, distinguishing fuel exhaustion from a trap.
    fn map_call_error(e: wasmtime::Error) -> GuestError {
        if e.downcast_ref::<wasmtime::Trap>() == Some(&wasmtime::Trap::OutOfFuel) {
            GuestError::FuelExhausted
        } else {
            GuestError::Trap(e.to_string())
        }
    }
}

/// Read a UTF-8 string from the guest's linear memory (wasmtime backend).
fn read_str(caller: &mut Caller<'_, WasmtimeHost>, ptr: i32, len: i32) -> String {
    let Some(mem) = caller.get_export(abi::MEMORY_EXPORT).and_then(|e| e.into_memory()) else {
        return String::new();
    };
    abi::read_str_from(mem.data(&*caller), ptr, len)
}

/// Read raw bytes from the guest's linear memory (wasmtime backend).
fn read_bytes(caller: &mut Caller<'_, WasmtimeHost>, ptr: i32, len: i32) -> Vec<u8> {
    let Some(mem) = caller.get_export(abi::MEMORY_EXPORT).and_then(|e| e.into_memory()) else {
        return Vec::new();
    };
    abi::read_bytes_from(mem.data(&*caller), ptr, len)
}

/// Write bytes into the guest's linear memory (wasmtime backend).
fn write_bytes(caller: &mut Caller<'_, WasmtimeHost>, ptr: i32, bytes: &[u8]) -> i32 {
    let Some(mem) = caller.get_export(abi::MEMORY_EXPORT).and_then(|e| e.into_memory()) else {
        return -1;
    };
    abi::write_bytes_to(mem.data_mut(caller), ptr, bytes)
}

/// Write a UTF-8 string into the guest's linear memory (wasmtime backend).
fn write_str(caller: &mut Caller<'_, WasmtimeHost>, ptr: i32, s: &str) -> i32 {
    write_bytes(caller, ptr, s.as_bytes())
}

/// Write two `f64`s into the guest's linear memory (wasmtime backend).
fn write_f64_pair(caller: &mut Caller<'_, WasmtimeHost>, ptr: i32, a: f64, b: f64) -> i32 {
    let Some(mem) = caller.get_export(abi::MEMORY_EXPORT).and_then(|e| e.into_memory()) else {
        return -1;
    };
    abi::write_f64_pair_to(mem.data_mut(caller), ptr, a, b)
}

/// Write three `f64`s into the guest's linear memory (wasmtime backend).
fn write_f64_triple(
    caller: &mut Caller<'_, WasmtimeHost>,
    ptr: i32,
    a: f64,
    b: f64,
    c: f64,
) -> i32 {
    let Some(mem) = caller.get_export(abi::MEMORY_EXPORT).and_then(|e| e.into_memory()) else {
        return -1;
    };
    abi::write_f64_triple_to(mem.data_mut(caller), ptr, a, b, c)
}
