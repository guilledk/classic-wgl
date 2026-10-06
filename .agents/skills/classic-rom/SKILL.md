---
name: classic-rom
description: >
  The ROM layer of classic-wgl, engine side.  Covers the `classic-rom` crate
  (archive formats, `Rom` load/pack, `RomManifest`, `ResourceSet`, the
  multi-ROM `LoadedRoms` DAG, boot events), how the engine namespaces a ROM
  and rewrites its entity and resource references, and how a resolved DAG is
  hydrated through the `BootPipeline`.  Load it before changing
  `crates/classic-rom`, the ROM manifest or format, the multi-ROM DAG,
  namespacing or resource-ref rewriting (`boot_api.rs`), or the boot
  hydration of a ROM.  Authoring a scene ROM is classic-roms' `rom-authoring`
  skill.  Trigger phrases: "ROM", "manifest.json", "RomManifest",
  "LoadedRoms", "deps", "namespace", "ns::name", "resolve_resource",
  "rewrite_resource_refs", "frame_name", "format_version", "roms.lock.json".
---

# classic-rom — the ROM layer, engine side

**Read this before working on `crates/classic-rom` or on ROM hydration in
`crates/classic-engine/src/boot_api.rs` / `boot/`.**

A ROM is one archive holding a `manifest.json`, an entity `state.json`, the
resources the manifest declares, and optional guest `.wasm`.  ROMs are built
and published by `classic-roms` and **fetched, never built, here**
(`cargo xtask fetch-roms`).  This skill is the engine half.  The authoring
half (what `scene.json` says, the `SCENES` table, the per-field naming
checklist, changelog streams) is classic-roms' **`rom-authoring`** skill
(`classic-roms/.agents/skills/rom-authoring/SKILL.md`).  The mechanism lives
here and the authoring checklist lives there, so do not copy it here.

Paths below are relative to `crates/` unless they start with another top-level
directory.  Line numbers are as of the `wkt/project_circle_position` tip
(`3bb3bdf`).  Re-grep a symbol before trusting a number.

## 1. Crate map (`classic-rom/src/`)

| File | Holds |
|---|---|
| `lib.rs` | re-exports; `rom_path` (strips a leading `/`: manifest `/res/x.png` is archive `res/x.png`) |
| `format.rs` | `RomFormat {Zip, TarGz, TarZst}`, `detect_format` by magic (zstd `28 b5 2f fd`, gzip `1f 8b`, zip `PK\x03\x04` / empty `PK\x05\x06`).  `tar.xz` is deliberately absent |
| `archive.rs` | `RomArchive`: decompresses **every** entry into a `BTreeMap` on open (no streaming).  `read`, `read_string`, `take` (drain), `list` |
| `rom.rs` | `Rom { manifest, manifest_json, resources, state }`; `MANIFEST_ENTRY = "manifest.json"`; `Rom::load` (`:42`); `Rom::pack` (`:64`) = `tar.zst` level 19 on native, deflate zip on wasm |
| `manifest.rs` | `RomManifest`, `CodeEntry`, `GridEntry`, `DEFAULT_STATE_ENTRY` |
| `resource.rs` | `ResourceKind` (11 kinds), `ResourceSet` (`Arc<[u8]>` per name per kind), `from_archive` / `from_loader` |
| `loader.rs` | `AssetLoader` trait, `AssetBytes`, `EmbeddedAssetLoader`, `FsAssetLoader` (native) |
| `source.rs` | `RomSource {Embedded, Url, Path, Data}`, `parse_rom_spec` (the `CLASSIC_ROM` / `?rom=` grammar), `DEFAULT_ROM` |
| `loaded.rs` | `LoadedRom`, `LoadedRoms`: the dependency DAG (§3) |
| `boot.rs` | `BootEvent`, `BootSink`, `NullBootSink` / `VecBootSink` / `TeeBootSink` |

Two crates outside `classic-rom` do the I/O.  `classic-platform/src/rom.rs`
turns a selector into bytes (`resolve_rom_source`, `resolve_roms` `:124`
native, `resolve_roms_async` `:297` web).  `classic-engine` hydrates the
result (§5).  `classic-rom` itself does no network or GL work.

`Rom::load` reads the manifest, then the manifest-declared `state` entry,
then **drains** every declared resource out of the archive
(`ResourceSet::from_archive`, `resource.rs:149`).  A declared entry that is
missing from the archive is a load error, and an archive entry nothing
declares is ignored.  Entries that share one `src` (every frame-table texture
of a packed atlas points at the same sheet) alias one `Arc`, and `pack` writes
each archive path once (`pack_writes_a_shared_sheet_once`).

## 2. The manifest surface (`manifest.rs:35`)

`RomManifest` is the core `classic_core::types::Manifest`
(`classic-core/src/types.rs:454`), `#[serde(flatten)]`ed, plus ROM fields.
classic-roms writes all of it in `xtask/src/pack.rs`
(`pack_scene_with_catalog`), mostly copied from `scene.json` and the
`dist.json` catalog.

Flattened `Manifest`:

| Field | Serde | Read by |
|---|---|---|
| `shaders` | `default` | `Engine::ensure_gfx` (`boot_api.rs:28`), **root ROM only**.  The engine owns the builtin catalog; a non-empty entry overrides a builtin *by name*.  classic-roms always writes `[]` |
| `textures` | **required** (no default) | `ResourceSet` (`src`, optional `depth`, `normal`, `frames`), `begin_boot` (decode/upload or `.basis` via `format`), `register_manifest_metadata` |
| `sdf_fonts` | `default` | `ResourceSet.fonts` (the `metrics` JSON); the atlas is the texture named `"{font}-sdf"` |
| `animations` | `default` | registered in `register_manifest_metadata`; the optional `metadata` blob goes through `load_animation_channels` (`KACH` / `KAOS` / legacy dense) |
| `vehicles` | `default` | `VehicleDef` JSON sidecars, keyed by qualified name |
| `models` | `default` | `.glb` → `BootStep::LoadModel` |
| `data` | `default` | parsed as `VehicleAnchors` into `vehicle_anchors` |

ROM fields:

| Field | Serde default | Read by |
|---|---|---|
| `format_version` | `1` | **nothing**.  No code checks it, and classic-roms always writes `1`.  It is a label, not a gate |
| `entrypoint` | `""` | the ROM's display/resolver name (`BootEvent` names, `rom_name`, `load_rom`), and the effective-namespace fallback (§4) |
| `namespace` | `""` | §4.  An empty value means global, or the entrypoint in a multi-ROM DAG |
| `deps` | `[]` | `LoadedRoms::resolve` (§3) |
| `state` | `"state.json"` | `Rom::load` |
| `code` | `[]` | `ResourceSet.code`.  Only the names `main` (foreground guest, every ROM in the DAG) and `worker` (Tier-3, root ROM only) are installed by `classic-demo` |
| `grids` | `[]` | `load_grids` (`boot_api.rs:973`), raw LE `u32` (tiles/nav) or `f32` (heights), named from `Tilemap`/`NavMesh` components |
| `host_features` | `false` | `classic-demo/src/lib.rs:357`, **root ROM only**: editor/HUD layer, unless `CLASSIC_NO_UI` |
| `trusted` | `false` | `guest_limits` (`classic-demo/src/lib.rs:165`): fuel + memory sandbox off when `true`; also gates the native compiled-module cache (`module_cache.rs`).  Note that classic-roms writes `true` when `scene.json` omits it |
| `version` | `None` | nothing in the engine.  It is content versioning for classic-roms and `roms.json` |
| `items`, `inventory_types` | `[]` | `ItemRegistry::build` in `finish_hydrate_roms`, **root ROM only** (per-ROM merge deferred) |
| `vehicle_overrides` | `{}` | `apply_vehicle_overrides` (`boot_api.rs:656`), root ROM only, after the whole closure hydrates (§5).  Keys are already qualified (`lunar-common::lrv`); an unknown key is skipped silently |

Unknown keys are ignored (no `deny_unknown_fields`).  classic-roms also writes
`assets_hash`, which `RomManifest` does not model.

**Changing the format.**  There is no compatibility gate.  `format_version`
is never compared, and every field but `textures` has a serde default, so an
old engine silently ignores a new field and a new engine reads an old ROM with
defaults.  A new field must have a default that reproduces today's behaviour.
Any rename, removal or semantic change is a breaking change to every
published ROM.  Because wgl fetches ROMs from the bucket, a wgl change that
reads a *new* field cannot be tested until classic-roms has published ROMs
that carry it (§6).

## 3. The DAG (`loaded.rs`)

`LoadedRoms::resolve(root, load)` (`:51`) and
`resolve_async(root, load)` (`:66`, web) walk `manifest.deps` depth-first
from the root:

- **deps before dependents.**  `order` is topological and the root is last
  (`root_rom()`).  Deps are visited in their declared order.
- **diamonds are deduplicated.**  `load` is called at most once per distinct
  name.
- **cycles are rejected**, with the error `ROM dependency cycle: a -> b -> a`.
  A self-dep is a cycle too.
- **a missing dep** fails with `load ROM dependency `<name>`` (sync path).

The module is pure graph logic: `load` is the caller's.  `classic-platform`
supplies it (a native file read or blocking `ureq`, or a web `fetch` with a
Cache-API cache keyed by the `roms.json` sha256).  A direct path/URL root is
loaded once and then resolved under its `entrypoint` (else `"root"`).

`LoadedRom` holds `name` (the resolver key), `namespace` (the **declared**
value, verbatim), `rom`, and `sha256`.  `resolve` always leaves `sha256`
`None`.  Native `resolve_roms` fills it afterwards from the archive bytes it
read (`classic-platform/src/rom.rs:171`), and `classic-demo/src/module_cache.rs`
uses it as the compiled-guest cache key (trusted ROMs only).  Web and the
legacy `Engine::load_rom` path leave it `None`.

## 4. Namespaces

### Effective namespace

`Engine::effective_namespace(entry, multi)` (private, `boot_api.rs:708`;
public view `Engine::rom_namespace(name)` `:695`) decides:

1. the declared `namespace` when it is non-empty;
2. otherwise, when the DAG has more than one ROM, the `entrypoint`, falling
   back to the resolver `name`;
3. otherwise `""` (global): a legacy single ROM keeps bare keys, so its
   golden traces do not change.

`begin_boot` computes it once per entry.  `BootStep::RegisterMetadata` and
`BootStep::HydrateEntry` set `Engine::namespace` to it before they run, so
`entity_key(name)` qualifies under the ROM being hydrated.

### Qualification

`Engine::qualify(ns, name)` (`:684`) = `name` when `ns` is empty **or `name`
already contains `::`**, else `"{ns}::{name}"`.  `entity_key_ns` (`:679`) is
the public wrapper and `entity_key` (`:671`) uses the current
`Engine::namespace`.  `namespace_of("a::b") = "a"`.

At hydration every ROM-owned key is qualified under its own namespace: entity
names (`load_state_in`, `hooks.rs:69`), texture names and their
`-depth`/`-normal` siblings, fonts, animations (name and `src`), models,
vehicles (and each part's `texture` and its `anchors`), data artifacts, and
frame tables.  For a frame table, the table key, every `sheets[].name` **and
every frame key** are qualified (`register_manifest_metadata`,
`boot_api.rs:350–362`).  So `lunar-common`'s container frame is keyed
`lunar-common::shippingContainerBody_56` inside its texture's table, which is
itself keyed by the qualified texture name.

### Resolution: the rule

`resolve_entity_name(referring_ns, name)` (`:1209`) and
`resolve_resource(referring_ns, kind, name)` (`:1247`) share one rule:

- **qualified `ns::name`**: returned **only if it exists** (`names` /
  `resource_exists` `:1229`), else `None`.
- **bare `name`**: try `"{referring_ns}::{name}"`, then the global `name`,
  else `None`.
- **dependencies are never searched.**  A scene that wants `lunar-common`'s
  `landing` model must say `lunar-common::landing`.  A bare `landing` in the
  `lunar` ROM resolves to `lunar::landing` or a global `landing`, or to
  nothing.

`ResourceKind` here is the **engine's** (`classic-engine/src/lib.rs:170`:
`Texture`, `Font`, `Animation`, `FrameTable`, `Vehicle`, `Model`), not
`classic_rom::ResourceKind`.  A texture exists if it is in `texture_names`
(registered at metadata time, so it exists before its upload) or already
uploaded to `gfx`.

### The boot rewrite passes

`hydrate_rom_entry` (`:606`) runs, per ROM, `load_state_in` →
`rewrite_cross_refs` → `rewrite_resource_refs` → `load_grids`, over **only
that ROM's** freshly inserted keys, with that ROM's namespace as the
referring namespace.  When resolution returns `None` the field is **left
untouched**: no error, and the draw path reports the dangling name.

`rewrite_cross_refs` (`:731`) resolves entity references with
`resolve_entity_name`:

- `NavMesh.map_entity`
- `IsoSprite.tilemap`, `IsoAgent.tilemap`, `Model.tilemap`
- `Light.parent`.  `gather_lights` looks the parent up verbatim, so if it is
  left bare the light is skipped every frame.
- `Animator.target`: only the entity segment before the first `.`
  (`rocket.Light` → `scene::rocket.Light`)
- `IsoVehicle.tilemap`, each `wheel_entities[i]`, each `tire_entities[i]`

`rewrite_resource_refs` (`:856`) resolves resource references with
`resolve_resource`:

- `Tilemap.tile_set`, `NavMesh.tile_set` (Texture)
- `IsoSprite.texture`, then **`qualify_sprite_frame_name`** (below)
- `IsoAgent.texture`, `SpriteRender.texture` (Texture)
- `SdfTextRender.atlas_name` (Font)
- `Animator.animation` (Animation)
- `Model.model` (Model)

Not rewritten, by design: `Model.clip`.  Each `.glb` carries one clip named
after the model, so a clip name is never namespaced.
`Engine::start_model_clip` (`classic-engine/src/model.rs:63`) sets
`clip = model.rsplit("::").next()`, the model's last `::` segment.  Also not
rewritten: grid names (`tiles_grid`, `heights_grid`, `data_grid`), which are
looked up per ROM in that ROM's own `ResourceSet`.

### `IsoSprite.frame_name` and #117

`qualify_sprite_frame_name` (`:1182`) is the one rewrite that does **not**
follow the referring-namespace rule.  The frame table belongs to the
texture, so a bare `frame_name` is qualified with the namespace of the
**already-resolved texture** (`namespace_of(texture)`), which can be a
dependency's.  The qualified name is adopted **only if that texture's frame
table contains it**.  A name that is already qualified, or unknown, is left
as is.  `resolve_frame` (`render.rs:300`) then looks the name up verbatim.

The cautionary tale: until #117 nothing qualified `frame_name`.
`scene.json` authors bare frame names and the tables key qualified ones, so
every packed-atlas sprite that nothing re-frames at runtime (no animator, no
vehicle sim, no guest `set_sprite_frame`) **silently resolved no frame**.
`basetest`'s containers drew nothing while still appearing in the trace.
The demo's `house`, `tree` and `semaphore01/02` fell back to their whole
standalone textures.  Nothing failed.  The demo and lunar goldens were
re-baselined over the broken render at `7e5764b` ("re-baseline goldens for
the namespaced scenes"), where the demo trace's `house` flips from
`demo_atlas` to the `demo::house` fallback, and `basetest` was retired on the
belief that its content was bad.  The lessons:

- a reference that fails to resolve degrades silently;
- a re-baseline is not a fix, so diff the trace lines you are accepting;
- **when you add a name-bearing component field, add it to the rewrite pass
  and to its test in the same change.**

### The guest SDK path

Guest strings never pass through the boot rewrite; they resolve at call time
in `classic-guest/src/sdk.rs` against the guest's namespace
(`set_namespace`, set from `rom_namespace` by `classic-demo`'s `init_guests`):

- `qualify(name)` (`:111`) is used to **spawn**: own-namespace prefix.
- `resolve(name)` (`:119`) is used to **look up** entities.
- `resolve_resource(kind, name)` (`:132`) looks up animation, model, vehicle
  and `has_resource` names.

Unlike the engine functions, the SDK versions **never return a miss**.  A
qualified name passes through verbatim *without* an existence check, and a
bare name that resolves nowhere falls back to `qualify(name)`, the guest's
own namespace (`resolve("missing") == "scene::missing"`).  The dependency
rule is the same: a guest that wants a dependency's resource must pass
`ns::name` (classic-roms #45 qualifies the lunar guest's `RocketClip`
strings for this reason).
`start_model_clip` (`:246`) resolves the model this way, then the engine
takes the clip from its last `::` segment.

## 5. Boot hydration

A resolved `LoadedRoms` goes to a `BootPipeline` (AGENTS.md *Patterns* 3,
`classic-engine/src/boot/`), driven by `run_sync` or `InterleavedBoot`.  Apps
never sequence these steps.  `Engine::begin_boot` (`boot_api.rs:143`) builds
the `BootPlan` once.  For each ROM in DAG order (deps first), the plan holds:

1. `RegisterMetadata`: texture names, depth/normal bookkeeping, animations
   and their channels, frame tables, vehicles, data (all qualified).
2. Texture `Decode` + `Upload`, one per unique `src`, with `AliasTexture` for
   entries that share a `src`.  Entries with a `format` (`.basis`) become
   `basis_jobs` instead.  Then the depth and normal maps.  SDF atlas textures
   are skipped here.
3. `LoadSdfFont`, then `LoadModel`.  Both run before hydration so that
   `Model.model` and `atlas_name` resolve in the rewrite pass.
4. `HydrateEntry`: state → `rewrite_cross_refs` → `rewrite_resource_refs`
   → grids (§4).  An unknown component type in `state.json` **panics** here
   (`.expect("load ROM state")`).

After all ROMs comes one `Finish` step (`finish_hydrate_roms`, `:628`).  It
records `loaded_roms` (which makes `rom_namespace` multi-aware), builds the
root's item catalog, mirrors the root's manifest and resources into the
single-ROM fields (`dump_rom`, F10 save), and **then** applies the root's
`vehicle_overrides`, after the dependency closure has registered the shared
`VehicleDef`.

The pipeline stages are `Uploading` (shader compile from the root manifest,
then the plan) → `UploadingBasis` (the `.basis` sheets, **after** every
`HydrateEntry`; their names were registered in step 1, so resolution does not
need the GL texture) → `Finishing` (the app's `BootFinish`) → `Done`.
`classic-demo`'s `DemoFinish` installs the Tier-3 worker (root `worker`
code), then one foreground guest per ROM that ships `main`, in DAG order, so a
dependent's `init` can see its deps' entities.

`dump_roms` (`:1059`) rebuilds the DAG.  Only the root's `state` and grids
are refreshed, and deps keep their boot-time resources.

## 6. Testing

Unit tests, all GL-free:

- **`classic-rom/tests/roundtrip.rs`**: `RomArchive` over all three
  containers (`zip_roundtrip`, `tar_gz_roundtrip`, `tar_zst_roundtrip`, the
  last against an embedded base64 fixture), `list_is_sorted_and_complete`,
  `unknown_format_is_rejected`.
- **`rom.rs` tests**: `pack_and_load_round_trips`,
  `pack_manifest_round_trips_verbatim`, `pack_emits_zstd_magic`, depth,
  normal and model round-trips, `pack_writes_a_shared_sheet_once`.
  **`manifest.rs` tests**: the defaulted fields.  **`resource.rs` tests**:
  `from_archive_*`.
- **`loaded.rs` DAG tests**: `single_rom_with_no_deps`,
  `resolves_linear_chain_in_topological_order`,
  `dedups_diamond_dependencies`, `records_declared_namespaces`,
  `rejects_direct_cycle`, `rejects_self_cycle`,
  `surfaces_missing_dependency_name`, and the async twins
  `resolve_async_matches_sync_topological_order` /
  `resolve_async_rejects_cycle`.
- **`classic-engine/src/lib.rs` tests** (`two_rom_dag()` `:960` is the
  `common` + `scene` fixture; `hydrate_roms` is the GL-free `load_roms`):
  `load_state_qualifies_entity_names_under_namespace`,
  `hydrate_roms_tracks_dag_in_topological_order`,
  `apply_vehicle_overrides_merges_root_tuning_into_shared_def`,
  `hydrate_roms_qualifies_light_parent` (`:1067`),
  `rewrite_resource_refs_qualifies_sprite_frame_name` (`:1108`: dependency,
  own, unknown and already-qualified frames),
  `resolve_entity_name_applies_namespace_rule`,
  `resolve_resource_applies_namespace_rule`,
  `rewrite_resource_refs_qualifies_resource_references`,
  `rewrite_cross_refs_qualifies_entity_references`.  A new rewritten field
  belongs in the last two.
- **`classic-guest/src/sdk.rs` tests**:
  `resolve_scopes_names_to_guest_namespace` (`:1115`),
  `resolve_resource_scopes_names_to_guest_namespace` (`:1142`).

Unit tests use hand-built manifests.  Only the goldens run the **published**
ROMs, and #117 shows that the goldens are only as good as the last
re-baseline.

**ROM-lock lockstep.**  The engine is tested against whatever is in
`roms/out/`, and `fetch-roms` always pulls the bucket's *latest*.  A change
to the ROM format or to its contents is therefore gated on published ROMs
(the bidirectional dependency in the `classic-engine` skill): publish the
roms layer first, then re-pin `tests/golden/roms.lock.json`
(`cargo xtask lock-roms`) and re-baseline the goldens in the same wgl change.

- **Never trust a `roms/out/` you did not fetch yourself.**  A stale or
  locally built set measures something other than CI does.  Before reading
  a golden, compare each `roms/out/*.rom` size (and sha256) against
  `tests/golden/roms.lock.json`.
- `cargo xtask check-roms` (the CI golden job) compares the published
  `roms.json` against the lock **for `demo`, `lunar` and `lrvtest` only**
  (`xtask/src/main.rs:239`).  The lock also pins `common` and `lunar-common`,
  but drift in those dependency ROMs is not caught there.  Check them by
  hand when a dependency changes.
- To test unpublished ROMs, point `CLASSIC_ROM_DIR` (or `CLASSIC_ROM=<path>`)
  at a classic-roms `roms/out/`, and never commit goldens or a lock from that
  run.

## 7. Gotchas

- **A missing reference is silent.**  Rewrites leave unresolved names as they
  are, and the SDK falls back to the own-namespace key.  Expect a missing
  sprite, light or model, not an error.
- **Dependencies are not searched.**  The only automatic cross-ROM step is
  `frame_name`, which follows its texture.
- **`format_version` is not a gate**, and a serde default is the only
  compatibility story (§2).
- **Root-only fields**: `shaders`, `host_features`, `items`,
  `inventory_types`, `vehicle_overrides`, and `worker` code.  Setting them on a
  dependency ROM does nothing.
- **Single-ROM boots stay global.**  Adding a first dependency to a ROM that
  declares no `namespace` moves all its keys from `x` to `<entrypoint>::x`,
  which changes every trace line.
- The engine's `ResourceKind` and `classic_rom::ResourceKind` are different
  enums with overlapping names.
