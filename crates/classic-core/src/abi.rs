//! Backend-agnostic wire-format marshalling for the guest ABI.
//!
//! These pure byte-slice helpers convert between guest linear-memory views
//! (`&[u8]` / `&mut [u8]`) and host-side values (strings, `f64` pairs/triples,
//! `f32`/`u32` arrays, path cells).  They live here so both the foreground
//! guest runtimes (`classic-guest`) and the background worker runtime
//! (`classic-worker`) share one implementation without depending on each other.

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
    bytes.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect()
}

/// Decode little-endian `f32` values from a byte slice.
pub fn bytes_to_f32(bytes: &[u8]) -> Vec<f32> {
    bytes.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect()
}

/// Serialize a path (a sequence of `(i32, i32)` cell coordinates) to
/// little-endian bytes: 8 bytes per waypoint (`x` then `y`).
pub fn path_cells_bytes(cells: &[(i32, i32)]) -> Vec<u8> {
    let mut out = Vec::with_capacity(cells.len() * 8);
    for (x, y) in cells {
        out.extend_from_slice(&x.to_le_bytes());
        out.extend_from_slice(&y.to_le_bytes());
    }
    out
}
