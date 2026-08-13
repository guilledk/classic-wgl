//! classic-guest: the WASM guest runtime for classic-wgl ROMs.
//!
//! ROM guest code is compiled to `.wasm` and run by the host (the "emulator")
//! against a stable host API — the "console SDK".  This crate provides the
//! runtime abstraction ([`GuestRuntime`]), the wasmi-backed implementation
//! ([`WasmiRuntime`]), the host-side SDK ([`sdk::GuestHost`]) that bridges
//! guest imports to the engine, and the ABI contract ([`abi`]).

pub mod abi;
pub mod runtime;
pub mod sdk;

pub use runtime::{GuestError, GuestLimits, GuestRuntime, WasmiRuntime};
