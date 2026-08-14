//! The browser-native WebAssembly guest runtime (web only, trusted guests).
//!
//! Trusted guests run on the browser's own `WebAssembly` engine (near-native
//! speed) instead of wasmi-on-wasm.  Browser Wasm has no fuel API, so this
//! backend is only selected for `trusted` ROMs; untrusted ROMs stay on
//! `WasmiRuntime` (interruptible fuel metering).
//!
//! `WebAssembly.Module` / `WebAssembly.Instance` are constructed synchronously
//! (`js_sys::WebAssembly::{Module, Instance}::new`), so no async restructuring
//! is needed.  Host imports are `Closure`-wrapped functions that read/write the
//! guest's linear memory through a `Uint8Array` view of its `WebAssembly.Memory`
//! and dispatch into the shared [`GuestHost`].

use std::cell::RefCell;
use std::rc::Rc;

use classic_engine::Engine;
use js_sys::WebAssembly::{Instance, Memory, Module};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};

use crate::abi;
use crate::runtime::{GuestError, GuestLimits, GuestRuntime};
use crate::sdk::GuestHost;

/// A `Uint8Array` view over the guest's current linear memory.
fn mem_view(mem: &Memory) -> js_sys::Uint8Array {
    let buffer = Memory::buffer(mem);
    js_sys::Uint8Array::new(&buffer)
}

/// Read a UTF-8 string from the guest's linear memory.
fn read_str(mem: &Memory, ptr: i32, len: i32) -> String {
    let view = mem_view(mem);
    let start = ptr.max(0) as u32;
    let end = (start + len.max(0) as u32).min(view.length());
    String::from_utf8_lossy(&view.subarray(start, end).to_vec()).into_owned()
}

/// Read raw bytes from the guest's linear memory.
fn read_bytes(mem: &Memory, ptr: i32, len: i32) -> Vec<u8> {
    let view = mem_view(mem);
    let start = ptr.max(0) as u32;
    let end = (start + len.max(0) as u32).min(view.length());
    view.subarray(start, end).to_vec()
}

/// Write bytes into the guest's linear memory, returning the number of bytes
/// written (`-1` if the buffer overruns guest memory).
fn write_bytes(mem: &Memory, ptr: i32, bytes: &[u8]) -> i32 {
    let view = mem_view(mem);
    let start = ptr.max(0) as u32;
    if start as usize + bytes.len() > view.length() as usize {
        return -1;
    }
    let sub = view.subarray(start, start + bytes.len() as u32);
    sub.copy_from(bytes);
    bytes.len() as i32
}

fn write_str(mem: &Memory, ptr: i32, s: &str) -> i32 {
    write_bytes(mem, ptr, s.as_bytes())
}

fn write_f64_pair(mem: &Memory, ptr: i32, a: f64, b: f64) -> i32 {
    write_bytes(mem, ptr, &abi::f64_pair_bytes(a, b))
}

fn write_f64_triple(mem: &Memory, ptr: i32, a: f64, b: f64, c: f64) -> i32 {
    write_bytes(mem, ptr, &abi::f64_triple_bytes(a, b, c))
}

/// A JS exception's message, for trap reporting.
fn js_err(e: &JsValue) -> String {
    e.as_string().unwrap_or_else(|| format!("{e:?}"))
}

// wasm-bindgen `Closure` supports at most 8 arguments, so the eight host imports
// with 9–11 arguments route through a single dispatcher closure that reads its
// `arguments` array (see `WebWasmRuntime::build_imports`).
const OP_SET_LIGHT: u32 = 0;
const OP_SPAWN_RECT: u32 = 1;
const OP_SPAWN_TEXT: u32 = 2;
const OP_UI_TEXT: u32 = 3;
const OP_UI_BUTTON: u32 = 4;
const OP_UI_ARRAY: u32 = 5;
const OP_UI_PADDING: u32 = 6;
const OP_UI_SPRITE: u32 = 7;
const OP_FBM_FIELD: u32 = 8;
const OP_RIDGED_FIELD: u32 = 9;
const OP_BILLOW_FIELD: u32 = 10;
const OP_TILING_FIELD: u32 = 11;
const OP_NOISE_FIELD: u32 = 12;

/// The global symbol the high-arity import shims call into.
const DISPATCHER_SYMBOL: &str = "__classic_guest_import";

/// Read an `i32` argument from a JS `arguments` array.
fn arg_i32(args: &js_sys::Array, i: u32) -> i32 {
    js_sys::Reflect::get(args, &JsValue::from(i)).ok().and_then(|v| v.as_f64()).unwrap_or(0.0)
        as i32
}

/// Read an `f64` argument from a JS `arguments` array.
fn arg_f64(args: &js_sys::Array, i: u32) -> f64 {
    js_sys::Reflect::get(args, &JsValue::from(i)).ok().and_then(|v| v.as_f64()).unwrap_or(0.0)
}

/// Browser-native [`GuestRuntime`] (web target only, trusted guests).
pub struct WebWasmRuntime {
    host: Rc<RefCell<GuestHost>>,
    init: Option<js_sys::Function>,
    update: js_sys::Function,
    start: Option<js_sys::Function>,
}

impl WebWasmRuntime {
    /// Build the `env` import object: one `Closure` per host import, bridged to
    /// the shared [`GuestHost`].
    fn build_imports(
        host: &Rc<RefCell<GuestHost>>,
        mem: &Rc<RefCell<Option<Memory>>>,
    ) -> Result<js_sys::Object, GuestError> {
        let env = js_sys::Object::new();

        macro_rules! set_import {
            ($name:literal, $closure:expr) => {{
                let c = Closure::wrap($closure);
                js_sys::Reflect::set(&env, &JsValue::from($name), c.as_ref())
                    .map_err(|e| GuestError::Instantiate(js_err(&e)))?;
                c.forget();
            }};
        }

        macro_rules! set_import_str {
            ($name:literal, $op:expr, $args:literal) => {{
                let body = format!("return {}({}, Array.from(arguments));", DISPATCHER_SYMBOL, $op);
                let f = js_sys::Function::new_with_args($args, &body);
                js_sys::Reflect::set(&env, &JsValue::from($name), &f)
                    .map_err(|e| GuestError::Instantiate(js_err(&e)))?;
            }};
        }

        // Dispatcher for the high-arity host imports (see `OP_*`).  A single
        // `Closure` of arity 2 that reads a JS `arguments` array, marshals the
        // args, and dispatches into the shared `GuestHost`.
        {
            let host = host.clone();
            let mem = mem.clone();
            let dispatcher =
                Closure::wrap(Box::new(move |op: u32, args: js_sys::Array| -> JsValue {
                    let result: i32 = match op {
                        OP_SET_LIGHT => host.borrow_mut().set_light(
                            arg_f64(&args, 0),
                            arg_f64(&args, 1),
                            arg_f64(&args, 2),
                            arg_f64(&args, 3),
                            arg_f64(&args, 4),
                            arg_f64(&args, 5),
                            arg_f64(&args, 6),
                            arg_f64(&args, 7),
                            arg_f64(&args, 8),
                        ),
                        OP_SPAWN_RECT => {
                            let name = {
                                let mem = mem.borrow();
                                read_str(
                                    mem.as_ref().unwrap(),
                                    arg_i32(&args, 0),
                                    arg_i32(&args, 1),
                                )
                            };
                            host.borrow_mut().spawn_rect(
                                &name,
                                arg_f64(&args, 2),
                                arg_f64(&args, 3),
                                arg_f64(&args, 4),
                                arg_f64(&args, 5),
                                arg_f64(&args, 6),
                                arg_f64(&args, 7),
                                arg_f64(&args, 8),
                                arg_f64(&args, 9),
                            )
                        }
                        OP_SPAWN_TEXT => {
                            let (name, text) = {
                                let mem = mem.borrow();
                                let m = mem.as_ref().unwrap();
                                (
                                    read_str(m, arg_i32(&args, 0), arg_i32(&args, 1)),
                                    read_str(m, arg_i32(&args, 4), arg_i32(&args, 5)),
                                )
                            };
                            host.borrow_mut().spawn_text(
                                &name,
                                arg_f64(&args, 2),
                                arg_f64(&args, 3),
                                &text,
                                arg_f64(&args, 6),
                                arg_f64(&args, 7),
                                arg_f64(&args, 8),
                                arg_f64(&args, 9),
                                arg_f64(&args, 10),
                            )
                        }
                        OP_UI_TEXT => {
                            let (name, text) = {
                                let mem = mem.borrow();
                                let m = mem.as_ref().unwrap();
                                (
                                    read_str(m, arg_i32(&args, 0), arg_i32(&args, 1)),
                                    read_str(m, arg_i32(&args, 2), arg_i32(&args, 3)),
                                )
                            };
                            host.borrow_mut().ui_text(
                                &name,
                                &text,
                                arg_f64(&args, 4),
                                arg_f64(&args, 5),
                                arg_f64(&args, 6),
                                arg_f64(&args, 7),
                                arg_f64(&args, 8),
                                arg_f64(&args, 9),
                                arg_i32(&args, 10),
                            )
                        }
                        OP_UI_BUTTON => {
                            let (name, text) = {
                                let mem = mem.borrow();
                                let m = mem.as_ref().unwrap();
                                (
                                    read_str(m, arg_i32(&args, 0), arg_i32(&args, 1)),
                                    read_str(m, arg_i32(&args, 2), arg_i32(&args, 3)),
                                )
                            };
                            host.borrow_mut().ui_button(
                                &name,
                                &text,
                                arg_f64(&args, 4),
                                arg_f64(&args, 5),
                                arg_f64(&args, 6),
                                arg_f64(&args, 7),
                                arg_f64(&args, 8),
                                arg_f64(&args, 9),
                            )
                        }
                        OP_UI_ARRAY => {
                            let name = {
                                let mem = mem.borrow();
                                read_str(
                                    mem.as_ref().unwrap(),
                                    arg_i32(&args, 0),
                                    arg_i32(&args, 1),
                                )
                            };
                            host.borrow_mut().ui_array(
                                &name,
                                arg_i32(&args, 2),
                                arg_i32(&args, 3),
                                arg_f64(&args, 4),
                                arg_f64(&args, 5),
                                arg_f64(&args, 6),
                                arg_f64(&args, 7),
                                arg_f64(&args, 8),
                            )
                        }
                        OP_UI_PADDING => {
                            let name = {
                                let mem = mem.borrow();
                                read_str(
                                    mem.as_ref().unwrap(),
                                    arg_i32(&args, 0),
                                    arg_i32(&args, 1),
                                )
                            };
                            host.borrow_mut().ui_padding(
                                &name,
                                arg_f64(&args, 2),
                                arg_f64(&args, 3),
                                arg_f64(&args, 4),
                                arg_f64(&args, 5),
                                arg_f64(&args, 6),
                                arg_f64(&args, 7),
                                arg_f64(&args, 8),
                                arg_f64(&args, 9),
                            )
                        }
                        OP_UI_SPRITE => {
                            let (name, texture) = {
                                let mem = mem.borrow();
                                let m = mem.as_ref().unwrap();
                                (
                                    read_str(m, arg_i32(&args, 0), arg_i32(&args, 1)),
                                    read_str(m, arg_i32(&args, 2), arg_i32(&args, 3)),
                                )
                            };
                            host.borrow_mut().ui_sprite(
                                &name,
                                &texture,
                                arg_f64(&args, 4),
                                arg_f64(&args, 5),
                                arg_f64(&args, 6),
                                arg_f64(&args, 7),
                                arg_f64(&args, 8),
                            )
                        }
                        OP_FBM_FIELD => {
                            let seed = {
                                let mem = mem.borrow();
                                read_str(
                                    mem.as_ref().unwrap(),
                                    arg_i32(&args, 2),
                                    arg_i32(&args, 3),
                                )
                            };
                            let field = host.borrow_mut().fbm_field(
                                arg_i32(&args, 0),
                                arg_i32(&args, 1),
                                &seed,
                                arg_i32(&args, 4).max(0) as u32,
                                arg_f64(&args, 5),
                                arg_f64(&args, 6),
                                arg_f64(&args, 7),
                            );
                            let bytes = abi::f32_array_bytes(&field);
                            let out_ptr = arg_i32(&args, 8);
                            let out_cap = arg_i32(&args, 9);
                            if bytes.len() > out_cap.max(0) as usize {
                                -1
                            } else {
                                let mem = mem.borrow();
                                write_bytes(mem.as_ref().unwrap(), out_ptr, &bytes);
                                bytes.len() as i32
                            }
                        }
                        OP_RIDGED_FIELD => {
                            let seed = {
                                let mem = mem.borrow();
                                read_str(
                                    mem.as_ref().unwrap(),
                                    arg_i32(&args, 2),
                                    arg_i32(&args, 3),
                                )
                            };
                            let field = host.borrow_mut().ridged_field(
                                arg_i32(&args, 0),
                                arg_i32(&args, 1),
                                &seed,
                                arg_i32(&args, 4).max(0) as u32,
                                arg_f64(&args, 5),
                                arg_f64(&args, 6),
                                arg_f64(&args, 7),
                                arg_f64(&args, 8),
                            );
                            let bytes = abi::f32_array_bytes(&field);
                            let out_ptr = arg_i32(&args, 9);
                            let out_cap = arg_i32(&args, 10);
                            if bytes.len() > out_cap.max(0) as usize {
                                -1
                            } else {
                                let mem = mem.borrow();
                                write_bytes(mem.as_ref().unwrap(), out_ptr, &bytes);
                                bytes.len() as i32
                            }
                        }
                        OP_BILLOW_FIELD => {
                            let seed = {
                                let mem = mem.borrow();
                                read_str(
                                    mem.as_ref().unwrap(),
                                    arg_i32(&args, 2),
                                    arg_i32(&args, 3),
                                )
                            };
                            let field = host.borrow_mut().billow_field(
                                arg_i32(&args, 0),
                                arg_i32(&args, 1),
                                &seed,
                                arg_i32(&args, 4).max(0) as u32,
                                arg_f64(&args, 5),
                                arg_f64(&args, 6),
                                arg_f64(&args, 7),
                            );
                            let bytes = abi::f32_array_bytes(&field);
                            let out_ptr = arg_i32(&args, 8);
                            let out_cap = arg_i32(&args, 9);
                            if bytes.len() > out_cap.max(0) as usize {
                                -1
                            } else {
                                let mem = mem.borrow();
                                write_bytes(mem.as_ref().unwrap(), out_ptr, &bytes);
                                bytes.len() as i32
                            }
                        }
                        OP_TILING_FIELD => {
                            let seed = {
                                let mem = mem.borrow();
                                read_str(
                                    mem.as_ref().unwrap(),
                                    arg_i32(&args, 2),
                                    arg_i32(&args, 3),
                                )
                            };
                            let field = host.borrow_mut().tiling_field(
                                arg_i32(&args, 0),
                                arg_i32(&args, 1),
                                &seed,
                                arg_f64(&args, 4),
                                arg_i32(&args, 5).max(0) as u32,
                                arg_f64(&args, 6),
                            );
                            let bytes = abi::f32_array_bytes(&field);
                            let out_ptr = arg_i32(&args, 7);
                            let out_cap = arg_i32(&args, 8);
                            if bytes.len() > out_cap.max(0) as usize {
                                -1
                            } else {
                                let mem = mem.borrow();
                                write_bytes(mem.as_ref().unwrap(), out_ptr, &bytes);
                                bytes.len() as i32
                            }
                        }
                        OP_NOISE_FIELD => {
                            let seed = {
                                let mem = mem.borrow();
                                read_str(
                                    mem.as_ref().unwrap(),
                                    arg_i32(&args, 2),
                                    arg_i32(&args, 3),
                                )
                            };
                            let field = host.borrow_mut().noise_field(
                                arg_i32(&args, 0),
                                arg_i32(&args, 1),
                                &seed,
                                arg_f64(&args, 4),
                                arg_f64(&args, 5),
                            );
                            let bytes = abi::f32_array_bytes(&field);
                            let out_ptr = arg_i32(&args, 6);
                            let out_cap = arg_i32(&args, 7);
                            if bytes.len() > out_cap.max(0) as usize {
                                -1
                            } else {
                                let mem = mem.borrow();
                                write_bytes(mem.as_ref().unwrap(), out_ptr, &bytes);
                                bytes.len() as i32
                            }
                        }
                        _ => 0,
                    };
                    JsValue::from_f64(result as f64)
                }) as Box<dyn FnMut(u32, js_sys::Array) -> JsValue>);
            js_sys::Reflect::set(
                &js_sys::global(),
                &JsValue::from(DISPATCHER_SYMBOL),
                dispatcher.as_ref(),
            )
            .map_err(|e| GuestError::Instantiate(js_err(&e)))?;
            dispatcher.forget();
        }

        // log
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "log",
                Box::new(move |ptr: i32, len: i32| {
                    let msg = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), ptr, len)
                    };
                    host.borrow_mut().log(&msg);
                }) as Box<dyn FnMut(i32, i32)>
            );
        }

        // spawn
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "spawn",
                Box::new(move |ptr: i32, len: i32| -> i32 {
                    let name = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), ptr, len)
                    };
                    host.borrow_mut().spawn(&name)
                }) as Box<dyn FnMut(i32, i32) -> i32>
            );
        }

        // despawn
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "despawn",
                Box::new(move |ptr: i32, len: i32| -> i32 {
                    let name = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), ptr, len)
                    };
                    host.borrow_mut().despawn(&name)
                }) as Box<dyn FnMut(i32, i32) -> i32>
            );
        }

        // has
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "has",
                Box::new(move |ptr: i32, len: i32| -> i32 {
                    let name = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), ptr, len)
                    };
                    host.borrow_mut().has(&name)
                }) as Box<dyn FnMut(i32, i32) -> i32>
            );
        }

        // names
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "names",
                Box::new(move |out_ptr: i32, out_cap: i32| -> i32 {
                    let json = host.borrow_mut().names();
                    if out_cap < json.len() as i32 {
                        return -1;
                    }
                    let mem = mem.borrow();
                    write_str(mem.as_ref().unwrap(), out_ptr, &json)
                }) as Box<dyn FnMut(i32, i32) -> i32>
            );
        }

        // get
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "get",
                Box::new(move |ptr: i32, len: i32, out_ptr: i32, out_cap: i32| -> i32 {
                    let name = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), ptr, len)
                    };
                    let json = host.borrow_mut().get(&name);
                    if out_cap < json.len() as i32 {
                        return -1;
                    }
                    let mem = mem.borrow();
                    write_str(mem.as_ref().unwrap(), out_ptr, &json)
                }) as Box<dyn FnMut(i32, i32, i32, i32) -> i32>
            );
        }

        // get_comp
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "get_comp",
                Box::new(
                    move |ptr: i32,
                          len: i32,
                          comp_ptr: i32,
                          comp_len: i32,
                          out_ptr: i32,
                          out_cap: i32|
                          -> i32 {
                        let (name, comp) = {
                            let mem = mem.borrow();
                            let m = mem.as_ref().unwrap();
                            (read_str(m, ptr, len), read_str(m, comp_ptr, comp_len))
                        };
                        let json = host.borrow_mut().get_comp(&name, &comp);
                        if out_cap < json.len() as i32 {
                            return -1;
                        }
                        let mem = mem.borrow();
                        write_str(mem.as_ref().unwrap(), out_ptr, &json)
                    },
                ) as Box<dyn FnMut(i32, i32, i32, i32, i32, i32) -> i32>
            );
        }

        // set
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "set",
                Box::new(move |ptr: i32, len: i32, json_ptr: i32, json_len: i32| -> i32 {
                    let (name, json) = {
                        let mem = mem.borrow();
                        let m = mem.as_ref().unwrap();
                        (read_str(m, ptr, len), read_str(m, json_ptr, json_len))
                    };
                    host.borrow_mut().set(&name, &json)
                }) as Box<dyn FnMut(i32, i32, i32, i32) -> i32>
            );
        }

        // set_comp
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "set_comp",
                Box::new(
                    move |ptr: i32,
                          len: i32,
                          comp_ptr: i32,
                          comp_len: i32,
                          json_ptr: i32,
                          json_len: i32|
                          -> i32 {
                        let (name, comp, json) = {
                            let mem = mem.borrow();
                            let m = mem.as_ref().unwrap();
                            (
                                read_str(m, ptr, len),
                                read_str(m, comp_ptr, comp_len),
                                read_str(m, json_ptr, json_len),
                            )
                        };
                        host.borrow_mut().set_comp(&name, &comp, &json)
                    },
                ) as Box<dyn FnMut(i32, i32, i32, i32, i32, i32) -> i32>
            );
        }

        // set_pos
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "set_pos",
                Box::new(move |ptr: i32, len: i32, x: f64, y: f64, z: f64| -> i32 {
                    let name = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), ptr, len)
                    };
                    host.borrow_mut().set_pos(&name, x, y, z)
                }) as Box<dyn FnMut(i32, i32, f64, f64, f64) -> i32>
            );
        }

        // get_pos
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "get_pos",
                Box::new(move |ptr: i32, len: i32, out_ptr: i32| -> i32 {
                    let name = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), ptr, len)
                    };
                    let Some((x, y, z)) = host.borrow_mut().get_pos(&name) else {
                        return 0;
                    };
                    let mem = mem.borrow();
                    write_f64_triple(mem.as_ref().unwrap(), out_ptr, x, y, z);
                    1
                }) as Box<dyn FnMut(i32, i32, i32) -> i32>
            );
        }

        // mouse
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "mouse",
                Box::new(move |out_ptr: i32| -> i32 {
                    let (x, y) = host.borrow_mut().mouse();
                    let mem = mem.borrow();
                    write_f64_pair(mem.as_ref().unwrap(), out_ptr, x, y);
                    1
                }) as Box<dyn FnMut(i32) -> i32>
            );
        }

        // mouse_iso
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "mouse_iso",
                Box::new(move |out_ptr: i32| -> i32 {
                    let Some((x, y)) = host.borrow_mut().mouse_iso() else {
                        return 0;
                    };
                    let mem = mem.borrow();
                    write_f64_pair(mem.as_ref().unwrap(), out_ptr, x, y);
                    1
                }) as Box<dyn FnMut(i32) -> i32>
            );
        }

        // height_at
        {
            let host = host.clone();
            set_import!(
                "height_at",
                Box::new(move |x: f64, y: f64| -> f64 { host.borrow_mut().height_at(x, y) })
                    as Box<dyn FnMut(f64, f64) -> f64>
            );
        }

        // set_anim
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "set_anim",
                Box::new(move |ptr: i32, len: i32, anim_ptr: i32, anim_len: i32| -> i32 {
                    let (name, anim) = {
                        let mem = mem.borrow();
                        let m = mem.as_ref().unwrap();
                        (read_str(m, ptr, len), read_str(m, anim_ptr, anim_len))
                    };
                    host.borrow_mut().set_anim(&name, &anim)
                }) as Box<dyn FnMut(i32, i32, i32, i32) -> i32>
            );
        }

        // agent_selected
        {
            let host = host.clone();
            set_import!(
                "agent_selected",
                Box::new(move || -> i32 { host.borrow_mut().agent_selected() })
                    as Box<dyn FnMut() -> i32>
            );
        }

        // ui_consumed_click
        {
            let host = host.clone();
            set_import!(
                "ui_consumed_click",
                Box::new(move || -> i32 { host.borrow_mut().ui_consumed_click() })
                    as Box<dyn FnMut() -> i32>
            );
        }

        // delta
        {
            let host = host.clone();
            set_import!(
                "delta",
                Box::new(move || -> f64 { host.borrow_mut().delta() }) as Box<dyn FnMut() -> f64>
            );
        }

        // elapsed
        {
            let host = host.clone();
            set_import!(
                "elapsed",
                Box::new(move || -> f64 { host.borrow_mut().elapsed() }) as Box<dyn FnMut() -> f64>
            );
        }

        // was_pressed
        {
            let host = host.clone();
            set_import!(
                "was_pressed",
                Box::new(move |btn: i32| -> i32 { host.borrow_mut().was_pressed(btn) })
                    as Box<dyn FnMut(i32) -> i32>
            );
        }

        // key_down
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "key_down",
                Box::new(move |ptr: i32, len: i32| -> i32 {
                    let key = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), ptr, len)
                    };
                    host.borrow_mut().key_down(&key)
                }) as Box<dyn FnMut(i32, i32) -> i32>
            );
        }

        // was_key_pressed
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "was_key_pressed",
                Box::new(move |ptr: i32, len: i32| -> i32 {
                    let key = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), ptr, len)
                    };
                    host.borrow_mut().was_key_pressed(&key)
                }) as Box<dyn FnMut(i32, i32) -> i32>
            );
        }

        // generate_terrain
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "generate_terrain",
                Box::new(
                    move |kind_ptr: i32,
                          kind_len: i32,
                          seed_ptr: i32,
                          seed_len: i32,
                          height_scale: f64|
                          -> i32 {
                        let (kind, seed) = {
                            let mem = mem.borrow();
                            let m = mem.as_ref().unwrap();
                            (read_str(m, kind_ptr, kind_len), read_str(m, seed_ptr, seed_len))
                        };
                        host.borrow_mut().generate_terrain(&kind, &seed, height_scale)
                    },
                ) as Box<dyn FnMut(i32, i32, i32, i32, f64) -> i32>
            );
        }

        // set_tile
        {
            let host = host.clone();
            set_import!(
                "set_tile",
                Box::new(move |x: i32, y: i32, id: i32| -> i32 {
                    host.borrow_mut().set_tile(x, y, id)
                }) as Box<dyn FnMut(i32, i32, i32) -> i32>
            );
        }

        // set_height
        {
            let host = host.clone();
            set_import!(
                "set_height",
                Box::new(move |x: i32, y: i32, h: f64| -> i32 {
                    host.borrow_mut().set_height(x, y, h)
                }) as Box<dyn FnMut(i32, i32, f64) -> i32>
            );
        }

        // rebuild_terrain
        {
            let host = host.clone();
            set_import!(
                "rebuild_terrain",
                Box::new(move || -> i32 { host.borrow_mut().rebuild_terrain() })
                    as Box<dyn FnMut() -> i32>
            );
        }

        // find_path
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "find_path",
                Box::new(
                    move |sx: i32, sy: i32, ex: i32, ey: i32, out_ptr: i32, out_cap: i32| -> i32 {
                        let cells = host.borrow_mut().find_path(sx, sy, ex, ey);
                        let mut bytes = Vec::with_capacity(cells.len() * 8);
                        for (x, y) in &cells {
                            bytes.extend_from_slice(&x.to_le_bytes());
                            bytes.extend_from_slice(&y.to_le_bytes());
                        }
                        if bytes.len() > out_cap.max(0) as usize {
                            return -1;
                        }
                        let mem = mem.borrow();
                        write_bytes(mem.as_ref().unwrap(), out_ptr, &bytes);
                        cells.len() as i32
                    },
                ) as Box<dyn FnMut(i32, i32, i32, i32, i32, i32) -> i32>
            );
        }

        // get_camera
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "get_camera",
                Box::new(move |out_ptr: i32| -> i32 {
                    let (x, y, s) = host.borrow_mut().get_camera();
                    let mem = mem.borrow();
                    write_f64_triple(mem.as_ref().unwrap(), out_ptr, x, y, s);
                    1
                }) as Box<dyn FnMut(i32) -> i32>
            );
        }

        // set_camera
        {
            let host = host.clone();
            set_import!(
                "set_camera",
                Box::new(move |x: f64, y: f64, scale: f64| -> i32 {
                    host.borrow_mut().set_camera(x, y, scale)
                }) as Box<dyn FnMut(f64, f64, f64) -> i32>
            );
        }

        // pick_at
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "pick_at",
                Box::new(move |x: f64, y: f64, out_ptr: i32, out_cap: i32| -> i32 {
                    let name = host.borrow_mut().pick_at(x, y);
                    if out_cap < name.len() as i32 {
                        return -1;
                    }
                    let mem = mem.borrow();
                    write_str(mem.as_ref().unwrap(), out_ptr, &name)
                }) as Box<dyn FnMut(f64, f64, i32, i32) -> i32>
            );
        }

        // mouse_down
        {
            let host = host.clone();
            set_import!(
                "mouse_down",
                Box::new(move |btn: i32| -> i32 { host.borrow_mut().mouse_down(btn) })
                    as Box<dyn FnMut(i32) -> i32>
            );
        }

        // mouse_released
        {
            let host = host.clone();
            set_import!(
                "mouse_released",
                Box::new(move |btn: i32| -> i32 { host.borrow_mut().mouse_released(btn) })
                    as Box<dyn FnMut(i32) -> i32>
            );
        }

        // mouse_wheel
        {
            let host = host.clone();
            set_import!(
                "mouse_wheel",
                Box::new(move || -> f64 { host.borrow_mut().mouse_wheel() })
                    as Box<dyn FnMut() -> f64>
            );
        }

        // key_up
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "key_up",
                Box::new(move |ptr: i32, len: i32| -> i32 {
                    let key = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), ptr, len)
                    };
                    host.borrow_mut().key_up(&key)
                }) as Box<dyn FnMut(i32, i32) -> i32>
            );
        }

        // get_light
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "get_light",
                Box::new(move |out_ptr: i32| -> i32 {
                    let (a, d, c) = host.borrow_mut().get_light();
                    let mut buf = Vec::with_capacity(72);
                    for v in a.iter().chain(d.iter()).chain(c.iter()) {
                        buf.extend_from_slice(&v.to_le_bytes());
                    }
                    let mem = mem.borrow();
                    write_bytes(mem.as_ref().unwrap(), out_ptr, &buf);
                    1
                }) as Box<dyn FnMut(i32) -> i32>
            );
        }

        // set_light
        set_import_str!("set_light", OP_SET_LIGHT, "a,b,c,d,e,f,g,h,i");

        // spawn_rect
        set_import_str!("spawn_rect", OP_SPAWN_RECT, "a,b,c,d,e,f,g,h,i,j");

        // spawn_text
        set_import_str!("spawn_text", OP_SPAWN_TEXT, "a,b,c,d,e,f,g,h,i,j,k");

        // set_text
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "set_text",
                Box::new(move |name_ptr: i32, name_len: i32, text_ptr: i32, text_len: i32| -> i32 {
                    let (name, text) = {
                        let mem = mem.borrow();
                        let m = mem.as_ref().unwrap();
                        (read_str(m, name_ptr, name_len), read_str(m, text_ptr, text_len))
                    };
                    host.borrow_mut().set_text(&name, &text)
                }) as Box<dyn FnMut(i32, i32, i32, i32) -> i32>
            );
        }

        // ui_container
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "ui_container",
                Box::new(
                    move |name_ptr: i32,
                          name_len: i32,
                          w: f64,
                          h: f64,
                          r: f64,
                          g: f64,
                          b: f64,
                          a: f64|
                          -> i32 {
                        let name = {
                            let mem = mem.borrow();
                            read_str(mem.as_ref().unwrap(), name_ptr, name_len)
                        };
                        host.borrow_mut().ui_container(&name, w, h, r, g, b, a)
                    },
                ) as Box<dyn FnMut(i32, i32, f64, f64, f64, f64, f64, f64) -> i32>
            );
        }

        // ui_text
        set_import_str!("ui_text", OP_UI_TEXT, "a,b,c,d,e,f,g,h,i,j,k");

        // ui_button
        set_import_str!("ui_button", OP_UI_BUTTON, "a,b,c,d,e,f,g,h,i,j");

        // ui_array
        set_import_str!("ui_array", OP_UI_ARRAY, "a,b,c,d,e,f,g,h,i");

        // ui_padding
        set_import_str!("ui_padding", OP_UI_PADDING, "a,b,c,d,e,f,g,h,i,j");

        // ui_sprite
        set_import_str!("ui_sprite", OP_UI_SPRITE, "a,b,c,d,e,f,g,h,i");

        // ui_add_child
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "ui_add_child",
                Box::new(
                    move |parent_ptr: i32,
                          parent_len: i32,
                          child_ptr: i32,
                          child_len: i32,
                          self_anchor: i32,
                          child_anchor: i32|
                          -> i32 {
                        let (parent, child) = {
                            let mem = mem.borrow();
                            let m = mem.as_ref().unwrap();
                            (read_str(m, parent_ptr, parent_len), read_str(m, child_ptr, child_len))
                        };
                        host.borrow_mut().ui_add_child(&parent, &child, self_anchor, child_anchor)
                    },
                ) as Box<dyn FnMut(i32, i32, i32, i32, i32, i32) -> i32>
            );
        }

        // ui_add_to_root
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "ui_add_to_root",
                Box::new(
                    move |name_ptr: i32,
                          name_len: i32,
                          self_anchor: i32,
                          child_anchor: i32|
                          -> i32 {
                        let name = {
                            let mem = mem.borrow();
                            read_str(mem.as_ref().unwrap(), name_ptr, name_len)
                        };
                        host.borrow_mut().ui_add_to_root(&name, self_anchor, child_anchor)
                    }
                ) as Box<dyn FnMut(i32, i32, i32, i32) -> i32>
            );
        }

        // ui_set_size
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "ui_set_size",
                Box::new(move |name_ptr: i32, name_len: i32, w: f64, h: f64| -> i32 {
                    let name = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), name_ptr, name_len)
                    };
                    host.borrow_mut().ui_set_size(&name, w, h)
                }) as Box<dyn FnMut(i32, i32, f64, f64) -> i32>
            );
        }

        // ui_set_anchor
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "ui_set_anchor",
                Box::new(move |name_ptr: i32, name_len: i32, anchor: i32| -> i32 {
                    let name = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), name_ptr, name_len)
                    };
                    host.borrow_mut().ui_set_anchor(&name, anchor)
                }) as Box<dyn FnMut(i32, i32, i32) -> i32>
            );
        }

        // ui_set_color
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "ui_set_color",
                Box::new(
                    move |name_ptr: i32, name_len: i32, r: f64, g: f64, b: f64, a: f64| -> i32 {
                        let name = {
                            let mem = mem.borrow();
                            read_str(mem.as_ref().unwrap(), name_ptr, name_len)
                        };
                        host.borrow_mut().ui_set_color(&name, r, g, b, a)
                    }
                ) as Box<dyn FnMut(i32, i32, f64, f64, f64, f64) -> i32>
            );
        }

        // ui_set_fixed
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "ui_set_fixed",
                Box::new(move |name_ptr: i32, name_len: i32, fixed: i32| -> i32 {
                    let name = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), name_ptr, name_len)
                    };
                    host.borrow_mut().ui_set_fixed(&name, fixed)
                }) as Box<dyn FnMut(i32, i32, i32) -> i32>
            );
        }

        // subscribe
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "subscribe",
                Box::new(move |ptr: i32, len: i32| -> i32 {
                    let name = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), ptr, len)
                    };
                    host.borrow_mut().subscribe(&name)
                }) as Box<dyn FnMut(i32, i32) -> i32>
            );
        }

        // poll_event
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "poll_event",
                Box::new(move |out_ptr: i32, out_cap: i32| -> i32 {
                    let Some((kind, name)) = host.borrow_mut().poll_event() else {
                        return 0;
                    };
                    let mut bytes = Vec::with_capacity(8 + name.len());
                    bytes.extend_from_slice(&kind.to_le_bytes());
                    bytes.extend_from_slice(&(name.len() as u32).to_le_bytes());
                    bytes.extend_from_slice(name.as_bytes());
                    if bytes.len() > out_cap.max(0) as usize {
                        return -1;
                    }
                    let mem = mem.borrow();
                    write_bytes(mem.as_ref().unwrap(), out_ptr, &bytes);
                    1
                }) as Box<dyn FnMut(i32, i32) -> i32>
            );
        }

        // spawn_collider
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "spawn_collider",
                Box::new(
                    move |name_ptr: i32, name_len: i32, x: f64, y: f64, w: f64, h: f64| -> i32 {
                        let name = {
                            let mem = mem.borrow();
                            read_str(mem.as_ref().unwrap(), name_ptr, name_len)
                        };
                        host.borrow_mut().spawn_collider(&name, x, y, w, h)
                    }
                ) as Box<dyn FnMut(i32, i32, f64, f64, f64, f64) -> i32>
            );
        }

        // get_anim
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "get_anim",
                Box::new(move |name_ptr: i32, name_len: i32, out_ptr: i32, out_cap: i32| -> i32 {
                    let name = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), name_ptr, name_len)
                    };
                    let Some((anim, frame)) = host.borrow_mut().get_anim(&name) else {
                        return 0;
                    };
                    let mut bytes = Vec::with_capacity(12 + anim.len());
                    bytes.extend_from_slice(&frame.to_le_bytes());
                    bytes.extend_from_slice(&(anim.len() as u32).to_le_bytes());
                    bytes.extend_from_slice(anim.as_bytes());
                    if bytes.len() > out_cap.max(0) as usize {
                        return -1;
                    }
                    let mem = mem.borrow();
                    write_bytes(mem.as_ref().unwrap(), out_ptr, &bytes);
                    1
                }) as Box<dyn FnMut(i32, i32, i32, i32) -> i32>
            );
        }

        // has_resource
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "has_resource",
                Box::new(move |kind: i32, ptr: i32, len: i32| -> i32 {
                    let name = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), ptr, len)
                    };
                    host.borrow_mut().has_resource(kind, &name)
                }) as Box<dyn FnMut(i32, i32, i32) -> i32>
            );
        }

        // texture_size
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "texture_size",
                Box::new(move |ptr: i32, len: i32, out_ptr: i32| -> i32 {
                    let name = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), ptr, len)
                    };
                    let Some((w, h)) = host.borrow_mut().texture_size(&name) else {
                        return 0;
                    };
                    let mem = mem.borrow();
                    write_f64_pair(mem.as_ref().unwrap(), out_ptr, w, h);
                    1
                }) as Box<dyn FnMut(i32, i32, i32) -> i32>
            );
        }

        // Bulk noise fields (>8 args → dispatcher).
        set_import_str!("fbm_field", OP_FBM_FIELD, "a,b,c,d,e,f,g,h,i,j");
        set_import_str!("ridged_field", OP_RIDGED_FIELD, "a,b,c,d,e,f,g,h,i,j,k");
        set_import_str!("billow_field", OP_BILLOW_FIELD, "a,b,c,d,e,f,g,h,i,j");
        set_import_str!("tiling_field", OP_TILING_FIELD, "a,b,c,d,e,f,g,h,i");
        set_import_str!("noise_field", OP_NOISE_FIELD, "a,b,c,d,e,f,g,h");

        // noise2d
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "noise2d",
                Box::new(move |seed_ptr: i32, seed_len: i32, x: f64, y: f64| -> f64 {
                    let seed = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), seed_ptr, seed_len)
                    };
                    host.borrow_mut().noise2d(&seed, x, y)
                }) as Box<dyn FnMut(i32, i32, f64, f64) -> f64>
            );
        }

        // Bulk terrain upload (guest → host).
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "set_tiles",
                Box::new(move |ptr: i32, len: i32| -> i32 {
                    let tiles = abi::bytes_to_u32(&{
                        let mem = mem.borrow();
                        read_bytes(mem.as_ref().unwrap(), ptr, len)
                    });
                    host.borrow_mut().set_tiles(&tiles)
                }) as Box<dyn FnMut(i32, i32) -> i32>
            );
        }

        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "set_heights",
                Box::new(move |ptr: i32, len: i32| -> i32 {
                    let heights = abi::bytes_to_f32(&{
                        let mem = mem.borrow();
                        read_bytes(mem.as_ref().unwrap(), ptr, len)
                    });
                    host.borrow_mut().set_heights(&heights)
                }) as Box<dyn FnMut(i32, i32) -> i32>
            );
        }

        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "set_nav",
                Box::new(move |ptr: i32, len: i32| -> i32 {
                    let nav = abi::bytes_to_u32(&{
                        let mem = mem.borrow();
                        read_bytes(mem.as_ref().unwrap(), ptr, len)
                    });
                    host.borrow_mut().set_nav(&nav)
                }) as Box<dyn FnMut(i32, i32) -> i32>
            );
        }

        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "set_tileset",
                Box::new(move |ptr: i32, len: i32, w: i32, h: i32| -> i32 {
                    let rgba = {
                        let mem = mem.borrow();
                        read_bytes(mem.as_ref().unwrap(), ptr, len)
                    };
                    host.borrow_mut().set_tileset(&rgba, w.max(0) as u32, h.max(0) as u32)
                }) as Box<dyn FnMut(i32, i32, i32, i32) -> i32>
            );
        }

        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "set_spawn_points",
                Box::new(move |ptr: i32, len: i32| -> i32 {
                    let pairs = abi::bytes_to_i32(&{
                        let mem = mem.borrow();
                        read_bytes(mem.as_ref().unwrap(), ptr, len)
                    });
                    host.borrow_mut().set_spawn_points(&pairs)
                }) as Box<dyn FnMut(i32, i32) -> i32>
            );
        }

        {
            let host = host.clone();
            set_import!(
                "commit_terrain",
                Box::new(move |hs: f64| -> i32 { host.borrow_mut().commit_terrain(hs) })
                    as Box<dyn FnMut(f64) -> i32>
            );
        }

        Ok(env)
    }
}

impl GuestRuntime for WebWasmRuntime {
    fn new(wasm: &[u8], limits: &GuestLimits) -> Result<Self, GuestError> {
        // Browser Wasm has no fuel API; only trusted guests reach this backend
        // (selected by `create_runtime`).
        let _ = limits;

        let host = Rc::new(RefCell::new(GuestHost::new()));
        let mem: Rc<RefCell<Option<Memory>>> = Rc::new(RefCell::new(None));

        let env = Self::build_imports(&host, &mem)?;

        let imports = js_sys::Object::new();
        js_sys::Reflect::set(&imports, &JsValue::from(abi::HOST_MODULE), &env)
            .map_err(|e| GuestError::Instantiate(js_err(&e)))?;

        let bytes = js_sys::Uint8Array::new_from_slice(wasm);
        let module = Module::new(&bytes.into()).map_err(|e| GuestError::Compile(js_err(&e)))?;

        let instance =
            Instance::new(&module, &imports).map_err(|e| GuestError::Instantiate(js_err(&e)))?;

        let exports = Instance::exports(&instance);
        let get = |name: &str| js_sys::Reflect::get(&exports, &JsValue::from(name)).ok();

        let update = get(abi::UPDATE_EXPORT)
            .and_then(|v| v.dyn_into::<js_sys::Function>().ok())
            .ok_or_else(|| GuestError::MissingExport(abi::UPDATE_EXPORT.to_string()))?;
        let init = get(abi::INIT_EXPORT).and_then(|v| v.dyn_into::<js_sys::Function>().ok());
        let start = get(abi::START_EXPORT).and_then(|v| v.dyn_into::<js_sys::Function>().ok());

        if let Some(m) = get(abi::MEMORY_EXPORT).and_then(|v| v.dyn_into::<Memory>().ok()) {
            *mem.borrow_mut() = Some(m);
        }

        Ok(Self { host, init, update, start })
    }

    fn init(&mut self, engine: &mut Engine) -> Result<(), GuestError> {
        let Some(init) = self.init.clone() else { return Ok(()) };
        self.host.borrow_mut().set_engine(engine);
        init.call0(&JsValue::undefined()).map(|_| ()).map_err(|e| GuestError::Trap(js_err(&e)))
    }

    fn update(&mut self, engine: &mut Engine, dt: f64) -> Result<(), GuestError> {
        self.host.borrow_mut().set_engine(engine);
        self.update
            .call1(&JsValue::undefined(), &JsValue::from_f64(dt))
            .map(|_| ())
            .map_err(|e| GuestError::Trap(js_err(&e)))
    }

    fn start(&mut self, engine: &mut Engine) -> Result<(), GuestError> {
        let Some(start) = self.start.clone() else { return Ok(()) };
        self.host.borrow_mut().set_engine(engine);
        start.call0(&JsValue::undefined()).map(|_| ()).map_err(|e| GuestError::Trap(js_err(&e)))
    }
}
