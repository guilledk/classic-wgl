//! Browser tests for the wasm-only guest backends (`WebWasmRuntime`,
//! `WorkerWasmRuntime`), run in headless Chromium by `wasm-bindgen-test-runner`
//! (cross-origin isolated, so `SharedArrayBuffer` works):
//!
//! ```text
//! cargo test --target wasm32-unknown-unknown -p classic-guest --test web
//! ```
#![cfg(target_arch = "wasm32")]

use classic_core::abi_manifest::{imports_for, Backend};
use classic_engine::Engine;
use classic_guest::{GuestError, GuestLimits, GuestRuntime, WebWasmRuntime, WorkerWasmRuntime};
use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

wasm_bindgen_test_configure!(run_in_browser);

/// Resolve after `ms` milliseconds, yielding to the event loop (lets a freshly
/// spawned `Worker` boot before the main thread busy-polls it).
async fn sleep(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

fn limits() -> GuestLimits {
    GuestLimits { max_frame_millis: 10_000, ..GuestLimits::default() }
}

async fn worker(wat: &str) -> WorkerWasmRuntime {
    let rt = WorkerWasmRuntime::new(&wat::parse_str(wat).unwrap(), &limits()).unwrap();
    sleep(100).await;
    rt
}

/// A module importing every `backend` table entry with its table signature.
fn wat_importing_all(backend: Backend) -> String {
    let imports: Vec<String> = imports_for(backend).map(|i| i.wat_import("env")).collect();
    format!(
        "(module {} (memory (export \"memory\") 1) (func (export \"update\") (param f64)))",
        imports.join(" ")
    )
}

/// Guest: allocate a 512x512 `f32` field, upload 1 MiB of patterned data from
/// guest memory, download it into a second region, compare byte by byte, and
/// spawn `ok` only if both directions round-tripped intact.
const FIELD_ROUND_TRIP: &str = r#"(module
    (import "env" "alloc_field" (func $alloc (param i32 i32 i32 i32 i32) (result i32)))
    (import "env" "write_field" (func $write (param i32 i32 i32 i32) (result i32)))
    (import "env" "read_field" (func $read (param i32 i32 i32 i32) (result i32)))
    (import "env" "spawn" (func $spawn (param i32 i32) (result i32)))
    (memory (export "memory") 64)
    (data (i32.const 0) "f")
    (data (i32.const 8) "ok")
    (func (export "update") (param f64)
        (local $i i32)
        ;; data region A at 1 MiB: f32(i * 0.5) for 262144 samples
        (loop $fill
            (f32.store (i32.add (i32.const 1048576) (i32.shl (local.get $i) (i32.const 2)))
                (f32.mul (f32.convert_i32_u (local.get $i)) (f32.const 0.5)))
            (local.set $i (i32.add (local.get $i) (i32.const 1)))
            (br_if $fill (i32.lt_u (local.get $i) (i32.const 262144))))
        (drop (call $alloc (i32.const 0) (i32.const 1) (i32.const 512) (i32.const 512) (i32.const 0)))
        (drop (call $write (i32.const 0) (i32.const 1) (i32.const 1048576) (i32.const 1048576)))
        ;; read back into region B at 3 MiB
        (if (i32.ne (call $read (i32.const 0) (i32.const 1) (i32.const 3145728) (i32.const 2097152))
                    (i32.const 1048576))
            (then (return)))
        (local.set $i (i32.const 0))
        (loop $cmp
            (if (i32.ne (i32.load8_u (i32.add (i32.const 1048576) (local.get $i)))
                        (i32.load8_u (i32.add (i32.const 3145728) (local.get $i))))
                (then (return)))
            (local.set $i (i32.add (local.get $i) (i32.const 1)))
            (br_if $cmp (i32.lt_u (local.get $i) (i32.const 1048576))))
        (drop (call $spawn (i32.const 8) (i32.const 2)))))"#;

#[wasm_bindgen_test]
async fn worker_runs_update() {
    let mut rt = worker(
        r#"(module
            (import "env" "spawn" (func $spawn (param i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "ran")
            (func (export "update") (param f64)
                (drop (call $spawn (i32.const 0) (i32.const 3)))))"#,
    )
    .await;
    let mut engine = Engine::new_for_test();
    rt.update(&mut engine, 0.016).unwrap();
    assert!(engine.has_name("ran"));
}

#[wasm_bindgen_test]
async fn worker_round_trips_a_one_mib_field() {
    let mut rt = worker(FIELD_ROUND_TRIP).await;
    let mut engine = Engine::new_for_test();
    rt.update(&mut engine, 0.016).unwrap();
    assert!(engine.has_name("ok"), "1 MiB field did not round-trip through the worker");
}

#[wasm_bindgen_test]
async fn worker_links_every_worker_table_import() {
    let mut rt = worker(&wat_importing_all(Backend::Worker)).await;
    rt.update(&mut Engine::new_for_test(), 0.016).expect("every worker table import links");
}

#[wasm_bindgen_test]
async fn worker_rejects_imports_outside_its_subset() {
    let mut rt = worker(
        r#"(module
            (import "env" "task_arg" (func (param i32 i32) (result i32)))
            (memory (export "memory") 1)
            (func (export "update") (param f64)))"#,
    )
    .await;
    match rt.update(&mut Engine::new_for_test(), 0.016) {
        Err(GuestError::Trap(msg)) => assert!(msg.contains("task_arg"), "{msg}"),
        other => panic!("expected a link error, got {other:?}"),
    }
}

#[wasm_bindgen_test]
async fn worker_reports_a_guest_trap_and_recovers() {
    let mut rt = worker(
        r#"(module
            (import "env" "spawn" (func $spawn (param i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "second")
            (global $calls (mut i32) (i32.const 0))
            (func (export "update") (param f64)
                (global.set $calls (i32.add (global.get $calls) (i32.const 1)))
                (if (i32.eq (global.get $calls) (i32.const 1)) (then unreachable))
                (drop (call $spawn (i32.const 0) (i32.const 6)))))"#,
    )
    .await;
    let mut engine = Engine::new_for_test();
    assert!(matches!(rt.update(&mut engine, 0.016), Err(GuestError::Trap(_))));
    rt.update(&mut engine, 0.016).unwrap();
    assert!(engine.has_name("second"));
}

#[wasm_bindgen_test]
async fn worker_and_web_pass_wide_imports() {
    // `set_light` takes 9 wasm params: past wasm-bindgen's 8-arg `Closure` limit
    // on the web backend, and past the old fixed numeric slots on the worker.
    let wat = r#"(module
        (import "env" "set_light" (func $set_light
            (param f64 f64 f64 f64 f64 f64 f64 f64 f64) (result i32)))
        (memory (export "memory") 1)
        (func (export "update") (param f64)
            (drop (call $set_light
                (f64.const 0.1) (f64.const 0.2) (f64.const 0.3)
                (f64.const 0.0) (f64.const 0.0) (f64.const 1.0)
                (f64.const 0.7) (f64.const 0.8) (f64.const 0.9)))))"#;
    let mut engine = Engine::new_for_test();
    worker(wat).await.update(&mut engine, 0.016).unwrap();
    assert_eq!(engine.get_light().0, [0.1, 0.2, 0.3]);

    let mut engine = Engine::new_for_test();
    let mut web = WebWasmRuntime::new(&wat::parse_str(wat).unwrap(), &limits()).unwrap();
    web.update(&mut engine, 0.016).unwrap();
    assert_eq!(engine.get_light().2, [0.7, 0.8, 0.9]);
}

#[wasm_bindgen_test]
fn web_links_every_web_table_import() {
    let wasm = wat::parse_str(wat_importing_all(Backend::Web)).unwrap();
    WebWasmRuntime::new(&wasm, &limits()).expect("every web table import links");
}

#[wasm_bindgen_test]
fn web_rejects_imports_outside_its_subset() {
    let wasm = wat::parse_str(
        r#"(module
            (import "env" "task_arg" (func (param i32 i32) (result i32)))
            (memory (export "memory") 1)
            (func (export "update") (param f64)))"#,
    )
    .unwrap();
    assert!(WebWasmRuntime::new(&wasm, &limits()).is_err());
}
