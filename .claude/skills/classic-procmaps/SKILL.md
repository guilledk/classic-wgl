---
name: classic-procmaps
description: >
    Authoring and scaling procedural maps for classic-wgl's Rust port.
    Covers the generic `terrain/` module contract (fractal noise, material
    table, tileset painter, generator), the recipe for adding a new biome or
    scene, the scale-free-vs-absolute parameter discipline, scene wiring
    (entity-name reuse, init order, DemoAssets, Scene::parse), and the
    map-size scaling envelope (mesh capacity bound, tile data texture,
    pathfinder on the render thread).  Use when adding a generator, scaling
    a map, wiring a new scene, or deciding whether a parameter is safe to
    reuse at another size.
    Trigger phrases: "procedural map", "new biome", "new generator",
    "add a scene", "map size", "scale the map", "terrain module",
    "generate_lunar", "LunarParams", "scene wiring", "CLASSIC_SCENE",
    "state_lunar.json", "scale-free", "GL_MAX_TEXTURE_SIZE".
---

# Procedural Maps in classic-wgl

This is the *general* reference for procedural map authoring and scaling.  For
the lunar generator's algorithms specifically (mare mask, crater stamping,
slope relaxation, tile classification) read `classic-terrain`.  For the mesh,
`classic-iso`.  For the pathfinder, `classic-physics`.  For texture upload,
`classic-gfx`.  This skill owns the parts that are reusable across any
generator.

---

## 1. The `terrain/` module contract

`classic-core/src/terrain/` is deliberately split so the reusable machinery is
not entangled with one specific biome:

| Module | Owns | Generic? |
|---|---|---|
| `fractal.rs` | fBm / ridged / billow, domain warp, periodic (tiling) noise | Yes |
| `material.rs` | The material table — `MATERIALS`, `tile_id`, `spec_for` | Yes |
| `tileset.rs` | Paints an RGBA tile sheet from `MATERIALS` | Yes |
| `lunar.rs` | The lunar generator (`generate_lunar`) | No |

**The one invariant that makes this work:** the generator *classifies* tiles
into materials and the tileset *paints* those materials, and both read the
same `material.rs` table.  That shared table is what stops the two from
drifting apart.  A new generator must define its materials in `material.rs`
(not as a local enum) so its tileset is derived, not hand-matched.

Every function is pure (`&SimplexNoise`, `&params`) → deterministic from a seed
string, GL-free, unit-testable, and safe on `wasm32`.  Never call
`Random::from_entropy` — it reads the system clock, which breaks golden traces
and web builds.  (`Random::next_f64` was once silently returning `[0, 2)`
instead of the documented `[0, 1)` — see `docs/TS-PARITY.md` — so a seed that
"looks fine" can still hide a broken RNG if you copy the old bug.)

---

## 2. Recipe: add a new generator / scene

The `lunar` scene is the worked example.  To add another (`<name>`):

1. **Generator** — `classic-core/src/terrain/<name>.rs`, exporting
   `<Name>Params` (with `Default`) and `generate_<name>(&Params) -> Terrain`.
   The output struct must carry `heights` (vertex grid, `(sx+1)*(sy+1)`),
   `tiles` (tile grid, `sx*sy`, ids >= 1), and `nav` (tile grid, 1=walkable).
   Watch the grid-layout mismatch — it is the #1 latent bug (`build_mesh`
   asserts both lengths).
2. **Materials** — add `<Name>Material` variants + `MaterialSpec`s to
   `material.rs`, keeping `tile_count() < cols*rows` (id 0 is reserved).
3. **Tileset** — the shared `build_lunar_tileset` (or a new
   `build_<name>_tileset`) already paints `MATERIALS`; if you reuse it, this
   step is automatic.
4. **Scene description** — `public/state_<name>.json` with the SAME entity
   names as `state.json` / `state_lunar.json` (`tilemap`, `tilemapNavigation`,
   `navAgent`, `cursor`, `camController`).  See §4 for why.
5. **Scene enum** — add a variant + `parse()` arm in `classic-demo`.
6. **Dispatch** — a `Scene::<Name>` arm in `init_engine` that does
   `load_state(state_<name>_json)` then `init_<name>_terrain(...)`, and installs
   nav via `init_navigation_data` (NOT `init_navigation`, which would
   re-derive walkability from heights and clobber the generator's own).
7. **Wiring fn** — `classic-demo/src/scenes/lunar.rs` (or a sibling) with
   `init_<name>_terrain`, `regenerate_<name>_terrain`, `focus_camera_on_spawn`.
8. **Golden** — `tests/golden/<name>/` + a CI job (the lunar job needs
   `CLASSIC_FIXED_DT` so the idle animator lands on a deterministic frame).
9. **Tests** — a `classic-core/tests/terrain_<name>.rs` asserting the
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
`classic-core/tests/terrain_lunar.rs`) generates at 200/400/600 and asserts
crater density, walkable/buildable fractions, relief, and slope bound all stay
in band.  Add the same test for a new generator.  It is the difference between
"a parameter changed the map" and "a parameter broke the map at a new size."

---

## 4. Scene wiring

- **Reuse the demo entity names.**  `apply_editor_selection`,
  `sync_nav_heights`, and the `dump_map_data` / `dump_nav_data` /
  `dump_height_data` dumpers all look up `"tilemap"` and `"tilemapNavigation"`
  by string.  Reusing those names means the entire editor toolchain (height
  brush, tile palette, nav editor, agent click-to-move) works on a generated
  map with zero extra code.  A new scene with fresh names loses all of it.
- **Init order:** `load_state` → `init_<name>_terrain` → shared texture/UI init
  → `init_navigation_data` → `init_nav_mesh_render`.  The terrain step must
  precede navigation because the nav installer reads the generated `nav`.
- **Generated tilesets skip the PNG pipeline.**  `init_tilemap_generated`
  registers the in-memory tileset via `Gfx::add_texture_rgba8`, so no commit to
  the private `assets/` submodule and no new `include_bytes!`.
- **Scene selection:** `CLASSIC_SCENE=<name>` (native) / `?scene=<name>` (web)
  → `Scene::parse`, unknown values fall back to `Demo`.

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
- **Click-to-move rejects impassable destinations before A*.**  A* against a
  blocked cell cannot succeed but still exhausts every reachable cell before it
  can return `None`.  Rejecting `nav[cy*sx+cx] == 0` up front removes the only
  routine way to trigger the 21 ms exhaustive case (and is the correct
  behaviour: clicking a cliff should do nothing).
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

1. **Structure/stats first:** `cargo run --release -p classic-core --example
   dbg_lunar -- <seed> <size>` prints stats plus ASCII height/material/nav
   maps — far faster than rendering for iterating on parameters.
2. **Appearance:** render a frame (pixel capture fires only on
   `golden_capture_frame`, default 55, so `CLASSIC_FRAMES` must exceed it):
   ```
   CLASSIC_SCENE=lunar CLASSIC_HEADLESS=1 CLASSIC_OFFSCREEN=1 CLASSIC_FRAMES=60 \
   CLASSIC_WIDTH=1280 CLASSIC_HEIGHT=720 CLASSIC_GOLDEN_PNG=1 CLASSIC_GOLDEN=update \
   CLASSIC_GOLDEN_DIR=/tmp/shot cargo run -p classic-desktop
   ```
3. **Guarantees:** `cargo test -p classic-core --test terrain_<name> --
   --test-threads=1`.
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
