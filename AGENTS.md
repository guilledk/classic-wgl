# AGENTS.md

Guidance for AI coding agents (and humans) working on `classic-wgl`.

## What this is

`classic-wgl` is a small isometric game engine with a retained-mode UI/layout
layer, written in Rust.  Two targets: **native** (winit+glutin,
desktop GL) and **web** (web-sys+trunk, WebGL 2).  There is no framework — the
whole app is a single `<canvas>` / winit window.  The main dependencies are
`hecs`, `glam`, `glow`, `winit`, and `glutin`.

## Commands

```bash
# Build
cargo build -p classic-desktop         # native binary
cargo build --target wasm32-unknown-unknown -p classic-web   # wasm
nix develop                             # enter dev shell (sets LD_LIBRARY_PATH + wasm linker)

# Run
cargo run -p classic-desktop            # native, interactive
trunk serve apps/web/index.html         # web dev server
trunk build apps/web/index.html --release  # web release

# Test
cargo test                            # all unit/integration tests
CLASSIC_HEADLESS=1 CLASSIC_FRAMES=60 CLASSIC_TEST=all CLASSIC_GOLDEN=check cargo run -p classic-desktop
                                        # headless e2e + golden trace check (needs libEGL)

# Lint
cargo fmt --all -- --check              # formatting
cargo clippy -p classic-core -p classic-gfx -p classic-engine -p classic-platform -p classic-rom -p classic-guest -p classic-demo --all-targets -- -D warnings

# ROMs (must run once after checkout, and when the published ROMs bump)
cargo xtask fetch-roms               # downloads the staged demo/lunar ROMs (and worker wasm) into roms/out/ (gitignored)
cargo xtask lock-roms                # pin published checksums to tests/golden/roms.lock.json
cargo xtask check-roms               # fail fast when the published bucket drifts from the lock (CI golden gate)

# Web pathfinder module (must run before any --target wasm32 build/check;
# trunk serve/build do it automatically via the `pre_build` hook in `Trunk.toml`)
cargo xtask build-pathfinder         # compiles crates/classic-pathfinder-wasm to pathfinder.wasm and stages it next to web.rs

# Versioning / releases (see VERSIONING.md)
cargo xtask check-version            # fail when Cargo.toml/CHANGELOG.md drift
cargo xtask release patch            # bump version + freeze changelog (prints commit/tag cmds)
```

CI (`.github/workflows/ci.yml`) runs `cargo fmt` + `cargo clippy` + `cargo test` + `wasm check` +
`headless golden test` on every push to `master` and every PR.  Run
`cargo fmt -- --check`, `cargo clippy`, and `cargo test`
before considering a task done.  The CI golden job calls `cargo xtask fetch-roms`
instead of the old `cargo xtask all` — the ROMs come from the published
Cloudflare R2 bucket (`classic-roms.com`), not from the deleted `guest/`
submodule.

## Directory map

```
Cargo.toml               workspace root (11 members)
crates/
  classic-core/           fundamental types, components, ECS registry, math, collision, tilemap,
                          instrument (CLASSIC_LOG), GJK, quadtree, sdf_builder, plus the
                          shared ABI marshalling (abi.rs) and field-buffer registry (fields.rs);
                          re-exports `classic-pathfinder` as `pathfinder`
  classic-pathfinder/     #![no_std] A* + footprint/slope/jump vehicle search (single source of
                          truth for native + web; compiled to `pathfinder.wasm` for the web Worker)
  classic-pathfinder-wasm/ thin `#[no_mangle]` wasm ABI over `classic-pathfinder` (cdylib)
  classic-worker/         background workers: generic native ThreadPool, PathfinderWorker (native
                          thread + web Worker), and the Tier-3 GuestWorker (a second .wasm instance
                          running pure guest entries against a reduced import surface)
  classic-terrain/        #![no_std] open terrain/noise toolkit (simplex, fractal combinators, bulk
                          noise fields, and the grid-kernel catalog in kernels.rs) — the reusable
                          primitives ROM guests build map algorithms on
  classic-gfx/            GL rendering layer: Gfx struct, draw_* fns, GlBuffer, GlFrameBuffer, shaders
  classic-platform/       Platform trait: native (winit), web (web-sys), headless (EGL), InputState
  classic-engine/         generic engine: lib.rs (lifecycle + hook surface), ui.rs (UIManager),
                          golden.rs (traces), env_config.rs, vehicle.rs (IsoVehicle sim + spawn API)
  classic-rom/            ROM layer: RomArchive (zip/tar.gz/tar.zst), Rom (load/pack), RomManifest,
                          ResourceSet, AssetLoader trait (re-exported by classic-platform)
  classic-guest/          WASM guest runtime: GuestRuntime trait, WasmiRuntime + WasmtimeRuntime
                           (native) + create_runtime, the guest ABI (abi.rs) and host-side SDK
                           (sdk.rs) bridging guest imports to the engine
  classic-demo/           application/prefab layer: init_engine(gl, &Rom) bootstrap, DemoState +
                          EditorState (state.rs), prefabs.rs, lighting.rs, editor.rs, hud.rs,
                          testing.rs, scenes/ (demo + lunar assemblers)
apps/
  desktop/                native binary: include_bytes! demo.rom/lunar.rom, Rom::load, winit loop
  web/                    wasm cdylib: wasm-bindgen main, trunk build, canvas pointer-lock
tests/
  golden/baseline/        demo-scene baseline.{trace.jsonl,png}
  golden/lunar/           lunar-scene render-trace baseline
roms/
  out/                    GENERATED (gitignored) staged ROMs — populated by `cargo xtask fetch-roms`
xtask/                    Rust build tool: `cargo xtask fetch-roms` (stages published demo/lunar ROMs)
docs/
  picom-nixos-i3.md       dev-env notes (picom + nixos + i3 compositor setup)
plans/
  opencode/               per-session plans and audit notes
```

## Architecture essentials

- **ECS via hecs.**  Components are plain `#[derive(Debug, Clone)]` structs in
  `crates/classic-core/src/components/mod.rs`.  Entities are `hecs::Entity` handles.
  There is no system scheduler; update logic lives in `Engine::on_update(FnMut(&mut Engine))`
  closures registered by `init_*` prefabs.
- **The `Engine` struct** (`crates/classic-engine/src/lib.rs`) is the generic engine core:
  `World`, `PhysicsProvider`, `Camera`, `Time`, `InputState`, `Gfx`, `UIManager`, a
  `vehicles: HashMap<String, VehicleDef>` registry, and tilemap/nav plumbing.  It holds
  **no demo state** — editor/widget handles and light
  presets live in `classic-demo`'s `DemoState`.  `Engine::frame(input, vw, vh, delta)` runs
  once per render frame: physics → pre-update hooks → update closures → test runner →
  build render list → draw sorted items → overlay hooks.
- **Component registry** (`crates/classic-core/src/registry.rs`) is bidirectional:
  `ComponentReg { name, spawn, dump, order, subsumes }`.  The `spawn` fn constructs a
  component from JSON; the `dump` fn serializes it back.  `subsumes` prevents fan-out
  duplicates (e.g. `IsoAgent ⊃ {IsoSprite, Transform}`).  The registry is an immutable
  `OnceLock<Vec<ComponentReg>>` populated once by `register_all_components()`; tests run in
  parallel with no `--test-threads=1` requirement.
- **Prefab functions** (`init_*` free functions in `classic-demo`) are the idiomatic way to
  build gameplay: they take `&mut Engine` + `Rc<RefCell<DemoState>>`, spawn entities, add
  components, and register `on_update` / hook closures.  The demo layer is installed via
  `Engine`'s hook surface (`on_update`, `on_pre_update`, `on_selection_end`, `add_overlay`,
  `set_test_runner`).
- **GL rendering** (`classic-gfx/src/lib.rs`) provides 7 `draw_*` functions (`draw_tilemap`,
  `draw_iso_sprite`, `draw_sprite`, `draw_rect`, `draw_sdf`, `draw_line_loop`, `draw_line_strip`).
  Each binds a named shader, sets projection/camera/model uniforms, and draws.
  **Important**: `begin_frame` does NOT enable `DEPTH_TEST` globally — tilemap/iso_sprite
  toggle it within their scopes.  The UI/SDF phase runs with depth test off; layering is
   purely draw-order (z-sort).  Enabling it globally depth-rejects UI under ortho projection.
   See `classic-gfx` skill.
- **Dynamic lights (UBO)**: beyond the Lambertian sun (`light_ambient`/`light_dir`/
  `light_color`), dynamic point/spot lights are **first-class ECS entities** — a
  `Light` component (registered + dumpable, declarable in `state.json`) gathered
  from the world each frame and uploaded to a `std140` `LightBlock` UBO consumed
  by `sheet.frag`/`iso_tilemap.frag` (shared `evaluateLight`).  The guest
  `light_spawn`/`light_set`/`light_release` API returns a stable handle backed by
  a `LightHandles` entity table (`classic-engine/src/light.rs`) with optional TTL
  decay.  A `Light` may set `parent` (an entity name): its `position` is then a
  **light-space offset from the parent's ground point** (`iso_to_world` of the
  parent's tile position), so lights follow moving objects.  `Engine::iso_to_world(x, y,
  elevation)` is the single iso-tile → light-space conversion.  An animation may
  carry typed `light.*` channels (see `AnimationData::channels`); an `Animator`
  targeting `"<entity>.Light"` samples them and drives the light in lockstep with
  the sprite.  The **sun casts a directional shadow map**
  (`classic-engine/src/shadow.rs` + a depth-only `DepthFramebuffer` in
  `classic-gfx`), sampled in both lit shaders to shadow the sun diffuse term
  (terrain self-shadowing, terrain→sprite, sprite→terrain); ambient and point
  lights stay unoccluded.  Disable via `CLASSIC_SHADOWS=0`.
  See `classic-gfx` §16–17, `classic-ecs` ("Dynamic lights"), `classic-iso` §13.
- **⚠️ Two spaces: light space vs screen space.**  This is the single most
  dangerous thing in the renderer; conflating them produced a shadow map that
  compiled, ran, passed its tests and cast nothing.
  - **Light space** — `model * iso_matrix * vertex`, **+Z is up**.  `light_dir`,
    `vNormal`, `Light::position`, `iso_to_world`, the shadow map and the
    `vLightPos` varying all live here.  All lighting maths happens here.
  - **Screen space** — the above, then `y -= vertex.z`.  This isometric shear is
    what makes height read as height on screen.  It carries height in **both**
    y and z, so its up axis is `(0,-1,1)/√2`; projecting it along a +Z-up
    `light_dir` presents the sun at ~2.7° instead of 30°.  It is used for
    rasterisation only and is deliberately **not** exposed as a varying.
  - Sprite billboards are screen-aligned quads, so shadow code unprojects them
    about their ground anchor (screen up → world +Z) via the `sprite_anchor`
    uniform.  `shadow_sprite.vert`, `direct_tex.vert` and
    `shadow.rs::sprite_billboard_corners` must agree exactly.
  - Bring-up aids: `CLASSIC_SHADOW_DEBUG=1` (render raw sun visibility, white
    lit / black occluded) and `CLASSIC_NO_UI=1` (drop the editor/HUD layer).
    Set `SHADOW_STRENGTH = 0.0` while changing shadow geometry.
- **UI layer** (`crates/classic-engine/src/ui.rs`) is a retained-mode layout system with
  anchor-based positioning.  `UIManager` holds a root container and provides factory methods
  (`spawn_container`, `spawn_sdf_text`, `spawn_array`, `spawn_padding`, `spawn_sprite`,
  `spawn_button`).  Layout is `refresh_layout()` which walks the root tree recursively.
  Colliders are synced via `add_collider_to_elem` → `PhysicsProvider` → `sync_colliders`.
  See `classic-ui` skill.
- **Isometric/pathfinding**: `classic-core/src/tilemap.rs` builds the 3D tilemap mesh;
  the A* + vehicle search lives in the standalone `classic-pathfinder` crate
  (re-exported as `classic_core::pathfinder`), over an immutable `NavSnapshot`
  (`Arc`-shared).  The engine offloads searches to a host `PathfinderWorker`
  (`classic-worker`, native thread / web `Worker` running the compiled
  `pathfinder.wasm`); guests drive it through the
  async `request_path`/`poll_path` SDK imports (with a synchronous fallback for
  the deterministic harness).  See `classic-iso` and `classic-physics` skills.
- **Wheeled vehicles**: `classic-engine/src/vehicle.rs` implements the `IsoVehicle`
  system — `spawn_vehicle` assembles a body + 4 wheel `IsoSprite`s from a
  `VehicleDef` sidecar (per-direction ground-origin anchors emitted by the Blender
  exporter), and `update_vehicles` drives the body as a single chassis plane
  (`altitude` + `pitch` + `roll`) fit to the four wheel contacts, with per-wheel
  suspension springs clamped to a travel envelope (compression/droop, derived from
  the def geometry) and a point-mass moon-gravity jump.  Guests drive vehicles
  through the `vehicle_spawn`/`vehicle_teleport`/`vehicle_goto`/`vehicle_stop` host
  API.  See `classic-ecs` (`IsoVehicle` component) and `classic-iso` (pitch/roll
  frame layout).
- **Background guest work (Tier 3)**: the engine hosts a second `.wasm` instance
  (`classic-worker::GuestWorker`) running pure guest entries (`spawn_task`/
  `poll_task`) against a reduced import surface (noise/fields/kernels/path plus a
  result buffer; engine-mutating imports trap).  The lunar ROM's worker wasm
  (staged under `roms/out/code/lunar_worker.wasm`) generates the lunar map
  off-thread and returns the grids; the foreground `lunar` ROM guest bulk-uploads
  and commits them.
- **Procedural terrain**: `classic-terrain` is the *open* noise toolkit plus a
  grid-kernel catalog (`kernels.rs`); the
  `lunar` map algorithm (a 400x400 surface of layered simplex noise plus an
  age-ordered meteorite crater field, slope relaxation and stamped landing
  zones) lives in the lunar ROM's compiled guest + worker wasm modules, which
  build on that toolkit and bulk-upload the grids + tileset to the host via the
  guest SDK.  Pure, GL-free and natively unit-tested.  The host is a generic
  terrain engine (storage + rebuild + pathfinding); the map algorithm lives in
  the ROM.  See `classic-terrain` skill.
- **SDF text**: `classic-core/src/sdf_builder.rs` builds interleaved glyph buffers;
  `classic-gfx` renders them with the `sdf` shader (`dejavusans-sdf` font atlas).
  Entities with `SdfTextRender` are in the main z-sorted render list (`DrawKind::SdfText`),
  not a post-pass.  See `classic-text` skill.
- **ROM guest code**: each ROM bundles a compiled `.wasm` module (`manifest.code`) run by
  `classic-guest` (wasmtime on native, wasmi on wasm) against the host SDK (entity lifecycle,
  component JSON round-trip via the registry, 3D position, mouse/mouse_iso/key input, time,
  `height_at`, `set_anim`, `request_path`/`poll_path`, `agent_selected`/`ui_consumed_click`, log).
  Untrusted guests
  (`trusted: false`, default) are sandboxed with fuel metering + a memory cap; the shipped
  demo/lunar ROMs set `trusted: true`.  The demo's `navAgent` behaviour (click-to-move +
  idle/walk animation + terrain-z) lives in the demo ROM's compiled guest wasm, not Rust.  Heavy systems
  (UIManager layout, animator, physics, pathfinding, terrain) stay host-side; guests
  register + update into them rather than reimplementing them.  See `classic-guest` skill.

## Conventions

- Rust-stock: `cargo fmt` (default style, width 100 via `rustfmt.toml`), `cargo clippy` strict.
- `PascalCase` for structs/enums, `snake_case` for fields/functions/variables.
  Trait names are `PascalCase` (no `I` prefix — hecs uses `Component`, not `IComponent`).
  `SCREAMING_SNAKE_CASE` for constants.  Prefab initializers follow `init_*()`.
- Crate-prefixed imports preferred (`classic_core::`, `classic_gfx::`, `classic_engine::`).
- `#[cfg(test)]` modules inline or in `tests/` directories mirroring the crate layout.
- All `unsafe` is in `classic-gfx` (GL calls), `classic-platform/headless.rs` (EGL FFI), and
  `classic-guest/src/sdk.rs` (the raw-pointer `GuestHost` bridge to `Engine`, contained to a
  single `update` call).

## Testing

- **Unit/integration tests**: `cargo test`.  Tests in `classic-core`
  cover pathfinding, GJK, quadtree, camera, tile mesh building, SDF builder, dumper
  round-trips, camera math, fractal noise, and the lunar terrain generator.
- **Terrain generator**: `crates/classic-core/tests/terrain_lunar.rs` is the primary
  regression net for the lunar scene.  It needs no GL and asserts the *gameplay*
  guarantees — bounded slopes, flat landing pads, buildable area, and mutual
  reachability of every spawn pair (checked with the engine's own A*) across
  several seeds — rather than pixel output.
- **classic-engine** unit tests live in the `vehicle.rs` `#[cfg(test)]` module
  (spawn/teleport/goto/stop, pitch/roll quantization + spring, wheel offset
  derivation) using `Engine::new_for_test()` — no GL needed.  **classic-gfx**
  still has no unit tests (no mock GL — deferred).
- **CLASSIC_TEST e2e**: `CLASSIC_TEST=1 CLASSIC_FRAMES=60 cargo run -p classic-desktop`.
  One scenario (12 assertions) testing height blend/set, tile painting, zero-delta,
  menu text centering, and text demo visibility.  `build_test_scenario(name)` is defined
  but the `name` parameter is currently ignored (one hardcoded scenario).
  See `classic-testing` skill for the complete DSL.
- **Golden trace**: `CLASSIC_GOLDEN=check|update` compares a render-trace `.jsonl`
  against `tests/golden/baseline/baseline.trace.jsonl`.  Run with:
  `CLASSIC_HEADLESS=1 CLASSIC_FRAMES=60 CLASSIC_TEST=all CLASSIC_GOLDEN=check cargo run -p classic-desktop`.
- **Lunar golden trace**: a second baseline in `tests/golden/lunar/`.  Requires an
  explicit `CLASSIC_FIXED_DT` — without `CLASSIC_TEST` the idle animator advances on
  real delta and lands on a different frame each run:
  `CLASSIC_ROM=rom:lunar CLASSIC_HEADLESS=1 CLASSIC_FRAMES=60 CLASSIC_FIXED_DT=0.016666668 CLASSIC_WIDTH=1280 CLASSIC_HEIGHT=720 CLASSIC_GOLDEN=check CLASSIC_GOLDEN_DIR=tests/golden/lunar cargo run -p classic-desktop`
  Under `LIBGL_ALWAYS_SOFTWARE=1`, llvmpipe's multithreaded rasterizer can race on
  the sprite ghost-pass depth rendering (the landing rocket is the first iso
  sprite in the lunar scene); pin it to one thread with `LP_NUM_THREADS=0`
  (CI does this — see `.github/workflows/ci.yml`).
- **Pixel golden**: `CLASSIC_GOLDEN_PNG=1 CLASSIC_GOLDEN=check` compares a pixel buffer
  against `tests/golden/baseline/baseline.png` (not run in CI by default — software-
  rasteriser version-dependent).
- CI golden job needs: `cargo xtask fetch-roms` (stages the published ROMs into
  `roms/out/`), and `CLASSIC_FRAMES=60` (controls headless loop exit timing).

## Assets / ROMs

- The staged scene ROMs (`demo.rom`, `lunar.rom`, `lrvtest.rom` + worker wasm) are
  built by the `classic-roms` repo and published to a Cloudflare R2 public bucket
  served at `classic-roms.com` (CORS-enabled, so the browser web app can fetch
  them too).  They are **fetched, not built**: `cargo xtask fetch-roms` downloads
  them (verifying against `roms.json`) into `roms/out/` (gitignored).  The
  `guest/` submodule and its ROM-generation pipelines were deleted.
- `roms/out/` is GENERATED and gitignored.  Regenerate with: `cargo xtask fetch-roms`.
- The desktop app reads the ROMs from `roms/out/` at runtime (overridable via
  `CLASSIC_ROM_DIR`); the web app fetches them from `classic-roms.com`.  Both boot
  them with `RomArchive::from_bytes` → `Rom::load` → `classic_demo::init_engine`
  (see `apps/desktop/src/main.rs` and `apps/web/src/lib.rs`).
- CI runs `cargo xtask fetch-roms` before `cargo build`; deploy stages nothing
  (the web app fetches R2 directly).
- **ROM-lock lockstep.** Publishing new ROMs to the bucket must move together
  with re-pinning `tests/golden/roms.lock.json` (`cargo xtask lock-roms`) and
  re-baselining the goldens — never publish without re-pinning in the same
  change, or `cargo xtask check-roms` (the CI golden job) fails fast the moment
  the bucket diverges from the committed lockfile.

## CLASSIC_* environment variables

| Var | Purpose | Default |
|---|---|---|
| `CLASSIC_TEST` | Enable e2e test runner | off |
| `CLASSIC_ROM` | ROM to boot: `rom:<name>` (embedded `demo`/`lunar`), a file path, or an `http(s)://` URL | `rom:demo` |
| `CLASSIC_FRAMES` | Max frames in headless mode | unlimited |
| `CLASSIC_FIXED_DT` | Fixed delta time (auto 1/60 under test) | real dt |
| `CLASSIC_WIDTH` / `CLASSIC_HEIGHT` | Forced viewport | window size |
| `CLASSIC_HEADLESS` | Surfaceless EGL, no window | off |
| `CLASSIC_OFFSCREEN` | Render to FBO | off |
| `CLASSIC_GOLDEN` | `check` or `update` golden trace | off |
| `CLASSIC_GOLDEN_PNG` | Enable pixel PNG capture | off |
| `CLASSIC_GOLDEN_TOL` | Pixel channel tolerance | 2 |
| `CLASSIC_GOLDEN_DIR` | Golden baseline directory (per-scene) | `tests/golden/baseline` |
| `CLASSIC_SHADOWS` | `0` disables the directional shadow map | on |
| `CLASSIC_SHADOW_DEBUG` | Render raw sun visibility (white lit / black occluded) | off |
| `CLASSIC_NO_UI` | Skip the demo editor/HUD/overlay layer (clean lit scene) | off |
| `CLASSIC_DUMP_DIR` | Native dump output dir | `./dump/` |
| `CLASSIC_LOG` | Channel-gated logging (see `classic-debugging` skill) | off |
| `CLASSIC_UI_DEBUG` | Per-frame UI entity dump (first 120 frames) | off |

## Git / PR notes

- Default branch is `master` (CI triggers on push to `master` and all PRs).
- Commit messages are short, lowercase, imperative (`fix nav walkability transpose`).
- No git submodules remain; the `assets/` submodule and its `GH_PAT` checkout token were removed.

## Versioning / releases

- Workspace-wide [semver](https://semver.org) (0.x: MINOR = breaking, PATCH =
  fix/feature), tracked in `[workspace.package.version]` + `CHANGELOG.md`.
  See `VERSIONING.md` for the full policy; the `classic-release` skill is the
  runbook.
- Releases are cut **once per merge window** via `cargo xtask release` — never
  hand-edit versions on a feature branch; accumulate under `[Unreleased]`.
- `cargo xtask check-version` (run in CI) fails on any `Cargo.toml` ↔
  `CHANGELOG.md` drift.

## Skills

Engine skills (in `.agents/skills/`):

| Skill | Covers |
|---|---|
| `classic-iso` | Iso coords, tilemap, depth formula, sprite occlusion, mesh gen (Rust-only) |
| `classic-terrain` | Open noise toolkit (simplex, fractal combinators, bulk noise fields) + the guest-side lunar generator recipe |
| `classic-procmaps` | Authoring/scaling procedural maps: terrain module, new-generator recipe, scale-free params, size envelope |
| `classic-ui` | UIManager, anchor layout, collider sync, set_enabled, spawn_button |
| `classic-text` | SdfText renderer, atlas generator, is_ui justify, scissor clipping |
| `classic-gfx` | draw_* functions, GL state contract, GlBuffer, begin_frame, z-clipping |
| `classic-physics` | PhysicsProvider, click/hover/enter/exit dispatch, selection, pathfinder |
| `classic-ecs` | hecs patterns, component model, registry, update_fns, camera math |
| `classic-platform` | Native/web/headless backends, InputState, window/keyboard/mouse |
| `classic-testing` | CLASSIC_TEST v2, golden harness, mock GL, scenario authoring workflow |
| `classic-debugging` | CLASSIC_LOG channels, JSON format, runtime toggles, debugging playbook |
| `classic-guest` | WASM guest runtime, ABI (host imports/guest exports), GuestRuntime trait, sandbox (fuel + memory) |
