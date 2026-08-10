---
name: classic-logging
description: >
    CLASSIC_LOG channel-gated instrumentation system for classic-wgl's Rust port.
    Covers the Chan enum (22 channels — `Chan::Dump as usize + 1`), atomic level table, env-var grammar,
    macros (`cl_error!/warn!/info!/debug!/trace!/every!/first!/once!`),
    frame counter, native vs web backends, and per-channel level conventions.
    Use when adding log statements to the engine, debugging with selective
    channel output, or diagnosing missing log output on web.
    Trigger phrases: "CLASSIC_LOG", "cl_info", "cl_debug", "cl_trace",
    "cl_every", "cl_first", "cl_once", "instrument", "channel logging",
    "log channel", "RUST_LOG", "frame counter".
compatibility: log 0.4, env_logger 0.11, web_sys::console
metadata:
    author: classic-wgl
    version: '1.0'
allowed-tools: Read, Grep, Glob, Bash(git *), Edit
---

# Skill: classic-logging

## Scope

Covers `crates/classic-core/src/instrument.rs` (~400 LOC), the macros exported
from `classic_core`, the native `env_logger` integration, and the web
`WebLogger` + `?classic_log=` query-param fallback.

---

## 1. Architecture

### Chan enum (22 channels)

```rust
pub enum Chan {
    Frame, Input, Ui, Layout, Collision, Click, Render, Gfx, GlState,
    Text, Iso, Nav, Path, Ecs, State, Editor, Asset, Camera, Anim,
    Test, Golden, Dump,
}
```

### Level per-channel

A static `[AtomicU8; CHAN_COUNT]` table stores each channel's current level.
`CHAN_COUNT` is derived from the enum: `pub const CHAN_COUNT: usize = Chan::Dump as usize + 1;`
(= 22). The `enabled(chan, level)` check performs one `Relaxed` atomic load —
disabled channels cost virtually nothing.

### Frame counter

A global `AtomicU64` incremented by `set_frame(n)` each frame (called from
`Engine::frame()` at `lib.rs:~655`). Macros auto-insert `[f000001]` prefix.

### Initialization

`init_from_env()` reads `CLASSIC_LOG` env var, parses, applies. Called at
`Engine::new()` time. Web entry point (`apps/web/src/lib.rs`) additionally
checks `?classic_log=` query param and calls `init(…)` directly.

---

## 2. CLASSIC_LOG Grammar

```
CLASSIC_LOG=ui,collision=trace          # ui=info (default), collision=trace
CLASSIC_LOG=all=info,gfx=trace,-nav     # everything info, gfx trace, nav off
CLASSIC_LOG=help                        # prints channel list + grammar, continues
CLASSIC_LOG=<empty>                     # all channels off (default)
```

Tokens are comma-separated. Each token is:
- `chan` → set that channel to `Info`
- `chan=LEVEL` → set to explicit level
- `-chan` → disable the channel (set to `Off`)
- `all`, `all=LEVEL` → set all channels
- `help` → print channel names to stderr

Aliases exist for convenience: `physics` → `{Collision, Click}`,
`render-all` / `draw` → `{Render, Gfx, GlState}`,
`editor-all` → `{Editor, Camera}`.

---

## 3. Macros

All macros exported from `classic_core` via `#[macro_export]`. Use as
`classic_core::cl_info!` or import.

### Basic level macros

```rust
cl_error!(Chan::Gfx, "shader compile failed: {e}");
cl_warn!(Chan::Ecs, "unknown component {}", name);
cl_info!(Chan::Asset, "loaded texture '{}' {}x{}", name, w, h);
cl_debug!(Chan::Ui, "layout dirty, refreshing");
cl_trace!(Chan::Render, "draw sprite '{}' at z={}", name, z);
```

### Throttle macros

```rust
cl_every!(Chan::Frame, 60, log::Level::Info, "fps={}", fps);   // every 60th frame
cl_first!(Chan::Ui, 120, log::Level::Debug, "size={:?}", s);   // first 120 frames
cl_once!(Chan::Gfx, log::Level::Warn, "unusual GL state");      // once ever
```

Note: `cl_every!` and `cl_first!` take `log::Level` (the standard crate level),
not `instrument::Level`. The macro translates internally via `inst_level()`.

### How it works

Each macro expands to:
```rust
if instrument::enabled(chan, inst_level(level)) {
    log::log!(target: "classic::ui", level, "[f{:06}] …", frame(), …);
}
```

`log::log!` uses the standard `target` filter feature — you can also filter by
target prefix in `RUST_LOG` (e.g. `RUST_LOG=classic::Chan::Ui=trace,info`).
Note the target string emitted by the macros is `"classic::Chan::Ui"` (not
`"classic::ui"`) because the `Chan` enum variant names are used directly.
The `enabled()` check is an additional, cheaper gate that runs before the format
args are evaluated.

---

## 4. Level Conventions

| Level | When to use |
|---|---|
| `Error` | Unrecoverable failures, missing resources |
| `Warn` | Recoverable anomalies (unknown channel name, GL error drain) |
| `Info` | One-shot lifecycle events (texture load, state load, dump save) |
| `Debug` | State transitions (editor_target change, resize, set_enabled) |
| `Trace` | Per-frame detail (draw calls, collision queries, glyph rebuilds) |

Per-frame `Trace` logs should use `cl_every!` or `cl_first!` to avoid flooding.

---

## 5. Web Backend

On wasm, a `WebLogger` implementing `log::Log` is installed in
`apps/web/src/lib.rs` before `Engine::new()`. It routes messages to
`web_sys::console::log_1` / `error_1` / `warn_1` / `info_1`.

Log level is set to `Trace` so all CLASSIC_LOG channels work. The query
param `?classic_log=…` is parsed manually (not via `env_logger`) since
wasm has no environment.

---

## 6. Adding a New Channel

1. Add variant to `Chan` enum in `instrument.rs`.
2. Add a `const` entry in the `LEVELS` array initializer.
3. Add a match arm in `resolve_channels()` (plus any aliases).
4. Use the channel name in macros: `classic_core::cl_info!(Chan::MyChannel, …)`.

No other registration needed — the `enabled()` check uses `chan as usize`
to index the level table directly.

---

## 7. Common Pitfalls

| Symptom | Cause | Fix |
|---|---|---|
| No log output despite CLASSIC_LOG set | `env_logger` not installed or `RUST_LOG` filtering | Ensure `env_logger::init()` runs, and `RUST_LOG=info` (or the channel targets) |
| Web log output missing | `WebLogger` installed too late (after `Engine::new()` calls `log::info!`) | Install `WebLogger` before `Engine::new()` |
| `cl_every!` type error | Passing `instrument::Level` instead of `log::Level` | Use `log::Level::Info` (not `instrument::Level::Info`) |
| Channel name typo silently ignored | Unknown channels produce a `log::warn!` | Check stderr for "unknown channel" warnings; use `CLASSIC_LOG=help` |
