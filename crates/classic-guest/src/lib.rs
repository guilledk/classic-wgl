//! classic-guest: the WASM guest runtime for classic-wgl ROMs.
//!
//! ROM guest code is compiled to `.wasm` and run by the host (the "emulator")
//! against a stable host API — the "console SDK".  This crate provides the
//! runtime abstraction ([`GuestRuntime`]), the wasmi-backed implementation
//! ([`WasmiRuntime`]), the host-side SDK ([`sdk::GuestHost`]) that bridges
//! guest imports to the engine, and the ABI contract ([`abi`]).
//!
//! # Architecture (AGENTS.md "Patterns" 1, 4)
//!
//! - **One table, N generated views.**  Every backend's host imports are
//!   generated from `classic_core::abi_manifest::for_each_host_import!` — the
//!   single declaration of the guest ABI.  Adding an import means adding one
//!   table row (plus its `GuestHost` body), never editing a per-backend list:
//!   native wasmi / wasmtime ([`imports`]), the browser-`WebAssembly` runtime,
//!   the untrusted `Worker` runtime ([`worker_bridge`], whose `worker.js` builds
//!   its stubs from a posted descriptor, so the table index is the op code) and
//!   the Tier-3 worker in `classic-worker`.  Tests assert each backend exposes
//!   exactly its table subset; `cargo xtask check-patterns` rejects
//!   hand-numbered `OP_*` tables.
//! - **Backends split at the crate boundary** (`runtime_wasmtime` /
//!   `runtime_web` / `runtime_worker` behind [`GuestRuntime`]), with the
//!   `Worker` runtime reporting readiness through `GuestRuntime::is_ready`
//!   because a `Worker` only boots once the main thread yields.

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
#[doc(hidden)]
pub mod worker_bridge;

pub use runtime::{GuestError, GuestLimits, GuestRuntime, WasmiRuntime};
#[cfg(not(target_arch = "wasm32"))]
pub use runtime_wasmtime::{CompiledModule, WasmtimeRuntime};
#[cfg(target_arch = "wasm32")]
pub use runtime_web::WebWasmRuntime;
#[cfg(target_arch = "wasm32")]
pub use runtime_worker::WorkerWasmRuntime;

/// A placeholder for the compiled-native-module type on web, where guests are
/// compiled inline by the browser (no off-thread cranelift).  Keeps the
/// `CompiledModules` map type target-independent.
#[cfg(target_arch = "wasm32")]
pub struct CompiledModule {
    _priv: (),
}

/// Compile a guest module off the main thread (native wasmtime).  On web this
/// is a stub: browser `WebAssembly` compiles inline in [`create_runtime`], so
/// an off-thread compile is never requested.
pub fn compile_module(wasm: &[u8], limits: &GuestLimits) -> Result<CompiledModule, GuestError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        CompiledModule::compile(wasm, limits)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (wasm, limits);
        Ok(CompiledModule { _priv: () })
    }
}

/// Instantiate a runtime from a pre-compiled native module (native).  On web
/// this is unreachable — the compiled-modules map is always empty and guests
/// compile inline.
pub fn create_runtime_from_module(
    compiled: &CompiledModule,
    limits: &GuestLimits,
) -> Result<Box<dyn GuestRuntime>, GuestError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        WasmtimeRuntime::from_module(compiled, limits).map(|r| Box::new(r) as Box<dyn GuestRuntime>)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (compiled, limits);
        Err(GuestError::Compile("compiled guest modules are native-only".into()))
    }
}

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
