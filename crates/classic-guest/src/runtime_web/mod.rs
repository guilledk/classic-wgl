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

mod dispatch;
mod mem;

use std::cell::RefCell;
use std::rc::Rc;

use classic_core::pathfinder::PathPoll;
use classic_engine::Engine;
use js_sys::WebAssembly::{Instance, Memory, Module};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};

use crate::abi;
use crate::runtime::{GuestError, GuestLimits, GuestRuntime};
use crate::sdk::GuestHost;

use dispatch::{
    DISPATCHER_SYMBOL, OP_BILLOW_FIELD, OP_FBM_FIELD, OP_LIGHT_SET, OP_LIGHT_SPAWN, OP_NOISE_FIELD,
    OP_RIDGED_FIELD, OP_SET_LIGHT, OP_SPAWN_RECT, OP_SPAWN_TEXT, OP_TILING_FIELD, OP_UI_ARRAY,
    OP_UI_BUTTON, OP_UI_PADDING, OP_UI_SPRITE, OP_UI_TEXT,
};
use mem::{js_err, read_bytes, read_str, write_bytes, write_f64_pair, write_f64_triple, write_str};

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

        dispatch::install_dispatcher(host, mem)?;

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

        // set_sprite_frame
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "set_sprite_frame",
                Box::new(move |ptr: i32, len: i32, frame: f64| -> i32 {
                    let name = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), ptr, len)
                    };
                    host.borrow_mut().set_sprite_frame(&name, frame)
                }) as Box<dyn FnMut(i32, i32, f64) -> i32>
            );
        }

        // get_sprite_frame
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "get_sprite_frame",
                Box::new(move |ptr: i32, len: i32| -> f64 {
                    let name = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), ptr, len)
                    };
                    host.borrow_mut().get_sprite_frame(&name)
                }) as Box<dyn FnMut(i32, i32) -> f64>
            );
        }

        // set_sprite_color
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "set_sprite_color",
                Box::new(move |ptr: i32, len: i32, r: f64, g: f64, b: f64, a: f64| -> i32 {
                    let name = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), ptr, len)
                    };
                    host.borrow_mut().set_sprite_color(&name, r, g, b, a)
                }) as Box<dyn FnMut(i32, i32, f64, f64, f64, f64) -> i32>
            );
        }

        // set_sprite_offset
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "set_sprite_offset",
                Box::new(move |ptr: i32, len: i32, dx: f64, dy: f64, dz: f64| -> i32 {
                    let name = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), ptr, len)
                    };
                    host.borrow_mut().set_sprite_offset(&name, dx, dy, dz)
                }) as Box<dyn FnMut(i32, i32, f64, f64, f64) -> i32>
            );
        }

        // spawn_sprite_clone
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "spawn_sprite_clone",
                Box::new(move |t_ptr: i32, t_len: i32, n_ptr: i32, n_len: i32| -> i32 {
                    let (template, name) = {
                        let mem = mem.borrow();
                        let m = mem.as_ref().unwrap();
                        (read_str(m, t_ptr, t_len), read_str(m, n_ptr, n_len))
                    };
                    host.borrow_mut().spawn_sprite_clone(&template, &name)
                }) as Box<dyn FnMut(i32, i32, i32, i32) -> i32>
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

        // iso_to_screen
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "iso_to_screen",
                Box::new(move |x: f64, y: f64, out_ptr: i32| -> i32 {
                    let Some((sx, sy)) = host.borrow_mut().iso_to_screen(x, y) else {
                        return 0;
                    };
                    let mem = mem.borrow();
                    write_f64_pair(mem.as_ref().unwrap(), out_ptr, sx, sy);
                    1
                }) as Box<dyn FnMut(f64, f64, i32) -> i32>
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

        // start_anim
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "start_anim",
                Box::new(
                    move |ptr: i32, len: i32, anim_ptr: i32, anim_len: i32, repeat: i32| -> i32 {
                        let (name, anim) = {
                            let mem = mem.borrow();
                            let m = mem.as_ref().unwrap();
                            (read_str(m, ptr, len), read_str(m, anim_ptr, anim_len))
                        };
                        host.borrow_mut().start_anim(&name, &anim, repeat)
                    },
                ) as Box<dyn FnMut(i32, i32, i32, i32, i32) -> i32>
            );
        }

        // set_enabled
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "set_enabled",
                Box::new(move |ptr: i32, len: i32, enabled: i32| -> i32 {
                    let name = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), ptr, len)
                    };
                    host.borrow_mut().set_enabled(&name, enabled)
                }) as Box<dyn FnMut(i32, i32, i32) -> i32>
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

        // request_path
        {
            let host = host.clone();
            set_import!(
                "request_path",
                Box::new(move |sx: i32, sy: i32, ex: i32, ey: i32| -> i32 {
                    host.borrow_mut().request_path(sx, sy, ex, ey)
                }) as Box<dyn FnMut(i32, i32, i32, i32) -> i32>
            );
        }

        // poll_path
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "poll_path",
                Box::new(move |id: i32, out_ptr: i32, out_cap: i32| -> i32 {
                    match host.borrow_mut().poll_path(id) {
                        PathPoll::Pending => 0,
                        PathPoll::NoPath => -1,
                        PathPoll::Path(cells) => {
                            let bytes = abi::path_cells_bytes(&cells);
                            if bytes.len() > out_cap.max(0) as usize {
                                return -2;
                            }
                            let mem = mem.borrow();
                            write_bytes(mem.as_ref().unwrap(), out_ptr, &bytes);
                            cells.len() as i32
                        }
                    }
                }) as Box<dyn FnMut(i32, i32, i32) -> i32>
            );
        }

        // spawn_task
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "spawn_task",
                Box::new(move |entry_ptr: i32, entry_len: i32, arg_ptr: i32, arg_len: i32| -> i32 {
                    let mem = mem.borrow();
                    let entry = read_str(mem.as_ref().unwrap(), entry_ptr, entry_len);
                    let arg = read_bytes(mem.as_ref().unwrap(), arg_ptr, arg_len);
                    host.borrow_mut().spawn_task(&entry, &arg)
                }) as Box<dyn FnMut(i32, i32, i32, i32) -> i32>
            );
        }

        // poll_task
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "poll_task",
                Box::new(move |id: i32, out_ptr: i32, out_cap: i32| -> i32 {
                    match host.borrow_mut().poll_task(id) {
                        None => 0,
                        Some(Err(e)) => {
                            host.borrow_mut().log(&format!("task {id} failed: {e}"));
                            -1
                        }
                        Some(Ok(bytes)) => {
                            if bytes.len() > out_cap.max(0) as usize {
                                return -2;
                            }
                            let mem = mem.borrow();
                            write_bytes(mem.as_ref().unwrap(), out_ptr, &bytes);
                            bytes.len() as i32
                        }
                    }
                }) as Box<dyn FnMut(i32, i32, i32) -> i32>
            );
        }

        // vehicle_teleport
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "vehicle_teleport",
                Box::new(move |ptr: i32, len: i32, x: f64, y: f64| -> i32 {
                    let name = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), ptr, len)
                    };
                    host.borrow_mut().vehicle_teleport(&name, x, y)
                }) as Box<dyn FnMut(i32, i32, f64, f64) -> i32>
            );
        }

        // vehicle_spawn
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "vehicle_spawn",
                Box::new(
                    move |def_ptr: i32,
                          def_len: i32,
                          name_ptr: i32,
                          name_len: i32,
                          x: f64,
                          y: f64|
                          -> i32 {
                        let (def, name) = {
                            let mem = mem.borrow();
                            let m = mem.as_ref().unwrap();
                            (read_str(m, def_ptr, def_len), read_str(m, name_ptr, name_len))
                        };
                        host.borrow_mut().vehicle_spawn(&def, &name, x, y)
                    },
                ) as Box<dyn FnMut(i32, i32, i32, i32, f64, f64) -> i32>
            );
        }

        // vehicle_goto
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "vehicle_goto",
                Box::new(move |ptr: i32, len: i32, tx: i32, ty: i32| -> i32 {
                    let name = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), ptr, len)
                    };
                    host.borrow_mut().vehicle_goto(&name, tx, ty)
                }) as Box<dyn FnMut(i32, i32, i32, i32) -> i32>
            );
        }

        // vehicle_goto_poll
        {
            let host = host.clone();
            set_import!(
                "vehicle_goto_poll",
                Box::new(move |id: i32| -> i32 { host.borrow_mut().vehicle_goto_poll(id) })
                    as Box<dyn FnMut(i32) -> i32>
            );
        }

        // vehicle_stop
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "vehicle_stop",
                Box::new(move |ptr: i32, len: i32| -> i32 {
                    let name = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), ptr, len)
                    };
                    host.borrow_mut().vehicle_stop(&name)
                }) as Box<dyn FnMut(i32, i32) -> i32>
            );
        }

        // vehicle_set_speed
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "vehicle_set_speed",
                Box::new(move |ptr: i32, len: i32, speed: f64| -> i32 {
                    let name = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), ptr, len)
                    };
                    host.borrow_mut().vehicle_set_speed(&name, speed)
                }) as Box<dyn FnMut(i32, i32, f64) -> i32>
            );
        }

        // vehicle_probe
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "vehicle_probe",
                Box::new(move |ptr: i32, len: i32, tx: i32, ty: i32| -> i32 {
                    let name = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), ptr, len)
                    };
                    host.borrow_mut().vehicle_probe(&name, tx, ty)
                }) as Box<dyn FnMut(i32, i32, i32, i32) -> i32>
            );
        }

        // vehicle_probe_clear
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "vehicle_probe_clear",
                Box::new(move |ptr: i32, len: i32| -> i32 {
                    let name = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), ptr, len)
                    };
                    host.borrow_mut().vehicle_probe_clear(&name)
                }) as Box<dyn FnMut(i32, i32) -> i32>
            );
        }

        // vehicle_footprint_radius
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "vehicle_footprint_radius",
                Box::new(move |ptr: i32, len: i32| -> f64 {
                    let name = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), ptr, len)
                    };
                    host.borrow_mut().vehicle_footprint_radius(&name)
                }) as Box<dyn FnMut(i32, i32) -> f64>
            );
        }

        // selected_names
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "selected_names",
                Box::new(move |out_ptr: i32, out_cap: i32| -> i32 {
                    let json = host.borrow_mut().selected_names();
                    if out_cap < json.len() as i32 {
                        return -1;
                    }
                    let mem = mem.borrow();
                    write_str(mem.as_ref().unwrap(), out_ptr, &json)
                }) as Box<dyn FnMut(i32, i32) -> i32>
            );
        }

        // selection_clear
        {
            let host = host.clone();
            set_import!(
                "selection_clear",
                Box::new(move || -> i32 { host.borrow_mut().selection_clear() })
                    as Box<dyn FnMut() -> i32>
            );
        }

        // inventory_dump
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "inventory_dump",
                Box::new(move |ptr: i32, len: i32, out_ptr: i32, out_cap: i32| -> i32 {
                    let name = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), ptr, len)
                    };
                    let json = host.borrow_mut().inventory_dump(&name);
                    if out_cap < json.len() as i32 {
                        return -1;
                    }
                    let mem = mem.borrow();
                    write_str(mem.as_ref().unwrap(), out_ptr, &json)
                }) as Box<dyn FnMut(i32, i32, i32, i32) -> i32>
            );
        }

        // inventory_capacity
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "inventory_capacity",
                Box::new(move |ptr: i32, len: i32| -> i32 {
                    let name = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), ptr, len)
                    };
                    host.borrow_mut().inventory_capacity(&name)
                }) as Box<dyn FnMut(i32, i32) -> i32>
            );
        }

        // inventory_add
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "inventory_add",
                Box::new(move |ptr: i32, len: i32, item_ptr: i32, item_len: i32, n: i32| -> i32 {
                    let (name, item) = {
                        let mem = mem.borrow();
                        let m = mem.as_ref().unwrap();
                        (read_str(m, ptr, len), read_str(m, item_ptr, item_len))
                    };
                    host.borrow_mut().inventory_add(&name, &item, n)
                }) as Box<dyn FnMut(i32, i32, i32, i32, i32) -> i32>
            );
        }

        // inventory_remove
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "inventory_remove",
                Box::new(move |ptr: i32, len: i32, item_ptr: i32, item_len: i32, n: i32| -> i32 {
                    let (name, item) = {
                        let mem = mem.borrow();
                        let m = mem.as_ref().unwrap();
                        (read_str(m, ptr, len), read_str(m, item_ptr, item_len))
                    };
                    host.borrow_mut().inventory_remove(&name, &item, n)
                }) as Box<dyn FnMut(i32, i32, i32, i32, i32) -> i32>
            );
        }

        // inventory_transfer
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "inventory_transfer",
                Box::new(
                    move |from_ptr: i32,
                          from_len: i32,
                          to_ptr: i32,
                          to_len: i32,
                          item_ptr: i32,
                          item_len: i32,
                          n: i32|
                          -> i32 {
                        let (from, to, item) = {
                            let mem = mem.borrow();
                            let m = mem.as_ref().unwrap();
                            (
                                read_str(m, from_ptr, from_len),
                                read_str(m, to_ptr, to_len),
                                read_str(m, item_ptr, item_len),
                            )
                        };
                        host.borrow_mut().inventory_transfer(&from, &to, &item, n)
                    }
                ) as Box<dyn FnMut(i32, i32, i32, i32, i32, i32, i32) -> i32>
            );
        }

        // item_def
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "item_def",
                Box::new(move |ptr: i32, len: i32, out_ptr: i32, out_cap: i32| -> i32 {
                    let name = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), ptr, len)
                    };
                    let json = host.borrow_mut().item_def(&name);
                    if out_cap < json.len() as i32 {
                        return -1;
                    }
                    let mem = mem.borrow();
                    write_str(mem.as_ref().unwrap(), out_ptr, &json)
                }) as Box<dyn FnMut(i32, i32, i32, i32) -> i32>
            );
        }

        // inventory_ui_show
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "inventory_ui_show",
                Box::new(move |ptr: i32, len: i32| -> i32 {
                    let name = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), ptr, len)
                    };
                    host.borrow_mut().inventory_ui_show(&name)
                }) as Box<dyn FnMut(i32, i32) -> i32>
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

        // set_grid
        {
            let host = host.clone();
            set_import!(
                "set_grid",
                Box::new(move |show: i32| -> i32 { host.borrow_mut().set_grid(show) })
                    as Box<dyn FnMut(i32) -> i32>
            );
        }

        // pick_at
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "pick_at",
                Box::new(
                    move |x: f64,
                          y: f64,
                          filter_ptr: i32,
                          filter_len: i32,
                          out_ptr: i32,
                          out_cap: i32|
                          -> i32 {
                        let filter = {
                            let mem = mem.borrow();
                            read_str(mem.as_ref().unwrap(), filter_ptr, filter_len)
                        };
                        let name = host.borrow_mut().pick_at(x, y, &filter);
                        if out_cap < name.len() as i32 {
                            return -1;
                        }
                        let mem = mem.borrow();
                        write_str(mem.as_ref().unwrap(), out_ptr, &name)
                    },
                ) as Box<dyn FnMut(f64, f64, i32, i32, i32, i32) -> i32>
            );
        }

        // set_collider_blocks_nav
        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "set_collider_blocks_nav",
                Box::new(move |ptr: i32, len: i32, blocks: i32| -> i32 {
                    let name = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), ptr, len)
                    };
                    host.borrow_mut().set_collider_blocks_nav(&name, blocks)
                }) as Box<dyn FnMut(i32, i32, i32) -> i32>
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

        // light_spawn / light_set (high-arity), light_release (direct)
        set_import_str!("light_spawn", OP_LIGHT_SPAWN, "a,b,c,d,e,f,g,h,i,j,k");
        set_import_str!("light_set", OP_LIGHT_SET, "a,b,c,d,e,f,g,h,i");
        {
            let host = host.clone();
            set_import!(
                "light_release",
                Box::new(move |handle: i32| -> i32 { host.borrow_mut().light_release(handle) })
                    as Box<dyn FnMut(i32) -> i32>
            );
        }

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
            set_import!(
                "commit_terrain",
                Box::new(move |hs: f64| -> i32 { host.borrow_mut().commit_terrain(hs) })
                    as Box<dyn FnMut(f64) -> i32>
            );
        }

        // ---- Field-buffer registry + grid kernels --------------------------

        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "alloc_field",
                Box::new(move |ptr: i32, len: i32, w: i32, h: i32, dtype: i32| -> i32 {
                    let name = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), ptr, len)
                    };
                    host.borrow_mut().alloc_field(&name, w, h, dtype)
                }) as Box<dyn FnMut(i32, i32, i32, i32, i32) -> i32>
            );
        }

        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "free_field",
                Box::new(move |ptr: i32, len: i32| -> i32 {
                    let name = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), ptr, len)
                    };
                    host.borrow_mut().free_field(&name)
                }) as Box<dyn FnMut(i32, i32) -> i32>
            );
        }

        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "write_field",
                Box::new(move |ptr: i32, len: i32, data_ptr: i32, data_len: i32| -> i32 {
                    let (name, data) = {
                        let mem = mem.borrow();
                        let m = mem.as_ref().unwrap();
                        (
                            read_str(m, ptr, len),
                            abi::bytes_to_f32(&read_bytes(m, data_ptr, data_len)),
                        )
                    };
                    host.borrow_mut().write_field(&name, &data)
                }) as Box<dyn FnMut(i32, i32, i32, i32) -> i32>
            );
        }

        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "write_field_u32",
                Box::new(move |ptr: i32, len: i32, data_ptr: i32, data_len: i32| -> i32 {
                    let (name, data) = {
                        let mem = mem.borrow();
                        let m = mem.as_ref().unwrap();
                        (
                            read_str(m, ptr, len),
                            abi::bytes_to_u32(&read_bytes(m, data_ptr, data_len)),
                        )
                    };
                    host.borrow_mut().write_field_u32(&name, &data)
                }) as Box<dyn FnMut(i32, i32, i32, i32) -> i32>
            );
        }

        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "read_field",
                Box::new(move |ptr: i32, len: i32, out_ptr: i32, out_cap: i32| -> i32 {
                    let name = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), ptr, len)
                    };
                    let field = host.borrow_mut().read_field(&name);
                    let bytes = abi::f32_array_bytes(&field);
                    if bytes.len() > out_cap.max(0) as usize {
                        return -1;
                    }
                    let mem = mem.borrow();
                    write_bytes(mem.as_ref().unwrap(), out_ptr, &bytes);
                    bytes.len() as i32
                }) as Box<dyn FnMut(i32, i32, i32, i32) -> i32>
            );
        }

        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "map_field",
                Box::new(
                    move |op: i32, dst_ptr: i32, dst_len: i32, src_ptr: i32, src_len: i32| -> i32 {
                        let (dst, src) = {
                            let mem = mem.borrow();
                            let m = mem.as_ref().unwrap();
                            (read_str(m, dst_ptr, dst_len), read_str(m, src_ptr, src_len))
                        };
                        host.borrow_mut().map_field(op, &dst, &src)
                    },
                ) as Box<dyn FnMut(i32, i32, i32, i32, i32) -> i32>
            );
        }

        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "map_scalar",
                Box::new(move |op: i32, dst_ptr: i32, dst_len: i32, scalar: f64| -> i32 {
                    let dst = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), dst_ptr, dst_len)
                    };
                    host.borrow_mut().map_scalar(op, &dst, scalar)
                }) as Box<dyn FnMut(i32, i32, i32, f64) -> i32>
            );
        }

        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "blur_box_field",
                Box::new(move |ptr: i32, len: i32, radius: i32| -> i32 {
                    let name = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), ptr, len)
                    };
                    host.borrow_mut().blur_box_field(&name, radius)
                }) as Box<dyn FnMut(i32, i32, i32) -> i32>
            );
        }

        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "relax_slopes_field",
                Box::new(
                    move |ptr: i32,
                          len: i32,
                          max_slope: f64,
                          iterations: i32,
                          tolerance: f64,
                          pinned_ptr: i32,
                          pinned_len: i32|
                          -> f64 {
                        let (name, pinned) = {
                            let mem = mem.borrow();
                            let m = mem.as_ref().unwrap();
                            (read_str(m, ptr, len), read_str(m, pinned_ptr, pinned_len))
                        };
                        host.borrow_mut()
                            .relax_slopes_field(&name, max_slope, iterations, tolerance, &pinned)
                    },
                ) as Box<dyn FnMut(i32, i32, f64, i32, f64, i32, i32) -> f64>
            );
        }

        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "gradient_magnitude_field",
                Box::new(
                    move |heights_ptr: i32, heights_len: i32, dst_ptr: i32, dst_len: i32| -> i32 {
                        let (heights, dst) = {
                            let mem = mem.borrow();
                            let m = mem.as_ref().unwrap();
                            (read_str(m, heights_ptr, heights_len), read_str(m, dst_ptr, dst_len))
                        };
                        host.borrow_mut().gradient_magnitude_field(&heights, &dst)
                    },
                ) as Box<dyn FnMut(i32, i32, i32, i32) -> i32>
            );
        }

        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "threshold_le_field",
                Box::new(
                    move |src_ptr: i32, src_len: i32, dst_ptr: i32, dst_len: i32, t: f64| -> i32 {
                        let (src, dst) = {
                            let mem = mem.borrow();
                            let m = mem.as_ref().unwrap();
                            (read_str(m, src_ptr, src_len), read_str(m, dst_ptr, dst_len))
                        };
                        host.borrow_mut().threshold_le_field(&src, &dst, t)
                    },
                ) as Box<dyn FnMut(i32, i32, i32, i32, f64) -> i32>
            );
        }

        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "prune_components_field",
                Box::new(move |ptr: i32, len: i32| -> i32 {
                    let name = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), ptr, len)
                    };
                    host.borrow_mut().prune_components_field(&name)
                }) as Box<dyn FnMut(i32, i32) -> i32>
            );
        }

        {
            let host = host.clone();
            let mem = mem.clone();
            set_import!(
                "reduce_field",
                Box::new(move |ptr: i32, len: i32, op: i32| -> f64 {
                    let name = {
                        let mem = mem.borrow();
                        read_str(mem.as_ref().unwrap(), ptr, len)
                    };
                    host.borrow_mut().reduce_field(&name, op)
                }) as Box<dyn FnMut(i32, i32, i32) -> f64>
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

    fn set_namespace(&mut self, namespace: &str) {
        self.host.borrow_mut().set_namespace(namespace);
    }
}
