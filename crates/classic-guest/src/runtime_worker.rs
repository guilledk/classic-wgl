//! The web Worker guest runtime (wasm only, untrusted guests).
//!
//! Untrusted guests run on the browser's native `WebAssembly` engine inside a
//! `Worker`, with host imports bridged synchronously to the main thread over a
//! `SharedArrayBuffer` + `Atomics` channel.  Because browser Wasm has no fuel
//! API, the main thread enforces a wall-clock budget per `update` and
//! `worker.terminate()`s the guest if it exceeds it (surfaced as
//! [`GuestError::FuelExhausted`]).
//!
//! The SAB layout (byte offsets) must match `worker.js` exactly.  The main
//! thread busy-polls (`Atomics::wait` is disallowed on the main thread); only
//! the Worker blocks on `Atomics::wait`.

use std::cell::RefCell;
use std::rc::Rc;

use classic_core::pathfinder::PathPoll;
use classic_engine::Engine;
use wasm_bindgen::JsValue;

use crate::runtime::{GuestError, GuestLimits, GuestRuntime};
use crate::sdk::GuestHost;

// SAB layout (shared with worker.js).
const CONTROL_BYTES: u32 = 64; // Int32Array, 16 slots
const NUM_BYTES: u32 = 128; // Float64Array, 16 slots
const STR_BYTES: u32 = 6144;
const OUT_BYTES: u32 = 65536;
const NUM_OFFSET: u32 = CONTROL_BYTES;
const STR_OFFSET: u32 = NUM_OFFSET + NUM_BYTES;
const OUT_OFFSET: u32 = STR_OFFSET + STR_BYTES;
const WASM_OFFSET: u32 = OUT_OFFSET + OUT_BYTES;
const SAB_SIZE: u32 = 1048576; // 1 MiB

// Int32 flag indices.
const I_REQ_READY: u32 = 0;
const I_RESP_READY: u32 = 1;
const I_DONE: u32 = 2;
const I_GO: u32 = 3;
const I_COMMAND: u32 = 4;
const I_WASM_LEN: u32 = 5;
const I_REQ_OP: u32 = 6;
const I_REQ_STR_LEN: u32 = 7;
const I_REQ_NUM_COUNT: u32 = 8;
const I_RESP_OUT_LEN: u32 = 9;

// Float64 slots (element offsets within the numeric region).
const F_DT: u32 = 12;
const F_RET: u32 = 13;

// Commands.
const CMD_INIT: i32 = 0;
const CMD_UPDATE: i32 = 1;
const CMD_START: i32 = 2;

// Host-import op codes (shared with worker.js).
const OP_LOG: i32 = 0;
const OP_SPAWN: i32 = 1;
const OP_DESPAWN: i32 = 2;
const OP_HAS: i32 = 3;
const OP_NAMES: i32 = 4;
const OP_SET_POS: i32 = 9;
const OP_GET_POS: i32 = 10;
const OP_MOUSE: i32 = 11;
const OP_MOUSE_ISO: i32 = 12;
const OP_HEIGHT_AT: i32 = 13;
const OP_SET_ANIM: i32 = 14;
const OP_AGENT_SELECTED: i32 = 15;
const OP_UI_CONSUMED_CLICK: i32 = 16;
const OP_DELTA: i32 = 17;
const OP_ELAPSED: i32 = 18;
const OP_WAS_PRESSED: i32 = 19;
const OP_KEY_DOWN: i32 = 20;
const OP_WAS_KEY_PRESSED: i32 = 21;
const OP_SET_TILE: i32 = 23;
const OP_SET_HEIGHT: i32 = 24;
const OP_REBUILD_TERRAIN: i32 = 25;
const OP_REQUEST_PATH: i32 = 26;
const OP_GET_CAMERA: i32 = 27;
const OP_SET_CAMERA: i32 = 28;
const OP_PICK_AT: i32 = 29;
const OP_MOUSE_DOWN: i32 = 30;
const OP_MOUSE_RELEASED: i32 = 31;
const OP_MOUSE_WHEEL: i32 = 32;
const OP_KEY_UP: i32 = 33;
const OP_GET_LIGHT: i32 = 34;
const OP_SET_LIGHT: i32 = 35;
const OP_SPAWN_RECT: i32 = 36;
const OP_SPAWN_TEXT: i32 = 37;
const OP_SET_TEXT: i32 = 38;
const OP_UI_CONTAINER: i32 = 39;
const OP_UI_TEXT: i32 = 40;
const OP_UI_BUTTON: i32 = 41;
const OP_UI_ARRAY: i32 = 42;
const OP_UI_PADDING: i32 = 43;
const OP_UI_SPRITE: i32 = 44;
const OP_UI_ADD_CHILD: i32 = 45;
const OP_UI_ADD_TO_ROOT: i32 = 46;
const OP_UI_SET_SIZE: i32 = 47;
const OP_UI_SET_ANCHOR: i32 = 48;
const OP_UI_SET_COLOR: i32 = 49;
const OP_UI_SET_FIXED: i32 = 50;
const OP_SUBSCRIBE: i32 = 51;
const OP_POLL_EVENT: i32 = 52;
const OP_SPAWN_COLLIDER: i32 = 53;
const OP_GET_ANIM: i32 = 54;
const OP_HAS_RESOURCE: i32 = 55;
const OP_TEXTURE_SIZE: i32 = 56;
const OP_FBM_FIELD: i32 = 57;
const OP_RIDGED_FIELD: i32 = 58;
const OP_BILLOW_FIELD: i32 = 59;
const OP_TILING_FIELD: i32 = 60;
const OP_NOISE_FIELD: i32 = 61;
const OP_NOISE2D: i32 = 62;
const OP_SET_TILES: i32 = 63;
const OP_SET_HEIGHTS: i32 = 64;
const OP_SET_NAV: i32 = 65;
const OP_SET_TILESET: i32 = 66;
const OP_COMMIT_TERRAIN: i32 = 68;
const OP_ISO_TO_SCREEN: i32 = 69;
const OP_SET_GRID: i32 = 70;
const OP_START_ANIM: i32 = 71;
const OP_VEHICLE_TELEPORT: i32 = 72;
const OP_VEHICLE_GOTO: i32 = 73;
const OP_VEHICLE_STOP: i32 = 74;
const OP_VEHICLE_SPAWN: i32 = 75;
const OP_POLL_PATH: i32 = 76;
const OP_SET_SPRITE_FRAME: i32 = 77;
const OP_SET_SPRITE_COLOR: i32 = 78;
const OP_SPAWN_SPRITE_CLONE: i32 = 79;
const OP_SET_ENABLED: i32 = 80;

const WORKER_SRC: &str = include_str!("worker.js");

fn js_err(e: &JsValue) -> String {
    e.as_string().unwrap_or_else(|| format!("{e:?}"))
}

fn sab_available() -> bool {
    js_sys::eval("(function(){ try { new SharedArrayBuffer(1); return true; } catch (e) { return false; } })()")
        .map(|v| v.as_bool().unwrap_or(false))
        .unwrap_or(false)
}

/// Browser-native [`GuestRuntime`] running an untrusted guest in a Worker.
pub struct WorkerWasmRuntime {
    host: Rc<RefCell<GuestHost>>,
    worker: web_sys::Worker,
    flags: js_sys::Int32Array,
    bytes: js_sys::Uint8Array,
    limits: GuestLimits,
}

impl WorkerWasmRuntime {
    fn flag_load(&self, idx: u32) -> i32 {
        js_sys::Atomics::load(&self.flags, idx).unwrap_or(0)
    }

    fn flag_store(&self, idx: u32, val: i32) {
        let _ = js_sys::Atomics::store(&self.flags, idx, val);
    }

    fn notify(&self, idx: u32) {
        let _ = js_sys::Atomics::notify(&self.flags, idx);
    }

    fn read_u8(&self, offset: u32, len: u32) -> Vec<u8> {
        self.bytes.subarray(offset, offset + len).to_vec()
    }

    fn write_u8(&self, offset: u32, data: &[u8]) {
        self.bytes.subarray(offset, offset + data.len() as u32).copy_from(data);
    }

    fn read_f64(&self, offset: u32) -> f64 {
        let raw = self.read_u8(offset, 8);
        f64::from_le_bytes(raw.try_into().unwrap_or([0u8; 8]))
    }

    fn write_f64(&self, offset: u32, v: f64) {
        self.write_u8(offset, &v.to_le_bytes());
    }

    fn read_nums(&self) -> Vec<f64> {
        let count = self.flag_load(I_REQ_NUM_COUNT).max(0) as u32;
        (0..count).map(|i| self.read_f64(NUM_OFFSET + i * 8)).collect()
    }

    /// Service one host-import request against the engine.
    fn service(&mut self) {
        let op = self.flag_load(I_REQ_OP);
        let raw = self.read_u8(STR_OFFSET, self.flag_load(I_REQ_STR_LEN).max(0) as u32);
        let strs = decode_strings(&raw);
        let nums = self.read_nums();
        self.flag_store(I_REQ_READY, 0);
        let (ret, out) = self.dispatch(op, &strs, &nums, &raw);
        self.write_f64(NUM_OFFSET + F_RET * 8, ret);
        match out {
            Some(bytes) => {
                self.write_u8(OUT_OFFSET, &bytes);
                self.flag_store(I_RESP_OUT_LEN, bytes.len() as i32);
            }
            None => {
                self.flag_store(I_RESP_OUT_LEN, 0);
            }
        }
        self.flag_store(I_RESP_READY, 1);
        self.notify(I_RESP_READY);
    }

    fn dispatch(
        &mut self,
        op: i32,
        strs: &[String],
        nums: &[f64],
        raw: &[u8],
    ) -> (f64, Option<Vec<u8>>) {
        let nf = |i: usize| nums.get(i).copied().unwrap_or(0.0);
        let ni = |i: usize| nf(i) as i32;
        let enc_f64s = |v: &[f64]| -> Vec<u8> {
            let mut b = Vec::with_capacity(v.len() * 8);
            for x in v {
                b.extend_from_slice(&x.to_le_bytes());
            }
            b
        };
        let mut host = self.host.borrow_mut();

        match op {
            OP_LOG => {
                host.log(&strs[0]);
                (0.0, None)
            }
            OP_SPAWN => (host.spawn(&strs[0]) as f64, None),
            OP_DESPAWN => (host.despawn(&strs[0]) as f64, None),
            OP_HAS => (host.has(&strs[0]) as f64, None),
            OP_NAMES => {
                let json = host.names();
                if ni(1) < json.len() as i32 || json.len() as u32 > OUT_BYTES {
                    (-1.0, None)
                } else {
                    (json.len() as f64, Some(json.into_bytes()))
                }
            }
            OP_SET_POS => (host.set_pos(&strs[0], nf(0), nf(1), nf(2)) as f64, None),
            OP_GET_POS => match host.get_pos(&strs[0]) {
                Some((x, y, z)) => (1.0, Some(enc_f64s(&[x, y, z]))),
                None => (0.0, None),
            },
            OP_MOUSE => {
                let (x, y) = host.mouse();
                (1.0, Some(enc_f64s(&[x, y])))
            }
            OP_MOUSE_ISO => match host.mouse_iso() {
                Some((x, y)) => (1.0, Some(enc_f64s(&[x, y]))),
                None => (0.0, None),
            },
            OP_ISO_TO_SCREEN => match host.iso_to_screen(nf(0), nf(1)) {
                Some((sx, sy)) => (1.0, Some(enc_f64s(&[sx, sy]))),
                None => (0.0, None),
            },
            OP_HEIGHT_AT => (host.height_at(nf(0), nf(1)), None),
            OP_SET_ANIM => (host.set_anim(&strs[0], &strs[1]) as f64, None),
            OP_START_ANIM => (host.start_anim(&strs[0], &strs[1], ni(0)) as f64, None),
            OP_AGENT_SELECTED => (host.agent_selected() as f64, None),
            OP_UI_CONSUMED_CLICK => (host.ui_consumed_click() as f64, None),
            OP_DELTA => (host.delta(), None),
            OP_ELAPSED => (host.elapsed(), None),
            OP_WAS_PRESSED => (host.was_pressed(ni(0)) as f64, None),
            OP_KEY_DOWN => (host.key_down(&strs[0]) as f64, None),
            OP_WAS_KEY_PRESSED => (host.was_key_pressed(&strs[0]) as f64, None),
            OP_SET_TILE => (host.set_tile(ni(0), ni(1), ni(2)) as f64, None),
            OP_SET_HEIGHT => (host.set_height(ni(0), ni(1), nf(2)) as f64, None),
            OP_REBUILD_TERRAIN => (host.rebuild_terrain() as f64, None),
            OP_REQUEST_PATH => (host.request_path(ni(0), ni(1), ni(2), ni(3)) as f64, None),
            OP_POLL_PATH => match host.poll_path(ni(0)) {
                PathPoll::Pending => (0.0, None),
                PathPoll::NoPath => (-1.0, None),
                PathPoll::Path(cells) => {
                    let bytes = crate::abi::path_cells_bytes(&cells);
                    if bytes.len() > ni(2).max(0) as usize || bytes.len() as u32 > OUT_BYTES {
                        (-2.0, None)
                    } else {
                        (cells.len() as f64, Some(bytes))
                    }
                }
            },
            OP_VEHICLE_TELEPORT => (host.vehicle_teleport(&strs[0], nf(0), nf(1)) as f64, None),
            OP_VEHICLE_GOTO => (host.vehicle_goto(&strs[0], ni(0), ni(1)) as f64, None),
            OP_VEHICLE_STOP => (host.vehicle_stop(&strs[0]) as f64, None),
            OP_VEHICLE_SPAWN => (host.vehicle_spawn(&strs[0], &strs[1], nf(0), nf(1)) as f64, None),
            OP_SET_SPRITE_FRAME => (host.set_sprite_frame(&strs[0], nf(0)) as f64, None),
            OP_SET_SPRITE_COLOR => {
                (host.set_sprite_color(&strs[0], nf(0), nf(1), nf(2), nf(3)) as f64, None)
            }
            OP_SPAWN_SPRITE_CLONE => (host.spawn_sprite_clone(&strs[0], &strs[1]) as f64, None),
            OP_SET_ENABLED => (host.set_enabled(&strs[0], ni(0)) as f64, None),
            OP_GET_CAMERA => {
                let (x, y, s) = host.get_camera();
                (1.0, Some(enc_f64s(&[x, y, s])))
            }
            OP_SET_CAMERA => {
                let _ = host.set_camera(nf(0), nf(1), nf(2));
                (1.0, None)
            }
            OP_SET_GRID => (host.set_grid(ni(0)) as f64, None),
            OP_PICK_AT => {
                let name = host.pick_at(nf(0), nf(1));
                if ni(3) < name.len() as i32 || name.len() as u32 > OUT_BYTES {
                    (-1.0, None)
                } else {
                    (name.len() as f64, Some(name.into_bytes()))
                }
            }
            OP_MOUSE_DOWN => (host.mouse_down(ni(0)) as f64, None),
            OP_MOUSE_RELEASED => (host.mouse_released(ni(0)) as f64, None),
            OP_MOUSE_WHEEL => (host.mouse_wheel(), None),
            OP_KEY_UP => (host.key_up(&strs[0]) as f64, None),
            OP_GET_LIGHT => {
                let (a, d, c) = host.get_light();
                let vals: Vec<f64> = a.iter().chain(d.iter()).chain(c.iter()).copied().collect();
                (1.0, Some(enc_f64s(&vals)))
            }
            OP_SET_LIGHT => (
                host.set_light(nf(0), nf(1), nf(2), nf(3), nf(4), nf(5), nf(6), nf(7), nf(8))
                    as f64,
                None,
            ),
            OP_SPAWN_RECT => (
                host.spawn_rect(&strs[0], nf(0), nf(1), nf(2), nf(3), nf(4), nf(5), nf(6), nf(7))
                    as f64,
                None,
            ),
            OP_SPAWN_TEXT => (
                host.spawn_text(&strs[0], nf(0), nf(1), &strs[1], nf(2), nf(3), nf(4), nf(5), nf(6))
                    as f64,
                None,
            ),
            OP_SET_TEXT => (host.set_text(&strs[0], &strs[1]) as f64, None),
            OP_UI_CONTAINER => {
                (host.ui_container(&strs[0], nf(0), nf(1), nf(2), nf(3), nf(4), nf(5)) as f64, None)
            }
            OP_UI_TEXT => (
                host.ui_text(&strs[0], &strs[1], nf(0), nf(1), nf(2), nf(3), nf(4), nf(5), ni(6))
                    as f64,
                None,
            ),
            OP_UI_BUTTON => (
                host.ui_button(&strs[0], &strs[1], nf(0), nf(1), nf(2), nf(3), nf(4), nf(5)) as f64,
                None,
            ),
            OP_UI_ARRAY => (
                host.ui_array(&strs[0], ni(0), ni(1), nf(2), nf(3), nf(4), nf(5), nf(6)) as f64,
                None,
            ),
            OP_UI_PADDING => (
                host.ui_padding(&strs[0], nf(0), nf(1), nf(2), nf(3), nf(4), nf(5), nf(6), nf(7))
                    as f64,
                None,
            ),
            OP_UI_SPRITE => {
                (host.ui_sprite(&strs[0], &strs[1], nf(0), nf(1), nf(2), nf(3), nf(4)) as f64, None)
            }
            OP_UI_ADD_CHILD => (host.ui_add_child(&strs[0], &strs[1], ni(0), ni(1)) as f64, None),
            OP_UI_ADD_TO_ROOT => (host.ui_add_to_root(&strs[0], ni(0), ni(1)) as f64, None),
            OP_UI_SET_SIZE => (host.ui_set_size(&strs[0], nf(0), nf(1)) as f64, None),
            OP_UI_SET_ANCHOR => (host.ui_set_anchor(&strs[0], ni(0)) as f64, None),
            OP_UI_SET_COLOR => {
                (host.ui_set_color(&strs[0], nf(0), nf(1), nf(2), nf(3)) as f64, None)
            }
            OP_UI_SET_FIXED => (host.ui_set_fixed(&strs[0], ni(0)) as f64, None),
            OP_SUBSCRIBE => (host.subscribe(&strs[0]) as f64, None),
            OP_POLL_EVENT => match host.poll_event() {
                Some((kind, name)) => {
                    let mut bytes = Vec::with_capacity(8 + name.len());
                    bytes.extend_from_slice(&kind.to_le_bytes());
                    bytes.extend_from_slice(&(name.len() as u32).to_le_bytes());
                    bytes.extend_from_slice(name.as_bytes());
                    if bytes.len() > ni(1).max(0) as usize || bytes.len() as u32 > OUT_BYTES {
                        (-1.0, None)
                    } else {
                        (1.0, Some(bytes))
                    }
                }
                None => (0.0, None),
            },
            OP_SPAWN_COLLIDER => {
                (host.spawn_collider(&strs[0], nf(0), nf(1), nf(2), nf(3)) as f64, None)
            }
            OP_GET_ANIM => match host.get_anim(&strs[0]) {
                Some((anim, frame)) => {
                    let mut bytes = Vec::with_capacity(12 + anim.len());
                    bytes.extend_from_slice(&frame.to_le_bytes());
                    bytes.extend_from_slice(&(anim.len() as u32).to_le_bytes());
                    bytes.extend_from_slice(anim.as_bytes());
                    if bytes.len() > ni(2).max(0) as usize || bytes.len() as u32 > OUT_BYTES {
                        (-1.0, None)
                    } else {
                        (1.0, Some(bytes))
                    }
                }
                None => (0.0, None),
            },
            OP_HAS_RESOURCE => (host.has_resource(ni(0), &strs[0]) as f64, None),
            OP_TEXTURE_SIZE => match host.texture_size(&strs[0]) {
                Some((w, h)) => (1.0, Some(enc_f64s(&[w, h]))),
                None => (0.0, None),
            },
            OP_FBM_FIELD => {
                let field = host.fbm_field(
                    ni(0),
                    ni(1),
                    &strs[0],
                    ni(2).max(0) as u32,
                    nf(3),
                    nf(4),
                    nf(5),
                );
                let bytes = crate::abi::f32_array_bytes(&field);
                out_or_err(bytes, ni(6))
            }
            OP_RIDGED_FIELD => {
                let field = host.ridged_field(
                    ni(0),
                    ni(1),
                    &strs[0],
                    ni(2).max(0) as u32,
                    nf(3),
                    nf(4),
                    nf(5),
                    nf(6),
                );
                let bytes = crate::abi::f32_array_bytes(&field);
                out_or_err(bytes, ni(7))
            }
            OP_BILLOW_FIELD => {
                let field = host.billow_field(
                    ni(0),
                    ni(1),
                    &strs[0],
                    ni(2).max(0) as u32,
                    nf(3),
                    nf(4),
                    nf(5),
                );
                let bytes = crate::abi::f32_array_bytes(&field);
                out_or_err(bytes, ni(6))
            }
            OP_TILING_FIELD => {
                let field =
                    host.tiling_field(ni(0), ni(1), &strs[0], nf(2), ni(3).max(0) as u32, nf(4));
                let bytes = crate::abi::f32_array_bytes(&field);
                out_or_err(bytes, ni(5))
            }
            OP_NOISE_FIELD => {
                let field = host.noise_field(ni(0), ni(1), &strs[0], nf(2), nf(3));
                let bytes = crate::abi::f32_array_bytes(&field);
                out_or_err(bytes, ni(4))
            }
            OP_NOISE2D => (host.noise2d(&strs[0], nf(0), nf(1)), None),
            OP_SET_TILES => (host.set_tiles(&crate::abi::bytes_to_u32(raw)) as f64, None),
            OP_SET_HEIGHTS => (host.set_heights(&crate::abi::bytes_to_f32(raw)) as f64, None),
            OP_SET_NAV => (host.set_nav(&crate::abi::bytes_to_u32(raw)) as f64, None),
            OP_SET_TILESET => {
                (host.set_tileset(raw, ni(0).max(0) as u32, ni(1).max(0) as u32) as f64, None)
            }
            OP_COMMIT_TERRAIN => (host.commit_terrain(nf(0)) as f64, None),
            _ => (0.0, None),
        }
    }

    /// Run one guest entry point (init/update/start) with the wall-clock
    /// watchdog: service host imports until the worker signals done, or
    /// terminate it on budget exhaustion.
    fn run(&mut self, engine: &mut Engine, cmd: i32, dt: f64) -> Result<(), GuestError> {
        self.host.borrow_mut().set_engine(engine);
        self.write_f64(NUM_OFFSET + F_DT * 8, dt);
        self.flag_store(I_DONE, 0);
        self.flag_store(I_COMMAND, cmd);
        self.flag_store(I_GO, 1);
        self.notify(I_GO);

        let deadline = js_sys::Date::now() + self.limits.max_frame_millis as f64;
        loop {
            if self.flag_load(I_DONE) != 0 {
                return Ok(());
            }
            if self.flag_load(I_REQ_READY) != 0 {
                self.service();
                continue;
            }
            if js_sys::Date::now() > deadline {
                self.worker.terminate();
                return Err(GuestError::FuelExhausted);
            }
        }
    }
}

impl GuestRuntime for WorkerWasmRuntime {
    fn new(wasm: &[u8], limits: &GuestLimits) -> Result<Self, GuestError> {
        if !sab_available() {
            return Err(GuestError::Instantiate(
                "SharedArrayBuffer unavailable (needs cross-origin isolation)".into(),
            ));
        }

        let sab = js_sys::SharedArrayBuffer::new(SAB_SIZE);
        let flags = js_sys::Int32Array::new(&JsValue::from(sab.clone()));
        let bytes = js_sys::Uint8Array::new(&JsValue::from(sab.clone()));

        // Embed the guest module into the SAB for the worker to read.
        bytes.subarray(WASM_OFFSET, WASM_OFFSET + wasm.len() as u32).copy_from(wasm);
        let _ = js_sys::Atomics::store(&flags, I_WASM_LEN, wasm.len() as i32);

        let blob_parts = js_sys::Array::of1(&JsValue::from_str(WORKER_SRC));
        let blob = web_sys::Blob::new_with_str_sequence(blob_parts.as_ref())
            .map_err(|e| GuestError::Instantiate(js_err(&e)))?;
        let url = web_sys::Url::create_object_url_with_blob(&blob)
            .map_err(|e| GuestError::Instantiate(js_err(&e)))?;
        let worker = web_sys::Worker::new(&url).map_err(|e| GuestError::Instantiate(js_err(&e)))?;
        worker
            .post_message(&JsValue::from(sab))
            .map_err(|e| GuestError::Instantiate(js_err(&e)))?;

        Ok(Self {
            host: Rc::new(RefCell::new(GuestHost::new())),
            worker,
            flags,
            bytes,
            limits: limits.clone(),
        })
    }

    fn init(&mut self, engine: &mut Engine) -> Result<(), GuestError> {
        self.run(engine, CMD_INIT, 0.0)
    }

    fn update(&mut self, engine: &mut Engine, dt: f64) -> Result<(), GuestError> {
        self.run(engine, CMD_UPDATE, dt)
    }

    fn start(&mut self, engine: &mut Engine) -> Result<(), GuestError> {
        self.run(engine, CMD_START, 0.0)
    }
}

/// Decode the length-prefixed string stream the worker wrote to the SAB.
fn decode_strings(raw: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 4 <= raw.len() {
        let len = u32::from_le_bytes([raw[i], raw[i + 1], raw[i + 2], raw[i + 3]]) as usize;
        i += 4;
        if i + len > raw.len() {
            break;
        }
        out.push(String::from_utf8_lossy(&raw[i..i + len]).into_owned());
        i += len;
    }
    out
}

/// Pack a bulk out buffer, rejecting it if it exceeds the guest's `out_cap` or
/// the SAB's OUT region.
fn out_or_err(bytes: Vec<u8>, out_cap: i32) -> (f64, Option<Vec<u8>>) {
    if bytes.len() > out_cap.max(0) as usize || bytes.len() as u32 > OUT_BYTES {
        (-1.0, None)
    } else {
        (bytes.len() as f64, Some(bytes))
    }
}
