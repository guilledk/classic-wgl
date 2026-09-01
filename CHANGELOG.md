# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
See [`VERSIONING.md`](VERSIONING.md) for the release policy and process.

## [Unreleased]

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
