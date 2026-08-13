//! The guest ABI: the stable contract between a ROM guest module and the host.
//!
//! Guests import host functions from the module named [`HOST_MODULE`] and
//! export `fn update(dt: f64) -> ()` (plus an optional `init`).  Strings cross
//! the boundary as `(ptr, len)` pairs into guest linear memory; functions that
//! return a byte slice write into a caller-provided output buffer and return
//! the number of bytes written (`-1` if the buffer was too small).

use wasmi::Caller;

use crate::sdk::GuestHost;

/// The WASM module name under which host imports are defined.
pub const HOST_MODULE: &str = "env";

/// The guest export invoked once per frame.
pub const UPDATE_EXPORT: &str = "update";

/// The name of the guest's linear memory export.
pub const MEMORY_EXPORT: &str = "memory";

/// Read `len` bytes from the guest's linear memory at `ptr`.
pub fn read_bytes(caller: &Caller<'_, GuestHost>, ptr: i32, len: i32) -> Vec<u8> {
    let Some(mem) = caller.get_export(MEMORY_EXPORT).and_then(|e| e.into_memory()) else {
        return Vec::new();
    };
    let data = mem.data(caller);
    let start = ptr.max(0) as usize;
    let end = (start + len.max(0) as usize).min(data.len());
    data.get(start..end).map(|s| s.to_vec()).unwrap_or_default()
}

/// Read a UTF-8 string from the guest's linear memory (lossy on invalid UTF-8).
pub fn read_str(caller: &Caller<'_, GuestHost>, ptr: i32, len: i32) -> String {
    String::from_utf8_lossy(&read_bytes(caller, ptr, len)).into_owned()
}

/// Write bytes into the guest's linear memory at `ptr`, returning the number of
/// bytes written (`-1` if the buffer overruns guest memory).
pub fn write_bytes(caller: &mut Caller<'_, GuestHost>, ptr: i32, bytes: &[u8]) -> i32 {
    let Some(mem) = caller.get_export(MEMORY_EXPORT).and_then(|e| e.into_memory()) else {
        return -1;
    };
    let data = mem.data_mut(caller);
    let start = ptr.max(0) as usize;
    if start + bytes.len() > data.len() {
        return -1;
    }
    data[start..start + bytes.len()].copy_from_slice(bytes);
    bytes.len() as i32
}

/// Write a UTF-8 string into the guest's linear memory at `ptr`.
pub fn write_str(caller: &mut Caller<'_, GuestHost>, ptr: i32, s: &str) -> i32 {
    write_bytes(caller, ptr, s.as_bytes())
}

/// Write two `f64`s (16 bytes, little-endian native layout) into guest memory.
pub fn write_f64_pair(caller: &mut Caller<'_, GuestHost>, ptr: i32, a: f64, b: f64) -> i32 {
    let mut buf = [0u8; 16];
    buf[0..8].copy_from_slice(&a.to_le_bytes());
    buf[8..16].copy_from_slice(&b.to_le_bytes());
    write_bytes(caller, ptr, &buf)
}

/// Write three `f64`s (24 bytes) into guest memory.
pub fn write_f64_triple(
    caller: &mut Caller<'_, GuestHost>,
    ptr: i32,
    a: f64,
    b: f64,
    c: f64,
) -> i32 {
    let mut buf = [0u8; 24];
    buf[0..8].copy_from_slice(&a.to_le_bytes());
    buf[8..16].copy_from_slice(&b.to_le_bytes());
    buf[16..24].copy_from_slice(&c.to_le_bytes());
    write_bytes(caller, ptr, &buf)
}
