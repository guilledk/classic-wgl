# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
See [`VERSIONING.md`](VERSIONING.md) for the release policy and process.

## [Unreleased]

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
- Web Basis Universal transcoder: vendor the three.js `basis_transcoder`
  (MIT/Apache) and transcode `.basis` (ETC1S) sheets to S3TC/ETC2/BC7 on WebGL 2
  (RGBA8 fallback), so GPU-compressed sheets render on web.

### Fixed

- Make lighting metric: `iso_to_light_4` drops the isometric `diag(1, 0.5, 1)`
  squash so `length`/`normalize`/`dot` are isotropic — point-light pools were
  circles and sprite normals off by up to 153° (#83).
- Scale terrain normals by the tile size (issue #77) (#83).

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
