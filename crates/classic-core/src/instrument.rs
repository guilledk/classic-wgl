//! # Skill: `classic-debugging`
//!
//! **Read `.claude/skills/classic-debugging/SKILL.md` before working on this module.**
//!
/// Per-subsystem logging channels with atomic level gating.
///
/// ## Grammar for `CLASSIC_LOG`
/// ```text
/// CLASSIC_LOG=ui,collision=trace          # ui at default (debug), collision at trace
/// CLASSIC_LOG=all=info,gfx=trace,-nav     # everything info, gfx trace, nav off
/// CLASSIC_LOG=help                        # print channel list, continue
/// CLASSIC_LOG=<empty>                     # all channels off
/// ```
///
/// Unknown channel names log a warning listing valid names (typo protection).
/// No `CLASSIC_LOG` set → all channels off (zero-overhead checks).
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};

/// Number of channels in the `Chan` enum (auto-derived from the last variant).
pub const CHAN_COUNT: usize = Chan::Platform as usize + 1;

#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Chan {
    Frame,
    Input,
    Ui,
    Layout,
    Collision,
    Click,
    Render,
    Gfx,
    GlState,
    Text,
    Iso,
    Nav,
    Path,
    Ecs,
    State,
    Editor,
    Asset,
    Camera,
    Anim,
    Test,
    Golden,
    Dump,
    Platform,
}

#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Off = 0,
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
    Trace = 5,
}

/// Per-channel level table. Indexed by `chan as usize`.
/// Defaults to Info for all channels — so `cl_info!` works as a drop-in
/// replacement for `log::info!` when `CLASSIC_LOG` is unset.
/// Noise reduction requires explicit CLASSIC_LOG config.
static LEVELS: [AtomicU8; CHAN_COUNT] = [const { AtomicU8::new(3) }; CHAN_COUNT];

/// Global frame counter, incremented by the engine.
static FRAME: AtomicU64 = AtomicU64::new(0);

/// Auto-initialized from `CLASSIC_LOG` env var on native or
/// `?classic_log=` query param on web.
static INITIALIZED: AtomicU8 = AtomicU8::new(0);

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Reset the level table and initialization flag. Only for use in tests.
/// Resets all channels to Off for test isolation.
pub fn reset_for_test() {
    for i in 0..CHAN_COUNT {
        LEVELS[i].store(0, Ordering::Relaxed); // Off
    }
    INITIALIZED.store(0, Ordering::Relaxed);
}

/// Parse a `CLASSIC_LOG` spec string and apply levels.
///
/// Grammar tokens are comma-separated:
/// - `chan`            → set that channel to `Info` (the default)
/// - `chan=LEVEL`      → set channel to the given level
/// - `-chan`           → disable the channel
/// - `all`             → set ALL channels to `Info`
/// - `all=LEVEL`       → set ALL channels to the given level
/// - `help`            → print channel list + grammar to stderr (via `log_to_stderr`)
pub fn init(spec: &str) {
    if spec.is_empty() {
        return;
    }
    let spec = spec.trim();
    if spec.is_empty() {
        return;
    }

    if spec == "help" {
        channel_help();
        return;
    }

    // Stage: parse into a Vec<(target_channels_or_all, Level)> so we process
    // `all=X` first, then per-channel overrides, then `-chan` disables.
    struct Op {
        all: bool,
        negate: bool,
        targets: Vec<Chan>,
        level: Level,
    }
    let mut ops: Vec<Op> = Vec::new();

    for token in spec.split(',') {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }

        let negate = token.starts_with('-');
        let token_body = if negate { &token[1..] } else { token };

        if let Some((name, level_str)) = token_body.split_once('=') {
            let name = name.trim();
            let level = if negate { Level::Off } else { parse_level(level_str.trim()) };
            let all = name == "all";
            if all {
                ops.push(Op { all: true, negate, targets: vec![], level });
            } else {
                let targets = resolve_channels(name, negate);
                ops.push(Op { all: false, negate: false, targets, level });
            }
        } else {
            let name = token_body.trim();
            if name == "all" {
                ops.push(Op { all: true, negate, targets: vec![], level: Level::Info });
            } else if name == "help" {
                channel_help();
                return;
            } else {
                let targets = resolve_channels(name, negate);
                let level = if negate { Level::Off } else { Level::Info };
                ops.push(Op { all: false, negate, targets, level });
            }
        }
    }

    // Apply: all=LEVEL first, then -all, then per-channel.
    for op in &ops {
        if op.all && !op.negate {
            for c in 0..CHAN_COUNT {
                set_level_raw(c as u8, op.level);
            }
        }
    }
    for op in &ops {
        if op.all && op.negate {
            for c in 0..CHAN_COUNT {
                set_level_raw(c as u8, Level::Off);
            }
        }
    }
    for op in &ops {
        if !op.all {
            for &c in &op.targets {
                set_level_raw(c as usize as u8, op.level);
            }
        }
    }

    INITIALIZED.store(1, Ordering::Relaxed);
    log::info!(
        "CLASSIC_LOG: {} channels configured",
        ops.iter().filter(|o| !o.all || (o.all && !o.negate)).count()
    );
}

/// Called once at startup from the native/wasm main to read env.
pub fn init_from_env() {
    if INITIALIZED.load(Ordering::Relaxed) != 0 {
        return;
    }
    let raw = std::env::var("CLASSIC_LOG").unwrap_or_default();
    INITIALIZED.store(1, Ordering::Relaxed);
    if !raw.is_empty() {
        // CLASSIC_LOG overrides default; set log level to Trace so
        // channel-gated output passes through the log crate filter.
        log::set_max_level(log::LevelFilter::Trace);
    }
    init(&raw);
}

#[inline]
pub fn chan_name(chan: Chan) -> &'static str {
    match chan {
        Chan::Frame => "frame",
        Chan::Input => "input",
        Chan::Ui => "ui",
        Chan::Layout => "layout",
        Chan::Collision => "collision",
        Chan::Click => "click",
        Chan::Render => "render",
        Chan::Gfx => "gfx",
        Chan::GlState => "glstate",
        Chan::Text => "text",
        Chan::Iso => "iso",
        Chan::Nav => "nav",
        Chan::Path => "path",
        Chan::Ecs => "ecs",
        Chan::State => "state",
        Chan::Editor => "editor",
        Chan::Asset => "asset",
        Chan::Camera => "camera",
        Chan::Anim => "anim",
        Chan::Test => "test",
        Chan::Golden => "golden",
        Chan::Dump => "dump",
        Chan::Platform => "platform",
    }
}

#[inline]
pub fn enabled(chan: Chan, level: Level) -> bool {
    LEVELS[chan as usize].load(Ordering::Relaxed) >= level as u8
}

pub fn set_frame(n: u64) {
    FRAME.store(n, Ordering::Relaxed);
}

#[inline]
pub fn frame() -> u64 {
    FRAME.load(Ordering::Relaxed)
}

/// Print channel list + grammar to stderr for discovery.
#[rustfmt::skip]
pub fn channel_help() {
    let channels: [Chan; CHAN_COUNT] = [
        Chan::Frame, Chan::Input, Chan::Ui, Chan::Layout, Chan::Collision, Chan::Click,
        Chan::Render, Chan::Gfx, Chan::GlState, Chan::Text, Chan::Iso, Chan::Nav,
        Chan::Path, Chan::Ecs, Chan::State, Chan::Editor, Chan::Asset, Chan::Camera,
        Chan::Anim, Chan::Test, Chan::Golden, Chan::Dump, Chan::Platform,
    ];

    eprint!("CLASSIC_LOG channels ({total} total):", total = CHAN_COUNT);
    for ch in &channels {
        eprint!(" {}", chan_name(*ch));
    }
    eprintln!();
    eprintln!("  Aliases: physics→collision+click  draw→render+gfx+glstate");
    eprintln!("           editor-all→editor+camera  anim/animation→Anim");
    eprintln!();
    eprintln!("Grammar:");
    eprintln!("  CLASSIC_LOG=ui,collision=trace       # ui=info (default), collision=trace");
    eprintln!("  CLASSIC_LOG=all=info,gfx=trace,-nav  # all info, gfx trace, nav off");
    eprintln!("  CLASSIC_LOG=help                     # print this help");
    eprintln!("  CLASSIC_LOG=<empty>                  # all channels at info (default)");
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn set_level_raw(idx: u8, level: Level) {
    if (idx as usize) < CHAN_COUNT {
        LEVELS[idx as usize].store(level as u8, Ordering::Relaxed);
    }
}

fn parse_level(s: &str) -> Level {
    match s.to_lowercase().as_str() {
        "error" | "err" => Level::Error,
        "warn" | "warning" => Level::Warn,
        "info" => Level::Info,
        "debug" => Level::Debug,
        "trace" => Level::Trace,
        "off" | "none" => Level::Off,
        other => {
            log::warn!("CLASSIC_LOG: unknown level \"{other}\", using Info");
            Level::Info
        }
    }
}

fn resolve_channels(name: &str, _negate: bool) -> Vec<Chan> {
    match name {
        "frame" => vec![Chan::Frame],
        "input" => vec![Chan::Input],
        "ui" => vec![Chan::Ui],
        "layout" => vec![Chan::Layout],
        "collision" => vec![Chan::Collision],
        "click" => vec![Chan::Click],
        "render" => vec![Chan::Render],
        "gfx" => vec![Chan::Gfx],
        "glstate" | "gl" => vec![Chan::GlState],
        "text" => vec![Chan::Text],
        "iso" => vec![Chan::Iso],
        "nav" => vec![Chan::Nav],
        "path" => vec![Chan::Path],
        "ecs" => vec![Chan::Ecs],
        "state" => vec![Chan::State],
        "editor" => vec![Chan::Editor],
        "asset" => vec![Chan::Asset],
        "camera" => vec![Chan::Camera],
        "anim" | "animation" | "animator" => vec![Chan::Anim],
        "test" => vec![Chan::Test],
        "golden" => vec![Chan::Golden],
        "dump" => vec![Chan::Dump],
        // Alias groups for convenience
        "physics" => vec![Chan::Collision, Chan::Click],
        "render-all" | "draw" => vec![Chan::Render, Chan::Gfx, Chan::GlState],
        "editor-all" => vec![Chan::Editor, Chan::Camera],
        other => {
            log::warn!(
                "CLASSIC_LOG: unknown channel \"{other}\", use CLASSIC_LOG=help to list channels"
            );
            vec![]
        }
    }
}

// ---------------------------------------------------------------------------
// Convenience macros
// ---------------------------------------------------------------------------

/// Map a `log::Level` to our internal `Level` for the channel enabled check.
#[inline]
pub fn inst_level(l: log::Level) -> Level {
    match l {
        log::Level::Error => Level::Error,
        log::Level::Warn => Level::Warn,
        log::Level::Info => Level::Info,
        log::Level::Debug => Level::Debug,
        log::Level::Trace => Level::Trace,
    }
}

/// Conditionally log at a given channel + level.
#[macro_export]
macro_rules! cl_log {
    ($chan:expr, $lvl:expr, $($arg:tt)*) => {
        if $crate::instrument::enabled($chan, $crate::instrument::inst_level($lvl)) {
            log::log!(
                target: $crate::instrument::chan_name($chan),
                $lvl,
                "[f{:06}] {}",
                $crate::instrument::frame(),
                format_args!($($arg)*)
            );
        }
    };
}

/// One-shot: log once (sticky AtomicBool latch).
#[macro_export]
macro_rules! cl_once {
    ($chan:expr, $lvl:expr, $($arg:tt)*) => {{
        static LATCH: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if $crate::instrument::enabled($chan, $crate::instrument::inst_level($lvl))
            && !LATCH.swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            log::log!(
                target: $crate::instrument::chan_name($chan),
                $lvl,
                "[f{:06}] {}",
                $crate::instrument::frame(),
                format_args!($($arg)*)
            );
        }
    }};
}

#[macro_export]
macro_rules! cl_error {
    ($chan:expr, $($arg:tt)*) => {
        $crate::cl_log!($chan, log::Level::Error, $($arg)*)
    };
}
#[macro_export]
macro_rules! cl_warn {
    ($chan:expr, $($arg:tt)*) => {
        $crate::cl_log!($chan, log::Level::Warn, $($arg)*)
    };
}
#[macro_export]
macro_rules! cl_info {
    ($chan:expr, $($arg:tt)*) => {
        $crate::cl_log!($chan, log::Level::Info, $($arg)*)
    };
}
#[macro_export]
macro_rules! cl_debug {
    ($chan:expr, $($arg:tt)*) => {
        $crate::cl_log!($chan, log::Level::Debug, $($arg)*)
    };
}
#[macro_export]
macro_rules! cl_trace {
    ($chan:expr, $($arg:tt)*) => {
        $crate::cl_log!($chan, log::Level::Trace, $($arg)*)
    };
}

/// Log every Nth frame.
#[macro_export]
macro_rules! cl_every {
    ($chan:expr, $n:expr, $lvl:expr, $($arg:tt)*) => {
        if $crate::instrument::enabled($chan, $crate::instrument::inst_level($lvl)) {
            if $crate::instrument::frame().wrapping_rem($n) == 0 {
                log::log!(
                    target: $crate::instrument::chan_name($chan),
                    $lvl,
                    "[f{:06}] {}",
                    $crate::instrument::frame(),
                    format_args!($($arg)*)
                );
            }
        }
    };
}

/// Log only for the first N frames.
#[macro_export]
macro_rules! cl_first {
    ($chan:expr, $n:expr, $lvl:expr, $($arg:tt)*) => {
        if $crate::instrument::enabled($chan, $crate::instrument::inst_level($lvl)) {
            if $crate::instrument::frame() < $n {
                log::log!(
                    target: $crate::instrument::chan_name($chan),
                    $lvl,
                    "[f{:06}] {}",
                    $crate::instrument::frame(),
                    format_args!($($arg)*)
                );
            }
        }
    };
}

/// RAII scope guard that logs entry and exit with elapsed time.
pub struct ClScope {
    chan: Chan,
    label: &'static str,
    start: std::time::Instant,
}

impl Drop for ClScope {
    fn drop(&mut self) {
        let elapsed = self.start.elapsed();
        log::log!(
            target: chan_name(self.chan),
            log::Level::Info,
            "[f{:06}] ⤷ {} ({:.0}μs)",
            crate::instrument::frame(),
            self.label,
            elapsed.as_micros()
        );
    }
}

/// Log entry into a named scope, returning a guard that logs exit with elapsed μs.
///
/// ```ignore
/// let _scope = cl_scope!(Chan::Render, "draw_items");
/// ```
#[macro_export]
macro_rules! cl_scope {
    ($chan:expr, $label:expr) => {
        if $crate::instrument::enabled($chan, $crate::instrument::Level::Info) {
            log::log!(
                target: $crate::instrument::chan_name($chan),
                log::Level::Info,
                "[f{:06}] → {}",
                $crate::instrument::frame(),
                $label
            );
            Some($crate::instrument::ClScope {
                chan: $chan,
                label: $label,
                start: std::time::Instant::now(),
            })
        } else {
            None
        }
    };
}
