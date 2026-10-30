//! Web profiler backend: `performance.now()` clock (`Instant` panics on
//! wasm32), console summaries, and `EXT_disjoint_timer_query_webgl2` timer
//! queries on the raw `WebGl2RenderingContext` (glow reads query results as
//! numbers, but availability and the disjoint flag are JS booleans).

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::JsValue;
use web_sys::{WebGl2RenderingContext, WebGlQuery};

/// `EXT_disjoint_timer_query_webgl2` enums.
const TIME_ELAPSED_EXT: u32 = 0x88BF;
const GPU_DISJOINT_EXT: u32 = 0x8FBB;

thread_local! {
    static RAW: RefCell<Option<WebGl2RenderingContext>> = const { RefCell::new(None) };
}

/// Hand the profiler the raw context behind the engine's `glow::Context`
/// (GPU timing needs it; without it only CPU phases are recorded).
pub fn attach_webgl2(ctx: WebGl2RenderingContext) {
    RAW.with(|r| *r.borrow_mut() = Some(ctx));
}

/// Milliseconds since page load.
pub(super) fn now_ms() -> f64 {
    web_sys::window().and_then(|w| w.performance()).map_or_else(js_sys::Date::now, |p| p.now())
}

/// Console output: one summary line per period, never per-frame lines.
pub(super) struct Out;

impl Out {
    pub(super) fn open(_path: &str) -> Option<Self> {
        Some(Self)
    }

    pub(super) fn line(&mut self, s: &str) {
        web_sys::console::log_1(&JsValue::from_str(&format!("[perf] {s}")));
    }

    pub(super) fn frame_line(&mut self, _s: &str) {}

    pub(super) fn wants_periodic_summary(&self) -> bool {
        true
    }

    pub(super) fn flush(&mut self) {}
}

pub(super) type Query = WebGlQuery;

pub(super) struct GpuTimer(WebGl2RenderingContext);

impl GpuTimer {
    pub(super) fn new(_gl: &Rc<glow::Context>) -> Option<Self> {
        let Some(ctx) = RAW.with(|r| r.borrow().clone()) else {
            log::warn!("perf: no WebGL2 context attached; GPU timing off");
            return None;
        };
        match ctx.get_extension("EXT_disjoint_timer_query_webgl2") {
            Ok(Some(_)) => Some(Self(ctx)),
            _ => {
                log::warn!("perf: EXT_disjoint_timer_query_webgl2 unavailable; GPU timing off");
                None
            }
        }
    }

    pub(super) fn create(&self) -> Option<Query> {
        self.0.create_query()
    }

    pub(super) fn delete(&self, q: Query) {
        self.0.delete_query(Some(&q));
    }

    pub(super) fn begin(&self, q: &Query) {
        self.0.begin_query(TIME_ELAPSED_EXT, q);
    }

    pub(super) fn end(&self) {
        self.0.end_query(TIME_ELAPSED_EXT);
    }

    /// Whether `q`'s result is ready.  Never blocks: WebGL only publishes
    /// results between tasks.
    pub(super) fn available(&self, q: &Query, _block: bool) -> bool {
        self.0
            .get_query_parameter(q, WebGl2RenderingContext::QUERY_RESULT_AVAILABLE)
            .as_bool()
            .unwrap_or(false)
    }

    pub(super) fn result_ns(&self, q: &Query) -> u64 {
        self.0.get_query_parameter(q, WebGl2RenderingContext::QUERY_RESULT).as_f64().unwrap_or(0.0)
            as u64
    }

    /// The GPU was reset or throttled since the last check: every pending
    /// result is unreliable.
    pub(super) fn disjoint(&self) -> bool {
        self.0.get_parameter(GPU_DISJOINT_EXT).ok().and_then(|v| v.as_bool()).unwrap_or(false)
    }
}

/// The browser paces frames (`requestAnimationFrame`): no fencing.
pub(super) struct Pacer;

impl Pacer {
    pub(super) fn new(_gl: &Rc<glow::Context>, _max: usize) -> Option<Self> {
        None
    }

    pub(super) fn end_frame(&mut self) -> f64 {
        0.0
    }
}
