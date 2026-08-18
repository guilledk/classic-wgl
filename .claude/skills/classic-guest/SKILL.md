# Skill: classic-guest

## Scope

The WASM guest runtime for classic-wgl ROMs.  A ROM bundles a compiled `.wasm`
module (`manifest.code`) that the host runs each frame against a stable host
API — the "console SDK".  This skill covers the `classic-guest` crate
(`GuestRuntime`, `create_runtime`, the four runtime backends — `WasmiRuntime`,
`WasmtimeRuntime`, `WebWasmRuntime`, `WorkerWasmRuntime` — the ABI, the
host-side `GuestHost` SDK), the sandbox (fuel + memory + Worker watchdog), and
how the ROM wires it in.

## 1. Why WASM (not a scripting language)

An interpreted scripting layer was tried and retired.  A ROM is meant to be a
*full game like a real game ROM*: guest code is compiled to `.wasm`
(Rust/C/Zig/AssemblyScript), giving near-native speed, hardware memory
isolation, and one artifact for both native and web.  See
`plans/opencode/2026-08-14-wasm-guest-system.md`.

## 2. Crate layout

```
crates/classic-guest/
  src/lib.rs              GuestRuntime trait, GuestLimits, GuestError, create_runtime
  src/abi.rs              the ABI contract: host module name, guest exports, and
                          backend-agnostic string/buffer marshalling over a
                          linear-memory slice
  src/sdk.rs              GuestHost: raw-pointer bridge to Engine + the SDK methods
                          (shared by every runtime backend)
  src/imports.rs          the host-import surface (single source of truth): an
                          `install_host_imports!` macro expanded by the wasmi and
                          wasmtime backends (marshals args and forwards to `GuestHost`)
  src/runtime.rs          WasmiRuntime (native + wasm): config (fuel) + the shared
                          import macro + memory helpers
  src/runtime_wasmtime.rs WasmtimeRuntime (native only): config (fuel) + the shared
                          import macro + memory helpers
  src/runtime_web.rs      WebWasmRuntime (wasm only, trusted): browser-native
                          `WebAssembly`, host imports as `Closure`s (+ a dispatcher
                          for the 13 imports with >8 args)
  src/runtime_worker.rs   WorkerWasmRuntime (wasm only, untrusted): `Worker` +
                          SAB/Atomics synchronous host-import bridge + terminate watchdog
  src/worker.js           the Worker script (SAB host-import stubs + update loop)
  tests/guest.rs          inline WAT fixtures (wat crate) driven against every backend
```

`create_runtime(wasm, limits)` picks the backend: **wasmtime on native**; on
wasm, **browser-native `WebAssembly` for `trusted` guests** (no fuel API) and a
**`Worker`-isolated browser-native runtime for untrusted guests** (terminate
watchdog), falling back to **wasmi** when `SharedArrayBuffer` is unavailable.
All implement the same `GuestRuntime` trait and the same `env` import surface.
The wasmi and wasmtime linker layers are generated from the single
`imports.rs::install_host_imports!` macro; only the `GuestHost` SDK bodies
(`sdk.rs`) and the web/worker closure/SAB layers are backend-specific.

## 3. The ABI (host imports, module "env")

Guest exports (the host→guest side of the ABI):

| Export | Signature | When |
|---|---|---|
| `update` | `(dt: f64) -> ()` | every frame (required) |
| `init` | `() -> ()` | once, synchronously at install, before the first frame (optional) |
| `start` | `() -> ()` | once, after the first `update` completes (optional) |

Host imports (defined once in `imports.rs::install_host_imports`, expanded by
both the wasmi and wasmtime backends) are the SDK surface:

| Import | Signature | Purpose |
|---|---|---|
| `log` | `(ptr, len)` | log through `Chan::Guest` |
| `spawn` / `despawn` / `has` | `(name_ptr, name_len) -> i32` | entity lifecycle |
| `names` | `(out_ptr, out_cap) -> i32` | JSON array of names |
| `set_pos` | `(name_ptr, name_len, x: f64, y: f64, z: f64) -> i32` | write 3D position |
| `get_pos` | `(name_ptr, name_len, out_ptr) -> i32` | writes `[x, y, z]` as three f64 |
| `mouse` | `(out_ptr) -> i32` | screen mouse pos `[x, y]` |
| `mouse_iso` | `(out_ptr) -> i32` | iso tile coords under cursor `[x, y]` |
| `iso_to_screen` | `(x: f64, y: f64, out_ptr) -> i32` | project an iso tile coord to screen space `[sx, sy]` (two f64; `0` if no Tilemap) |
| `height_at` | `(x: f64, y: f64) -> f64` | terrain height (world z) at an iso tile |
| `set_anim` | `(name_ptr, name_len, anim_ptr, anim_len) -> i32` | set the entity's `Animator` to play a looping animation |
| `start_anim` | `(name_ptr, name_len, anim_ptr, anim_len, repeat: i32) -> i32` | reset the `Animator` from frame zero and play (one-shot if `repeat == 0`) |
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
| `set_tile` | `(x: i32, y: i32, id: i32) -> i32` | write one tile index at tile coordinate `(x, y)` (bounds-checked) |
| `set_height` | `(x: i32, y: i32, h: f64) -> i32` | write one height vertex at coordinate `(x, y)` (bounds-checked) |
| `rebuild_terrain` | `() -> i32` | rebuild the tilemap mesh + re-derive nav walkability after in-place edits |
| `get_camera` | `(out_ptr) -> i32` | writes `[x, y, scale]` (three f64) |
| `set_camera` | `(x: f64, y: f64, scale: f64) -> i32` | set camera position + uniform scale |
| `set_grid` | `(show: i32) -> i32` | show/hide the tilemap editor grid overlay |
| `pick_at` | `(x: f64, y: f64, out_ptr, out_cap) -> i32` | name of the top gameplay entity under a screen point (bytes written, `0` if none) |
| `get_light` | `(out_ptr) -> i32` | writes 9 f64 (ambient, direction, color) |
| `set_light` | `(a0..a2, d0..d2, c0..c2: f64) -> i32` | set light uniforms |
| `spawn_rect` | `(name, x, y, w, h, r, g, b, a) -> i32` | spawn a named screen-space solid-color rectangle |
| `spawn_text` | `(name, x, y, text, scale, r, g, b, a) -> i32` | spawn a named screen-space SDF text label |
| `set_text` | `(name, text) -> i32` | update a named SDF text label's string |
| `ui_container` | `(name, w, h, r, g, b, a) -> i32` | spawn a UIManager-managed container (responsive layout) |
| `ui_text` | `(name, text, scale, max_width, r, g, b, a, justify) -> i32` | spawn a UIManager-managed SDF text |
| `ui_button` | `(name, text, w, h, r, g, b, a) -> i32` | spawn a button (hover + click → event queue, auto-subscribed) |
| `ui_array` | `(name, vertical, align, spacing, r, g, b, a) -> i32` | spawn a stacking array container |
| `ui_padding` | `(name, top, right, bottom, left, r, g, b, a) -> i32` | spawn a padding wrapper |
| `ui_sprite` | `(name, texture, w, h, frame, tsx, tsy) -> i32` | spawn a texture-sprite UI element |
| `ui_add_child` | `(parent, child, self_anchor, child_anchor) -> i32` | attach a child for anchor-based layout |
| `ui_add_to_root` | `(name, self_anchor, child_anchor) -> i32` | attach to the root container (viewport-anchored) |
| `ui_set_size` / `ui_set_anchor` / `ui_set_color` / `ui_set_fixed` | `(name, …) -> i32` | update a managed element's layout/content |
| `subscribe` | `(name) -> i32` | subscribe a named entity to interaction events |
| `poll_event` | `(out_ptr, out_cap) -> i32` | pop the next event as `kind:u32` + `name_len:u32` + `name` (kind 0=click, 1=enter, 2=exit; `0` if none) |
| `spawn_collider` | `(name, x, y, w, h) -> i32` | attach an axis-aligned rectangle collider (screen space) to a named entity |
| `get_anim` | `(name, out_ptr, out_cap) -> i32` | write the entity's current `frame: f64` + `name_len: u32` + animation name (`0` if no `Animator`) |
| `has_resource` | `(kind: i32, name_ptr, name_len) -> i32` | resource existence (0=texture, 1=font, 2=animation) |
| `texture_size` | `(name_ptr, name_len, out_ptr) -> i32` | write a loaded texture's pixel size as two `f64` (`0` if not loaded) |
| `request_path` | `(sx, sy, ex, ey) -> i32` | submit an A* request over the nav mesh; returns a request id to poll (async — the search runs on a host worker) |
| `poll_path` | `(id, out_ptr, out_cap) -> i32` | poll a path request; `0` pending, `-1` no-path, `-2` buffer too small, `>0` waypoint count (writes little-endian `i32` `[x, y]` pairs) |
| `spawn_task` | `(entry_ptr, entry_len, arg_ptr, arg_len) -> i32` | submit a background-guest task (Tier 3): run the worker guest's named export with `arg` as input; returns a task id |
| `poll_task` | `(id, out_ptr, out_cap) -> i32` | poll a background task; `0` pending, `-1` error, `-2` buffer too small, `>0` bytes written |
| `vehicle_teleport` | `(name_ptr, name_len, x: f64, y: f64) -> i32` | reposition a wheeled vehicle (body + 4 wheels) and reset its physics |
| `vehicle_goto` | `(name_ptr, name_len, tx: i32, ty: i32) -> i32` | set a vehicle's destination; the host runs A* and stores the waypoints |
| `vehicle_stop` | `(name_ptr, name_len) -> i32` | stop a vehicle, clearing its movement path |
| `vehicle_spawn` | `(def_ptr, def_len, name_ptr, name_len, x: f64, y: f64) -> i32` | spawn a vehicle of a declared definition at `(x, y)` |

### 3a. Bulk terrain imports (guest-driven map generation)

The host owns the *noise primitives* and the *storage/rebuild* engine; ROM
guests own the map algorithm.  Guests compose a map by requesting noise fields
into their own linear memory and bulk-uploading the resulting grids.

Bulk noise (host fills a guest buffer with `f32` values, returns bytes written):

| Import | Signature | Purpose |
|---|---|---|
| `fbm_field` | `(w, h, seed, octaves, freq, lacunarity, gain, out_ptr, out_cap) -> i32` | summed-octave fBm, `[-1, 1]` |
| `ridged_field` | `(w, h, seed, octaves, freq, lacunarity, gain, warp_amp, out_ptr, out_cap) -> i32` | ridged multifractal, `[0, 1]`, optional domain warp |
| `billow_field` | `(w, h, seed, octaves, freq, lacunarity, gain, out_ptr, out_cap) -> i32` | billow (abs) fBm, `[0, 1]` |
| `tiling_field` | `(w, h, seed, period, octaves, radius, out_ptr, out_cap) -> i32` | seamlessly-tiling fBm |
| `noise_field` | `(w, h, seed, freq_x, freq_y, out_ptr, out_cap) -> i32` | raw single-octave 2D simplex |
| `noise2d` | `(seed_ptr, seed_len, x, y) -> f64` | raw 2D simplex at one point |

Bulk upload (guest writes grids into its memory, host reads them):

| Import | Signature | Purpose |
|---|---|---|
| `set_tiles` | `(ptr, len) -> i32` | bulk `u32` LE tile grid → `Tilemap.data` |
| `set_heights` | `(ptr, len) -> i32` | bulk `f32` LE vertex heights → `Tilemap.height_data` |
| `set_nav` | `(ptr, len) -> i32` | bulk `u32` LE walkability → `NavMesh.data` |
| `set_tileset` | `(ptr, len, w, h) -> i32` | raw RGBA → upload the tilemap's tileset texture |
| `commit_terrain` | `(height_scale: f64) -> i32` | install (first call) or rebuild the tilemap mesh + nav overlay (no slope re-derivation) |

The bulk fields are `f32` little-endian; the bulk grids are `u32`/`i32`/`f32`
little-endian, matching the path-waypoint binary convention.  Note: the
untrusted Worker backend's SAB bridge caps bulk payloads (see §4).

### 3b. Field-buffer registry + grid kernels (host-owned scratch)

Instead of round-tripping intermediate grids through guest memory mid-generation
(which hits the SAB bridge's payload cap on web), a guest can allocate a named
host-resident field, drive grid kernels over it by name, and only download the
final grids:

| Import | Signature | Purpose |
|---|---|---|
| `alloc_field` | `(name_ptr, name_len, w, h, dtype) -> i32` | allocate a zero-filled `w`×`h` field (`dtype` 0=f32, 1=u32) |
| `free_field` | `(name_ptr, name_len) -> i32` | remove a named field |
| `write_field` | `(name_ptr, name_len, data_ptr, data_len) -> i32` | overwrite an `f32` field from a guest buffer |
| `write_field_u32` | `(name_ptr, name_len, data_ptr, data_len) -> i32` | overwrite a `u32` field |
| `read_field` | `(name_ptr, name_len, out_ptr, out_cap) -> i32` | download an `f32` field |
| `map_field` | `(op, dst_ptr, dst_len, src_ptr, src_len) -> i32` | `dst = dst op src` (`op` 0=add, 1=sub, 2=mul, 3=min, 4=max) |
| `map_scalar` | `(op, dst_ptr, dst_len, scalar) -> i32` | `dst = dst op scalar` |
| `blur_box_field` | `(name_ptr, name_len, radius) -> i32` | N×N box blur |
| `relax_slopes_field` | `(name_ptr, name_len, max_slope, iterations, tolerance, pinned_ptr, pinned_len) -> f64` | slope relaxation (optional pinned `u32` mask), returns worst slope |
| `gradient_magnitude_field` | `(heights_ptr, heights_len, dst_ptr, dst_len) -> i32` | per-tile gradient under `dst` |
| `threshold_le_field` | `(src_ptr, src_len, dst_ptr, dst_len, t) -> i32` | `1` where `<= t` (u32) |
| `prune_components_field` | `(name_ptr, name_len) -> i32` | keep only the largest connected component |
| `reduce_field` | `(name_ptr, name_len, op) -> f64` | reduce (`op` 0=min, 1=max, 2=mean, 3=variance) |

The kernels are pure, deterministic, `#![no_std]` (`classic-terrain::kernels`,
re-exported as `classic_core::terrain::kernels`); the registry lives in
`classic-core::fields` (`FieldRegistry`).

**String convention**: all byte slices cross the boundary as `(ptr, len)` into
guest linear memory.  Functions returning bytes write into a caller-provided
`out_ptr`/`out_cap` buffer and return bytes written (`-1` if too small).
Position/mouse pairs are written as little-endian `f64`s (`get_pos` is a 3-f64
`[x, y, z]`; `mouse`/`mouse_iso` are 2-f64 `[x, y]`).

## 4. Sandbox (untrusted guests)

- **Fuel** (CPU): `Config::consume_fuel(true)` + `Store::set_fuel(per_frame)`
  before each `update`.  Exceeding it traps `TrapCode::OutOfFuel`, surfaced as
  `GuestError::FuelExhausted`.  Enabled only when `!trusted`.  (Native +
  wasmi/wasmtime backends.)
- **Memory**: `StoreLimitsBuilder::memory_size(cap)` + `trap_on_grow_failure`
  installed via `Store::limiter(|host| host.resource_limiter())`.  A `memory.grow`
  past the cap traps.
- **Web Worker watchdog**: browser Wasm has no fuel API, so `WorkerWasmRuntime`
  enforces a wall-clock budget (`GuestLimits.max_frame_millis`) per call and
  `worker.terminate()`s on overrun, surfacing `GuestError::FuelExhausted`.
- **Trusted**: `RomManifest.trusted` (`#[serde(default)]` = false).  The shipped
  demo/lunar ROMs set it true (skip fuel, intended for the fast browser path).

## 5. Host state & the unsafe bridge

`GuestHost` (`sdk.rs`) holds only `*mut Engine`.  Each native/wasm-interpreter
backend wraps it in its own store data (`WasmiHost` / `WasmtimeHost`) that also
owns that backend's resource limiter (`StoreLimits`); the web backends share it
via `Rc<RefCell<GuestHost>>` (captured by `'static` host-import closures, or
held by `WorkerWasmRuntime` and set fresh per service-loop).  Neither store's
host data has a `Send`/`Sync` bound, so the raw pointer is set fresh each
`init`/`update`/`start` via `GuestHost::set_engine` and deref'd only inside that
call (single-threaded, `engine` borrowed for the call).  The `unsafe` is
confined to `GuestHost::engine`/`engine_mut`.

## 6. Wiring (classic-demo)

- `init_guest(&mut Engine, &DemoStateRef, wasm, &GuestLimits)` calls
  `classic_guest::create_runtime` (wasmtime on native; browser-Wasm for trusted /
  Worker for untrusted, with a wasmi fallback, on wasm), runs the optional
  `init` hook synchronously (before the first frame), stores the boxed runtime
  on `DemoState.guest`, and registers an `on_update(|e| guest.update(e, dt))`
  closure that also runs the optional `start` hook once after the first update.
- `init_engine` reads `rom.resources.code().get("main")` and builds limits from
  `rom.manifest.trusted`; runs the guest on every frame (not gated by
  `host_features`).
- `cargo xtask guests` compiles the `guest/*` `#![no_std]` cdylib crates to
  `roms/out/code/demo.wasm` / `lunar.wasm`; `cargo xtask roms` injects
  `code: [{name:"main", src:"/code/<scene>.wasm"}]` + `trusted: true` and
  bundles the per-scene guest into each ROM (zip).
- Shipped guests link `dlmalloc` (`global` feature) as `#[global_allocator]`,
  so `alloc::String`/`Vec`/`format!` are available from guest code (memory
  still bounded by the wasmi memory cap).

## 7. Adding a host import (the SDK is a reviewed surface)

1. Add the method to `GuestHost` in `sdk.rs` (call the safe `Engine` helper).
2. Register it in every backend's import surface: the `imports.rs`
   `install_host_imports!` macro (shared by the wasmi and wasmtime backends);
   `runtime_web.rs` (browser-Wasm: a `Closure`, or a dispatcher arm for the
   >8-arg imports); `runtime_worker.rs`'s dispatch match plus the matching
   stub in `worker.js`.
3. Marshal strings with the local `read_str`/`write_str` helpers; pairs with
   `write_f64_pair` (they wrap the backend-agnostic `abi::read_str_from` /
   `abi::write_*_to` slice helpers).
4. Add a WAT test in `tests/guest.rs` (it runs against every backend).
5. Update this skill's import table.

Treat every new import as a sandbox-surface change: it is reachable by untrusted
guest code and must not expose raw engine internals or leak borrows.

## 8. Testing

`cargo test -p classic-guest` runs `tests/guest.rs`: every
guest-driven test runs against **both** `WasmiRuntime` and (on native)
`WasmtimeRuntime` — no-op run, spawn + move, fuel-exhaustion trap, memory-cap
trap, and the full SDK surface.  Fixtures are inline WAT (`wat::parse_str`) — no
committed binaries needed for tests.  The shipped ROM guests live as Rust
sources under `guest/` and are compiled to `roms/out/code/*.wasm` by
`cargo xtask guests` (`cargo xtask all`).  The web backends
(`WebWasmRuntime`, `WorkerWasmRuntime`) are wasm-only and have no unit test —
they're compile-verified via
`cargo check --target wasm32-unknown-unknown -p classic-web` and `trunk build`.

## 9. Background guest worker (Tier 3)

A ROM can bundle a second `.wasm` module (`code: [{name:"worker", …}]`) that the
host runs off-thread for heavy, engine-free computation.  It is **not** the
foreground `GuestRuntime` — see `classic-worker/src/guest_worker`:

- `WorkerHost` (Send, engine-free) owns an `Arc<NavSnapshot>`, a scratch
  `FieldRegistry`, and the current task's argument/result buffers.
- The reduced import surface (`install_worker_imports!`) exposes only the pure
  subset — `log`, the noise fields, the field/kernel registry, a synchronous
  `find_path`, and `task_arg`/`task_return`.  Engine-mutating imports
  (`spawn`, `set_*`, `commit_terrain`, camera/light/input/UI) are registered as
  **trap stubs**; the rest of the SDK is absent (link-fail).
- The foreground guest drives it via `spawn_task(entry, arg)` +
  `poll_task(id, out, cap)`; the worker guest reads `task_arg`, computes, and
  writes `task_return`.  Native runs it on a `std::thread` (wasmtime); web uses
  a synchronous wasmi fallback (a real async web `Worker` is deferred).
- `GuestWorker::new(wasm, nav, synchronous)` — `synchronous` runs entries inline
  (the deterministic harness forces it under `CLASSIC_TEST`/`CLASSIC_GOLDEN`).
