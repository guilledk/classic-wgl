//! The native host-import linker layer (the "console SDK").
//!
//! The import surface itself is declared once, in the ABI table
//! (`classic_core::abi_manifest`); this macro only binds the wasmi and wasmtime
//! backends to it.  The *bodies* live in the shared [`GuestHost`] (`sdk.rs`);
//! the generated closures marshal arguments in and out of guest linear memory
//! and forward to same-named `GuestHost` methods.  The two backends differ
//! solely in their `Caller`/`Memory` types, so they both expand this one macro
//! (passing their own `read_str`/`write_*` helpers).
//!
//! When adding an import, add its table entry, the `GuestHost` method, the
//! web/worker backends, and `tests/guest.rs` (see `classic-guest` skill §7).
//!
//! [`GuestHost`]: crate::sdk::GuestHost

/// Generate the body of `install_imports` for the wasmi and wasmtime backends.
///
/// `$linker` is the `&mut Linker<Host>` to register into, `$host` the store
/// host type (wasmi's `WasmiHost` or wasmtime's `WasmtimeHost`), and the
/// remaining arguments are the backend-local memory-marshalling helpers
/// (`read_str`, `read_bytes`, `write_str`, `write_bytes`, `write_f64_pair`,
/// `write_f64_triple`).
macro_rules! install_host_imports {
    ($linker:ident, $host:ty, $read_str:path, $read_bytes:path, $write_str:path, $write_bytes:path, $write_f64_pair:path, $write_f64_triple:path) => {{
        classic_core::link_host_imports!(native {
            linker: $linker,
            host: $host,
            module: crate::abi::HOST_MODULE,
            access: [.guest_mut()],
            read_str: $read_str,
            read_bytes: $read_bytes,
            write_str: $write_str,
            write_bytes: $write_bytes,
            write_f64_pair: $write_f64_pair,
            write_f64_triple: $write_f64_triple,
        });
        Ok(())
    }};
}

pub(crate) use install_host_imports;
