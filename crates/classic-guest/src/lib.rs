//! classic-guest: the WASM guest runtime for classic-wgl ROMs.
//!
//! ROM guest code is compiled to `.wasm` and run by the host (the "emulator")
//! against a stable host API — the "console SDK".  This crate provides the
//! runtime abstraction ([`GuestRuntime`]), the wasmi-backed implementation
//! ([`WasmiRuntime`]), the host-side SDK ([`sdk::GuestHost`]) that bridges
//! guest imports to the engine, and the ABI contract ([`abi`]).

pub mod abi;
pub mod imports;
pub mod runtime;
#[cfg(not(target_arch = "wasm32"))]
mod runtime_wasmtime;
#[cfg(target_arch = "wasm32")]
mod runtime_web;
#[cfg(target_arch = "wasm32")]
mod runtime_worker;
pub mod sdk;

pub use runtime::{GuestError, GuestLimits, GuestRuntime, WasmiRuntime};
#[cfg(not(target_arch = "wasm32"))]
pub use runtime_wasmtime::WasmtimeRuntime;
#[cfg(target_arch = "wasm32")]
pub use runtime_web::WebWasmRuntime;
#[cfg(target_arch = "wasm32")]
pub use runtime_worker::WorkerWasmRuntime;

/// Create the best guest runtime available for the current target: wasmtime on
/// native (near-native speed, fuel + memory limits); on wasm, browser-native
/// `WebAssembly` for trusted guests (no fuel API) and a `Worker`-isolated
/// browser-native runtime for untrusted guests (terminate watchdog), falling
/// back to wasmi when `SharedArrayBuffer` is unavailable.
pub fn create_runtime(
    wasm: &[u8],
    limits: &GuestLimits,
) -> Result<Box<dyn GuestRuntime>, GuestError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        WasmtimeRuntime::new(wasm, limits).map(|r| Box::new(r) as Box<dyn GuestRuntime>)
    }
    #[cfg(target_arch = "wasm32")]
    {
        if limits.trusted {
            return WebWasmRuntime::new(wasm, limits).map(|r| Box::new(r) as Box<dyn GuestRuntime>);
        }
        match WorkerWasmRuntime::new(wasm, limits) {
            Ok(rt) => Ok(Box::new(rt)),
            Err(_) => WasmiRuntime::new(wasm, limits).map(|r| Box::new(r) as Box<dyn GuestRuntime>),
        }
    }
}
