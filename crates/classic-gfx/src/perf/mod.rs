//! Opt-in frame profiler: per-phase CPU time, per-frame counters and per-phase
//! GPU time (`TIME_ELAPSED` queries), written as JSON lines.
//!
//! Off by default, and free when off: every hook first reads a `const`
//! thread-local flag and returns, so the instrumented engine costs one TLS
//! load per hook.  [`enable`] turns it on for the calling (render) thread.
//!
//! A frame is bracketed by [`begin_frame`] / [`end_frame`].  [`phase`]
//! closes the running phase and opens the next, so the phases tile the frame
//! (no nesting); a name used twice in a frame accumulates.  [`count`] adds to
//! a per-frame counter.
//!
//! With `gpu` on, each phase also runs inside its own `TIME_ELAPSED` query
//! (`EXT_disjoint_timer_query_webgl2` on the web).  A frame's query set is
//! read back a few frames later, once the GPU has finished it, so nothing
//! stalls; on the web a disjoint event (the GPU was reset or throttled)
//! drops every pending set.  `TIME_ELAPSED` spans GPU wall time, so a phase
//! that starves the GPU (a CPU-bound frame) also counts the GPU's idle gap.
//!
//! With `max_in_flight` set (headless runs only: a window's swap already
//! paces), [`end_frame`] fences the frame and waits until at most that many
//! frames are queued on the GPU.  Without it the headless loop runs ahead
//! until the driver blocks inside an arbitrary GL call, which smears the
//! GPU's back-pressure across whichever CPU phase happened to hit it.
//!
//! # Output (schema `perf: 1`)
//!
//! Native writes JSON lines to the configured file:
//!
//! ```text
//! {"perf":1,"kind":"header","renderer":"…","vendor":"…","version":"…","gpu":true,"max_in_flight":2}
//! {"kind":"frame","f":12,"wall":13.88,"cpu":8.89,"pace":0.41,"phases":{"update":3.07,…},"counters":{"lights":88,…}}
//! {"kind":"gpu","f":12,"total":11.95,"phases":{"models":2.13,…}}
//! {"kind":"section","f":120,"name":"models_z1","params":{"zoom":1.0,…}}
//! {"kind":"section_end","f":900}
//! {"kind":"summary","frames":781,"wall":{"mean":…,"median":…,"p95":…,"p99":…,"max":…},"cpu":{…},"gpu":{…},"phases_cpu":{…},"phases_gpu":{…},"counters":{…}}
//! ```
//!
//! All times are milliseconds.  `wall` is the time since the previous
//! frame's [`begin_frame`] (0 on the first frame); `cpu` is
//! [`begin_frame`]→[`end_frame`] minus `pace`, the time spent waiting on the
//! frame fence.  A `gpu` line arrives a few frames after its `frame` line
//! (join on `f`).  The `summary` line is written by [`finish`]; its
//! `phases_*` and `counters` are per-frame means.  The web writes no frame
//! lines: it logs one `summary` line to the console every
//! [`WEB_SUMMARY_FRAMES`] frames.
//!
//! A `section` line ([`section`], the guest `bench_section` import) starts a
//! named benchmark section at frame `f`: that frame and every later one
//! belong to it, until the next `section` or a `section_end`
//! ([`section_end`], `bench_end`).  `cargo xtask bench` builds its
//! per-section report from these lines.

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
use native as backend;

#[cfg(target_arch = "wasm32")]
mod web;
#[cfg(target_arch = "wasm32")]
use web as backend;
#[cfg(target_arch = "wasm32")]
pub use web::attach_webgl2;

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::fmt::Write as _;
use std::rc::Rc;

use glow::HasContext;

/// Frames a GPU query set may wait before it is read back blocking
/// (native) or dropped (web, where a read never blocks).
const GPU_RING: usize = 6;

/// The web summary period, in frames.
pub const WEB_SUMMARY_FRAMES: u64 = 600;

/// What to profile and where to write it.
#[derive(Clone, Debug, Default)]
pub struct PerfConfig {
    /// Native: the JSON-lines output file.  Web: unused (the console).
    pub out: String,
    /// Time each phase on the GPU too.
    pub gpu: bool,
    /// Fence each frame and wait until at most this many are in flight
    /// (headless only; `None` leaves pacing to the platform).
    pub max_in_flight: Option<usize>,
}

thread_local! {
    static ON: Cell<bool> = const { Cell::new(false) };
    static PERF: RefCell<Option<Perf>> = const { RefCell::new(None) };
}

/// Whether profiling is on for this thread (gate any counter that costs
/// something to compute).
#[inline]
pub fn enabled() -> bool {
    ON.with(|on| on.get())
}

/// Turn profiling on for the calling thread.  A second call is ignored; a
/// sink that cannot be opened logs an error and leaves profiling off.
pub fn enable(config: PerfConfig) {
    if enabled() {
        return;
    }
    let Some(out) = backend::Out::open(&config.out) else { return };
    PERF.with(|p| *p.borrow_mut() = Some(Perf::new(config, out)));
    ON.with(|on| on.set(true));
}

/// Start a frame (top of `Engine::frame`).
#[inline]
pub fn begin_frame(gl: &Rc<glow::Context>) {
    if enabled() {
        with(|p| p.begin_frame(gl));
    }
}

/// Close the running phase and open `name`.
#[inline]
pub fn phase(name: &'static str) {
    if enabled() {
        with(|p| p.phase(name));
    }
}

/// Open phase `name` until the returned guard drops, then reopen the phase
/// that was running (a sub-phase carved out of an enclosing one, e.g. the
/// model update inside `update`).
#[inline]
#[must_use = "the phase ends when the guard drops"]
pub fn scope(name: &'static str) -> Scope {
    let mut prev = None;
    if enabled() {
        with(|p| {
            prev = p.cur.map(|(n, _)| n);
            p.phase(name);
        });
    }
    Scope { prev }
}

/// The guard of [`scope`].
pub struct Scope {
    prev: Option<&'static str>,
}

impl Drop for Scope {
    #[inline]
    fn drop(&mut self) {
        if let Some(prev) = self.prev {
            phase(prev);
        }
    }
}

/// Add `n` to the per-frame counter `name`.
#[inline]
pub fn count(name: &'static str, n: u64) {
    if enabled() {
        with(|p| p.count(name, n));
    }
}

/// End the frame (bottom of `Engine::frame`).
#[inline]
pub fn end_frame() {
    if enabled() {
        with(|p| p.end_frame());
    }
}

/// Start the benchmark section `name` at the current frame; `params` is a
/// JSON object describing it (camera, viewport, ...).
pub fn section(name: &str, params: serde_json::Value) {
    if enabled() {
        with(|p| {
            let line = serde_json::json!({
                "kind": "section",
                "f": p.frame,
                "name": name,
                "params": params,
            });
            p.out.line(&line.to_string());
        });
    }
}

/// End the running benchmark section at the current frame.
pub fn section_end() {
    if enabled() {
        with(|p| {
            let line = format!("{{\"kind\":\"section_end\",\"f\":{}}}", p.frame);
            p.out.line(&line);
        });
    }
}

/// Write the summary and flush (process exit).  Touches no GL, so it is
/// safe after the context is gone; GPU sets still pending are dropped.
pub fn finish() {
    if enabled() {
        with(|p| p.finish());
        PERF.with(|p| *p.borrow_mut() = None);
        ON.with(|on| on.set(false));
    }
}

fn with(f: impl FnOnce(&mut Perf)) {
    PERF.with(|p| {
        if let Some(p) = p.borrow_mut().as_mut() {
            f(p);
        }
    });
}

/// A name → value list (a handful of entries: a linear scan beats hashing).
#[derive(Default)]
struct Tally(Vec<(&'static str, f64)>);

impl Tally {
    fn add(&mut self, name: &'static str, v: f64) {
        match self.0.iter_mut().find(|(k, _)| *k == name) {
            Some(e) => e.1 += v,
            None => self.0.push((name, v)),
        }
    }

    fn total(&self) -> f64 {
        self.0.iter().map(|(_, v)| v).sum()
    }

    fn write_json(&self, s: &mut String, prec: usize) {
        s.push('{');
        for (i, (k, v)) in self.0.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            let _ = write!(s, "\"{k}\":{v:.prec$}");
        }
        s.push('}');
    }
}

/// Per-run accumulation for the summary line.
#[derive(Default)]
struct Summary {
    wall: Vec<f64>,
    cpu: Vec<f64>,
    gpu: Vec<f64>,
    phases_cpu: Tally,
    phases_gpu: Tally,
    counters: Tally,
}

impl Summary {
    fn line(&self) -> String {
        let n = self.cpu.len().max(1) as f64;
        let ng = self.gpu.len().max(1) as f64;
        let mut s = format!("{{\"kind\":\"summary\",\"frames\":{}", self.cpu.len());
        for (key, v) in [("wall", &self.wall), ("cpu", &self.cpu), ("gpu", &self.gpu)] {
            let st = Stats::of(v);
            let _ = write!(
                s,
                ",\"{key}\":{{\"n\":{},\"mean\":{:.4},\"median\":{:.4},\"p95\":{:.4},\"p99\":{:.4},\"max\":{:.4}}}",
                v.len(),
                st.mean,
                st.median,
                st.p95,
                st.p99,
                st.max
            );
        }
        for (key, t, d) in [
            ("phases_cpu", &self.phases_cpu, n),
            ("phases_gpu", &self.phases_gpu, ng),
            ("counters", &self.counters, n),
        ] {
            let _ = write!(s, ",\"{key}\":");
            Tally(t.0.iter().map(|&(k, v)| (k, v / d)).collect()).write_json(&mut s, 4);
        }
        s.push('}');
        s
    }
}

/// Mean and order statistics of a sample (all 0 when empty).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stats {
    pub mean: f64,
    pub median: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
}

impl Stats {
    /// Nearest-rank percentiles over `v`.
    pub fn of(v: &[f64]) -> Self {
        if v.is_empty() {
            return Self::default();
        }
        let mut s = v.to_vec();
        s.sort_by(f64::total_cmp);
        let rank = |p: f64| s[((p * s.len() as f64).ceil() as usize).clamp(1, s.len()) - 1];
        Self {
            mean: s.iter().sum::<f64>() / s.len() as f64,
            median: rank(0.5),
            p95: rank(0.95),
            p99: rank(0.99),
            max: s[s.len() - 1],
        }
    }
}

/// One frame's GPU queries, in phase order.
struct GpuSet {
    frame: u64,
    queries: Vec<(&'static str, backend::Query)>,
}

struct Gpu {
    timer: backend::GpuTimer,
    /// The query of the running phase, if any.
    open: bool,
    current: Vec<(&'static str, backend::Query)>,
    pending: VecDeque<GpuSet>,
    pool: Vec<backend::Query>,
}

struct Perf {
    config: PerfConfig,
    out: backend::Out,
    /// Created on the first frame (needs the GL context).
    gpu: Option<Gpu>,
    pacer: Option<backend::Pacer>,
    started: bool,
    frame: u64,
    last_begin: Option<f64>,
    frame_start: f64,
    wall: f64,
    cur: Option<(&'static str, f64)>,
    cpu: Tally,
    counters: Tally,
    summary: Summary,
    line: String,
}

impl Perf {
    fn new(config: PerfConfig, out: backend::Out) -> Self {
        Self {
            config,
            out,
            gpu: None,
            pacer: None,
            started: false,
            frame: 0,
            last_begin: None,
            frame_start: 0.0,
            wall: 0.0,
            cur: None,
            cpu: Tally::default(),
            counters: Tally::default(),
            summary: Summary::default(),
            line: String::new(),
        }
    }

    fn start(&mut self, gl: &Rc<glow::Context>) {
        self.started = true;
        if self.config.gpu {
            self.gpu = backend::GpuTimer::new(gl).map(|timer| Gpu {
                timer,
                open: false,
                current: Vec::new(),
                pending: VecDeque::new(),
                pool: Vec::new(),
            });
        }
        if let Some(n) = self.config.max_in_flight {
            self.pacer = backend::Pacer::new(gl, n.max(1));
        }
        let (renderer, vendor, version) = unsafe {
            (
                gl.get_parameter_string(glow::RENDERER),
                gl.get_parameter_string(glow::VENDOR),
                gl.get_parameter_string(glow::VERSION),
            )
        };
        let header = serde_json::json!({
            "perf": 1,
            "kind": "header",
            "renderer": renderer,
            "vendor": vendor,
            "version": version,
            "gpu": self.gpu.is_some(),
            "max_in_flight": self.pacer.as_ref().and(self.config.max_in_flight),
        });
        self.out.line(&header.to_string());
    }

    fn begin_frame(&mut self, gl: &Rc<glow::Context>) {
        if !self.started {
            self.start(gl);
        }
        self.resolve_gpu();
        let now = backend::now_ms();
        self.wall = self.last_begin.map_or(0.0, |t| now - t);
        self.last_begin = Some(now);
        self.frame_start = now;
        self.cpu.0.clear();
        self.counters.0.clear();
        self.cur = None;
        self.phase("frame_head");
    }

    fn phase(&mut self, name: &'static str) {
        let now = backend::now_ms();
        if let Some((n, t)) = self.cur.take() {
            self.cpu.add(n, now - t);
        }
        self.cur = Some((name, now));
        let Some(g) = self.gpu.as_mut() else { return };
        if g.open {
            g.timer.end();
            g.open = false;
        }
        let q = match g.pool.pop() {
            Some(q) => Some(q),
            None => g.timer.create(),
        };
        if let Some(q) = q {
            g.timer.begin(&q);
            g.open = true;
            g.current.push((name, q));
        }
    }

    fn count(&mut self, name: &'static str, n: u64) {
        self.counters.add(name, n as f64);
    }

    fn end_frame(&mut self) {
        let now = backend::now_ms();
        if let Some((n, t)) = self.cur.take() {
            self.cpu.add(n, now - t);
        }
        if let Some(g) = self.gpu.as_mut() {
            if g.open {
                g.timer.end();
                g.open = false;
            }
            let queries = std::mem::take(&mut g.current);
            if !queries.is_empty() {
                g.pending.push_back(GpuSet { frame: self.frame, queries });
            }
        }
        let pace = self.pacer.as_mut().map_or(0.0, |p| p.end_frame());
        let cpu = backend::now_ms() - self.frame_start - pace;

        self.line.clear();
        let _ = write!(
            self.line,
            "{{\"kind\":\"frame\",\"f\":{},\"wall\":{:.4},\"cpu\":{:.4},\"pace\":{:.4},\"phases\":",
            self.frame, self.wall, cpu, pace
        );
        self.cpu.write_json(&mut self.line, 4);
        self.line.push_str(",\"counters\":");
        self.counters.write_json(&mut self.line, 0);
        self.line.push('}');
        self.out.frame_line(&self.line);

        let s = &mut self.summary;
        if self.frame > 0 {
            s.wall.push(self.wall);
        }
        s.cpu.push(cpu);
        for &(k, v) in &self.cpu.0 {
            s.phases_cpu.add(k, v);
        }
        for &(k, v) in &self.counters.0 {
            s.counters.add(k, v);
        }
        self.frame += 1;
        if self.out.wants_periodic_summary() && self.frame.is_multiple_of(WEB_SUMMARY_FRAMES) {
            let line = self.summary.line();
            self.out.line(&line);
            self.summary = Summary::default();
        }
    }

    /// Read back every finished GPU set; past [`GPU_RING`] pending, block on
    /// (native) or drop (web) the oldest.
    fn resolve_gpu(&mut self) {
        let Some(g) = self.gpu.as_mut() else { return };
        if g.timer.disjoint() {
            for set in g.pending.drain(..) {
                g.pool.extend(set.queries.into_iter().map(|(_, q)| q));
            }
            return;
        }
        while let Some(set) = g.pending.front() {
            let overdue = g.pending.len() > GPU_RING;
            let last = &set.queries[set.queries.len() - 1].1;
            if !g.timer.available(last, overdue) {
                if overdue {
                    // Web: a read never blocks, so give the set up.
                    let set = g.pending.pop_front().expect("front");
                    g.pool.extend(set.queries.into_iter().map(|(_, q)| q));
                    continue;
                }
                break;
            }
            let set = g.pending.pop_front().expect("front");
            let mut phases = Tally::default();
            for (name, q) in set.queries {
                phases.add(name, g.timer.result_ns(&q) as f64 / 1e6);
                g.pool.push(q);
            }
            let total = phases.total();
            self.line.clear();
            let _ = write!(
                self.line,
                "{{\"kind\":\"gpu\",\"f\":{},\"total\":{total:.4},\"phases\":",
                set.frame
            );
            phases.write_json(&mut self.line, 4);
            self.line.push('}');
            self.out.frame_line(&self.line);
            self.summary.gpu.push(total);
            for &(k, v) in &phases.0 {
                self.summary.phases_gpu.add(k, v);
            }
        }
    }

    fn finish(&mut self) {
        let line = self.summary.line();
        self.out.line(&line);
        self.out.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_are_nearest_rank() {
        let v: Vec<f64> = (1..=100).map(f64::from).collect();
        let s = Stats::of(&v);
        assert_eq!((s.mean, s.median, s.p95, s.p99, s.max), (50.5, 50.0, 95.0, 99.0, 100.0));
        assert_eq!(Stats::of(&[]), Stats::default());
        assert_eq!(Stats::of(&[3.0]).p99, 3.0);
    }

    #[test]
    fn tally_accumulates_and_serialises() {
        let mut t = Tally::default();
        t.add("a", 1.0);
        t.add("b", 2.5);
        t.add("a", 0.5);
        let mut s = String::new();
        t.write_json(&mut s, 2);
        assert_eq!(s, r#"{"a":1.50,"b":2.50}"#);
        assert_eq!(t.total(), 4.0);
    }

    #[test]
    fn off_hooks_are_inert() {
        // Never enabled on this test thread: every hook is a no-op.
        assert!(!enabled());
        phase("x");
        count("y", 3);
        end_frame();
        finish();
        assert!(!enabled());
    }

    #[test]
    fn summary_line_parses_as_json() {
        let mut s = Summary { cpu: vec![1.0, 2.0], wall: vec![2.0], ..Default::default() };
        s.phases_cpu.add("update", 3.0);
        s.counters.add("lights", 8.0);
        let v: serde_json::Value = serde_json::from_str(&s.line()).unwrap();
        assert_eq!(v["kind"], "summary");
        assert_eq!(v["frames"], 2);
        assert_eq!(v["phases_cpu"]["update"], 1.5);
        assert_eq!(v["counters"]["lights"], 4.0);
        assert_eq!(v["gpu"]["n"], 0);
    }
}
