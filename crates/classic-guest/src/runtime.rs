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
}

impl Default for GuestLimits {
    fn default() -> Self {
        Self { fuel_per_frame: 1_000_000, max_memory_bytes: 64 * 1024 * 1024, trusted: false }
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

    /// Run the guest's `update(dt)` once against the engine.
    fn update(&mut self, engine: &mut Engine, dt: f64) -> Result<(), GuestError>;
}

/// wasmi-backed [`GuestRuntime`] (native and wasm targets).
pub struct WasmiRuntime {
    store: Store<GuestHost>,
    update: wasmi::TypedFunc<(f64,), ()>,
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

    fn install_imports(linker: &mut Linker<GuestHost>) -> Result<(), wasmi::Error> {
        let m = abi::HOST_MODULE;

        linker.func_wrap(m, "log", |mut caller: Caller<'_, GuestHost>, ptr: i32, len: i32| {
            let msg = abi::read_str(&caller, ptr, len);
            caller.data_mut().log(&msg);
        })?;

        linker.func_wrap(
            m,
            "spawn",
            |mut caller: Caller<'_, GuestHost>, ptr: i32, len: i32| -> i32 {
                let name = abi::read_str(&caller, ptr, len);
                caller.data_mut().spawn(&name)
            },
        )?;

        linker.func_wrap(
            m,
            "despawn",
            |mut caller: Caller<'_, GuestHost>, ptr: i32, len: i32| -> i32 {
                let name = abi::read_str(&caller, ptr, len);
                caller.data_mut().despawn(&name)
            },
        )?;

        linker.func_wrap(
            m,
            "has",
            |mut caller: Caller<'_, GuestHost>, ptr: i32, len: i32| -> i32 {
                let name = abi::read_str(&caller, ptr, len);
                caller.data_mut().has(&name)
            },
        )?;

        linker.func_wrap(
            m,
            "names",
            |mut caller: Caller<'_, GuestHost>, out_ptr: i32, out_cap: i32| -> i32 {
                let json = caller.data_mut().names();
                if out_cap < json.len() as i32 {
                    return -1;
                }
                abi::write_str(&mut caller, out_ptr, &json)
            },
        )?;

        linker.func_wrap(
            m,
            "get",
            |mut caller: Caller<'_, GuestHost>,
             ptr: i32,
             len: i32,
             out_ptr: i32,
             out_cap: i32|
             -> i32 {
                let name = abi::read_str(&caller, ptr, len);
                let json = caller.data_mut().get(&name);
                if out_cap < json.len() as i32 {
                    return -1;
                }
                abi::write_str(&mut caller, out_ptr, &json)
            },
        )?;

        linker.func_wrap(
            m,
            "get_comp",
            |mut caller: Caller<'_, GuestHost>,
             ptr: i32,
             len: i32,
             comp_ptr: i32,
             comp_len: i32,
             out_ptr: i32,
             out_cap: i32|
             -> i32 {
                let name = abi::read_str(&caller, ptr, len);
                let comp = abi::read_str(&caller, comp_ptr, comp_len);
                let json = caller.data_mut().get_comp(&name, &comp);
                if out_cap < json.len() as i32 {
                    return -1;
                }
                abi::write_str(&mut caller, out_ptr, &json)
            },
        )?;

        linker.func_wrap(
            m,
            "set",
            |mut caller: Caller<'_, GuestHost>,
             ptr: i32,
             len: i32,
             json_ptr: i32,
             json_len: i32|
             -> i32 {
                let name = abi::read_str(&caller, ptr, len);
                let json = abi::read_str(&caller, json_ptr, json_len);
                caller.data_mut().set(&name, &json)
            },
        )?;

        linker.func_wrap(
            m,
            "set_comp",
            |mut caller: Caller<'_, GuestHost>,
             ptr: i32,
             len: i32,
             comp_ptr: i32,
             comp_len: i32,
             json_ptr: i32,
             json_len: i32|
             -> i32 {
                let name = abi::read_str(&caller, ptr, len);
                let comp = abi::read_str(&caller, comp_ptr, comp_len);
                let json = abi::read_str(&caller, json_ptr, json_len);
                caller.data_mut().set_comp(&name, &comp, &json)
            },
        )?;

        linker.func_wrap(
            m,
            "set_pos",
            |mut caller: Caller<'_, GuestHost>, ptr: i32, len: i32, x: f64, y: f64| -> i32 {
                let name = abi::read_str(&caller, ptr, len);
                caller.data_mut().set_pos(&name, x, y)
            },
        )?;

        linker.func_wrap(
            m,
            "get_pos",
            |mut caller: Caller<'_, GuestHost>, ptr: i32, len: i32, out_ptr: i32| -> i32 {
                let name = abi::read_str(&caller, ptr, len);
                let Some((x, y)) = caller.data_mut().get_pos(&name) else {
                    return 0;
                };
                abi::write_f64_pair(&mut caller, out_ptr, x, y);
                1
            },
        )?;

        linker.func_wrap(m, "mouse", |mut caller: Caller<'_, GuestHost>, out_ptr: i32| -> i32 {
            let (x, y) = caller.data_mut().mouse();
            abi::write_f64_pair(&mut caller, out_ptr, x, y);
            1
        })?;

        linker.func_wrap(m, "delta", |mut caller: Caller<'_, GuestHost>| -> f64 {
            caller.data_mut().delta()
        })?;

        linker.func_wrap(m, "elapsed", |mut caller: Caller<'_, GuestHost>| -> f64 {
            caller.data_mut().elapsed()
        })?;

        linker.func_wrap(
            m,
            "was_pressed",
            |mut caller: Caller<'_, GuestHost>, btn: i32| -> i32 {
                caller.data_mut().was_pressed(btn)
            },
        )?;

        linker.func_wrap(
            m,
            "key_down",
            |mut caller: Caller<'_, GuestHost>, ptr: i32, len: i32| -> i32 {
                let key = abi::read_str(&caller, ptr, len);
                caller.data_mut().key_down(&key)
            },
        )?;

        Ok(())
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
        let mut store = Store::new(&engine, GuestHost::new(store_limits));
        store.limiter(|host: &mut GuestHost| host.resource_limiter());

        let mut linker = Linker::new(&engine);
        Self::install_imports(&mut linker).map_err(|e| GuestError::Instantiate(e.to_string()))?;

        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|e| GuestError::Instantiate(e.to_string()))?
            .start(&mut store)
            .map_err(|e| GuestError::Instantiate(e.to_string()))?;

        let update = instance
            .get_typed_func::<(f64,), ()>(&store, abi::UPDATE_EXPORT)
            .map_err(|_| GuestError::MissingExport(abi::UPDATE_EXPORT.to_string()))?;

        Ok(Self { store, update, limits: limits.clone() })
    }

    fn update(&mut self, engine: &mut Engine, dt: f64) -> Result<(), GuestError> {
        self.store.data_mut().set_engine(engine);
        if !self.limits.trusted {
            self.store
                .set_fuel(self.limits.fuel_per_frame)
                .map_err(|e| GuestError::Trap(e.to_string()))?;
        }
        self.update.call(&mut self.store, (dt,)).map_err(|e| {
            if e.as_trap_code() == Some(wasmi::core::TrapCode::OutOfFuel) {
                GuestError::FuelExhausted
            } else {
                GuestError::Trap(e.to_string())
            }
        })
    }
}
