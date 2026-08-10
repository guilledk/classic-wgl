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
cargo clippy -p classic-core -p classic-gfx -p classic-engine -p classic-platform -p classic-demo --all-targets -- -D warnings

# Assets (must run once after checkout / submodule update)
npm ci && npm run assets                # generates public/res/ (gitignored, embedded via include_bytes!)
```

CI (`.github/workflows/ci.yml`) runs `cargo fmt` + `cargo clippy` + `cargo test` + `wasm check` +
`headless golden test` on every push to `master` / `rust-port` and every PR.  Run
`cargo fmt -- --check`, `cargo clippy`, and `cargo test -- --test-threads=1`
before considering a task done.

## Directory map

```
Cargo.toml               workspace root (7 members)
crates/
  classic-core/           fundamental types, components, ECS registry, math, collision, pathfinder,
                          tilemap, instrument (CLASSIC_LOG), simplex noise, GJK, quadtree, sdf_builder
  classic-gfx/            GL rendering layer: Gfx struct, draw_* fns, GlBuffer, GlFrameBuffer, shaders
  classic-platform/       Platform trait: native (winit), web (web-sys), headless (EGL), InputState
  classic-engine/         Engine god-object: lib.rs (lifecycle), ui.rs (UIManager), golden.rs (traces),
                          env_config.rs, testing/
  classic-demo/           init_engine() bootstrap (stub — init_* prefabs still live in classic-engine)
apps/
  desktop/                native binary: include_bytes! assets, init_* calls, winit event loop
  web/                    wasm cdylib: wasm-bindgen main, trunk build, canvas pointer-lock
tests/
  golden/                 baseline.{trace.jsonl,png} for render-trace + pixel golden checks
public/
  manifest.json           shader/texture/animation declarations (used by Rust loader)
  state.json              persisted demo entities
  map001.{txt,nav.txt}    tilemap + nav mesh data (base64 JSON arrays)
assets/                   git submodule -> guilledk/classic-assets (source assets)
scripts/
  copy-assets.mjs         copies assets/demo/*.png + buildings/*/spritesheet.png -> public/res/
  make-font-atlas.mjs     generates SDF font atlas from DejaVuSans.ttf
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
- **The `Engine` struct** (`crates/classic-engine/src/lib.rs`) is the god object:
  `World`, `PhysicsProvider`, `Camera`, `Time`, `InputState`, `Gfx`, `UIManager`,
  editor state, and test-harness state.  `Engine::frame(input, vw, vh, delta)` runs
  once per render frame: physics → update closures → build render list → draw sorted items.
- **Component registry** (`crates/classic-core/src/registry.rs`) is bidirectional:
  `ComponentReg { name, spawn, dump, order, subsumes }`.  The `spawn` fn constructs a
  component from JSON; the `dump` fn serializes it back.  `subsumes` prevents fan-out
  duplicates (e.g. `IsoAgent ⊃ {IsoSprite, Transform}`).  Tests sharing the global
  registry must use `--test-threads=1` (it's a global `RwLock<HashMap>`).
- **Prefab functions** (`init_*` methods on `Engine`) are the idiomatic way to build
  gameplay: they spawn entities, add components, register `on_update` closures, and
  return `Engine` handles for later reference.
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
- **SDF text**: `classic-core/src/sdf_builder.rs` builds interleaved glyph buffers;
  `classic-gfx` renders them with the `sdf` shader (`dejavusans-sdf` font atlas).
  Entities with `SdfTextRender` are in the main z-sorted render list (`DrawKind::SdfText`),
  not a post-pass.  See `classic-text` skill.

## Conventions

- Rust-stock: `cargo fmt` (default style, width 100 via `rustfmt.toml`), `cargo clippy` strict.
- `PascalCase` for structs/enums, `snake_case` for fields/functions/variables.
  Trait names are `PascalCase` (no `I` prefix — hecs uses `Component`, not `IComponent`).
  `SCREAMING_SNAKE_CASE` for constants.  Prefab initializers follow `init_*()`.
- Crate-prefixed imports preferred (`classic_core::`, `classic_gfx::`, `classic_engine::`).
- `#[cfg(test)]` modules inline or in `tests/` directories mirroring the crate layout.
- All `unsafe` is in `classic-gfx` (GL calls) and `classic-platform/headless.rs` (EGL FFI).
  No other crate uses `unsafe`.

## Testing

- **Unit/integration tests**: `cargo test -- --test-threads=1` (threads=1 required
  because the global component registry is shared state).  46 tests in `classic-core`
  cover pathfinding, GJK, quadtree, camera, tile mesh building, SDF builder, dumper
  round-trips, and camera math.
- **classic-engine** and **classic-gfx** have no unit tests (no mock GL — deferred).
- **CLASSIC_TEST e2e**: `CLASSIC_TEST=1 CLASSIC_FRAMES=60 cargo run -p classic-desktop`.
  One scenario (12 assertions) testing height blend/set, tile painting, zero-delta,
  menu text centering, and text demo visibility.  `build_test_scenario(name)` is defined
  but the `name` parameter is currently ignored (one hardcoded scenario).
  See `classic-testing` skill for the complete DSL.
- **Golden trace**: `CLASSIC_GOLDEN=check|update` compares a render-trace `.jsonl`
  against `tests/golden/baseline/baseline.trace.jsonl`.  Run with:
  `CLASSIC_HEADLESS=1 CLASSIC_FRAMES=60 CLASSIC_TEST=all CLASSIC_GOLDEN=check cargo run -p classic-desktop`.
- **Pixel golden**: `CLASSIC_GOLDEN_PNG=1 CLASSIC_GOLDEN=check` compares a pixel buffer
  against `tests/golden/baseline/baseline.png` (not run in CI by default — software-
  rasteriser version-dependent).
- CI golden job needs: `submodules: recursive`, `npm ci && npm run assets` (for `public/res/`),
  and `CLASSIC_FRAMES=60` (controls headless loop exit timing).

## Assets

- Source game assets live in the `assets/` git submodule (private repo `guilledk/classic-assets`).
- `public/res/` is GENERATED and gitignored.  Regenerate with: `npm run assets`.
- `scripts/copy-assets.mjs` maps `assets/demo/*.png` → `public/res/<name>.png` and
  `assets/buildings/*/spritesheet.png` → `public/res/<name>.png`.
- `scripts/make-font-atlas.mjs` generates SDF font atlas + metrics JSON.
- Rust apps embed assets at compile time via `include_bytes!`/`include_str!` (see
  `apps/desktop/src/main.rs` and `apps/web/src/lib.rs`).
- CI and deploy must run `npm run assets` before `cargo build` / `trunk build`.

## CLASSIC_* environment variables

| Var | Purpose | Default |
|---|---|---|
| `CLASSIC_TEST` | Enable e2e test runner | off |
| `CLASSIC_FRAMES` | Max frames in headless mode | unlimited |
| `CLASSIC_FIXED_DT` | Fixed delta time (auto 1/60 under test) | real dt |
| `CLASSIC_WIDTH` / `CLASSIC_HEIGHT` | Forced viewport | window size |
| `CLASSIC_HEADLESS` | Surfaceless EGL, no window | off |
| `CLASSIC_OFFSCREEN` | Render to FBO | off |
| `CLASSIC_GOLDEN` | `check` or `update` golden trace | off |
| `CLASSIC_GOLDEN_PNG` | Enable pixel PNG capture | off |
| `CLASSIC_GOLDEN_TOL` | Pixel channel tolerance | 2 |
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
| `classic-ui` | UIManager, anchor layout, collider sync, set_enabled, spawn_button |
| `classic-text` | SdfText renderer, atlas generator, is_ui justify, scissor clipping |
| `classic-gfx` | draw_* functions, GL state contract, GlBuffer, begin_frame, z-clipping |
| `classic-physics` | PhysicsProvider, click/hover/enter/exit dispatch, selection, pathfinder |
| `classic-ecs` | hecs patterns, component model, registry, update_fns, camera math |
| `classic-platform` | Native/web/headless backends, InputState, window/keyboard/mouse |
| `classic-testing` | CLASSIC_TEST v2, golden harness, mock GL, scenario authoring workflow |
| `classic-debugging` | CLASSIC_LOG channels, JSON format, runtime toggles, debugging playbook |
