---
name: classic-procmaps
description: >
    Authoring and scaling procedural maps for classic-wgl's Rust port.
    Covers the generic `terrain/` module contract (fractal noise, material
    table, tileset painter, generator), the recipe for adding a new biome or
    scene, the scale-free-vs-absolute parameter discipline, scene wiring
    (entity-name reuse, init order, ROM entrypoint dispatch), and the
    map-size scaling envelope (mesh capacity bound, tile data texture,
    pathfinder on the render thread).  Use when adding a generator, scaling
    a map, wiring a new scene, or deciding whether a parameter is safe to
    reuse at another size.
    Trigger phrases: "procedural map", "new biome", "new generator",
    "add a scene", "map size", "scale the map", "terrain module",
    "generate_lunar", "LunarParams", "scene wiring", "CLASSIC_ROM",
    "roms/<scene>/state.json", "scale-free", "GL_MAX_TEXTURE_SIZE".
---

# Procedural Maps in classic-wgl

This is the *general* reference for procedural map authoring and scaling.  For
the lunar generator's algorithms specifically (mare mask, crater stamping,
slope relaxation, tile classification) read `classic-terrain`.  For the mesh,
`classic-iso`.  For the pathfinder, `classic-physics`.  For texture upload,
`classic-gfx`.  This skill owns the parts that are reusable across any
generator.

---

## 1. The noise/terrain module contract

The open noise primitives live in `crates/classic-terrain`; the lunar-specific
material/tileset/generator live in `guest/lunar-guest`:

| Module | Owns | Generic? |
|---|---|---|
| `crates/classic-terrain/src/simplex_noise.rs` | seedable simplex + deterministic `Random` | Yes |
| `crates/classic-terrain/src/fractal.rs` | fBm / ridged / billow, domain warp, periodic (tiling) noise | Yes |
| `crates/classic-terrain/src/noise_fields.rs` | bulk noise fields | Yes |
| `guest/lunar-guest/src/material.rs` | `LunarMaterial` table — `MATERIALS`, `tile_id`, `spec_for` | No |
| `guest/lunar-guest/src/tileset.rs` | Paints an RGBA tile sheet from `MATERIALS` | No |
| `guest/lunar-guest/src/lunar.rs` | The lunar generator (`generate_lunar`) | No |

**The one invariant that makes this work:** the generator *classifies* tiles
into materials and the tileset *paints* those materials, and both read the
same `material.rs` table.  That shared table is what stops the two from
drifting apart.  A new generator must define its materials in its guest's
`material.rs` (not as a local enum) so its tileset is derived, not hand-matched.

Every function is pure (`&SimplexNoise`, `&params`) → deterministic from a seed
string, GL-free, unit-testable, and safe on `wasm32`.  `Random` only exposes
`from_seed` / `from_seed_str` — there is no entropy source, so nothing can
accidentally read the system clock and break golden traces.  (`Random::next_f64`
was once silently returning `[0, 2)` instead of the documented `[0, 1)` — see
`docs/TS-PARITY.md` — so a seed that "looks fine" can still hide a broken RNG
if you copy the old bug.)

---

## 2. Recipe: add a new generator / scene

The `lunar` scene is the worked example.  To add another (`<name>`):

1. **Generator** — `guest/<name>-guest/src/<name>.rs`, exporting
   `<Name>Params` (with `Default`) and `generate_<name>(&Params) -> Terrain`.
   The output struct must carry `heights` (vertex grid, `(sx+1)*(sy+1)`),
   `tiles` (tile grid, `sx*sy`, ids >= 1), and `nav` (tile grid, 1=walkable).
   Watch the grid-layout mismatch — it is the #1 latent bug (`build_mesh`
   asserts both lengths).
2. **Materials** — add `<Name>Material` variants + `MaterialSpec`s to
   `guest/<name>-guest/src/material.rs`, keeping `tile_count() < cols*rows`
   (id 0 is reserved).
3. **Tileset** — the shared `build_lunar_tileset` (or a new
   `build_<name>_tileset`) already paints `MATERIALS`; if you reuse it, this
   step is automatic.
4. **Scene description** — `roms/<name>/state.json` with the demo entity
   names (`tilemap`, `tilemapNavigation`, `cursor`; the agent `navAgent` is
   demo-only — generated scenes have no agent).  See §4 for why.
5. **ROM + entrypoint** — add a `pack_scene(...)` call in `xtask/src/main.rs`
   (injects `format_version`/`entrypoint`/`state`/`host_features`/`trusted`/
   `code`) so the scene ships as `roms/out/<name>.rom`; the apps resolve
   `CLASSIC_ROM`/`?rom=` (a `rom:<name>` selector, file path, or URL) to the
   embedded ROM.
6. **Guest** — `guest/<name>-guest/src/lib.rs`: `init()` generates the map,
   bulk-uploads the grids via `set_tiles`/`set_heights`/`set_nav`/`set_tileset`,
   `commit_terrain(height_scale)`, then owns its own view setup
   (`iso_to_screen` → `set_camera` + `set_light` + `set_grid`).  The host is a
   generic terrain store; the map algorithm lives in the ROM guest.
7. **Golden** — `tests/golden/<name>/` + a CI job (the lunar job needs
   `CLASSIC_FIXED_DT` so the idle animator lands on a deterministic frame).
8. **Tests** — a `guest/<name>-guest/tests/terrain_<name>.rs` asserting the
   *gameplay guarantees* (lengths, slope bound, flat pads, buildable fraction,
   mutual spawn reachability via `pathfinder::find_path`), not pixels.

---

## 3. Parameter design: scale-free vs absolute

This is the single most valuable rule from the whole session.  **Write every
parameter in physical, scale-free units**, and the map can be resized freely
without re-tuning:

| Parameter | Unit | Why it scales |
|---|---|---|
| `*_frequency` | cycles per tile | wavelength is a fixed physical size |
| `crater_density` | craters per 1000 tiles | population tracks area |
| `*_amplitude`, `*_ratio` | height units / fractions | relief is absolute, size-independent |
| `radius_min/max`, `skirt`, `edge` | tiles | absolute feature sizes |

The few genuinely **absolute** counts must be revisited when the map grows —
and only these: `size_x`/`size_y`, `auto_landing_zones`, `ray_crater_count`
(each pad/ray system covers a fixed absolute area, so it is a quarter of its
relative footprint on a 4x map).

**Guard:** `terrain_character_is_stable_across_map_sizes` (in
`guest/lunar-guest/tests/terrain_lunar.rs`) generates at 200/400/600 and
asserts crater density, walkable/buildable fractions, relief, and slope bound
all stay in band.  Add the same test for a new generator.  It is the
difference between "a parameter changed the map" and "a parameter broke the map
at a new size."

---

## 4. Scene wiring

- **Reuse the demo entity names.**  `apply_editor_selection` and
  `sync_nav_heights` look up `"tilemap"` and `"tilemapNavigation"` by role
  (`RoleKind::Tilemap` / `NavMesh`).  Reusing those names means the entire
  editor toolchain (height brush, tile palette, nav editor) works on a
  generated map with zero extra code.  A new scene with fresh names loses all
  of it.
- **Init order:** `load_state` → the guest's `init()` (generates + uploads +
  `commit_terrain`) → shared texture/UI init → `init_nav_mesh_render`.  The
  guest's `set_nav` upload is the authoritative nav grid.
- **Generated tilesets skip the PNG pipeline.**  `set_tileset` uploads the
  in-memory tileset via `Gfx::add_texture_rgba8`, so no commit to the private
  `assets/` submodule and no new `include_bytes!`.
- **Scene selection:** `CLASSIC_ROM=rom:<name>` (native) / `?rom=rom:<name>` (web)
  selects the embedded ROM (`demo.rom` / `lunar.rom`); `resolve_rom` →
  `Rom::load` → `init_engine`.  The selector is a URI-scheme grammar: `rom:<name>`
  for embedded ROMs, `http(s)://` to fetch, and a bare value is a file path
  (native) / relative URL (web).  Empty selects `rom:demo`.

---

## 5. Map-size scaling envelope

Four things bind as a map grows, in the order they bite.  Measured for the
shipping 400x400 (160K tiles) map:

| Constraint | Where | 400x400 | Fails at |
|---|---|---|---|
| Tile data texture is 1 px per tile | `tilemap::build_tile_texture` | 640 KB, 400x400 px | GPU `MAX_TEXTURE_SIZE` — **never queried anywhere in the engine**; oversize silently corrupts tile lookup, no GL error |
| Mesh vertex buffer | `tilemap::build_mesh` | ~970K verts, ~35 MB | Memory; the capacity bound is exact |
| A* allocates 4 arrays of `sx*sy` per query, runs **synchronously on the render thread** | `pathfinder::find_path` | ~0.9 ms typical | An exhaustive search is 21 ms at 400x400 and grows with area |
| Generation is `O(n²)` per stage | `generate_lunar` | ~230 ms release / ~2 s debug | ~1 s release at 800x800 |

Details worth internalising:

- **`build_mesh` capacity bound is exact.**  `6` vertices per non-empty tile's
  top face + `6` per wall, and walls only exist on the map perimeter, so
  `(6*size_x*size_y + 6*2*(size_x+size_y)) * 9` floats.  The old code reserved
  `30 * 9` for *every* tile — 5x too much, i.e. 173 MB instead of 35 MB at
  400x400, all of it touched by the allocator.  A test asserts `capacity ==
  bound` so it cannot drift into either reallocation or waste.
- **Click-to-move (guest) does not pre-reject impassable destinations.**  A*
  against a blocked cell cannot succeed but still exhausts every reachable
  cell before returning `None`; the guest's `find_path` treats the empty
  result as a no-op (correct behaviour: clicking a cliff does nothing), at the
  cost of the full search — ~21 ms at 400x400.
- **`GL_MAX_TEXTURE_SIZE` is never queried.**  `build_tile_texture` makes a
  texture exactly `size_x × size_y` pixels, no power-of-two padding, no limit
  check in `upload_data_texture` or `GlTexture::from_rgba8`.  WebGL 2
  guarantees only 2048; desktop GPUs usually allow 8192–16384.  The first
  thing to do before pushing past ~2000 in either dimension is query the limit
  (or split the data texture into pages indexed in the shader).
- **Next binding constraint is the pathfinder**, around 600x600 — not the
  generator.  Past that it needs an iteration budget or to move off the render
  thread (the TS original used a Web Worker; that was dropped in the port).

---

## 6. Verification workflow

1. **Structure/stats first:** `cargo run --manifest-path
   guest/lunar-guest/Cargo.toml --release --example dbg_lunar -- <seed> <size>`
   prints stats plus ASCII height/material/nav maps — far faster than rendering
   for iterating on parameters.
2. **Appearance:** render a frame (pixel capture fires only on
   `golden_capture_frame`, default 55, so `CLASSIC_FRAMES` must exceed it):
   ```
   CLASSIC_ROM=rom:lunar CLASSIC_HEADLESS=1 CLASSIC_OFFSCREEN=1 CLASSIC_FRAMES=60 \
   CLASSIC_WIDTH=1280 CLASSIC_HEIGHT=720 CLASSIC_GOLDEN_PNG=1 CLASSIC_GOLDEN=update \
   CLASSIC_GOLDEN_DIR=/tmp/shot cargo run -p classic-desktop
   ```
3. **Guarantees:** `cargo test --manifest-path guest/lunar-guest/Cargo.toml
   --test terrain_lunar`.
4. **Determinism/regression:** regenerate the scene's golden trace with
   `CLASSIC_FIXED_DT=0.016666668` and `CLASSIC_GOLDEN=update`, then `check`.
5. **Telemetry:** `CLASSIC_LOG=terrain=info` logs a one-line summary per
   generation (craters, relief, walkable %, corridors, timing).

Health-check bands for the 400x400 default are in `classic-terrain` §10.

Two red flags mean something upstream is running away, not that a budget is
too small: `max_height` sitting exactly on `max_height` (ejecta summing
instead of `max`-combining) and `relax_iterations_used` pinned at its budget
(an interior height discontinuity, since slope relaxation diffusion is
`O(L²)`).
