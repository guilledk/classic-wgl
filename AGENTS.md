# AGENTS.md

Guidance for AI coding agents (and humans) working on `classic-wgl`.

## What this is

`classic-wgl` is a small isometric game engine with a retained-mode UI/layout
layer, ported from TypeScript to Rust.  Two targets: **native** (winit+glutin,
desktop GL) and **web** (web-sys+trunk, WebGL 2).  There is no framework — the
whole app is a single `<canvas>` / winit window.  The main dependencies are
`hecs`, `glam`, `glow`, `winit`, and `glutin`.

The TypeScript original was deleted (`remove TypeScript engine and tooling`).
A parity reference lives at `docs/TS-PARITY.md`.

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
cargo test -- --test-threads=1          # all unit/integration tests
CLASSIC_HEADLESS=1 CLASSIC_FRAMES=60 CLASSIC_TEST=all CLASSIC_GOLDEN=check cargo run -p classic-desktop
                                        # headless e2e + golden trace check (needs libEGL)

# Lint
cargo fmt --all -- --check              # formatting
cargo clippy -p classic-core -p classic-gfx -p classic-engine -p classic-platform -p classic-rom -p classic-guest -p classic-demo --all-targets -- -D warnings

# Assets (must run once after checkout / submodule update)
npm ci && npm run assets                # generates public/res/ + demo.rom/lunar.rom (gitignored, embedded via include_bytes!)
```

CI (`.github/workflows/ci.yml`) runs `cargo fmt` + `cargo clippy` + `cargo test` + `wasm check` +
`headless golden test` on every push to `master` / `rust-port` and every PR.  Run
`cargo fmt -- --check`, `cargo clippy`, and `cargo test -- --test-threads=1`
before considering a task done.

## Directory map

```
Cargo.toml               workspace root (9 members)
crates/
  classic-core/           fundamental types, components, ECS registry, math, collision, pathfinder,
                          tilemap, instrument (CLASSIC_LOG), simplex noise, GJK, quadtree, sdf_builder,
                          terrain/ (fractal noise, lunar generator, material table, tileset painter)
  classic-gfx/            GL rendering layer: Gfx struct, draw_* fns, GlBuffer, GlFrameBuffer, shaders
  classic-platform/       Platform trait: native (winit), web (web-sys), headless (EGL), InputState
  classic-engine/         generic engine: lib.rs (lifecycle + hook surface), ui.rs (UIManager),
                          golden.rs (traces), env_config.rs
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
guest/
  demo-guest/             standalone #![no_std] cdylib guest for the demo ROM -> public/code/demo.wasm
  lunar-guest/            standalone #![no_std] cdylib guest for the lunar ROM -> public/code/lunar.wasm
tests/
  golden/baseline/        demo-scene baseline.{trace.jsonl,png}
  golden/lunar/           lunar-scene render-trace baseline
public/
  manifest.json           shader/texture/animation declarations (bundled into each ROM)
  state.json              persisted demo entities (bundled into demo.rom)
  state_lunar.json        lunar scene entities, 400x400 (bundled into lunar.rom; terrain generated)
  code/*.wasm             GENERATED (gitignored) per-scene guest modules — compiled from guest/
  demo.rom, lunar.rom     GENERATED (gitignored) scene ROMs — built from public/ by build-roms.mjs
assets/                   git submodule -> guilledk/classic-assets (source assets)
scripts/
  copy-assets.mjs         copies assets/demo/*.png + buildings/*/spritesheet.png -> public/res/
  make-font-atlas.mjs     generates SDF font atlas from DejaVuSans.ttf
  build-guest.mjs         compiles guest/* crates to wasm32 -> public/code/*.wasm
  build-roms.mjs          packs manifest + state + res/ + code/ into demo.rom + lunar.rom (tar.gz)
docs/
  TS-PARITY.md            formulas, LIGHT_PRESETS, dump key ordering, TS↔Rust divergence list
plans/
  opencode/               per-session plans and audit notes
```

## Architecture essentials

- **ECS via hecs.**  Components are plain `#[derive(Debug, Clone)]` structs in
  `crates/classic-core/src/components/mod.rs`.  Entities are `hecs::Entity` handles.
  There is no system scheduler; update logic lives in `Engine::on_update(FnMut(&mut Engine))`
  closures registered by `init_*` prefabs.
- **The `Engine` struct** (`crates/classic-engine/src/lib.rs`) is the generic engine core:
  `World`, `PhysicsProvider`, `Camera`, `Time`, `InputState`, `Gfx`, `UIManager`, and
  tilemap/nav plumbing.  It holds **no demo state** — editor/widget handles and light
  presets live in `classic-demo`'s `DemoState`.  `Engine::frame(input, vw, vh, delta)` runs
  once per render frame: physics → pre-update hooks → update closures → test runner →
  build render list → draw sorted items → overlay hooks.
- **Component registry** (`crates/classic-core/src/registry.rs`) is bidirectional:
  `ComponentReg { name, spawn, dump, order, subsumes }`.  The `spawn` fn constructs a
  component from JSON; the `dump` fn serializes it back.  `subsumes` prevents fan-out
  duplicates (e.g. `IsoAgent ⊃ {IsoSprite, Transform}`).  Tests sharing the global
  registry must use `--test-threads=1` (it's a global `RwLock<HashMap>`).
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
- **UI layer** (`crates/classic-engine/src/ui.rs`) is a retained-mode layout system with
  anchor-based positioning.  `UIManager` holds a root container and provides factory methods
  (`spawn_container`, `spawn_sdf_text`, `spawn_array`, `spawn_padding`, `spawn_sprite`,
  `spawn_button`).  Layout is `refresh_layout()` which walks the root tree recursively.
  Colliders are synced via `add_collider_to_elem` → `PhysicsProvider` → `sync_colliders`.
  See `classic-ui` skill.
- **Isometric/pathfinding**: `classic-core/src/tilemap.rs` builds the 3D tilemap mesh;
  `classic-core/src/pathfinder.rs` implements A* (single-threaded, no worker — the TS
  Web Worker pattern was dropped).  See `classic-iso` skill.
- **Procedural terrain**: `classic-core/src/terrain/` generates the `lunar` scene — a
  400x400 map of layered simplex noise plus an age-ordered meteorite crater field,
  with slope relaxation and stamped landing zones providing the flat, pathable ground
  an RTS needs.  Pure and GL-free, so it is unit-tested without a GL context.
  `classic-demo/src/scenes/lunar.rs` installs the result.  See `classic-terrain` skill.
- **SDF text**: `classic-core/src/sdf_builder.rs` builds interleaved glyph buffers;
  `classic-gfx` renders them with the `sdf` shader (`dejavusans-sdf` font atlas).
  Entities with `SdfTextRender` are in the main z-sorted render list (`DrawKind::SdfText`),
  not a post-pass.  See `classic-text` skill.
- **ROM guest code**: each ROM bundles a compiled `.wasm` module (`manifest.code`) run by
  `classic-guest` (wasmtime on native, wasmi on wasm) against the host SDK (entity lifecycle,
  component JSON round-trip via the registry, 3D position, mouse/mouse_iso/key input, time,
  `height_at`, `set_anim`, `find_path`, `agent_selected`/`ui_consumed_click`, log).  Untrusted guests
  (`trusted: false`, default) are sandboxed with fuel metering + a memory cap; the shipped
  demo/lunar ROMs set `trusted: true`.  The demo's `navAgent` behaviour (click-to-move +
  idle/walk animation + terrain-z) lives in `guest/demo-guest`, not Rust.  Heavy systems
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

- **Unit/integration tests**: `cargo test -- --test-threads=1` (threads=1 required
  because the global component registry is shared state).  Tests in `classic-core`
  cover pathfinding, GJK, quadtree, camera, tile mesh building, SDF builder, dumper
  round-trips, camera math, fractal noise, and the lunar terrain generator.
- **Terrain generator**: `crates/classic-core/tests/terrain_lunar.rs` is the primary
  regression net for the lunar scene.  It needs no GL and asserts the *gameplay*
  guarantees — bounded slopes, flat landing pads, buildable area, and mutual
  reachability of every spawn pair (checked with the engine's own A*) across
  several seeds — rather than pixel output.
- **classic-engine** and **classic-gfx** have no unit tests (no mock GL — deferred).
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
  `CLASSIC_SCENE=lunar CLASSIC_HEADLESS=1 CLASSIC_FRAMES=60 CLASSIC_FIXED_DT=0.016666668 CLASSIC_WIDTH=1280 CLASSIC_HEIGHT=720 CLASSIC_GOLDEN=check CLASSIC_GOLDEN_DIR=tests/golden/lunar cargo run -p classic-desktop`
- **Pixel golden**: `CLASSIC_GOLDEN_PNG=1 CLASSIC_GOLDEN=check` compares a pixel buffer
  against `tests/golden/baseline/baseline.png` (not run in CI by default — software-
  rasteriser version-dependent).
- CI golden job needs: `submodules: recursive`, `npm ci && npm run assets` (for `public/res/`),
  and `CLASSIC_FRAMES=60` (controls headless loop exit timing).

## Assets

- Source game assets live in the `assets/` git submodule (private repo `guilledk/classic-assets`).
- `public/res/`, `public/code/` and `public/*.rom` are GENERATED and gitignored.
  Regenerate with: `npm run assets`.
- `scripts/copy-assets.mjs` maps `assets/demo/*.png` → `public/res/<name>.png` and
  `assets/buildings/*/spritesheet.png` → `public/res/<name>.png`.
- `scripts/make-font-atlas.mjs` generates SDF font atlas + metrics JSON.
- `scripts/build-guest.mjs` compiles the `guest/*` `#![no_std]` cdylib crates to
  `wasm32-unknown-unknown` and copies the `.wasm` into `public/code/`.
- `scripts/build-roms.mjs` packs `manifest.json` (+ injected `format_version`/`entrypoint`/
  `host_features`/`trusted`/`code`) + `state.json` / `state_lunar.json` + the
  manifest-declared `res/` files + the per-scene `code/*.wasm` into `demo.rom` /
  `lunar.rom` (tar.gz).
- Rust apps embed the two ROMs at compile time via `include_bytes!` and boot them with
  `RomArchive::from_bytes` → `Rom::load` → `classic_demo::init_engine` (see
  `apps/desktop/src/main.rs` and `apps/web/src/lib.rs`).
- CI and deploy must run `npm run assets` before `cargo build` / `trunk build`.

## CLASSIC_* environment variables

| Var | Purpose | Default |
|---|---|---|
| `CLASSIC_TEST` | Enable e2e test runner | off |
| `CLASSIC_SCENE` | Demo scene to boot: `demo` or `lunar` | `demo` |
| `CLASSIC_FRAMES` | Max frames in headless mode | unlimited |
| `CLASSIC_FIXED_DT` | Fixed delta time (auto 1/60 under test) | real dt |
| `CLASSIC_WIDTH` / `CLASSIC_HEIGHT` | Forced viewport | window size |
| `CLASSIC_HEADLESS` | Surfaceless EGL, no window | off |
| `CLASSIC_OFFSCREEN` | Render to FBO | off |
| `CLASSIC_GOLDEN` | `check` or `update` golden trace | off |
| `CLASSIC_GOLDEN_PNG` | Enable pixel PNG capture | off |
| `CLASSIC_GOLDEN_TOL` | Pixel channel tolerance | 2 |
| `CLASSIC_GOLDEN_DIR` | Golden baseline directory (per-scene) | `tests/golden/baseline` |
| `CLASSIC_DUMP_DIR` | Native dump output dir | `./dump/` |
| `CLASSIC_LOG` | Channel-gated logging (see `classic-debugging` skill) | off |
| `CLASSIC_UI_DEBUG` | Per-frame UI entity dump (first 120 frames) | off |

## Git / PR notes

- Default branch is `master` (CI triggers on push to `master` / `rust-port` and all PRs).
- Commit messages are short, lowercase, imperative (`fix nav walkability transpose`).
- Submodule `assets/` requires a repo-scoped token (`GH_PAT` secret) in CI for checkout.

## Skills

Engine skills (in `.claude/skills/`):

| Skill | Covers |
|---|---|
| `classic-iso` | Iso coords, tilemap, depth formula, sprite occlusion, mesh gen (Rust-only) |
| `classic-terrain` | Procedural lunar generator, fractal noise, material table, tileset, scenes |
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
