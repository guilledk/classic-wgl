//! Guest linear-memory marshalling helpers for the browser-native runtime.

use js_sys::WebAssembly::Memory;
use wasm_bindgen::JsValue;

use crate::abi;

/// A `Uint8Array` view over the guest's current linear memory.
fn mem_view(mem: &Memory) -> js_sys::Uint8Array {
    let buffer = Memory::buffer(mem);
    js_sys::Uint8Array::new(&buffer)
}

/// Read a UTF-8 string from the guest's linear memory.
pub(super) fn read_str(mem: &Memory, ptr: i32, len: i32) -> String {
    let view = mem_view(mem);
    let start = ptr.max(0) as u32;
    let end = (start + len.max(0) as u32).min(view.length());
    String::from_utf8_lossy(&view.subarray(start, end).to_vec()).into_owned()
}

/// Read raw bytes from the guest's linear memory.
pub(super) fn read_bytes(mem: &Memory, ptr: i32, len: i32) -> Vec<u8> {
    let view = mem_view(mem);
    let start = ptr.max(0) as u32;
    let end = (start + len.max(0) as u32).min(view.length());
    view.subarray(start, end).to_vec()
}

/// Write bytes into the guest's linear memory, returning the number of bytes
/// written (`-1` if the buffer overruns guest memory).
pub(super) fn write_bytes(mem: &Memory, ptr: i32, bytes: &[u8]) -> i32 {
    let view = mem_view(mem);
    let start = ptr.max(0) as u32;
    if start as usize + bytes.len() > view.length() as usize {
        return -1;
    }
    let sub = view.subarray(start, start + bytes.len() as u32);
    sub.copy_from(bytes);
    bytes.len() as i32
}

pub(super) fn write_str(mem: &Memory, ptr: i32, s: &str) -> i32 {
    write_bytes(mem, ptr, s.as_bytes())
}

pub(super) fn write_f64_pair(mem: &Memory, ptr: i32, a: f64, b: f64) -> i32 {
    write_bytes(mem, ptr, &abi::f64_pair_bytes(a, b))
}

pub(super) fn write_f64_triple(mem: &Memory, ptr: i32, a: f64, b: f64, c: f64) -> i32 {
    write_bytes(mem, ptr, &abi::f64_triple_bytes(a, b, c))
}

/// A JS exception's message, for trap reporting.
pub(super) fn js_err(e: &JsValue) -> String {
    e.as_string().unwrap_or_else(|| format!("{e:?}"))
}
