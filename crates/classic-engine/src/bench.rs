//! Benchmark sections: a ROM's guest marks named stretches of frames
//! (`bench_section` / `bench_end` imports) so `cargo xtask bench` can report
//! each one on its own.  The markers land in the frame profiler's output
//! (`classic_gfx::perf::section`), so they only record anything while
//! `CLASSIC_PERF` is on.
//!
//! The mode (`CLASSIC_BENCH=full|short`, web `?bench` / `?bench=short`)
//! tells the guest what to run (`bench_mode`): interactive play, the full
//! benchmark, or a short one (fewer, shorter sections, for a smoke run).
//! In either bench mode `bench_end` also closes the app.

use crate::env_config::EnvConfig;
use crate::Engine;

/// What a bench-aware ROM should run (`bench_mode()` to the guest).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BenchMode {
    /// Not benchmarking: play normally (`0`).
    #[default]
    Interactive = 0,
    /// The full benchmark (`1`).
    Full = 1,
    /// A short benchmark, for smoke runs (`2`).
    Short = 2,
}

impl BenchMode {
    /// `None`/empty/`0`/`off` = interactive, `short` = short, any other
    /// value (including a bare `?bench`) = full.
    pub fn parse(v: Option<&str>) -> Self {
        match v.map(str::trim) {
            None | Some("0" | "off" | "false") => Self::Interactive,
            Some("short") => Self::Short,
            Some(_) => Self::Full,
        }
    }

    /// The `CLASSIC_BENCH` / `--bench` spelling.
    pub fn name(self) -> &'static str {
        match self {
            Self::Interactive => "interactive",
            Self::Full => "full",
            Self::Short => "short",
        }
    }
}

/// A section marker waiting for the end of the frame that set it.
#[derive(Debug, PartialEq)]
pub(crate) enum Marker {
    Section(String),
    End,
}

/// Per-engine bench state.
#[derive(Debug, Default)]
pub struct BenchState {
    /// The running section's name.
    pub section: Option<String>,
    /// `bench_end` was called.
    pub ended: bool,
    pending: Option<Marker>,
}

impl Engine {
    /// The process's bench mode.
    pub fn bench_mode(&self) -> BenchMode {
        EnvConfig::get().bench
    }

    /// Start the bench section `name` with this frame.  Its marker (with
    /// the camera and viewport as params) is written at the end of the
    /// frame, so the guest may still move the camera after the call.
    pub fn bench_section(&mut self, name: &str) {
        log::info!("bench: section `{name}`");
        self.bench.section = Some(name.to_owned());
        self.bench.pending = Some(Marker::Section(name.to_owned()));
    }

    /// End the running section with this frame (which is excluded); in a
    /// bench mode, also close the app.
    pub fn bench_end(&mut self) {
        log::info!("bench: end");
        self.bench.section = None;
        self.bench.ended = true;
        self.bench.pending = Some(Marker::End);
        if self.bench_mode() != BenchMode::Interactive {
            self.test_should_close = true;
        }
    }

    /// Write the frame's pending section marker (end of `Engine::frame`).
    pub(crate) fn flush_bench_marker(&mut self) {
        match self.bench.pending.take() {
            Some(Marker::Section(name)) => {
                let params = serde_json::json!({
                    "zoom": self.camera.scale.x,
                    "camera": self.camera.position.to_array(),
                    "viewport": [self.camera.size.x, self.camera.size.y],
                    "entities": self.world.len(),
                });
                classic_gfx::perf::section(&name, params);
            }
            Some(Marker::End) => classic_gfx::perf::section_end(),
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_parse() {
        assert_eq!(BenchMode::parse(None), BenchMode::Interactive);
        assert_eq!(BenchMode::parse(Some("0")), BenchMode::Interactive);
        assert_eq!(BenchMode::parse(Some("")), BenchMode::Full);
        assert_eq!(BenchMode::parse(Some("full")), BenchMode::Full);
        assert_eq!(BenchMode::parse(Some(" short ")), BenchMode::Short);
        assert_eq!(BenchMode::Short as i32, 2);
    }

    #[test]
    fn markers_queue_until_the_frame_ends() {
        let mut e = Engine::new_for_test();
        e.bench_section("models");
        assert_eq!(e.bench.section.as_deref(), Some("models"));
        assert_eq!(e.bench.pending, Some(Marker::Section("models".into())));
        e.flush_bench_marker();
        assert_eq!(e.bench.pending, None);
        e.bench_end();
        assert!(e.bench.ended && e.bench.section.is_none());
        // Not in a bench mode under `cargo test`: the app keeps running.
        assert!(!e.test_should_close);
        e.flush_bench_marker();
        assert_eq!(e.bench.pending, None);
    }
}
