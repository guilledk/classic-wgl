//! Boot progress events and sinks.
//!
//! The boot pipeline (ROM resolve → archive decompress → parse → resource
//! decode/upload → guest compile/instantiate → entity spawn) is made
//! *observable* through a stream of [`BootEvent`]s delivered to a [`BootSink`].
//! This one type feeds both loading-screen modes (console = log lines; visual
//! = graph state) and is the instrumentation backbone for boot-time
//! measurement and the async offload work that follows.

use std::time::Duration;

use crate::resource::ResourceKind;

/// A lifecycle event emitted while a ROM dependency DAG is fetched, parsed,
/// and hydrated.
///
/// Every variant owns its data (`Send`), so the stream can later cross the
/// main-thread / background-thread boundary with no borrowed references into
/// the archive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BootEvent {
    /// ROM resolution (fetch/read) is starting.
    ResolveStarted { spec: String },
    /// A named ROM's archive bytes have begun streaming in (download/read).
    /// `total` is the known byte length when advertised (e.g. `Content-Length`),
    /// otherwise `None` (indeterminate progress).
    RomFetchStarted { name: String, total: Option<u64> },
    /// Download/read progress for a named ROM's archive bytes.  `received` is
    /// the byte count so far; `total` is `0` when the length is unknown.
    RomFetchProgress { name: String, received: u64, total: u64 },
    /// A named ROM's archive bytes have been materialised.
    RomFetched { name: String, bytes: usize },
    /// A named ROM's archive was decompressed.
    RomDecompressed { name: String, entries: usize },
    /// A named ROM was parsed (manifest + resources + state).  `deps` is the
    /// ROM's declared dependency names, surfaced here so a loading screen can
    /// lay out the DAG before the full resource payload is resolved.
    RomParsed { name: String, resources: usize, deps: Vec<String> },
    /// A resource was decoded to pixels on the CPU.
    ResourceDecoded { rom: String, kind: ResourceKind, name: String, dims: (u32, u32) },
    /// A decoded resource was uploaded to the GL layer.
    TextureUploaded { name: String },
    /// A shader program was compiled.
    ShaderCompiled { name: String },
    /// A guest WASM module is being compiled.
    GuestCompiling { rom: String },
    /// A guest WASM module was compiled and instantiated.
    GuestInstantiated { rom: String },
    /// A ROM's entity graph was spawned into the world.
    StateSpawned { rom: String, entities: usize },
    /// The whole boot completed successfully.
    BootComplete { elapsed: Duration },
    /// The boot failed at a named phase.
    BootFailed { phase: &'static str, error: String },
    /// A periodic process resource sample taken during boot (native only; a
    /// debugging aid).  `cpu_percent` is process-wide over the sampling window
    /// and can exceed 100 on multi-core (parallel decode/transcode).
    ResourceUsage { cpu_percent: u32, rss_bytes: u64 },
}

impl BootEvent {
    /// A single-line, human-readable description for console/log rendering.
    pub fn describe(&self) -> String {
        match self {
            BootEvent::ResolveStarted { spec } => format!("resolve started (spec={spec})"),
            BootEvent::RomFetchStarted { name, total } => match total {
                Some(total) => format!("fetching `{name}` ({total} bytes)"),
                None => format!("fetching `{name}`"),
            },
            BootEvent::RomFetchProgress { name, received, total } => {
                if *total > 0 {
                    format!("fetching `{name}` {received}/{total} bytes")
                } else {
                    format!("fetching `{name}` {received} bytes")
                }
            }
            BootEvent::RomFetched { name, bytes } => format!("fetched `{name}` ({bytes} bytes)"),
            BootEvent::RomDecompressed { name, entries } => {
                format!("decompressed `{name}` ({entries} entries)")
            }
            BootEvent::RomParsed { name, resources, .. } => {
                format!("parsed `{name}` ({resources} resources)")
            }
            BootEvent::ResourceDecoded { rom, kind, name, dims } => {
                format!("decoded `{rom}` {kind:?} `{name}` {dims:?}")
            }
            BootEvent::TextureUploaded { name } => format!("uploaded texture `{name}`"),
            BootEvent::ShaderCompiled { name } => format!("compiled shader `{name}`"),
            BootEvent::GuestCompiling { rom } => format!("compiling guest `{rom}`"),
            BootEvent::GuestInstantiated { rom } => format!("instantiated guest `{rom}`"),
            BootEvent::StateSpawned { rom, entities } => {
                format!("spawned `{rom}` ({entities} entities)")
            }
            BootEvent::BootComplete { elapsed } => format!("boot complete in {elapsed:?}"),
            BootEvent::BootFailed { phase, error } => format!("boot failed at {phase}: {error}"),
            BootEvent::ResourceUsage { cpu_percent, rss_bytes } => {
                format!("cpu {cpu_percent}% rss {:.1} MiB", *rss_bytes as f64 / (1024.0 * 1024.0))
            }
        }
    }
}

/// A sink for [`BootEvent`]s.  Implementations render or record the stream
/// (console log, loading screen, test capture).
pub trait BootSink: Send + Sync {
    fn on_event(&self, event: BootEvent);
}

/// A [`BootSink`] that fans each event out to a set of inner sinks (e.g. the
/// visual loader *and* the console log).
pub struct TeeBootSink {
    sinks: Vec<std::sync::Arc<dyn BootSink>>,
}

impl TeeBootSink {
    pub fn new(sinks: Vec<std::sync::Arc<dyn BootSink>>) -> Self {
        Self { sinks }
    }
}

impl BootSink for TeeBootSink {
    fn on_event(&self, event: BootEvent) {
        for sink in &self.sinks {
            sink.on_event(event.clone());
        }
    }
}

/// A [`BootSink`] that discards every event (the default for production boots
/// that don't opt into the loader/logging).
#[derive(Clone, Copy, Debug, Default)]
pub struct NullBootSink;

impl BootSink for NullBootSink {
    fn on_event(&self, _event: BootEvent) {}
}

/// A [`BootSink`] that records events in a `Vec` for tests/diagnostics.
#[derive(Debug, Default)]
pub struct VecBootSink {
    events: std::sync::Mutex<Vec<BootEvent>>,
}

impl VecBootSink {
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot the recorded events in delivery order.
    pub fn events(&self) -> Vec<BootEvent> {
        self.events.lock().expect("VecBootSink poisoned").clone()
    }
}

impl BootSink for VecBootSink {
    fn on_event(&self, event: BootEvent) {
        self.events.lock().expect("VecBootSink poisoned").push(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_sink_discards() {
        let sink = NullBootSink;
        sink.on_event(BootEvent::RomFetched { name: "demo".into(), bytes: 10 });
    }

    #[test]
    fn vec_sink_captures_in_order() {
        let sink = VecBootSink::new();
        sink.on_event(BootEvent::RomFetched { name: "common".into(), bytes: 1 });
        sink.on_event(BootEvent::RomFetched { name: "demo".into(), bytes: 2 });
        let events = sink.events();
        assert_eq!(
            events,
            vec![
                BootEvent::RomFetched { name: "common".into(), bytes: 1 },
                BootEvent::RomFetched { name: "demo".into(), bytes: 2 },
            ]
        );
    }

    #[test]
    fn describe_is_human_readable() {
        let event = BootEvent::RomFetched { name: "lunar".into(), bytes: 42 };
        assert_eq!(event.describe(), "fetched `lunar` (42 bytes)");
    }
}
