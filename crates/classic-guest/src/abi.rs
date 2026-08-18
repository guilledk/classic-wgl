//! The guest ABI: the stable contract between a ROM guest module and the host.
//!
//! Guests import host functions from the module named [`HOST_MODULE`] and
//! export `fn update(dt: f64) -> ()`, plus the optional one-shot lifecycle
//! hooks `fn init()` (early, before the first frame) and `fn start()` (once,
//! after the first `update`).  Strings cross the boundary as `(ptr, len)`
//! pairs into guest linear memory; functions that return a byte slice write
//! into a caller-provided output buffer and return the number of bytes written
//! (`-1` if the buffer was too small).
//!
//! The marshalling helpers here are backend-agnostic: they operate on a
//! `&[u8]` / `&mut [u8]` view of the guest's linear memory, which each runtime
//! (wasmi, wasmtime) obtains from its own memory export.

/// The WASM module name under which host imports are defined.
pub const HOST_MODULE: &str = "env";

/// The guest export invoked once per frame.
pub const UPDATE_EXPORT: &str = "update";

/// The optional guest export invoked once, before the first frame.
pub const INIT_EXPORT: &str = "init";

/// The optional guest export invoked once, after the first `update`.
pub const START_EXPORT: &str = "start";

/// The name of the guest's linear memory export.
pub const MEMORY_EXPORT: &str = "memory";

/// Read `len` bytes from a guest linear-memory slice at `ptr`.
pub fn read_bytes_from(data: &[u8], ptr: i32, len: i32) -> Vec<u8> {
    let start = ptr.max(0) as usize;
    let end = (start + len.max(0) as usize).min(data.len());
    data.get(start..end).map(|s| s.to_vec()).unwrap_or_default()
}

/// Read a UTF-8 string from a guest linear-memory slice (lossy on invalid UTF-8).
pub fn read_str_from(data: &[u8], ptr: i32, len: i32) -> String {
    String::from_utf8_lossy(&read_bytes_from(data, ptr, len)).into_owned()
}

/// Write bytes into a guest linear-memory slice at `ptr`, returning the number
/// of bytes written (`-1` if the buffer overruns guest memory).
pub fn write_bytes_to(data: &mut [u8], ptr: i32, bytes: &[u8]) -> i32 {
    let start = ptr.max(0) as usize;
    if start + bytes.len() > data.len() {
        return -1;
    }
    data[start..start + bytes.len()].copy_from_slice(bytes);
    bytes.len() as i32
}

/// Write a UTF-8 string into a guest linear-memory slice at `ptr`.
pub fn write_str_to(data: &mut [u8], ptr: i32, s: &str) -> i32 {
    write_bytes_to(data, ptr, s.as_bytes())
}

/// Pack two `f64`s into a 16-byte little-endian buffer.
pub fn f64_pair_bytes(a: f64, b: f64) -> [u8; 16] {
    let mut buf = [0u8; 16];
    buf[0..8].copy_from_slice(&a.to_le_bytes());
    buf[8..16].copy_from_slice(&b.to_le_bytes());
    buf
}

/// Pack three `f64`s into a 24-byte little-endian buffer.
pub fn f64_triple_bytes(a: f64, b: f64, c: f64) -> [u8; 24] {
    let mut buf = [0u8; 24];
    buf[0..8].copy_from_slice(&a.to_le_bytes());
    buf[8..16].copy_from_slice(&b.to_le_bytes());
    buf[16..24].copy_from_slice(&c.to_le_bytes());
    buf
}

/// Write two `f64`s (16 bytes, little-endian native layout) into guest memory.
pub fn write_f64_pair_to(data: &mut [u8], ptr: i32, a: f64, b: f64) -> i32 {
    write_bytes_to(data, ptr, &f64_pair_bytes(a, b))
}

/// Write three `f64`s (24 bytes) into guest memory.
pub fn write_f64_triple_to(data: &mut [u8], ptr: i32, a: f64, b: f64, c: f64) -> i32 {
    write_bytes_to(data, ptr, &f64_triple_bytes(a, b, c))
}

/// Serialize an `f32` slice to little-endian bytes.
pub fn f32_array_bytes(data: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() * 4);
    for v in data {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Decode little-endian `u32` values from a byte slice.
pub fn bytes_to_u32(bytes: &[u8]) -> Vec<u32> {
    bytes.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

/// Decode little-endian `f32` values from a byte slice.
pub fn bytes_to_f32(bytes: &[u8]) -> Vec<f32> {
    bytes.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

/// Decode little-endian `i32` values from a byte slice.
pub fn bytes_to_i32(bytes: &[u8]) -> Vec<i32> {
    bytes.chunks_exact(4).map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}
