//! Native profiler backend: `Instant` clock, a JSON-lines file, `glow`
//! timer queries (desktop `ARB_timer_query`, or GLES
//! `EXT_disjoint_timer_query`) and fence pacing.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::rc::Rc;
use std::sync::LazyLock;
use std::time::Instant;

use glow::HasContext;

static EPOCH: LazyLock<Instant> = LazyLock::new(Instant::now);

/// Milliseconds since the first call.
pub(super) fn now_ms() -> f64 {
    EPOCH.elapsed().as_secs_f64() * 1e3
}

/// The JSON-lines output file.
pub(super) struct Out(Box<dyn Write>);

impl Out {
    pub(super) fn open(path: &str) -> Option<Self> {
        match File::create(path) {
            Ok(f) => Some(Self(Box::new(BufWriter::new(f)))),
            Err(e) => {
                log::error!("CLASSIC_PERF: cannot create {path:?}: {e}");
                None
            }
        }
    }

    /// Any writer (the unit tests' in-memory sink).
    #[cfg(test)]
    pub(super) fn writer(w: impl Write + 'static) -> Self {
        Self(Box::new(w))
    }

    pub(super) fn line(&mut self, s: &str) {
        let _ = writeln!(self.0, "{s}");
    }

    pub(super) fn frame_line(&mut self, s: &str) {
        self.line(s);
    }

    pub(super) fn wants_periodic_summary(&self) -> bool {
        false
    }

    pub(super) fn flush(&mut self) {
        let _ = self.0.flush();
    }
}

pub(super) type Query = glow::Query;

/// The context, and whether it is GLES (whose `EXT_disjoint_timer_query`
/// reports disjoint events; desktop timer queries have none).
pub(super) struct GpuTimer(Rc<glow::Context>, bool);

impl GpuTimer {
    pub(super) fn new(gl: &Rc<glow::Context>) -> Option<Self> {
        let v = gl.version();
        // Timer queries are core in desktop GL 3.3; GLES needs the extension.
        let ok = if v.is_embedded {
            gl.supported_extensions().contains("GL_EXT_disjoint_timer_query")
        } else {
            (v.major, v.minor) >= (3, 3) || gl.supported_extensions().contains("GL_ARB_timer_query")
        };
        if !ok {
            log::warn!("CLASSIC_PERF_GPU: no timer queries on this context; GPU timing off");
        }
        ok.then(|| Self(gl.clone(), v.is_embedded))
    }

    pub(super) fn create(&self) -> Option<Query> {
        unsafe { self.0.create_query().ok() }
    }

    pub(super) fn delete(&self, q: Query) {
        unsafe { self.0.delete_query(q) };
    }

    pub(super) fn begin(&self, q: &Query) {
        unsafe { self.0.begin_query(glow::TIME_ELAPSED, *q) };
    }

    pub(super) fn end(&self) {
        unsafe { self.0.end_query(glow::TIME_ELAPSED) };
    }

    /// Whether `q`'s result is ready; `block` waits for it instead.
    pub(super) fn available(&self, q: &Query, block: bool) -> bool {
        block || unsafe { self.0.get_query_parameter_u32(*q, glow::QUERY_RESULT_AVAILABLE) != 0 }
    }

    pub(super) fn result_ns(&self, q: &Query) -> u64 {
        unsafe { self.0.get_query_parameter_u32(*q, glow::QUERY_RESULT) as u64 }
    }

    /// The GPU was reset or throttled since the last check (GLES only).
    pub(super) fn disjoint(&self) -> bool {
        const GPU_DISJOINT_EXT: u32 = 0x8FBB;
        self.1 && unsafe { self.0.get_parameter_i32(GPU_DISJOINT_EXT) != 0 }
    }
}

/// Fences each frame and keeps at most `max` of them in flight.
pub(super) struct Pacer {
    gl: Rc<glow::Context>,
    max: usize,
    fences: std::collections::VecDeque<glow::Fence>,
}

impl Pacer {
    pub(super) fn new(gl: &Rc<glow::Context>, max: usize) -> Option<Self> {
        Some(Self { gl: gl.clone(), max, fences: Default::default() })
    }

    /// Fence this frame, then wait out the oldest while more than `max` are
    /// queued.  Returns the milliseconds spent waiting.
    pub(super) fn end_frame(&mut self) -> f64 {
        let gl = &self.gl;
        let start = now_ms();
        unsafe {
            if let Ok(f) = gl.fence_sync(glow::SYNC_GPU_COMMANDS_COMPLETE, 0) {
                self.fences.push_back(f);
            }
            while self.fences.len() > self.max {
                let f = self.fences.pop_front().expect("len > max");
                // 100 ms slices, so a lost context cannot hang the loop.
                for _ in 0..50 {
                    let r = gl.client_wait_sync(f, glow::SYNC_FLUSH_COMMANDS_BIT, 100_000_000);
                    if r != glow::TIMEOUT_EXPIRED {
                        break;
                    }
                }
                gl.delete_sync(f);
            }
        }
        now_ms() - start
    }
}
