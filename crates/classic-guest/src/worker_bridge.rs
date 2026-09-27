//! The transport-independent half of the untrusted web `Worker` backend
//! (`WorkerWasmRuntime`, wasm only).
//!
//! The Worker runs the guest; every host import is marshalled into a request
//! (its numeric wasm arguments, plus one byte blob per memory-backed parameter)
//! and serviced on the main thread against the shared [`GuestHost`].  This
//! module owns everything about that request except the `SharedArrayBuffer`
//! transport:
//!
//! - [`WorkerCall`] decodes one request and captures the response bytes;
//! - [`WorkerImports`] is the registry of host imports, generated from the ABI
//!   table's `worker` entries (`classic_core::abi_manifest`), indexed by op code;
//! - [`descriptor_json`] describes the same imports, in the same op order, for
//!   `worker.js` to build its import stubs from.
//!
//! Op codes are simply table positions, so there is no hand-numbered op table
//! on either side of the bridge.

use classic_core::abi_manifest::{imports_for, Backend, ParamKind};

use crate::sdk::GuestHost;

/// One decoded host-import request from the Worker, plus its response bytes.
pub struct WorkerCall {
    nums: Vec<f64>,
    next_num: usize,
    blobs: std::vec::IntoIter<Vec<u8>>,
    /// Bytes for the Worker to write to the call's `out_ptr` (if any).
    pub out: Option<Vec<u8>>,
}

impl WorkerCall {
    /// Decode a request: `nums` holds every wasm argument in order (including
    /// `ptr`/`len`/`out_ptr`/`out_cap`), `payload` the memory-backed parameters
    /// as consecutive `len: u32 LE` + bytes blobs, in declaration order.
    pub fn new(nums: Vec<f64>, payload: &[u8]) -> Self {
        let mut blobs = Vec::new();
        let mut i = 0usize;
        while i + 4 <= payload.len() {
            let len =
                u32::from_le_bytes([payload[i], payload[i + 1], payload[i + 2], payload[i + 3]])
                    as usize;
            i += 4;
            let end = (i + len).min(payload.len());
            blobs.push(payload[i..end].to_vec());
            i = end;
        }
        Self { nums, next_num: 0, blobs: blobs.into_iter(), out: None }
    }

    /// Encode memory-backed parameter blobs into a request payload (the inverse
    /// of the payload decoding in [`WorkerCall::new`]).
    pub fn encode_payload<'a>(blobs: impl IntoIterator<Item = &'a [u8]>) -> Vec<u8> {
        let mut payload = Vec::new();
        for blob in blobs {
            payload.extend_from_slice(&(blob.len() as u32).to_le_bytes());
            payload.extend_from_slice(blob);
        }
        payload
    }

    fn num(&mut self) -> f64 {
        let v = self.nums.get(self.next_num).copied().unwrap_or(0.0);
        self.next_num += 1;
        v
    }

    fn bytes(&mut self) -> Vec<u8> {
        self.blobs.next().unwrap_or_default()
    }

    fn str(&mut self) -> String {
        String::from_utf8_lossy(&self.bytes()).into_owned()
    }
}

/// A host import's return value as the `f64` carried back to the Worker.
trait RetF64 {
    fn ret_f64(self) -> f64;
}

impl RetF64 for () {
    fn ret_f64(self) -> f64 {
        0.0
    }
}

impl RetF64 for i32 {
    fn ret_f64(self) -> f64 {
        self as f64
    }
}

impl RetF64 for f64 {
    fn ret_f64(self) -> f64 {
        self
    }
}

type WorkerImport = Box<dyn Fn(&mut GuestHost, &mut WorkerCall) -> f64>;

/// The Worker frontend for `classic_core::host_imports!`.
///
/// `extra` is `[imports, host, call]`: the registry `Vec` being filled and the
/// names of each handler's `&mut GuestHost` / `&mut WorkerCall` parameters.
/// Guest memory lives in the Worker, so reads take the next request blob
/// (ignoring `ptr`/`len`) and writes stash the response bytes for the Worker to
/// write at `out_ptr`.
macro_rules! worker_frontend {
    (@host [$imports:ident, $host:ident, $call:ident] $c:ident) => {
        $host
    };
    (@read_str [$imports:ident, $host:ident, $call:ident] $c:ident $p:ident $l:ident) => {{
        let _ = ($p, $l);
        $call.str()
    }};
    (@read_bytes [$imports:ident, $host:ident, $call:ident] $c:ident $p:ident $l:ident) => {{
        let _ = ($p, $l);
        $call.bytes()
    }};
    (@write_str [$imports:ident, $host:ident, $call:ident] $c:ident $ptr:expr, $v:expr) => {{
        let _ = $ptr;
        let value: &str = $v;
        $call.out = Some(value.as_bytes().to_vec());
        value.len() as i32
    }};
    (@write_bytes [$imports:ident, $host:ident, $call:ident] $c:ident $ptr:expr, $v:expr) => {{
        let _ = $ptr;
        let value: &[u8] = $v;
        $call.out = Some(value.to_vec());
        value.len() as i32
    }};
    (@write_pair [$imports:ident, $host:ident, $call:ident] $c:ident $ptr:expr, $a:expr,
        $b:expr) => {{
        let _ = $ptr;
        $call.out = Some($crate::abi::f64_pair_bytes($a, $b).to_vec());
    }};
    (@write_triple [$imports:ident, $host:ident, $call:ident] $c:ident $ptr:expr, $a:expr,
        $b:expr, $z:expr) => {{
        let _ = $ptr;
        $call.out = Some($crate::abi::f64_triple_bytes($a, $b, $z).to_vec());
    }};
    (@register [$imports:ident, $host:ident, $call:ident] $c:ident $name:ident ($ret:ty)
        [$($p:ident: $t:ident,)*] {$($body:tt)*}) => {
        $imports.push((
            stringify!($name),
            Box::new(|$host: &mut GuestHost, $call: &mut WorkerCall| -> f64 {
                let _ = &$call;
                $( let $p: $t = worker_frontend!(@num $call $t); )*
                #[allow(clippy::redundant_closure_call, clippy::unused_unit)]
                let ret: $ret = (|| -> $ret { $($body)* })();
                RetF64::ret_f64(ret)
            }) as WorkerImport,
        ));
    };
    (@num $call:ident i32) => {
        $call.num() as i32
    };
    (@num $call:ident f64) => {
        $call.num()
    };
}

/// The Worker backend's host imports, indexed by op code (table order).
pub struct WorkerImports {
    imports: Vec<(&'static str, WorkerImport)>,
}

impl Default for WorkerImports {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkerImports {
    /// Build the registry from the ABI table's `worker` entries.
    pub fn new() -> Self {
        let mut imports: Vec<(&'static str, WorkerImport)> = Vec::new();
        classic_core::host_imports!(worker, [worker_frontend], [imports, host, call]);
        Self { imports }
    }

    /// Service one request: run import `op` against `host`, returning its
    /// value (response bytes, if any, are left in `call.out`).  Unknown ops
    /// return `0`.
    pub fn dispatch(&self, op: i32, host: &mut GuestHost, call: &mut WorkerCall) -> f64 {
        match usize::try_from(op).ok().and_then(|i| self.imports.get(i)) {
            Some((_, import)) => import(host, call),
            None => 0.0,
        }
    }

    /// Import names in op order.
    pub fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.imports.iter().map(|(name, _)| *name)
    }
}

/// The JSON descriptor `worker.js` builds its import stubs from, in op order:
/// `[[name, shape, out_params, returns_value], ...]`, where `shape` has one
/// character per parameter (`n` = one numeric wasm param, `m` = a guest-memory
/// `ptr, len` pair sent as a blob) and `out_params` counts the trailing
/// `out_ptr[, out_cap]` params.
pub fn descriptor_json() -> String {
    let entries: Vec<String> = imports_for(Backend::Worker)
        .map(|import| {
            let shape: String = import
                .params
                .iter()
                .map(|p| match p.kind {
                    ParamKind::I32 | ParamKind::F64 | ParamKind::U32 => 'n',
                    _ => 'm',
                })
                .collect();
            format!(
                "[\"{}\",\"{}\",{},{}]",
                import.name,
                shape,
                import.ret.out_params().len(),
                u8::from(import.wasm_result().is_some())
            )
        })
        .collect();
    format!("[{}]", entries.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;
    use classic_engine::Engine;

    fn op(imports: &WorkerImports, name: &str) -> i32 {
        imports.names().position(|n| n == name).unwrap() as i32
    }

    #[test]
    fn registry_follows_the_table_worker_subset() {
        let imports = WorkerImports::new();
        let table: Vec<&str> = imports_for(Backend::Worker).map(|i| i.name).collect();
        assert_eq!(imports.names().collect::<Vec<_>>(), table);
        assert_eq!(table.len(), 110);
    }

    #[test]
    fn descriptor_matches_registry_order_and_shapes() {
        let json = descriptor_json();
        let imports = WorkerImports::new();
        let expected: Vec<String> = imports.names().map(|n| format!("[\"{n}\",")).collect();
        let mut rest = json.as_str();
        for prefix in &expected {
            let at = rest.find(prefix.as_str()).expect("descriptor entry in op order");
            rest = &rest[at + prefix.len()..];
        }
        assert!(json.contains(r#"["pick_at","nnm",2,1]"#));
        assert!(json.contains(r#"["log","m",0,0]"#));
        assert!(json.contains(r#"["get_pos","m",1,1]"#));
    }

    #[test]
    fn field_round_trip_over_one_mib() {
        let mut engine = Engine::new_for_test();
        let mut host = GuestHost::new();
        host.set_engine(&mut engine);
        let imports = WorkerImports::new();

        let (w, h) = (512, 512);
        let data: Vec<f32> = (0..w * h).map(|i| i as f32 * 0.5).collect();
        let bytes = classic_core::abi::f32_array_bytes(&data);
        assert!(bytes.len() >= 1 << 20);

        let mut call = WorkerCall::new(
            vec![0.0, 1.0, w as f64, h as f64, 0.0],
            &WorkerCall::encode_payload([b"f".as_slice()]),
        );
        assert_eq!(imports.dispatch(op(&imports, "alloc_field"), &mut host, &mut call), 1.0);

        let mut call = WorkerCall::new(
            vec![0.0, 1.0, 0.0, bytes.len() as f64],
            &WorkerCall::encode_payload([b"f".as_slice(), bytes.as_slice()]),
        );
        assert_eq!(imports.dispatch(op(&imports, "write_field"), &mut host, &mut call), 1.0);

        let out_cap = 2 << 20;
        let mut call = WorkerCall::new(
            vec![0.0, 1.0, 64.0, out_cap as f64],
            &WorkerCall::encode_payload([b"f".as_slice()]),
        );
        let ret = imports.dispatch(op(&imports, "read_field"), &mut host, &mut call);
        assert_eq!(ret, bytes.len() as f64);
        assert_eq!(call.out.as_deref(), Some(bytes.as_slice()));
    }

    #[test]
    fn out_of_range_op_is_a_no_op() {
        let mut engine = Engine::new_for_test();
        let mut host = GuestHost::new();
        host.set_engine(&mut engine);
        let mut call = WorkerCall::new(Vec::new(), &[]);
        assert_eq!(WorkerImports::new().dispatch(-1, &mut host, &mut call), 0.0);
        assert_eq!(WorkerImports::new().dispatch(9999, &mut host, &mut call), 0.0);
    }
}
