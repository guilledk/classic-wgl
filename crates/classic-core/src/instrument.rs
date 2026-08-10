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

/// Number of channels in the `Chan` enum.
pub const CHAN_COUNT: usize = 23;

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
static LEVELS: [AtomicU8; CHAN_COUNT] = const {
    // All channels default to Off.
    [
        AtomicU8::new(0),
        AtomicU8::new(0),
        AtomicU8::new(0),
        AtomicU8::new(0),
        AtomicU8::new(0),
        AtomicU8::new(0),
        AtomicU8::new(0),
        AtomicU8::new(0),
        AtomicU8::new(0),
        AtomicU8::new(0),
        AtomicU8::new(0),
        AtomicU8::new(0),
        AtomicU8::new(0),
        AtomicU8::new(0),
        AtomicU8::new(0),
        AtomicU8::new(0),
        AtomicU8::new(0),
        AtomicU8::new(0),
        AtomicU8::new(0),
        AtomicU8::new(0),
        AtomicU8::new(0),
        AtomicU8::new(0),
        AtomicU8::new(0),
    ]
};

/// Global frame counter, incremented by the engine.
static FRAME: AtomicU64 = AtomicU64::new(0);

/// Auto-initialized from `CLASSIC_LOG` env var on native or
/// `?classic_log=` query param on web.
static INITIALIZED: AtomicU8 = AtomicU8::new(0);

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

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
            let level = parse_level(level_str.trim());
            let all = name == "all";
            if all {
                ops.push(Op { all: true, negate: false, targets: vec![], level });
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

    // Apply: all=LEVEL first, then per-channel
    for op in &ops {
        if op.all && !op.negate {
            for c in 0..CHAN_COUNT {
                set_level_raw(c as u8, op.level);
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
    init(&raw);
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
pub fn channel_help() {
    eprintln!("CLASSIC_LOG channels ({} total):", CHAN_COUNT);
    eprintln!("  Frame, Input, Ui, Layout, Collision, Click, Render, Gfx, GlState,");
    eprintln!("  Text, Iso, Nav, Path, Ecs, State, Editor, Asset, Camera, Anim,");
    eprintln!("  Test, Golden, Dump");
    eprintln!();
    eprintln!("Grammar:");
    eprintln!("  CLASSIC_LOG=ui,collision=trace       # ui=info (default), collision=trace");
    eprintln!("  CLASSIC_LOG=all=info,gfx=trace,-nav  # all info, gfx trace, nav off");
    eprintln!("  CLASSIC_LOG=help                     # print this help");
    eprintln!("  CLASSIC_LOG=<empty>                  # all channels off (default)");
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
                target: concat!("classic::", stringify!($chan)),
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
                target: concat!("classic::", stringify!($chan)),
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
                    target: concat!("classic::", stringify!($chan)),
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
                    target: concat!("classic::", stringify!($chan)),
                    $lvl,
                    "[f{:06}] {}",
                    $crate::instrument::frame(),
                    format_args!($($arg)*)
                );
            }
        }
    };
}
