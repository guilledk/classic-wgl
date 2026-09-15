//! The high-arity host-import dispatcher: wasm-bindgen `Closure` supports at
//! most 8 arguments, so the wider imports route through one global JS function.

use std::cell::RefCell;
use std::rc::Rc;

use js_sys::WebAssembly::Memory;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsValue;

use crate::abi;
use crate::runtime::GuestError;
use crate::sdk::GuestHost;

use super::mem::{js_err, read_str, write_bytes};

// wasm-bindgen `Closure` supports at most 8 arguments, so the eight host imports
// with 9–11 arguments route through a single dispatcher closure that reads its
// `arguments` array (see `WebWasmRuntime::build_imports`).
pub(super) const OP_SET_LIGHT: u32 = 0;
pub(super) const OP_SPAWN_RECT: u32 = 1;
pub(super) const OP_SPAWN_TEXT: u32 = 2;
pub(super) const OP_UI_TEXT: u32 = 3;
pub(super) const OP_UI_BUTTON: u32 = 4;
pub(super) const OP_UI_ARRAY: u32 = 5;
pub(super) const OP_UI_PADDING: u32 = 6;
pub(super) const OP_UI_SPRITE: u32 = 7;
pub(super) const OP_FBM_FIELD: u32 = 8;
pub(super) const OP_RIDGED_FIELD: u32 = 9;
pub(super) const OP_BILLOW_FIELD: u32 = 10;
pub(super) const OP_TILING_FIELD: u32 = 11;
pub(super) const OP_NOISE_FIELD: u32 = 12;
pub(super) const OP_LIGHT_SPAWN: u32 = 13;
pub(super) const OP_LIGHT_SET: u32 = 14;

/// The global symbol the high-arity import shims call into.
pub(super) const DISPATCHER_SYMBOL: &str = "__classic_guest_import";

/// Read an `i32` argument from a JS `arguments` array.
fn arg_i32(args: &js_sys::Array, i: u32) -> i32 {
    js_sys::Reflect::get(args, &JsValue::from(i)).ok().and_then(|v| v.as_f64()).unwrap_or(0.0)
        as i32
}

/// Read an `f64` argument from a JS `arguments` array.
fn arg_f64(args: &js_sys::Array, i: u32) -> f64 {
    js_sys::Reflect::get(args, &JsValue::from(i)).ok().and_then(|v| v.as_f64()).unwrap_or(0.0)
}

/// Dispatcher for the high-arity host imports (see `OP_*`).  A single
/// `Closure` of arity 2 that reads a JS `arguments` array, marshals the
/// args, and dispatches into the shared `GuestHost`.
pub(super) fn install_dispatcher(
    host: &Rc<RefCell<GuestHost>>,
    mem: &Rc<RefCell<Option<Memory>>>,
) -> Result<(), GuestError> {
    let host = host.clone();
    let mem = mem.clone();
    let dispatcher = Closure::wrap(Box::new(move |op: u32, args: js_sys::Array| -> JsValue {
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
                    read_str(mem.as_ref().unwrap(), arg_i32(&args, 0), arg_i32(&args, 1))
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
                    read_str(mem.as_ref().unwrap(), arg_i32(&args, 0), arg_i32(&args, 1))
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
                    read_str(mem.as_ref().unwrap(), arg_i32(&args, 0), arg_i32(&args, 1))
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
                    read_str(mem.as_ref().unwrap(), arg_i32(&args, 2), arg_i32(&args, 3))
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
                    read_str(mem.as_ref().unwrap(), arg_i32(&args, 2), arg_i32(&args, 3))
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
                    read_str(mem.as_ref().unwrap(), arg_i32(&args, 2), arg_i32(&args, 3))
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
                    read_str(mem.as_ref().unwrap(), arg_i32(&args, 2), arg_i32(&args, 3))
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
                    read_str(mem.as_ref().unwrap(), arg_i32(&args, 2), arg_i32(&args, 3))
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
            OP_LIGHT_SPAWN => host.borrow_mut().light_spawn(
                arg_i32(&args, 0),
                arg_f64(&args, 1),
                arg_f64(&args, 2),
                arg_f64(&args, 3),
                arg_f64(&args, 4),
                arg_f64(&args, 5),
                arg_f64(&args, 6),
                arg_f64(&args, 7),
                arg_f64(&args, 8),
                arg_f64(&args, 9),
            ),
            OP_LIGHT_SET => host.borrow_mut().light_set(
                arg_i32(&args, 0),
                arg_f64(&args, 1),
                arg_f64(&args, 2),
                arg_f64(&args, 3),
                arg_f64(&args, 4),
                arg_f64(&args, 5),
                arg_f64(&args, 6),
                arg_f64(&args, 7),
                arg_f64(&args, 8),
            ),
            _ => 0,
        };
        JsValue::from_f64(result as f64)
    }) as Box<dyn FnMut(u32, js_sys::Array) -> JsValue>);
    js_sys::Reflect::set(&js_sys::global(), &JsValue::from(DISPATCHER_SYMBOL), dispatcher.as_ref())
        .map_err(|e| GuestError::Instantiate(js_err(&e)))?;
    dispatcher.forget();
    Ok(())
}
