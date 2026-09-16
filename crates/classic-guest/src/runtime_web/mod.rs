//! The browser-native WebAssembly guest runtime (web only, trusted guests).
//!
//! Trusted guests run on the browser's own `WebAssembly` engine (near-native
//! speed) instead of wasmi-on-wasm.  Browser Wasm has no fuel API, so this
//! backend is only selected for `trusted` ROMs; untrusted ROMs stay on
//! `WasmiRuntime` (interruptible fuel metering).
//!
//! `WebAssembly.Module` / `WebAssembly.Instance` are constructed synchronously
//! (`js_sys::WebAssembly::{Module, Instance}::new`), so no async restructuring
//! is needed.  Host imports are `Closure`-wrapped functions, generated from the
//! ABI table (`classic_core::abi_manifest`), that read/write the guest's linear
//! memory through a `Uint8Array` view of its `WebAssembly.Memory` and dispatch
//! into the shared [`GuestHost`].  wasm-bindgen `Closure`s take at most 8
//! arguments, so wider imports are one arity-1 `Closure` behind a JS shim that
//! forwards its `arguments` array.

mod mem;

use std::cell::RefCell;
use std::rc::Rc;

use classic_engine::Engine;
use js_sys::WebAssembly::{Instance, Memory, Module};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};

use crate::abi;
use crate::runtime::{GuestError, GuestLimits, GuestRuntime};
use crate::sdk::GuestHost;

use mem::{js_err, read_bytes, read_str, write_bytes, write_f64_pair, write_f64_triple, write_str};

/// The browser-`WebAssembly` frontend for `classic_core::host_imports!`.
///
/// `extra` is `[env, host, mem]`: the `env` import object being filled, and the
/// shared `Rc<RefCell<GuestHost>>` / `Rc<RefCell<Option<Memory>>>` cells.
macro_rules! web_frontend {
    (@host [$env:ident, $host:ident, $mem:ident] $c:ident) => {
        $host.borrow_mut()
    };
    (@read_str [$env:ident, $host:ident, $mem:ident] $c:ident $p:ident $l:ident) => {{
        let mem = $mem.borrow();
        read_str(mem.as_ref().unwrap(), $p, $l)
    }};
    (@read_bytes [$env:ident, $host:ident, $mem:ident] $c:ident $p:ident $l:ident) => {{
        let mem = $mem.borrow();
        read_bytes(mem.as_ref().unwrap(), $p, $l)
    }};
    (@write_str [$env:ident, $host:ident, $mem:ident] $c:ident $ptr:expr, $v:expr) => {{
        let mem = $mem.borrow();
        write_str(mem.as_ref().unwrap(), $ptr, $v)
    }};
    (@write_bytes [$env:ident, $host:ident, $mem:ident] $c:ident $ptr:expr, $v:expr) => {{
        let mem = $mem.borrow();
        write_bytes(mem.as_ref().unwrap(), $ptr, $v)
    }};
    (@write_pair [$env:ident, $host:ident, $mem:ident] $c:ident $ptr:expr, $a:expr, $b:expr) => {{
        let mem = $mem.borrow();
        write_f64_pair(mem.as_ref().unwrap(), $ptr, $a, $b)
    }};
    (@write_triple [$env:ident, $host:ident, $mem:ident] $c:ident $ptr:expr, $a:expr, $b:expr,
        $z:expr) => {{
        let mem = $mem.borrow();
        write_f64_triple(mem.as_ref().unwrap(), $ptr, $a, $b, $z)
    }};
    (@register $x:tt $c:ident $name:ident ($ret:ty) [$($p:ident: $t:ident,)*] {$($body:tt)*}) => {
        web_frontend!(@arity $x $name ($ret) [$($p: $t,)*] [$($p: $t,)*] {$($body)*});
    };
    // More than 8 wasm params: an arity-1 `Closure` behind a JS `arguments` shim.
    (@arity [$env:ident, $host:ident, $mem:ident] $name:ident ($ret:ty) [$($p:ident: $t:ident,)*]
        [$p1:ident: $t1:ident, $p2:ident: $t2:ident, $p3:ident: $t3:ident, $p4:ident: $t4:ident,
         $p5:ident: $t5:ident, $p6:ident: $t6:ident, $p7:ident: $t7:ident, $p8:ident: $t8:ident,
         $p9:ident: $t9:ident, $($more:tt)*]
        {$($body:tt)*}) => {{
        let $host = $host.clone();
        #[allow(unused_variables)]
        let $mem = $mem.clone();
        let closure = Closure::wrap(Box::new(move |args: js_sys::Array| -> JsValue {
            let mut args = args.iter();
            $( let $p: $t = web_frontend!(@arg args $t); )*
            #[allow(clippy::redundant_closure_call)]
            let ret: $ret = (|| -> $ret { $($body)* })();
            JsValue::from(ret)
        }) as Box<dyn FnMut(js_sys::Array) -> JsValue>);
        let shim = js_sys::Function::new_with_args(
            "f",
            "return function () { return f(Array.from(arguments)); };",
        )
        .call1(&JsValue::NULL, closure.as_ref())
        .map_err(|e| GuestError::Instantiate(js_err(&e)))?;
        js_sys::Reflect::set(&$env, &JsValue::from(stringify!($name)), &shim)
            .map_err(|e| GuestError::Instantiate(js_err(&e)))?;
        closure.forget();
    }};
    // Up to 8 wasm params: a typed `Closure`.
    (@arity [$env:ident, $host:ident, $mem:ident] $name:ident ($ret:ty) [$($p:ident: $t:ident,)*]
        $all:tt {$($body:tt)*}) => {{
        let $host = $host.clone();
        #[allow(unused_variables)]
        let $mem = $mem.clone();
        #[allow(clippy::unused_unit)]
        let closure = Closure::wrap(
            Box::new(move |$($p: $t),*| -> $ret { $($body)* }) as Box<dyn FnMut($($t),*) -> $ret>
        );
        js_sys::Reflect::set(&$env, &JsValue::from(stringify!($name)), closure.as_ref())
            .map_err(|e| GuestError::Instantiate(js_err(&e)))?;
        closure.forget();
    }};
    (@arg $args:ident i32) => {
        $args.next().and_then(|v| v.as_f64()).unwrap_or(0.0) as i32
    };
    (@arg $args:ident f64) => {
        $args.next().and_then(|v| v.as_f64()).unwrap_or(0.0)
    };
}

/// Browser-native [`GuestRuntime`] (web target only, trusted guests).
pub struct WebWasmRuntime {
    host: Rc<RefCell<GuestHost>>,
    init: Option<js_sys::Function>,
    update: js_sys::Function,
    start: Option<js_sys::Function>,
}

impl WebWasmRuntime {
    /// Build the `env` import object: one registration per `web` entry of the
    /// ABI table, bridged to the shared [`GuestHost`].
    fn build_imports(
        host: &Rc<RefCell<GuestHost>>,
        mem: &Rc<RefCell<Option<Memory>>>,
    ) -> Result<js_sys::Object, GuestError> {
        let env = js_sys::Object::new();
        classic_core::host_imports!(web, [web_frontend], [env, host, mem]);
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
