# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
See [`VERSIONING.md`](VERSIONING.md) for the release policy and process.

## [Unreleased]

### Added

- `cargo xtask check-patterns`: a CI-gated guard for the codified architecture
  patterns (no raw thread/`Worker` spawns outside `classic-worker`, no
  hand-numbered `OP_*` tables), with a per-line `xtask-allow` opt-out (#NN).
- `AGENTS.md` "Patterns": the eight rules this codebase keeps itself to, with
  `# Architecture` rustdoc notes on `classic-worker`, `classic-guest` and
  `classic-engine::boot`, and a refreshed `docs/architecture.d2`/`.svg` (#NN).
- Declarative host-import ABI table (`classic_core::abi_manifest`): one entry
  per import records its typed params, return kind and backend set, with a
  runtime `HOST_IMPORTS` descriptor.  The wasmi/wasmtime linker layer and the
  Tier-3 worker surface are now generated from it, and new tests check that
  every table import links on each native backend (#NN).
- Headless-browser test harness for the wasm-only guest backends
  (`wasm-bindgen-test` in Chromium, `crates/classic-guest/tests/web.rs`; the
  flake pins the matching `wasm-bindgen-cli` and provides Chromium +
  chromedriver) and a `web tests` CI job (#NN).
- `GuestRuntime::is_ready()` readiness handshake: the untrusted web `Worker`
  runtime reports ready once its `Worker` has booted, and the demo defers a
  not-yet-ready guest's `init` to its first ready frame.  `trunk serve` now
  sends COOP/COEP (cross-origin isolation), so `SharedArrayBuffer` and that
  runtime are available in development; the GitHub Pages deploy stays
  non-isolated (#NN).

### Changed

- Dump every registered component through one generic
  `classic_core::registry::dump_as::<T>` instead of 15 hand-written dumpers;
  `ComponentReg::dump_value` adds the `"type"` key, so a `Dumper` now returns
  the component body only (#NN).
- Collapse the app boot drivers onto `classic_engine::boot`'s `run_sync`
  (headless) and `InterleavedBoot` (per frame while the loader renders; the
  windowed desktop's `InterleavedBoot::threaded` owns the boot thread, caps
  hand-off and event forwarding), removing the desktop's `BootMsg` and
  `ChannelBootSink` and the web app's hand-rolled boot loop (#NN).
- Drive every boot through one `classic_engine::boot::BootPipeline` stage
  machine (`Uploading` → `UploadingBasis` → `Finishing` → `Done`): headless
  and golden runs poll it to completion, the windowed desktop prepares it on
  the boot thread (`BootPipeline::prepare`: parallel decode, basis transcode,
  guest compile) and polls it per frame, and the web app polls it per animation
  frame, awaiting the `.basis` Worker stage.  The demo's post-load setup is the
  `classic_demo::DemoFinish` hook.  Removed: `Engine::begin_boot_gfx`,
  `boot_step_predecoded`, `upload_basis_predecoded_at`, `upload_pending_basis`,
  `boot::decode_assets` and `BootPlan`'s cursor/decoded setters; `BootPlan` no
  longer borrows the ROMs or sink.  Now crate-private: `Engine::begin_boot`,
  `Engine::boot_step`, `boot::decode_plan`,
  `classic_demo::{finish_init_engine, compile_guest_modules}` (#NN).
- Fan boot-time texture decode and native `.basis` transcode out on a pooled
  `JobQueue` (`JobQueue::pooled` + ordered `run_all`, loader threads now named
  `classic-decode-N`/`classic-basis-N`) instead of a separate thread pool; the
  web basis transcode `Worker` moves into `classic_worker::transcoder_worker`
  (#NN).
- Web workers exchange bytes as transferred buffers instead of structured
  clones: nav snapshots, guest/transcoder modules, task arguments and transcode
  inputs going in, and path, task and transcode results coming out.  The
  pathfinder result no longer clones the whole Worker wasm heap per search
  (#NN).
- Run background work on a generic `classic_worker::JobQueue<J>` (threaded or
  synchronous): `PathfinderWorker` and `GuestWorker` share its thread, FIFO
  update, flush barrier and result map, and `Engine::set_synchronous_workers`
  is now the single determinism switch — the engine always submits to the
  pathfinder instead of branching into separate inline paths (synchronous
  vehicle searches still use a snapshot of the live world).
  `GuestLimits::synchronous_workers` and the `synchronous` argument of
  `Engine::install_guest_worker{,_compiled}` are removed (#NN).
- Spawn every background thread and web `Worker` through shared helpers
  (`classic_worker::spawn_thread`, `classic_worker::spawn_web_worker`) instead
  of five copies of the thread setup and four copies of the Blob/URL/`onmessage`
  worker setup; native background threads are now named (#NN).
- Generate the trusted browser `WebAssembly` guest backend's host imports from
  the ABI table, replacing ~1,500 hand-written closures and its hand-numbered
  `OP_*` dispatcher; imports with more than 8 wasm params are now one
  self-contained shim each instead of a shared global dispatcher (#NN).
- Generate the untrusted Worker guest backend from the ABI table: the
  main-thread dispatch is a table-built registry and `worker.js` builds its
  import stubs from a descriptor, so both hand-numbered `OP_*` tables are gone.
  The guest module is posted to the Worker instead of being copied into its
  1 MiB `SharedArrayBuffer` (#NN).
- Split the oversized source files into focused modules with no behaviour
  change: `classic-engine`'s `lib.rs` (`lifecycle`, `hooks`, `boot_api`,
  `render`) and `vehicle.rs` (`vehicle/`), `classic-gfx`'s `lib.rs`, and
  `classic-guest`'s `runtime_web.rs` (`runtime_web/`) (#NN).

### Removed

- Unused `classic-gfx` dependency from `classic-platform` (#NN).
- `classic_worker::ThreadPool`, superseded by `JobQueue::pooled` (#NN).

### Fixed

- Web (trusted) guests: a failed background task no longer panics with a
  `RefCell` double borrow in `poll_task`, and a second guest runtime no longer
  re-points every earlier runtime's wide host imports at itself (#NN).
- Web (untrusted Worker) guests: expose the 16 host imports the backend was
  missing (the field/kernel registry, `spawn_task`/`poll_task`,
  `vehicle_goto_poll`), stream host-import payloads of any size instead of
  failing above 6 KiB in / 64 KiB out, match the native return values (e.g.
  `set_camera`), and report guest traps and link errors as `GuestError::Trap`
  instead of timing out (#NN).

## [0.2.0] - 2026-09-09

### Added

- Web Basis Universal transcoder: build our own Emscripten `basis_transcoder`
  wasm from BinomialLLC's basis_universal (Apache 2.0) and transcode `.basis`
  (ETC1S) sheets to S3TC/ETC2/BC7 on WebGL 2 (RGBA8 fallback), matching the
  native codec byte-for-byte.
- Read vehicle ground-anchor definitions from the packed `data[]` artifact
  (namespace-qualified `ref`), replacing the sidecar anchor lookup.
- Close the web depth gap: a `BC4_R` depth sheet transcodes to
  `COMPRESSED_R11_EAC` (`ETC2_EAC_R11`) on a WebGL 2 device without RGTC instead
  of the RGBA8 fallback.
- Worker-ize the web transcode: `.basis` sheets transcode in a dedicated web
  `Worker` (async) with a synchronous main-thread fallback, so large sheets no
  longer block the frame loop.
- Golden layout map: emit a deterministic, GPU-free `baseline.layout.txt` (one
  line per draw item with its screen-space rect) alongside the golden trace, so
  text-only models can introspect a rendered frame.
- Boot progress events: a `BootEvent`/`BootSink` stream (no-op, test, and log
  sinks) threaded through ROM resolve, archive open, resource load, and guest
  init, plus a `CLASSIC_LOADER`/`CLASSIC_BOOT_LOG` config and a `boot` log
  channel.
- Boot plan: split ROM hydration into a precomputed, incrementally-consumable
  `BootPlan`/`BootStep` pipeline (`Engine::begin_boot`/`boot_step`), splitting
  each texture into CPU decode (`DecodedTexture`) and GL upload, with a
  `boot_step(usize::MAX)` synchronous fast path.
- Async desktop boot: create the window first, then run ROM resolve, archive
  decompress, texture decode, and wasmtime module compile on a background thread
  (streaming `BootEvent` over `mpsc`) while the main thread drains events and
  uploads decoded textures; the headless/golden boot stays synchronous.
- Interleaved web hydration: compile shaders + build the boot plan up front,
  then drain a time-budgeted slice of boot steps per animation frame (instead
  of stalling the first frame), keeping the browser responsive and the DOM boot
  overlay on screen while the large atlases decode.
- Faster boot: drain ROM archive entries into `Arc<[u8]>` resources (no double
  copy), decode textures/depth/normals in parallel on the loader thread pool
  (`CLASSIC_LOADER_THREADS`), and cache compiled `wasmtime::Module`s on disk
  keyed by the published ROM sha256 for `trusted` ROMs so repeat launches skip
  cranelift.
- Parallel basis transcode: transcode GPU-compressed (`.basis`) sheets off the
  GL thread on the loader pool (`CLASSIC_LOADER_THREADS`), then upload the
  decoded payloads on the render thread — mirrors the PNG decode/upload split
  for the compressed path.
- Boot resource sampling: a native `/proc`-based sampler emits periodic
  `ResourceUsage` boot events (process CPU% + RSS) during boot, so
  `CLASSIC_BOOT_LOG` carries a perf trace through the whole pipeline.
- Boot loading screen: a GL-only `visual` loader (the default) driven by the
  boot event stream — a dependency DAG, per-sheet resource chips, a progress/log
  footer, and a live CPU/RSS header — with `console`/`off` modes and forced-off
  for headless/golden/test. Embeds the DejaVu Sans SDF atlas so text renders from
  frame 0, emits per-sheet `ResourceDecoded` for `.basis` sheets (native pool +
  web worker), unifies ROM downloads into the boot stream (`RomFetchStarted`/
  `RomFetchProgress`, desktop CDN fallback + web `ReadableStream` progress), and
  switches metrics to `sysinfo` (desktop CPU%+RSS; web JS-heap memory).
- Boot loader UI migration: render the loading screen through the retained-mode
  UI system — SDF-text/rect/sprite entities driven from the boot state each
  frame (`install`/`sync`/`uninstall`), with the DAG connector edges drawn in an
  overlay hook — instead of hand-rolled `draw_*` calls.  Text now goes through
  the per-entity glyph-buffer cache (no per-frame rebuild), and the duplicated
  `draw_text`/`measure_text` path is deleted.
- Abortable boot: Esc during the desktop loading screen aborts the load and
  stops the process; on web it stops hydration and leaves the loader hanging.
- Off-thread worker compile: split the Tier-3 `GuestWorker` wasmtime build into
  a background compile (cranelift `Module`) and a GL-thread instantiate, and
  compile the root ROM's worker module alongside the foreground guests in the
  desktop boot, so the loader no longer freezes on the last ~1s of lunar boot.

### Changed

- Read the vehicle anchors `data[]` artifact as `name → anchors` maps
  (`BTreeMap`), dropping the redundant `name`/`directions`/per-part `texture`
  fields (#89).
- Stream `Rom` archive entries into the pack writers (`for_each_entry`),
  avoiding a full `Vec<(String, Vec<u8>)>` materialization.
- Remove the separate light-space coordinate system: positions, normals, the
  directional shadow map and the light UBO now evaluate lighting in world
  metres, and the sun is authored in world space.  The point-light `radius`
  stays in its legacy light-space px unit and is converted to metres at gather
  time, so falloff is unchanged.
- Pick the terrain point under the cursor with a camera raycast
  (`iso_camera_ray` + `raycast_terrain`) that intersects the height field and
  prefers the front, occluding surface, replacing the fixed-point ground-plane
  parallax.

### Fixed

- Fix the `KeyL` light debug overlay so its markers track the cursor over
  slopes: project through `iso_camera_px` and sample mesh-matched heights, and
  account for the tilemap `Transform.position` offset in the mouse raycast and
  shadow pass.
- Fix the web boot loading screen panicking on `std::time::Instant::now()`
  ("time not implemented on this platform") by using a platform-neutral boot
  timer (`BootTimer`: `Instant` on native, `Date::now()` on web).
- Fix the editor/HUD UI holding the 1280x720 reference size (until the next
  window resize) after the loading screen hands off to the game — size it to
  the engine's actual viewport at install time.

## [0.1.1] - 2026-09-01

### Added

- Dynamic point/spot lights in a `std140` UBO (`MAX_LIGHTS` = 256), evaluated
  in the lit shaders with a windowed inverse-square falloff modulated by
  albedo (#83).
- Directional shadow map: an orthographic light-space box fit around the
  tilemap and sprite billboards, sampled to shadow the sun diffuse term (#83).
- Sprite billboards cast and receive shadows as standing geometry via a `vec3`
  ground anchor, and baked sprite normals rotate into light space via
  `blender_to_light_3` (#83).
- Entity-backed `LightHandles` with guest `light_spawn`/`light_set`/
  `light_release` imports and a `KeyL` light-debug overlay (#83).

### Fixed

- Make lighting metric: `iso_to_light_4` drops the isometric `diag(1, 0.5, 1)`
  squash so `length`/`normalize`/`dot` are isotropic — point-light pools were
  circles and sprite normals off by up to 153° (#83).
- Scale terrain normals by the tile size (issue #77) (#83).

## [0.1.0] - 2026-09-01

### Added

- `Selectable` component and a host-owned `SelectionSet`
  (`select_at`/`select_box`/`selected_names`) with unit/building group
  semantics, plus a per-sprite selection silhouette (#80).
- Inventory system: item catalog (`ItemId`/`ItemClass`/`StackRule`/`ItemDef`/
  `InventoryType`/`Inventory`/`ItemRegistry`), host inventory I/O mechanics,
  and `items`/`inventory_types` in the ROM manifest (#80).
- Guest SDK surface for selection, inventory and vehicle control
  (`selected_names`, `selection_clear`, `inventory_*`, `item_def`,
  `vehicle_set_speed`, `vehicle_probe`, `vehicle_probe_clear`,
  `get_sprite_frame`, `set_sprite_offset`, `inventory_capacity`) (#80).
- Drop-preview path overlay and right-click deselect in the demo (#80).
- Container hover tooltip: a `UiKind::Grid` layout, packed-atlas icon sprites,
  and a host-owned `InventoryUi` overlay showing item icons + counts above a
  hovered container (#80).
- Navigation blocking: `ColliderData.blocks_nav` + `set_collider_blocks_nav`
  rasterize blocking footprints into the nav grid for humanoid and vehicle
  pathfinding; `vehicle_footprint_radius` lets guests derive pickup/drop
  clearance from the real footprints (#80).


## [0.1.0-alpha.0] - 2026-08-28

The first recorded release covers the Rust rewrite and everything that landed
since: the TypeScript engine was dropped and the codebase became the current
Rust + wasm multi-target engine with ROM-driven content.

### Added

- Rust port of the engine — 7 crates, native (winit+glutin) and web
  (web-sys+trunk) targets, parity with the TypeScript engine (#24).
- `classic-demo` crate, isolating the prefab, editor, and test-runner layers
  from the engine core (#26).
- Procedural `lunar` terrain generator (layered simplex noise + crater field)
  and demo scene (#27).
- ROM layer (`classic-rom`): `RomArchive`/`Rom`, manifest, resources,
  `load_rom`/`dump_rom` (#28).
- WASM guest runtime (`classic-guest`): wasmtime (native) / wasmi (wasm) with
  sandbox fuel + memory caps, and per-scene `#![no_std]` guests (#29).
- `classic-terrain` noise toolkit; terrain generation moved into ROM guests
  (`guest-driven maps`) (#30).
- Per-frame animation offsets and the lunar landing rocket (#32).
- `IsoVehicle` wheeled-vehicle system with the LRV rover (#34).
- Worker offload for pathfinding and guest map generation (#41).
- ROM release fetching from the `classic-roms.com` bucket (#44).
- LRV vehicle pathing, turning, and cliff-jump (#46).
- Packed-atlas sprites: frame tables + packed rendering for LRV, props, rocket
  (#52).
- Per-texture depth maps with a unified iso depth scale (#55).
- Shared `#![no_std]` `pathfinder` crate (A* + vehicle search) and
  `vehicle_goto` offload (#62).
- Unified sprite draws and per-sheet normal/depth maps (#63).
- Web guest worker offloaded to a browser `Worker` (#64).
- Iso height axis re-expressed in metres (#67).
- Clamped chassis-plane vehicle suspension (#68).
- Front-wheel steering for the LRV (#69).
- Vehicle speed/turn-rate tuning panel (#70).
- Sprite tinting and guest-driven sprites (#71).
- Vehicle pathing that climbs by pitch/roll rather than the walk grid (#72).
- Packed-atlas companions uploaded in native channel counts (#73).
- Coordinated steering, reverse recovery, and turn-cost routing for the LRV
  (#74).
- Sparse rocket offset interpolation and the landing/launch cycle (#75).
- ROM content-hash lock (`cargo xtask lock-roms`/`check-roms`) to fail fast on
  bucket drift (#76).

### Changed

- `classic-guest` guests own the scene and map; the engine was de-demo-ified
  (#31).

### Removed

- TypeScript engine and tooling (−19 000 LOC) (#66).
- TS/Vite conventions from ROMs, state, and tooling (#33).

### Fixed

- Headless EGL teardown segfault and CI coverage gaps (#51).
- Iso depth clipping and mouse-iso debug overlay (#65).
- Clippy `chunks_exact_to_as_chunks` in the byte decoders.
- Pages deploy: stage `pathfinder.wasm` before `trunk build`.
