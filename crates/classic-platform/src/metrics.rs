//! Periodic process resource sampling for boot-perf debugging.
//!
//! [`ResourceUsageSampler`] emits a [`classic_rom::BootEvent::ResourceUsage`] to
//! a [`classic_rom::BootSink`] every `interval`, so the boot event stream (and
//! the loading screen's header) carries a CPU% + memory trace through the whole
//! boot.
//!
//! - **Native** — the [`sysinfo`] crate samples the current process's CPU% and
//!   RSS on Linux/macOS/Windows.  `cpu_percent` is process-wide (all threads),
//!   so parallel decode/transcode pushes it past 100.
//! - **Web** — the browser sandbox does not expose per-process CPU, so only the
//!   JS heap (`performance.memory.usedJSHeapSize`, Chromium; `0` elsewhere) is
//!   reported; `cpu_percent` stays `0`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use classic_rom::{BootEvent, BootSink};

/// A short-lived background sampler emitting [`BootEvent::ResourceUsage`]
/// events.  Drop (or call [`stop`](Self::stop)) to end sampling.  It never
/// fails the boot: unreadable samples are silently skipped.
pub struct ResourceUsageSampler {
    stop: Arc<AtomicBool>,
    #[cfg(not(target_arch = "wasm32"))]
    handle: Option<std::thread::JoinHandle<()>>,
    #[cfg(target_arch = "wasm32")]
    _handle: (),
}

impl ResourceUsageSampler {
    /// Start emitting a [`BootEvent::ResourceUsage`] to `sink` every `interval`.
    pub fn start(sink: Arc<dyn BootSink>, interval: Duration) -> Self {
        let stop = Arc::new(AtomicBool::new(false));

        #[cfg(not(target_arch = "wasm32"))]
        {
            let stop_flag = Arc::clone(&stop);
            let handle = std::thread::spawn(move || native_loop(stop_flag, sink, interval));
            Self { stop, handle: Some(handle) }
        }

        #[cfg(target_arch = "wasm32")]
        {
            let stop_flag = Arc::clone(&stop);
            wasm_bindgen_futures::spawn_local(web_loop(stop_flag, sink, interval));
            Self { stop, _handle: () }
        }
    }

    /// Signal the sampler to stop (and, natively, join its thread).
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for ResourceUsageSampler {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Native sampler: poll the current process's CPU% + RSS via [`sysinfo`].
#[cfg(not(target_arch = "wasm32"))]
fn native_loop(stop: Arc<AtomicBool>, sink: Arc<dyn BootSink>, interval: Duration) {
    use sysinfo::{Pid, ProcessesToUpdate, System};

    let pid = Pid::from_u32(std::process::id());
    let mut sys = System::new();
    // First refresh establishes the cpu-usage baseline (the delta is computed
    // against the previous refresh).
    sys.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);

    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(interval);
        if stop.load(Ordering::Relaxed) {
            break;
        }
        sys.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
        if let Some(process) = sys.process(pid) {
            let cpu_percent = process.cpu_usage().round() as u32;
            let rss_bytes = process.memory();
            sink.on_event(BootEvent::ResourceUsage { cpu_percent, rss_bytes });
        }
    }
}

/// Web sampler: poll the JS heap only (no per-process CPU in the sandbox).
#[cfg(target_arch = "wasm32")]
async fn web_loop(stop: Arc<AtomicBool>, sink: Arc<dyn BootSink>, interval: Duration) {
    let millis = interval.as_millis().clamp(1, u32::MAX as u128) as i32;
    while !stop.load(Ordering::Relaxed) {
        sleep(millis).await;
        if stop.load(Ordering::Relaxed) {
            break;
        }
        sink.on_event(BootEvent::ResourceUsage { cpu_percent: 0, rss_bytes: js_heap_bytes() });
    }
}

/// Await a `setTimeout` tick on web.
#[cfg(target_arch = "wasm32")]
async fn sleep(millis: i32) {
    use wasm_bindgen::JsCast;

    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        let resolve = resolve.clone();
        let cb = wasm_bindgen::closure::Closure::once(move || {
            let _ = resolve.call0(&wasm_bindgen::JsValue::UNDEFINED);
        });
        if let Some(window) = web_sys::window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
                cb.as_ref().unchecked_ref(),
                millis,
            );
        }
        cb.forget();
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

/// The JS heap size in bytes (`performance.memory.usedJSHeapSize`, Chromium
/// only; `0` where the browser doesn't expose it).
#[cfg(target_arch = "wasm32")]
fn js_heap_bytes() -> u64 {
    js_sys::eval("(performance.memory && performance.memory.usedJSHeapSize) || 0")
        .ok()
        .and_then(|v| v.as_f64())
        .map(|v| v as u64)
        .unwrap_or(0)
}
