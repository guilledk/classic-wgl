//! The guest runtime: sandboxed execution of ROM guest `.wasm` modules.

use classic_engine::Engine;
use wasmi::{Caller, Config, Engine as WasmEngine, Linker, Module, Store};

use crate::abi;
use crate::sdk::GuestHost;

/// Per-frame guest resource limits.
#[derive(Clone, Debug)]
pub struct GuestLimits {
    /// Fuel (operation) budget per `update` call; enforced when `!trusted`.
    pub fuel_per_frame: u64,
    /// Maximum guest linear-memory size in bytes.
    pub max_memory_bytes: usize,
    /// Trusted guests skip fuel metering and use the fast path.
    pub trusted: bool,
}

impl Default for GuestLimits {
    fn default() -> Self {
        Self { fuel_per_frame: 1_000_000, max_memory_bytes: 64 * 1024 * 1024, trusted: false }
    }
}

/// Errors surfaced while compiling or running a guest module.
#[derive(Debug)]
pub enum GuestError {
    Compile(String),
    Instantiate(String),
    MissingExport(String),
    FuelExhausted,
    Trap(String),
}

impl std::fmt::Display for GuestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GuestError::Compile(e) => write!(f, "guest compile failed: {e}"),
            GuestError::Instantiate(e) => write!(f, "guest instantiate failed: {e}"),
            GuestError::MissingExport(e) => write!(f, "guest missing export: {e}"),
            GuestError::FuelExhausted => write!(f, "guest exceeded its fuel budget"),
            GuestError::Trap(e) => write!(f, "guest trapped: {e}"),
        }
    }
}

impl std::error::Error for GuestError {}

/// A loaded, runnable ROM guest.
pub trait GuestRuntime {
    /// Compile and instantiate a guest module from its `.wasm` bytes.
    fn new(wasm: &[u8], limits: &GuestLimits) -> Result<Self, GuestError>
    where
        Self: Sized;

    /// Run the guest's optional `init()` once, before the first frame.  The
    /// default is a no-op for guests that do not export `init`.
    fn init(&mut self, _engine: &mut Engine) -> Result<(), GuestError> {
        Ok(())
    }

    /// Run the guest's `update(dt)` once against the engine.
    fn update(&mut self, engine: &mut Engine, dt: f64) -> Result<(), GuestError>;

    /// Run the guest's optional `start()` once, after the first `update`.  The
    /// default is a no-op for guests that do not export `start`.
    fn start(&mut self, _engine: &mut Engine) -> Result<(), GuestError> {
        Ok(())
    }
}

/// wasmi-backed [`GuestRuntime`] (native and wasm targets).
pub struct WasmiRuntime {
    store: Store<GuestHost>,
    init: Option<wasmi::TypedFunc<(), ()>>,
    update: wasmi::TypedFunc<(f64,), ()>,
    start: Option<wasmi::TypedFunc<(), ()>>,
    limits: GuestLimits,
}

impl WasmiRuntime {
    fn build_engine(limits: &GuestLimits) -> WasmEngine {
        let mut config = Config::default();
        // Fuel metering is the untrusted path's CPU guard; trusted guests
        // skip it (no per-operation overhead).
        config.consume_fuel(!limits.trusted);
        WasmEngine::new(&config)
    }

    fn install_imports(linker: &mut Linker<GuestHost>) -> Result<(), wasmi::Error> {
        let m = abi::HOST_MODULE;

        linker.func_wrap(m, "log", |mut caller: Caller<'_, GuestHost>, ptr: i32, len: i32| {
            let msg = abi::read_str(&caller, ptr, len);
            caller.data_mut().log(&msg);
        })?;

        linker.func_wrap(
            m,
            "spawn",
            |mut caller: Caller<'_, GuestHost>, ptr: i32, len: i32| -> i32 {
                let name = abi::read_str(&caller, ptr, len);
                caller.data_mut().spawn(&name)
            },
        )?;

        linker.func_wrap(
            m,
            "despawn",
            |mut caller: Caller<'_, GuestHost>, ptr: i32, len: i32| -> i32 {
                let name = abi::read_str(&caller, ptr, len);
                caller.data_mut().despawn(&name)
            },
        )?;

        linker.func_wrap(
            m,
            "has",
            |mut caller: Caller<'_, GuestHost>, ptr: i32, len: i32| -> i32 {
                let name = abi::read_str(&caller, ptr, len);
                caller.data_mut().has(&name)
            },
        )?;

        linker.func_wrap(
            m,
            "names",
            |mut caller: Caller<'_, GuestHost>, out_ptr: i32, out_cap: i32| -> i32 {
                let json = caller.data_mut().names();
                if out_cap < json.len() as i32 {
                    return -1;
                }
                abi::write_str(&mut caller, out_ptr, &json)
            },
        )?;

        linker.func_wrap(
            m,
            "get",
            |mut caller: Caller<'_, GuestHost>,
             ptr: i32,
             len: i32,
             out_ptr: i32,
             out_cap: i32|
             -> i32 {
                let name = abi::read_str(&caller, ptr, len);
                let json = caller.data_mut().get(&name);
                if out_cap < json.len() as i32 {
                    return -1;
                }
                abi::write_str(&mut caller, out_ptr, &json)
            },
        )?;

        linker.func_wrap(
            m,
            "get_comp",
            |mut caller: Caller<'_, GuestHost>,
             ptr: i32,
             len: i32,
             comp_ptr: i32,
             comp_len: i32,
             out_ptr: i32,
             out_cap: i32|
             -> i32 {
                let name = abi::read_str(&caller, ptr, len);
                let comp = abi::read_str(&caller, comp_ptr, comp_len);
                let json = caller.data_mut().get_comp(&name, &comp);
                if out_cap < json.len() as i32 {
                    return -1;
                }
                abi::write_str(&mut caller, out_ptr, &json)
            },
        )?;

        linker.func_wrap(
            m,
            "set",
            |mut caller: Caller<'_, GuestHost>,
             ptr: i32,
             len: i32,
             json_ptr: i32,
             json_len: i32|
             -> i32 {
                let name = abi::read_str(&caller, ptr, len);
                let json = abi::read_str(&caller, json_ptr, json_len);
                caller.data_mut().set(&name, &json)
            },
        )?;

        linker.func_wrap(
            m,
            "set_comp",
            |mut caller: Caller<'_, GuestHost>,
             ptr: i32,
             len: i32,
             comp_ptr: i32,
             comp_len: i32,
             json_ptr: i32,
             json_len: i32|
             -> i32 {
                let name = abi::read_str(&caller, ptr, len);
                let comp = abi::read_str(&caller, comp_ptr, comp_len);
                let json = abi::read_str(&caller, json_ptr, json_len);
                caller.data_mut().set_comp(&name, &comp, &json)
            },
        )?;

        linker.func_wrap(
            m,
            "set_pos",
            |mut caller: Caller<'_, GuestHost>,
             ptr: i32,
             len: i32,
             x: f64,
             y: f64,
             z: f64|
             -> i32 {
                let name = abi::read_str(&caller, ptr, len);
                caller.data_mut().set_pos(&name, x, y, z)
            },
        )?;

        linker.func_wrap(
            m,
            "get_pos",
            |mut caller: Caller<'_, GuestHost>, ptr: i32, len: i32, out_ptr: i32| -> i32 {
                let name = abi::read_str(&caller, ptr, len);
                let Some((x, y, z)) = caller.data_mut().get_pos(&name) else {
                    return 0;
                };
                abi::write_f64_triple(&mut caller, out_ptr, x, y, z);
                1
            },
        )?;

        linker.func_wrap(m, "mouse", |mut caller: Caller<'_, GuestHost>, out_ptr: i32| -> i32 {
            let (x, y) = caller.data_mut().mouse();
            abi::write_f64_pair(&mut caller, out_ptr, x, y);
            1
        })?;

        linker.func_wrap(
            m,
            "mouse_iso",
            |mut caller: Caller<'_, GuestHost>, out_ptr: i32| -> i32 {
                let Some((x, y)) = caller.data_mut().mouse_iso() else {
                    return 0;
                };
                abi::write_f64_pair(&mut caller, out_ptr, x, y);
                1
            },
        )?;

        linker.func_wrap(
            m,
            "height_at",
            |mut caller: Caller<'_, GuestHost>, x: f64, y: f64| -> f64 {
                caller.data_mut().height_at(x, y)
            },
        )?;

        linker.func_wrap(
            m,
            "set_anim",
            |mut caller: Caller<'_, GuestHost>,
             ptr: i32,
             len: i32,
             anim_ptr: i32,
             anim_len: i32|
             -> i32 {
                let name = abi::read_str(&caller, ptr, len);
                let anim = abi::read_str(&caller, anim_ptr, anim_len);
                caller.data_mut().set_anim(&name, &anim)
            },
        )?;

        linker.func_wrap(m, "agent_selected", |mut caller: Caller<'_, GuestHost>| -> i32 {
            caller.data_mut().agent_selected()
        })?;

        linker.func_wrap(m, "ui_consumed_click", |mut caller: Caller<'_, GuestHost>| -> i32 {
            caller.data_mut().ui_consumed_click()
        })?;

        linker.func_wrap(m, "delta", |mut caller: Caller<'_, GuestHost>| -> f64 {
            caller.data_mut().delta()
        })?;

        linker.func_wrap(m, "elapsed", |mut caller: Caller<'_, GuestHost>| -> f64 {
            caller.data_mut().elapsed()
        })?;

        linker.func_wrap(
            m,
            "was_pressed",
            |mut caller: Caller<'_, GuestHost>, btn: i32| -> i32 {
                caller.data_mut().was_pressed(btn)
            },
        )?;

        linker.func_wrap(
            m,
            "key_down",
            |mut caller: Caller<'_, GuestHost>, ptr: i32, len: i32| -> i32 {
                let key = abi::read_str(&caller, ptr, len);
                caller.data_mut().key_down(&key)
            },
        )?;

        linker.func_wrap(
            m,
            "was_key_pressed",
            |mut caller: Caller<'_, GuestHost>, ptr: i32, len: i32| -> i32 {
                let key = abi::read_str(&caller, ptr, len);
                caller.data_mut().was_key_pressed(&key)
            },
        )?;

        linker.func_wrap(
            m,
            "generate_terrain",
            |mut caller: Caller<'_, GuestHost>,
             kind_ptr: i32,
             kind_len: i32,
             seed_ptr: i32,
             seed_len: i32,
             height_scale: f64|
             -> i32 {
                let kind = abi::read_str(&caller, kind_ptr, kind_len);
                let seed = abi::read_str(&caller, seed_ptr, seed_len);
                caller.data_mut().generate_terrain(&kind, &seed, height_scale)
            },
        )?;

        linker.func_wrap(
            m,
            "find_path",
            |mut caller: Caller<'_, GuestHost>,
             sx: i32,
             sy: i32,
             ex: i32,
             ey: i32,
             out_ptr: i32,
             out_cap: i32|
             -> i32 {
                let cells = caller.data_mut().find_path(sx, sy, ex, ey);
                let mut bytes = Vec::with_capacity(cells.len() * 8);
                for (x, y) in &cells {
                    bytes.extend_from_slice(&x.to_le_bytes());
                    bytes.extend_from_slice(&y.to_le_bytes());
                }
                if bytes.len() > out_cap.max(0) as usize {
                    return -1;
                }
                abi::write_bytes(&mut caller, out_ptr, &bytes);
                cells.len() as i32
            },
        )?;

        linker.func_wrap(
            m,
            "get_camera",
            |mut caller: Caller<'_, GuestHost>, out_ptr: i32| -> i32 {
                let (x, y, s) = caller.data_mut().get_camera();
                abi::write_f64_triple(&mut caller, out_ptr, x, y, s);
                1
            },
        )?;

        linker.func_wrap(
            m,
            "set_camera",
            |mut caller: Caller<'_, GuestHost>, x: f64, y: f64, scale: f64| -> i32 {
                caller.data_mut().set_camera(x, y, scale)
            },
        )?;

        linker.func_wrap(
            m,
            "pick_at",
            |mut caller: Caller<'_, GuestHost>,
             x: f64,
             y: f64,
             out_ptr: i32,
             out_cap: i32|
             -> i32 {
                let name = caller.data_mut().pick_at(x, y);
                if out_cap < name.len() as i32 {
                    return -1;
                }
                abi::write_str(&mut caller, out_ptr, &name)
            },
        )?;

        linker.func_wrap(
            m,
            "mouse_down",
            |mut caller: Caller<'_, GuestHost>, btn: i32| -> i32 {
                caller.data_mut().mouse_down(btn)
            },
        )?;

        linker.func_wrap(
            m,
            "mouse_released",
            |mut caller: Caller<'_, GuestHost>, btn: i32| -> i32 {
                caller.data_mut().mouse_released(btn)
            },
        )?;

        linker.func_wrap(m, "mouse_wheel", |mut caller: Caller<'_, GuestHost>| -> f64 {
            caller.data_mut().mouse_wheel()
        })?;

        linker.func_wrap(
            m,
            "key_up",
            |mut caller: Caller<'_, GuestHost>, ptr: i32, len: i32| -> i32 {
                let key = abi::read_str(&caller, ptr, len);
                caller.data_mut().key_up(&key)
            },
        )?;

        linker.func_wrap(
            m,
            "get_light",
            |mut caller: Caller<'_, GuestHost>, out_ptr: i32| -> i32 {
                let (a, d, c) = caller.data_mut().get_light();
                let mut buf = Vec::with_capacity(72);
                for v in a.iter().chain(d.iter()).chain(c.iter()) {
                    buf.extend_from_slice(&v.to_le_bytes());
                }
                abi::write_bytes(&mut caller, out_ptr, &buf);
                1
            },
        )?;

        linker.func_wrap(
            m,
            "set_light",
            |mut caller: Caller<'_, GuestHost>,
             a0: f64,
             a1: f64,
             a2: f64,
             d0: f64,
             d1: f64,
             d2: f64,
             c0: f64,
             c1: f64,
             c2: f64|
             -> i32 { caller.data_mut().set_light(a0, a1, a2, d0, d1, d2, c0, c1, c2) },
        )?;

        linker.func_wrap(
            m,
            "spawn_rect",
            |mut caller: Caller<'_, GuestHost>,
             name_ptr: i32,
             name_len: i32,
             x: f64,
             y: f64,
             w: f64,
             h: f64,
             r: f64,
             g: f64,
             b: f64,
             a: f64|
             -> i32 {
                let name = abi::read_str(&caller, name_ptr, name_len);
                caller.data_mut().spawn_rect(&name, x, y, w, h, r, g, b, a)
            },
        )?;

        linker.func_wrap(
            m,
            "spawn_text",
            |mut caller: Caller<'_, GuestHost>,
             name_ptr: i32,
             name_len: i32,
             x: f64,
             y: f64,
             text_ptr: i32,
             text_len: i32,
             scale: f64,
             r: f64,
             g: f64,
             b: f64,
             a: f64|
             -> i32 {
                let name = abi::read_str(&caller, name_ptr, name_len);
                let text = abi::read_str(&caller, text_ptr, text_len);
                caller.data_mut().spawn_text(&name, x, y, &text, scale, r, g, b, a)
            },
        )?;

        linker.func_wrap(
            m,
            "set_text",
            |mut caller: Caller<'_, GuestHost>,
             name_ptr: i32,
             name_len: i32,
             text_ptr: i32,
             text_len: i32|
             -> i32 {
                let name = abi::read_str(&caller, name_ptr, name_len);
                let text = abi::read_str(&caller, text_ptr, text_len);
                caller.data_mut().set_text(&name, &text)
            },
        )?;

        linker.func_wrap(
            m,
            "ui_container",
            |mut caller: Caller<'_, GuestHost>,
             name_ptr: i32,
             name_len: i32,
             w: f64,
             h: f64,
             r: f64,
             g: f64,
             b: f64,
             a: f64|
             -> i32 {
                let name = abi::read_str(&caller, name_ptr, name_len);
                caller.data_mut().ui_container(&name, w, h, r, g, b, a)
            },
        )?;

        linker.func_wrap(
            m,
            "ui_text",
            |mut caller: Caller<'_, GuestHost>,
             name_ptr: i32,
             name_len: i32,
             text_ptr: i32,
             text_len: i32,
             scale: f64,
             max_width: f64,
             r: f64,
             g: f64,
             b: f64,
             a: f64,
             justify: i32|
             -> i32 {
                let name = abi::read_str(&caller, name_ptr, name_len);
                let text = abi::read_str(&caller, text_ptr, text_len);
                caller.data_mut().ui_text(&name, &text, scale, max_width, r, g, b, a, justify)
            },
        )?;

        linker.func_wrap(
            m,
            "ui_button",
            |mut caller: Caller<'_, GuestHost>,
             name_ptr: i32,
             name_len: i32,
             text_ptr: i32,
             text_len: i32,
             w: f64,
             h: f64,
             r: f64,
             g: f64,
             b: f64,
             a: f64|
             -> i32 {
                let name = abi::read_str(&caller, name_ptr, name_len);
                let text = abi::read_str(&caller, text_ptr, text_len);
                caller.data_mut().ui_button(&name, &text, w, h, r, g, b, a)
            },
        )?;

        linker.func_wrap(
            m,
            "ui_array",
            |mut caller: Caller<'_, GuestHost>,
             name_ptr: i32,
             name_len: i32,
             vertical: i32,
             align: i32,
             spacing: f64,
             r: f64,
             g: f64,
             b: f64,
             a: f64|
             -> i32 {
                let name = abi::read_str(&caller, name_ptr, name_len);
                caller.data_mut().ui_array(&name, vertical, align, spacing, r, g, b, a)
            },
        )?;

        linker.func_wrap(
            m,
            "ui_padding",
            |mut caller: Caller<'_, GuestHost>,
             name_ptr: i32,
             name_len: i32,
             top: f64,
             right: f64,
             bottom: f64,
             left: f64,
             r: f64,
             g: f64,
             b: f64,
             a: f64|
             -> i32 {
                let name = abi::read_str(&caller, name_ptr, name_len);
                caller.data_mut().ui_padding(&name, top, right, bottom, left, r, g, b, a)
            },
        )?;

        linker.func_wrap(
            m,
            "ui_sprite",
            |mut caller: Caller<'_, GuestHost>,
             name_ptr: i32,
             name_len: i32,
             texture_ptr: i32,
             texture_len: i32,
             w: f64,
             h: f64,
             frame: f64,
             tsx: f64,
             tsy: f64|
             -> i32 {
                let name = abi::read_str(&caller, name_ptr, name_len);
                let texture = abi::read_str(&caller, texture_ptr, texture_len);
                caller.data_mut().ui_sprite(&name, &texture, w, h, frame, tsx, tsy)
            },
        )?;

        linker.func_wrap(
            m,
            "ui_add_child",
            |mut caller: Caller<'_, GuestHost>,
             parent_ptr: i32,
             parent_len: i32,
             child_ptr: i32,
             child_len: i32,
             self_anchor: i32,
             child_anchor: i32|
             -> i32 {
                let parent = abi::read_str(&caller, parent_ptr, parent_len);
                let child = abi::read_str(&caller, child_ptr, child_len);
                caller.data_mut().ui_add_child(&parent, &child, self_anchor, child_anchor)
            },
        )?;

        linker.func_wrap(
            m,
            "ui_add_to_root",
            |mut caller: Caller<'_, GuestHost>,
             name_ptr: i32,
             name_len: i32,
             self_anchor: i32,
             child_anchor: i32|
             -> i32 {
                let name = abi::read_str(&caller, name_ptr, name_len);
                caller.data_mut().ui_add_to_root(&name, self_anchor, child_anchor)
            },
        )?;

        linker.func_wrap(
            m,
            "ui_set_size",
            |mut caller: Caller<'_, GuestHost>,
             name_ptr: i32,
             name_len: i32,
             w: f64,
             h: f64|
             -> i32 {
                let name = abi::read_str(&caller, name_ptr, name_len);
                caller.data_mut().ui_set_size(&name, w, h)
            },
        )?;

        linker.func_wrap(
            m,
            "ui_set_anchor",
            |mut caller: Caller<'_, GuestHost>, name_ptr: i32, name_len: i32, anchor: i32| -> i32 {
                let name = abi::read_str(&caller, name_ptr, name_len);
                caller.data_mut().ui_set_anchor(&name, anchor)
            },
        )?;

        linker.func_wrap(
            m,
            "ui_set_color",
            |mut caller: Caller<'_, GuestHost>,
             name_ptr: i32,
             name_len: i32,
             r: f64,
             g: f64,
             b: f64,
             a: f64|
             -> i32 {
                let name = abi::read_str(&caller, name_ptr, name_len);
                caller.data_mut().ui_set_color(&name, r, g, b, a)
            },
        )?;

        linker.func_wrap(
            m,
            "ui_set_fixed",
            |mut caller: Caller<'_, GuestHost>, name_ptr: i32, name_len: i32, fixed: i32| -> i32 {
                let name = abi::read_str(&caller, name_ptr, name_len);
                caller.data_mut().ui_set_fixed(&name, fixed)
            },
        )?;

        linker.func_wrap(
            m,
            "subscribe",
            |mut caller: Caller<'_, GuestHost>, ptr: i32, len: i32| -> i32 {
                let name = abi::read_str(&caller, ptr, len);
                caller.data_mut().subscribe(&name)
            },
        )?;

        linker.func_wrap(
            m,
            "poll_event",
            |mut caller: Caller<'_, GuestHost>, out_ptr: i32, out_cap: i32| -> i32 {
                let Some((kind, name)) = caller.data_mut().poll_event() else {
                    return 0;
                };
                let mut bytes = Vec::with_capacity(8 + name.len());
                bytes.extend_from_slice(&kind.to_le_bytes());
                bytes.extend_from_slice(&(name.len() as u32).to_le_bytes());
                bytes.extend_from_slice(name.as_bytes());
                if bytes.len() > out_cap.max(0) as usize {
                    return -1;
                }
                abi::write_bytes(&mut caller, out_ptr, &bytes);
                1
            },
        )?;

        linker.func_wrap(
            m,
            "spawn_collider",
            |mut caller: Caller<'_, GuestHost>,
             name_ptr: i32,
             name_len: i32,
             x: f64,
             y: f64,
             w: f64,
             h: f64|
             -> i32 {
                let name = abi::read_str(&caller, name_ptr, name_len);
                caller.data_mut().spawn_collider(&name, x, y, w, h)
            },
        )?;

        linker.func_wrap(
            m,
            "get_anim",
            |mut caller: Caller<'_, GuestHost>,
             name_ptr: i32,
             name_len: i32,
             out_ptr: i32,
             out_cap: i32|
             -> i32 {
                let name = abi::read_str(&caller, name_ptr, name_len);
                let Some((anim, frame)) = caller.data_mut().get_anim(&name) else {
                    return 0;
                };
                let mut bytes = Vec::with_capacity(12 + anim.len());
                bytes.extend_from_slice(&frame.to_le_bytes());
                bytes.extend_from_slice(&(anim.len() as u32).to_le_bytes());
                bytes.extend_from_slice(anim.as_bytes());
                if bytes.len() > out_cap.max(0) as usize {
                    return -1;
                }
                abi::write_bytes(&mut caller, out_ptr, &bytes);
                1
            },
        )?;

        Ok(())
    }
}

impl GuestRuntime for WasmiRuntime {
    fn new(wasm: &[u8], limits: &GuestLimits) -> Result<Self, GuestError> {
        let engine = Self::build_engine(limits);
        let module = Module::new(&engine, wasm).map_err(|e| GuestError::Compile(e.to_string()))?;

        let store_limits = wasmi::StoreLimitsBuilder::new()
            .memory_size(limits.max_memory_bytes)
            .trap_on_grow_failure(true)
            .build();
        let mut store = Store::new(&engine, GuestHost::new(store_limits));
        store.limiter(|host: &mut GuestHost| host.resource_limiter());

        let mut linker = Linker::new(&engine);
        Self::install_imports(&mut linker).map_err(|e| GuestError::Instantiate(e.to_string()))?;

        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|e| GuestError::Instantiate(e.to_string()))?
            .start(&mut store)
            .map_err(|e| GuestError::Instantiate(e.to_string()))?;

        let init = instance.get_typed_func::<(), ()>(&store, abi::INIT_EXPORT).ok();
        let update = instance
            .get_typed_func::<(f64,), ()>(&store, abi::UPDATE_EXPORT)
            .map_err(|_| GuestError::MissingExport(abi::UPDATE_EXPORT.to_string()))?;
        let start = instance.get_typed_func::<(), ()>(&store, abi::START_EXPORT).ok();

        Ok(Self { store, init, update, start, limits: limits.clone() })
    }

    fn init(&mut self, engine: &mut Engine) -> Result<(), GuestError> {
        let Some(init) = self.init else { return Ok(()) };
        self.store.data_mut().set_engine(engine);
        self.set_fuel_budget()?;
        init.call(&mut self.store, ()).map_err(Self::map_call_error)
    }

    fn update(&mut self, engine: &mut Engine, dt: f64) -> Result<(), GuestError> {
        self.store.data_mut().set_engine(engine);
        self.set_fuel_budget()?;
        self.update.call(&mut self.store, (dt,)).map_err(Self::map_call_error)
    }

    fn start(&mut self, engine: &mut Engine) -> Result<(), GuestError> {
        let Some(start) = self.start else { return Ok(()) };
        self.store.data_mut().set_engine(engine);
        self.set_fuel_budget()?;
        start.call(&mut self.store, ()).map_err(Self::map_call_error)
    }
}

impl WasmiRuntime {
    /// Reset the store's fuel budget for the next guest entry point (no-op for
    /// trusted guests).
    fn set_fuel_budget(&mut self) -> Result<(), GuestError> {
        if !self.limits.trusted {
            self.store
                .set_fuel(self.limits.fuel_per_frame)
                .map_err(|e| GuestError::Trap(e.to_string()))?;
        }
        Ok(())
    }

    /// Map a wasmi call error, distinguishing fuel exhaustion from a trap.
    fn map_call_error(e: wasmi::Error) -> GuestError {
        if e.as_trap_code() == Some(wasmi::core::TrapCode::OutOfFuel) {
            GuestError::FuelExhausted
        } else {
            GuestError::Trap(e.to_string())
        }
    }
}
