# Skill: classic-guest

## Scope

The WASM guest runtime for classic-wgl ROMs.  A ROM bundles a compiled `.wasm`
module (`manifest.code`) that the host runs each frame against a stable host
API — the "console SDK".  This skill covers the `classic-guest` crate
(`GuestRuntime`, `WasmiRuntime`, the ABI, the host-side `GuestHost` SDK), the
sandbox (fuel + memory), and how the ROM wires it in.

## 1. Why WASM (not a scripting language)

An interpreted scripting layer was tried and retired.  A ROM is meant to be a
*full game like a real game ROM*: guest code is compiled to `.wasm`
(Rust/C/Zig/AssemblyScript), giving near-native speed, hardware memory
isolation, and one artifact for both native and web.  See
`plans/opencode/2026-08-14-wasm-guest-system.md`.

## 2. Crate layout

```
crates/classic-guest/
  src/lib.rs        GuestRuntime trait, GuestLimits, GuestError, WasmiRuntime
  src/abi.rs        the ABI contract: host module name, guest export, string/buffer
                    marshalling helpers over guest linear memory
  src/sdk.rs        GuestHost: raw-pointer bridge to Engine + the SDK methods
  src/runtime.rs    WasmiRuntime: config (fuel), Store<GuestHost>, Linker imports,
                    instantiate + update
  tests/guest.rs    inline WAT fixtures (wat crate) driving wasmi
```

## 3. The ABI (host imports, module "env")

Guest exports `fn update(dt: f64) -> ()` (invoked once per frame).  Host
imports (defined in `runtime.rs::install_imports`) are the SDK surface:

| Import | Signature | Purpose |
|---|---|---|
| `log` | `(ptr, len)` | log through `Chan::Guest` |
| `spawn` / `despawn` / `has` | `(name_ptr, name_len) -> i32` | entity lifecycle |
| `names` | `(out_ptr, out_cap) -> i32` | JSON array of names |
| `get` / `get_comp` | `(name, [comp], out_ptr, out_cap) -> i32` | dump JSON |
| `set` / `set_comp` | `(name, [comp], json_ptr, json_len) -> i32` | set from JSON |
| `set_pos` | `(name_ptr, name_len, x: f64, y: f64, z: f64) -> i32` | write 3D position |
| `get_pos` | `(name_ptr, name_len, out_ptr) -> i32` | writes `[x, y, z]` as three f64 |
| `mouse` | `(out_ptr) -> i32` | screen mouse pos `[x, y]` |
| `mouse_iso` | `(out_ptr) -> i32` | iso tile coords under cursor `[x, y]` |
| `height_at` | `(x: f64, y: f64) -> f64` | terrain height (world z) at an iso tile |
| `set_anim` | `(name_ptr, name_len, anim_ptr, anim_len) -> i32` | set the entity's `Animator` to play a looping animation |
| `agent_selected` | `() -> i32` | editor agent-tool flag |
| `ui_consumed_click` | `() -> i32` | whether a UI element consumed this frame's click |
| `delta` / `elapsed` | `() -> f64` | frame time |
| `was_pressed` | `(btn: i32) -> i32` | mouse press (0=left…) |
| `mouse_down` | `(btn: i32) -> i32` | mouse held |
| `mouse_released` | `(btn: i32) -> i32` | mouse released this frame |
| `mouse_wheel` | `() -> f64` | current wheel value (decays to 0) |
| `key_down` | `(key_ptr, key_len) -> i32` | key held |
| `key_up` | `(key_ptr, key_len) -> i32` | key released this frame |
| `was_key_pressed` | `(key_ptr, key_len) -> i32` | key pressed this frame (edge-triggered) |
| `generate_terrain` | `(kind_ptr, kind_len, seed_ptr, seed_len, height_scale: f64) -> i32` | generate/install a named terrain (generic engine prefab; `kind = "lunar"` today) |
| `get_camera` | `(out_ptr) -> i32` | writes `[x, y, scale]` (three f64) |
| `set_camera` | `(x: f64, y: f64, scale: f64) -> i32` | set camera position + uniform scale |
| `pick_at` | `(x: f64, y: f64, out_ptr, out_cap) -> i32` | name of the top gameplay entity under a screen point (bytes written, `0` if none) |
| `get_light` | `(out_ptr) -> i32` | writes 9 f64 (ambient, direction, color) |
| `set_light` | `(a0..a2, d0..d2, c0..c2: f64) -> i32` | set light uniforms |
| `spawn_rect` | `(name, x, y, w, h, r, g, b, a) -> i32` | spawn a named screen-space solid-color rectangle |
| `spawn_text` | `(name, x, y, text, scale, r, g, b, a) -> i32` | spawn a named screen-space SDF text label |
| `set_text` | `(name, text) -> i32` | update a named SDF text label's string |
| `find_path` | `(sx, sy, ex, ey, out_ptr, out_cap) -> i32` | A* over the nav mesh; writes little-endian `i32` `[x, y]` waypoint pairs, returns the waypoint count (`-1` if buffer too small) |

**String convention**: all byte slices cross the boundary as `(ptr, len)` into
guest linear memory.  Functions returning bytes write into a caller-provided
`out_ptr`/`out_cap` buffer and return bytes written (`-1` if too small).
Position/mouse pairs are written as little-endian `f64`s (`get_pos` is a 3-f64
`[x, y, z]`; `mouse`/`mouse_iso` are 2-f64 `[x, y]`).

## 4. Sandbox (untrusted guests)

- **Fuel** (CPU): `Config::consume_fuel(true)` + `Store::set_fuel(per_frame)`
  before each `update`.  Exceeding it traps `TrapCode::OutOfFuel`, surfaced as
  `GuestError::FuelExhausted`.  Enabled only when `!trusted`.
- **Memory**: `StoreLimitsBuilder::memory_size(cap)` + `trap_on_grow_failure`
  installed via `Store::limiter(|host| host.resource_limiter())`.  A `memory.grow`
  past the cap traps.
- **Trusted**: `RomManifest.trusted` (`#[serde(default)]` = false).  The shipped
  demo/lunar ROMs set it true (skip fuel, intended for the fast browser path).

## 5. Host state & the unsafe bridge

`GuestHost` (`sdk.rs`) holds `*mut Engine` + the `StoreLimits`.  wasmi's
`Store<T>` host data has no `Send`/`Sync` bound, so the raw pointer is set fresh
each `update` via `GuestHost::set_engine` and deref'd only inside that call
(single-threaded, `engine` borrowed for the call).  The `unsafe` is confined to
`GuestHost::engine`/`engine_mut`.

## 6. Wiring (classic-demo)

- `init_guest(&mut Engine, &DemoStateRef, wasm, &GuestLimits)` instantiates a
  `WasmiRuntime`, stores it on `DemoState.guest`, and registers
  `on_update(|e| guest.update(e, dt))`.
- `init_engine` reads `rom.resources.code().get("main")` and builds limits from
  `rom.manifest.trusted`; runs the guest on every frame (not gated by
  `host_features`).
- `scripts/build-guest.mjs` compiles the `guest/*` `#![no_std]` cdylib crates to
  `public/code/demo.wasm` / `lunar.wasm`; `scripts/build-roms.mjs` injects
  `code: [{name:"main", src:"/code/<scene>.wasm"}]` + `trusted: true` and
  bundles the per-scene guest into each ROM.

## 7. Adding a host import (the SDK is a reviewed surface)

1. Add the method to `GuestHost` in `sdk.rs` (call the safe `Engine` helper).
2. Register it in `runtime.rs::install_imports` via `linker.func_wrap("env", name, …)`.
3. Marshal strings with `abi::read_str`/`write_str`; pairs with `abi::write_f64_pair`.
4. Add a WAT test in `tests/guest.rs`.
5. Update this skill's import table.

Treat every new import as a sandbox-surface change: it is reachable by untrusted
guest code and must not expose raw engine internals or leak borrows.

## 8. Testing

`cargo test -p classic-guest -- --test-threads=1` runs `tests/guest.rs`: no-op
guest runs, spawn + move via the SDK, fuel-exhaustion trap, memory-cap trap.
Fixtures are inline WAT (`wat::parse_str`) — no committed binaries needed for
tests.  The shipped ROM guests live as Rust sources under `guest/` and are
compiled to `public/code/*.wasm` by `scripts/build-guest.mjs` (`npm run assets`).
