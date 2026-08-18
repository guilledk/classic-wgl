---
name: classic-terrain
description: >
    Procedural terrain generation for classic-wgl's Rust port.  Covers the
    lunar surface generator (mare mask, regolith fBm, wrinkle ridges,
    meteorite crater field, slope relaxation, landing zones, corridor
    carving), the fractal noise combinators, the material table, the
    procedurally painted tileset, and the `lunar` scene wiring.  Use when
    tuning terrain parameters, adding a generator, debugging pathability or
    buildable area, or working on scene selection.
    Trigger phrases: "generate_lunar", "LunarParams", "LunarTerrain",
    "crater", "mare", "regolith", "landing zone", "slope relaxation",
    "angle of repose", "Fbm", "tiling_noise_2d", "build_lunar_tileset",
    "LunarMaterial", "CLASSIC_ROM", "lunar scene", "terrain".
---

# Procedural Terrain in classic-wgl

Everything here is **pure, GL-free and deterministic**: a function of a seed
string only.  That is what makes it unit-testable without a GL context,
reproducible for golden traces, and safe on `wasm32`.

| Where | What |
|---|---|
| `crates/classic-terrain/src/{simplex_noise,fractal,noise_fields}.rs` | open noise primitives (simplex, fBm/ridged/billow, domain warp, bulk fields) |
| `guest/lunar-guest/src/lunar.rs` | The lunar surface generator (`generate_lunar`) |
| `guest/lunar-guest/src/material.rs` | `LunarMaterial` table shared by generator and tileset |
| `guest/lunar-guest/src/tileset.rs` | Procedurally painted tile sheet |
| `guest/lunar-guest/src/lib.rs` | wasm entrypoint: bulk-uploads grids + `commit_terrain` + view setup |
| `guest/lunar-guest/tests/terrain_lunar.rs` | The regression net (25 tests) |

---

## 1. Design principle

The features that make a lunar surface *look* right are the same ones that
make it *play* right for an RTS.  They are not traded off against each other:

| Layer | Lunar analogue | Gameplay effect |
|---|---|---|
| Low-frequency mare mask | Maria are basaltic flood plains — genuinely flat | Large organic buildable regions |
| Roughness attenuated by the mask | Highlands are crater-saturated, maria are not | Roughness only where harmless |
| Fewer craters over maria | Mare surfaces are geologically much younger | Basins stay flat |
| Age-ordered crater field | Young craters overprint old ones | Chokepoints and cover |
| Wrinkle ridges, mare-weighted | A real mare-only tectonic feature | Low-amplitude interest |
| Slope relaxation (talus) | Regolith mass wasting at the angle of repose | Bounds every slope |
| Landing pads with a wide skirt | Reads as a dust-filled basin floor | Guaranteed start positions |
| Bright ejecta rays | Copernicus / Tycho ray systems | Pure albedo, zero gameplay cost |

**Corollary:** when a knob makes the map play worse, look for the physical
process that is missing rather than adding a gameplay override.

---

## 2. Two slope thresholds

This is the single most important parameter relationship.

- `max_slope` (default `1.15`) — the **angle of repose**, enforced by
  relaxation.  Nothing on the map is ever steeper.
- `nav_max_slope` (default `0.62`) — the **walkability** cut.
- `build_max_slope` (default `0.22`) — buildability, reported in stats only.

Terrain between `nav_max_slope` and `max_slope` exists, is steep, and is
impassable: that band *is* the chokepoint system.  Collapse the two and you
get either absurd cliffs or a map with no impassable terrain at all.

`max_slope` also protects the renderer: `build_mesh` emits wall geometry only
at **map borders**, so an interior discontinuity renders as a stretched,
badly-lit top face.  Continuous, slope-bounded terrain avoids the problem
entirely instead of needing new mesh code.

---

## 3. Pipeline (`generate_lunar`)

1. **Mare mask** — 2-octave fBm + smoothstep → `0` = mare, `1` = highland.
2. **Macro relief** — `highland_amplitude * m` plus broad undulation.
3. **Landing-zone placement** — chosen from the *mask and macro relief only*,
   before craters exist, so crater sites can be rejected against them.
   Ring-symmetric (fair starts), then snapped to the lowest-variance
   candidate in a local window (so they do not look mechanically placed).
4. **Regolith roughness** — fBm scaled by `mare_roughness + (1-mare_roughness)*m`.
5. **Wrinkle ridges** — domain-warped ridged fBm, weighted by `(1 - m)`.
6. **Crater field** — see §4.
7. **Slope relaxation** — see §5.
8. **Landing-zone flattening**, then a second relaxation with pad cores
   **pinned** (a pad next to a highland otherwise leaves an over-steep skirt).
9. **Normalisation** — shift the minimum to `floor_height`.  A shift, not a
   clamp: clamping the low end creates flat "lakes".
10. **Tile classification** — see §6.
11. **Navigation + connectivity** — see §7.

---

## 4. Crater field

Sites use a power-law radius (`crater_size_exponent = 3.2`, biasing hard
toward small craters, matching the real size-frequency curve) and a density
per 1000 tiles rather than an absolute count — an absolute count silently
over-saturates small maps.

Craters are stamped **largest first**, so smaller (younger) craters overprint
older ones.  That ordering is the single biggest contributor to a field
reading as genuinely lunar.

Per crater:

- **Anchor**: the reference elevation blends from the crater centre out to the
  *local pre-impact surface* at the rim (`anchor = h0 + (pre[i] - h0) * t²`).
  Anchoring purely to the centre leaves a hard step wherever the rim crosses
  terrain of a different elevation — which happens constantly for craters
  whose centres fall off the map edge.
- **Bowl**: parabolic for simple craters; flat floor + central peak past
  `crater_complex_radius`.  Applied with `min`, so a young crater inside an
  older one does not fill it back in.
- **Excavation limit**: `bowl.max(pre_impact[i] - max_excavation)`.  Nesting
  compounds without it, dragging the global minimum down and (via the stage-9
  shift) shoving the rest of the map into the ceiling.  Bottoming out gives a
  flat floor — which deep lunar craters really have, filled with impact melt.
- **Rim + ejecta**: combined with **`max`, never `+`**, via a `deposit`
  buffer.  Ejecta is excavated material redistributed, so a saturated field
  must not inflate the mean elevation.  Summing sends the whole map into the
  height ceiling as soon as blankets start overlapping.
- **Radius wobble**: the effective radius is perturbed by directional noise so
  rims are lobed rather than perfect circles.

### Traps

- **Do not switch deposits back to `+=`.**  It looks harmless on one crater
  and destroys the map at realistic densities.
- **Do not raise `crater_density` to compensate for a flat-looking map**; check
  `mare_crater_factor` and the mask frequency first.
- Rays are **albedo only** (`ray_boost`, tile grid).  Never let them touch
  heights: they extend several radii and would wreck pathability for free.

---

## 5. Slope relaxation

Jacobi talus/thermal-erosion iteration: any vertex overhanging a 4-neighbour by
more than `max_slope` sheds `0.18 * excess`.  Runs to `relax_tolerance` or the
`relax_iterations` budget, whichever comes first; both the iterations used and
the worst remaining slope land in `LunarStats`.

Long steep walls converge slowly (diffusion is `O(L²)`), so if
`relax_iterations_used` pins at the budget, the cause is almost always a
*discontinuity* being introduced upstream, not too small a budget.  Fix the
upstream discontinuity.

`pinned` holds pad cores fixed while still letting them pull on neighbours.

---

## 6. Tile classification

Per tile: gradient magnitude from the four corner heights.

**Walkability uses the raw slope; materials use a 3x3 box-blurred copy.**
Regolith slope varies enough tile-to-tile that hard thresholds on the raw
value flip classes back and forth across a boundary, and the large albedo gaps
between classes turn that into a visible checkerboard.

Priority order (earlier wins): `LandingPad` → `Rocky` (slope) → `RimBright` →
`Ray` → `CraterFloor` → `RegolithCoarse` → mare/`Regolith`.  Slope is checked
before provenance so impassable terrain stays visually legible — that matters
more for an RTS than geological accuracy.

The mare/highland contact is **dithered** (`0.5 + noise * 0.18`): a fixed cut
traces a clean contour, and with a ~2:1 albedo gap that reads as a hard
stair-stepped shoreline.

Rim and floor masks carry a **strength scaled by crater radius**, so the
countless small old craters on a saturated surface do not each paint a bright
rim.

---

## 7. Navigation and connectivity

`walkable = slope <= nav_max_slope`, then:

1. `connect_spawns` — any spawn not in the main component gets a corridor
   carved: a BFS route is opened and the heights along it are relaxed until
   genuinely traversable, not merely flagged.  Degraded lunar craters really
   do have breached rims, so it reads as a natural saddle.
2. `prune_to_main_component` — unreachable walkable pockets are blanked.
   Reachable-looking ground the pathfinder cannot use is worse than no ground.

`terrain_lunar.rs` verifies mutual spawn reachability with the engine's own
`pathfinder::find_path`, across eight seeds.

---

## 8. Tileset

`build_lunar_tileset(seed, tile_px, cols, rows)` paints an 8x8 grid of 32px
cells (256x256) from `MATERIALS`, registered at runtime via
`Gfx::add_texture_rgba8`.  No PNG, so no commit to the separate private
`assets/` submodule, and the texture can never drift from the classifier.

**Every cell must tile seamlessly.**  The fragment shader samples cells with
`fract()`, so wherever two tiles of the same id meet, a cell's left edge abuts
its own right edge.  All noise goes through `tiling_fbm_2d` (the 4D torus
trick over `noise_4d`) and microcraters use toroidal distance.

`tiling_noise_2d` applies `rem_euclid(period)` first so the seam is
*bit-exact*; without it the two ends differ by an ULP in the angle, which the
noise gradient amplifies into a faint but real seam.

---

## 9. Engine wiring

```rust
e.load_state(state_lunar_json)?;      // must come first
// ... shared init ...
// the lunar guest uploads nav + tiles + heights, then commit_terrain(height_scale)
```

- The lunar scene **reuses the demo entity names** (`tilemap`,
  `tilemapNavigation`, `cursor`).  `apply_editor_selection` and
  `sync_nav_heights` look those names up by role, so reuse means the whole
  editor toolchain works on a generated map for free.
- `HEIGHT_SCALE = 14.0` (guest `guest/lunar-guest/src/lib.rs`, passed to
  `commit_terrain`) overrides the default `tile_pixel_size[0]` (32).
  Generated terrain spans ~7 height units; at 32 the relief is overblown and
  the 3-pass mouse-picking parallax solve is stretched to ~7 tiles of
  correction, where it converges poorly.
- `Engine::base_height_scale` records what the mesh was built with, so the
  height widget's multiplier scales relative to it rather than assuming
  `tile_pixel_size[0]`.
- `Engine::nav_slope_threshold` is a configurable field (default `2.0`) used by
  `sync_nav_heights`; it is **not** automatically set from the generator's
  `nav_max_slope`, so a height edit on a generated map classifies slopes with a
  different (coarser) rule than the generator used.

### Scene selection

`CLASSIC_ROM=rom:lunar` (native) / `?rom=rom:lunar` (web) selects the embedded
`lunar.rom` (empty selector → `rom:demo`).  The selector is a URI-scheme
grammar: `rom:<name>` for embedded ROMs, `http(s)://` to fetch, and a bare
value is a file path (native) / relative URL (web).

---

## 10. Tuning workflow

1. `cargo run --manifest-path guest/lunar-guest/Cargo.toml --release --example
   dbg_lunar -- <seed> <size>` prints stats plus ASCII height / material /
   navigation maps.  Use this for structure and statistics — it is far faster
   than rendering.
2. Render a frame for anything about *appearance*:
   ```
   CLASSIC_ROM=rom:lunar CLASSIC_HEADLESS=1 CLASSIC_OFFSCREEN=1 CLASSIC_FRAMES=60 \
   CLASSIC_WIDTH=1280 CLASSIC_HEIGHT=720 CLASSIC_GOLDEN_PNG=1 CLASSIC_GOLDEN=update \
   CLASSIC_GOLDEN_DIR=/tmp/shot cargo run -p classic-desktop
   ```
   (Pixel capture only fires on `golden_capture_frame`, default 55 — so
   `CLASSIC_FRAMES` must exceed it.)
3. In-app: dev menu → **Lunar Gen** → seed / craters / mare + Regenerate.
4. `cargo test --manifest-path guest/lunar-guest/Cargo.toml --test terrain_lunar`.
5. `CLASSIC_LOG=terrain=info` — note the `terrain` channel exists but the guest
   generator emits no host logs through it (the generator runs inside wasm and
   has no `log` import).

### Health check

For the 400x400 default map, expect roughly:

| Stat | Healthy |
|---|---|
| `craters` | ~12 per 1000 tiles (≈1900 at 400x400) |
| relief (`max_height - min_height`) | 6–9, **independent of map size** |
| `max_slope_actual` | just above `max_slope` |
| `relax_iterations_used` | well under the budget |
| `walkable_fraction` | 0.80–0.95 |
| `buildable_fraction` | 0.35–0.60 |
| `mare_fraction` | 0.3–0.6 |
| generation time | ~230 ms release, ~2 s debug |

`terrain_character_is_stable_across_map_sizes` pins these across 200/400/600,
so a parameter that accidentally becomes size-dependent gets caught.

`max_height` sitting exactly on `LunarParams::max_height`, or
`relax_iterations_used` pinned at the budget, both mean something upstream is
running away — check the deposit and excavation limits first.

---

## 11. Known limitations

- **One generator.**  The noise primitives (`classic-terrain`) are generic, but
  the only map algorithm is `lunar` (in `guest/lunar-guest`).  See
  `classic-procmaps` §2 for the recipe to add a second generator/scene.
- **No erosion by water/wind** — deliberately: there is none on the Moon.
- **Rays are approximate.**  Angular noise thresholded on direction; real ray
  systems are discontinuous and clumpy.
- **No secondary crater chains** or basin-scale multi-ring structures.
- **Landing zones are circular.**  Fine at this scale, but they read as discs
  under close zoom.
- **The generator does not know about buildings.**  `spawn_points` are single
  cells; nothing reserves resource-node sites yet.
- **`sync_nav_heights` uses an absolute `|Δh|` rule**, not the gradient
  magnitude the generator uses, so editor height edits classify slightly
  differently from generated terrain.

---

## 12. Map size

The shipping size is **400x400** (160K tiles), set in three places that must
agree — `LunarParams::default()` and both the `Tilemap` and
`IsometricNavMesh` entries in `public/state_lunar.json`.  `commit_terrain`
builds the mesh from the uploaded grids, so the state-file dimensions must
agree with `LunarParams::default()`.

Almost every parameter is in scale-free units (cycles per tile, craters per
1000 tiles, height units) and needs no adjustment.  Only genuinely absolute
counts do: `auto_landing_zones` and `ray_crater_count`, both of which were
scaled with the area when the map grew from 200x200.

The full scaling envelope — mesh capacity bound, tile data texture size, the
pathfinder's synchronous-on-render-thread cost, generation `O(n²)` — and the
scale-free-vs-absolute parameter discipline live in `classic-procmaps` §3 and
§5.  That skill is the single source of truth for those numbers; if you scale
the map again, update it there.
