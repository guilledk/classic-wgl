# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
See [`VERSIONING.md`](VERSIONING.md) for the release policy and process.

## [Unreleased]

### Added

- `classic_core::meshopt`: a pure-Rust `EXT_meshopt_compression`
  decoder (builds for wasm32): meshoptimizer's vertex codec (versions 0
  and 1), index codec (`TRIANGLES`, versions 0 and 1) and index sequence
  (`INDICES`), and the `OCTAHEDRAL` / `QUATERNION` / `EXPONENTIAL`
  filters.  Tested byte-for-byte against the C library (a native-only
  dev-dependency); the filters within one unit of its SIMD rounding
  (#151).
- Model atlas binding.  A glb material's `extras` may carry
  `"classic_atlas": "<ns>::<texture>"`: its albedo is that ROM catalog
  texture (the lossless PNG the ROM ships once), sampled with TEXCOORD_0
  like a `baseColorTexture`, so the glb needs no image of its own and a
  building ships its atlas once instead of in every clip glb.
  `ModelMesh.atlas` carries it (it wins over an embedded image), the team
  and emissive rects name regions of it, a bound mesh never takes the
  alpha-tested path, and the glb resolves the texture by the key when it
  loads (else its bare name, for a single-ROM boot).  Embedded images work
  as before (#152).
- `parse_model_glb` reads `KHR_mesh_quantization` and
  `EXT_meshopt_compression` models (both in `extensionsRequired`, which
  the `gltf` crate rejects): `classic_core::model_ext::normalize_glb`
  decodes every compressed buffer view into buffer 0 (dropping the
  fallback buffers) and strips both extensions before the `gltf` parse,
  and positions / normals are read by component type (normalized
  int8/int16 too; quantized normals renormalized), UVs and rotation keys
  through the crate's own normalized readers.  Positions dequantize
  through the mesh node's TRS, as the encoder folds them; the mesh is
  still uploaded as `f32` (#153).
- Vehicle definition v2 and sprite sheet v2 contracts
  (`classic_core::vehicle_def`), with no behaviour change yet.  A v2
  `VehicleDef` (`version: 2`) lists `layers` (`hull` / `wheel` /
  `gear_side` / `turret` / `tool` / `legs` / `cargo` / `lift`, each with
  its frame axes, a mixed-radix `{texture}_{index}` frame index, parent
  and offset), `sockets`, `px_per_m`, `ghost_group`, `depth_range_m` and
  `pivot_turn`, at 8, 16 or 24 directions; a v1 def keeps loading and
  maps onto layers with identical frame indices.  `frames.json` version
  2 adds per-sheet `depth_mode: "anchor"` + `depth_range_m`
  (anchor-relative depth), `team_mask` (team weight in the normal
  sheet's alpha) and per-table `dir_interp`; v1 tables read and write
  unchanged.  Boot logs contract violations (#154).
- Anchor-relative sprite depth (sheet v2): `gl_FragDepth = depth_base +
  (gray - 0.5) * depth_scale`, where a `depth_mode: "anchor"` sheet over
  `±R` m scales by `2R / (DEPTH_NEAR - DEPTH_FAR)` (legacy sheets keep
  1), carried per instance.  Boot fills an anchor sheet's missing range
  from its vehicle layer's (else the def's) `depth_range_m`, and v2
  layer textures are namespace-qualified like v1 parts.  8-bit depth is
  now ~2.4 cm a step at R = 3 m instead of ~1.7 m, which fixes the mule's
  near rail sorting against its container.  A `spawnEntities` test
  action loads hand-placed entities for review scenes (#156).
- Sprite direction interpolation (sheet v2 `dir_interp`): a part whose
  texture opts in draws its two baked directions warped to the
  continuous vehicle heading (`IsoSprite.dir_blend`, set by the vehicle
  system) with a two-neighbour backward warp in `sheet.frag#INTERP`:
  4 fixed-point steps per neighbour, accepted within 0.75 px, merged by
  angle where depths agree within 6 cm (else the nearer), normals
  rotated, warped `gl_FragDepth`, the normal alpha carried along.  The
  quad is the union of both directions plus 4 px; selected sprites and
  shadow casters draw the nearer direction.  The instance grows to 13
  `vec4`s (the model's unused z column dropped) and direction B binds on
  units 13-15 (#157).
- Vehicles bake 8, 16 or 24 directions: `IsoVehicle.directions` (from the
  def; serialized only when not 8) sizes the per-direction anchor and
  wheel-offset arrays (now `Vec`s) and drives the heading quantization
  and frame stride of non-interpolated sheets and the direction count of
  interpolated ones.  The LRV stays at 8 (#158).
- Layered vehicles (vehicle def v2): `spawn_vehicle` spawns a v2 def as
  one sprite per layer (`{vehicle}.{layer}`), driven by the v1 chassis
  sim over four contacts from the wheel / gear layers.  Layers sit at
  their (parent-composed) offsets on the chassis plane; wheels and tracks
  spin with the odometer and turn differential, wheels steer, turrets
  slew their own yaw over their own directions with gun pitch and fire
  frames, and tool / legs / cargo / lift layers play clips.  `pivot_turn`
  turns tracked vehicles in place; `ghost: false` layers skip the ghost
  pass.  New ABI rows `vehicle_aim`, `vehicle_aim_angles`,
  `vehicle_play`, `vehicle_layer_visible`, `vehicle_layer_color`; the
  contract gains turret `pitch_pivot` (#159).
- Sprite team colour and emissive strength.  `IsoSprite.team_tint`
  multiplies the team-masked texels of a `team_mask` sheet (weight
  `clamp((1 - a) · 2, 0, 1)` of the normal alpha), separately from
  `color`; `IsoSprite.emissive` scales the marker-normal pixels, and a
  sprite above 1 turns bloom on.  ABI rows `vehicle_team` and
  `vehicle_emissive` (every part of a vehicle); `spawnVehicles` takes
  `team` / `emissive` (#160).
- Vehicle sockets: `Engine::vehicle_socket_frame` resolves a def socket
  to a world frame that turns with the heading, body pitch / roll, the
  layer's composed yaw and (on a turret) the gun pitch about
  `pitch_pivot`.  A `Light` / `Emitter` with `parent: <vehicle>` and
  `node` (JSON alias `socket`) rides it, so vehicle lamps cast real light.
  ABI rows `vehicle_socket_pos` (world metres) and `vehicle_socket_fx`
  (a one-shot effect along the socket's forward axis) (#161).
- Model guest ABI.  ABI rows `model_node_pos` (a `Model` entity's glTF
  node world position at this frame's pose, `triple_opt`, world metres,
  +Z up), `model_tint` and `model_emissive` (`Model.tint` /
  `Model.emissive`, until now settable only in `state.json`) and
  `model_clip_time` (seconds, `-1` without a clip), so a guest can track
  a rocket's or crane's node and light or tint a model at runtime
  (#164).
- Sprite tilt for v2 units.  Tool / legs / cargo / lift layers may carry
  `pitch` / `roll` and ride the hull's drawn tilt (turned into their own
  yaw), so carried cargo leans with its carrier; a frames.json `axes`
  (`SpriteAxes`: directions × pitch × roll) lets a plain `IsoSprite` be
  posed by heading + tilt, ABI rows `sprite_pose` and `sprite_fit_terrain`
  (fit to the terrain under a footprint, resting on its lowest corner).
  Opt-in `tilt_interp` (per frame table, with `dir_interp`) makes tilt
  continuous: the direction warp also rotates each neighbour from its
  baked tilt level to the exact tilt (`tilt_residual`, about the ground
  anchor), carried in the instance model's w row; on tilted blends the
  warp search takes an adaptive Newton step and retries just outside the
  silhouette, and the quad is bounded by the warped content.  v2 layers
  are placed by the full body tilt, matching sockets.  A v2 hull's tilt
  dead zone now uses the finer of its pitch / roll half-steps (#165).
- Model team tint.  A glb material's `extras` may carry
  `"classic_team_rect": [u0, v0, u1, v1]`: a region of its
  `baseColorTexture` in TEXCOORD_0 space (origin top-left, `0..1`,
  `0 <= u0 < u1 <= 1`, `0 <= v0 < v1 <= 1`, half-open on the far edges)
  whose texels are team-coloured faces baked neutral (light grey/white
  times AO/wear).  `parse_model_glb` reads it into `ModelMesh.team_rect`
  (a malformed value fails the parse; the `gltf` crate now builds with its
  `extras` feature).  A `Model` with the new optional `tint: [r, g, b]`
  (serde default `None`, omitted from dumps) multiplies the albedo of
  fragments inside the rect by it in `mesh.frag`, before lighting.  The
  tint is in the same space as the texture's stored values (sRGB-encoded
  colours for sRGB textures, 0..1), not linear, like `IsoSprite.color`;
  the tint shows as the `Model` trace item's `color`.  Models without the
  extra or without a tint render and trace exactly as before (#132).
- `classic_core::model::parse_model_glb_with`: skips decoding images the
  caller already holds (by encoded-byte SHA-256,
  `ModelImage.content_hash`) (#132).
- `fixed_radius` on effect light layers (`LightLayerDef`, serde default
  `false`, omitted when `false`): the light's `radius` stays in absolute
  metres whatever the instance scale, while its `offset` still scales
  (`LightLayerDef::radius_at`) (#137).
- Model emissive regions.  A glb material's `extras` may carry
  `"classic_emissive": [{"rect": [u0, v0, u1, v1], "strength": s}, ...]`
  (at most 8; each rect under the `classic_team_rect` rules, `s` finite
  and `>= 0`; malformed fails the parse), read into
  `ModelMesh.emissive` (`classic_core::model::EMISSIVE_EXTRA`,
  `EmissiveRect`).  `mesh.frag` adds `albedo × strength` of the first
  rect containing a fragment's UV after lighting, unlit, so windows and
  lamps glow at night and in shadow.  A `Model` may scale every strength
  with the new optional `emissive` (serde default `None` = 1, omitted
  from dumps; `0` = off), and a drawn glowing model turns the bloom pass
  on.  `Gfx::draw_model` takes the rects.  Models without the extra
  render and trace exactly as before; no manifest or ABI change (#141).
- `classic_gfx::perf`, an opt-in frame profiler.  `CLASSIC_PERF=<file>`
  writes JSON lines (schema `perf: 1`): a header (GL renderer), one
  `frame` line per frame (wall, CPU, fence wait and per-phase CPU
  milliseconds, plus counters), one `gpu` line per frame with
  `CLASSIC_PERF_GPU=1` (a `TIME_ELAPSED` query per phase, read back a few
  frames late so nothing stalls), and a `summary` line at exit
  (mean/median/p95/p99/max).  Phases tile `Engine::frame` across the
  update, lifecycle, light grid, shadow, model, particle and bloom passes.
  Headless perf runs fence each frame and keep at most 2 in flight, so the
  GPU's back-pressure shows as `pace` instead of inside a random GL call.
  The web reads `?perf` / `?perf_gpu`, times the GPU through
  `EXT_disjoint_timer_query_webgl2` (dropping results across a disjoint
  event) and logs a console summary every 600 frames.  Off, each hook is
  one thread-local flag read (#145).
- Benchmark sections.  Three guest imports (`abi_manifest`, `native`
  `web` `worker`): `bench_mode() -> i32` (`0` interactive, `1` full, `2`
  short, from `CLASSIC_BENCH=full|short` or web `?bench[=short]`),
  `bench_section(name)` and `bench_end()`, which write `section` /
  `section_end` lines (with camera and viewport params) into the
  `CLASSIC_PERF` output and, in a bench mode, exit the app.  The web
  `EnvConfig` also reads `?fixed_dt=` and `?frames=N` (the loop stops
  after N frames).  `cargo xtask bench [--rom <path>] [--short] [--runs N]
  [--out <file>]` runs a ROM headless at 4K behind an idle-machine gate
  (warmup + N runs, plus one GPU-timer run; a run during which another
  process takes over half a core is killed and retried) and writes a
  per-section report (schema `classic-bench` v1: engine rev, ROM
  sha256, GL renderer, resolution, dt, mode; per section params, frames,
  wall/CPU/GPU mean/median/p95/p99/max, per-phase means, counters);
  `cargo xtask bench-compare a.json b.json` prints per-section deltas
  against a noise floor (#146).
- A lockstep test for model clips and guest timers: a WAT guest times
  itself while `Engine::frame` drives the demo's model system and guest
  closure for 150 fixed frames, swapping clips mid-run, and its timer
  must match `model_clip_time` every frame (`classic-demo`, test only)
  (#169).

### Changed

- Iso sprites draw as instanced batches (`classic_gfx::sprite_batch`):
  one 160-byte instance per sprite, and runs on one atlas page share a
  `drawElementsInstanced` in the normal, ghost and shadow passes
  (`plan_batches` only reorders sprites whose screen rects overlap
  nothing drawn in between, so the result matches one-by-one).  The
  ghost-group test moved into the shader: a ghost-id pass writes each
  sprite's group where it passes the normal pass's depth test and the
  ghost pass discards over its own group, so groups no longer split
  draws (the stencil per group stays as the fallback when the depth
  blit is unavailable).  200 mules in `unittest` at 4K: 4230 sprite
  draws -> 17.  New perf phases `sprite_plan` / `sprite_ids` /
  `sprite_ids_draw` / `sprite_ghosts` and counters `sprite_instances` /
  `sprite_batches` / `sprite_draws`; a `spawnVehicles` test action for
  stress scenes (#155).
- Model textures are shared by content.  An embedded glb image whose
  encoded bytes match an already loaded one is neither decoded nor
  uploaded again: its `"{model}::image::{i}"` name aliases the existing
  GL texture, so traces keep their per-model names.  A kit building
  (one glb per clip around one 2048² atlas) now holds one 16 MiB texture
  instead of one per clip (8 clips: 128 MiB -> 16 MiB) (#132).
- Dynamic lights are culled per screen tile.  Each frame the CPU bins
  every light's projected sphere into 32 px tiles of the main target
  (`classic_gfx::light_grid`, uploaded as one `R32UI` texture), and the
  lit fragment shaders (terrain, models, iso sprites) loop over their
  tile's list instead of every light; a tile with more than 64 lights,
  or a draw through another camera, runs the full loop.  The shared
  lighting block also skips a light outright beyond its radius, where its
  falloff is exactly 0.  The image is unchanged (lists keep light order,
  culled lights contribute 0).  On a Radeon 680M at 4K, the 93-light
  `buildtest` scene goes from 104 ms to 17 ms a frame at zoom 1 and from
  19 ms to 8 ms at the default zoom (#133).
- Particle bookkeeping scales with the effect count.  Instances are found
  by id through a hash index (rebuilt when instances retire) instead of a
  linear scan per emitter and burst; each layer draws only its ring's
  resident window (`ParticleRing::live_range`: the dead old end trimmed,
  one or two instanced draws with the attributes re-pointed on wrap)
  instead of all `capacity` slots; and a global soft budget
  (`PARTICLE_BUDGET`, 65,536 resident particles) scales continuous
  spawning by `budget / resident` above it.  Bursts are never throttled
  and spawning never depends on the camera, so fixed-timestep runs stay
  reproducible.  `Gfx::draw_particles` takes the slot ranges to draw
  (#138).
- The model pass keeps early depth testing.  `mesh.frag`'s alpha
  `discard` disabled early-Z for every model draw, so the sun PCF and the
  point-light loop ran on hidden fragments.  The cut now lives only in a
  `meshAlphaTest` variant (`mesh.frag#ALPHA_TEST`; the shader registry
  resolves `file#DEFINE` URLs), used by meshes whose material is not
  `OPAQUE` or whose UVs reach a texel with alpha below 0.01
  (`ModelMesh::alpha_test`, `AlphaMask`, `mark_alpha_test`); the rest
  draw without it, opaque instances front to back.  Traces keep their
  order and frames are pixel-identical.  `buildtest` at 4K on a Radeon
  680M: zoom 1 from 16.3 to 13.3 ms a frame (model pass GPU 7.9 to
  5.0 ms), zoom 0.15 from 7.2 to 6.7 ms.  `Gfx::draw_model` takes the
  `alpha_test` flag (#142).
- The sun shadow lookup uses hardware depth compare.  The lit shaders
  sampled the shadow map 9 times per fragment and compared by hand (3x3
  box PCF); the map is now a `sampler2DShadow` (`COMPARE_REF_TO_TEXTURE`,
  `LEQUAL`, `LINEAR`) read with 4 bilinear taps on the texel corners, a
  tent over the same 3x3 texels, in one shared, pinned block.  Draws
  without a shadow bind a 1x1 compare-mode placeholder (WebGL 2 requires
  one).  Shadow edges stay as soft; `buildtest` at 4K on a Radeon 680M:
  zoom 1 from 13.3 to 12.5 ms a frame, zoom 0.15 from 6.7 to 6.6 ms
  (#143).
- Bloom bright-passes straight from an offscreen main target.  It
  copied the full-resolution scene colour to a texture every bloom
  frame; with an offscreen main target (headless and golden runs) it now
  samples that target's texture through a `LINEAR` sampler object, and
  only the default framebuffer (a window, a canvas) is still copied
  (the copy target is created on first use).  Frames are pixel-identical;
  `buildtest` at 4K on a Radeon 680M: the 0.56 ms copy is gone, zoom 1
  from 12.5 to 12.2 ms a frame (#144).
- Model poses are sampled once per change.  Clips index their channels
  per node at parse (`ModelClip::node_channels`, `index_channels`), a
  pose samples into reused buffers
  (`ModelAsset::node_world_transforms_into` + `PoseScratch`), and
  `Engine::update_models` skips a model whose glb, load generation,
  clip, clip time and scale are unchanged (`Model.pose_key`, a
  transient `PoseKey`) or that is hidden with no `Emitter` parented to
  it.  Poses are bit-identical; 4K `buildtest` model sampling goes
  from 2.1-3.2 ms to 0.15-0.2 ms a frame.  `classic_gfx::perf::scope`
  splits `update_models` and `animator` out of the `update` phase
  (#147).
- The animator and node attachments stop rescanning per frame.  The
  demo animator looks animations up in place (no per-frame map of every
  animation), borrows each write's `AnimationData`, parses the target
  without allocating and replaces a sprite's `frame_name` only when it
  changes; `interpolate_offset` and channel sampling binary-search the
  sorted keys (`AnimChannel::sample_fixed` returns a fixed
  `ChannelValue`, `channel_sample` shares the lookup), and the
  `light.*` / `emitter.*` channels apply in one pass.  Node-attached
  emitters and lights look their glb nodes up through a per-(glb,
  name/prefix) index cache (`Engine::node_indices`), cleared when a
  model reloads.  Frames are pixel-identical; 4K `buildtest`: animator
  1.3 to 0.43 ms, emitter sync 0.59 to 0.37 ms a frame (#148).
- Model draws set per-pass state once.  `Gfx::begin_model_draws` /
  `draw_model_instance` / `end_model_draws` replace `Gfx::draw_model`:
  the camera, sun, shadow-map and light-grid uniforms (and the grid
  lookup) are set once per pass on both model programs and the depth
  test once per run, so an instance sets only its own uniforms and
  switches programs only between the opaque and alpha-tested groups;
  shadow casters likewise (`Gfx::begin_shadow_models`).  A shader
  caches uniform-location misses, model instances address their GPU
  mesh and texture through an engine slot store instead of formatting
  `"{model}::{mesh}"` keys per frame, emissive uniforms use fixed
  arrays, and the light UBO and light-grid texture rotate through 3
  copies (`FRAME_RING`).  `GlBuffer::delete` frees the HUD's per-frame
  debug buffers and replaced SDF glyph buffers, which leaked.  Frames
  are pixel-identical; 4K `buildtest` at zoom 0.15: CPU 4.1 to 2.8 ms
  a frame (#149).
- Lights are binned into the tiles their projected ball touches, not
  its whole bounding rect.  Under the orthographic iso camera a light's
  sphere projects to exactly a filled ellipse (`LightGridView::footprint`,
  `Footprint`); `Footprint::row_span` gives each tile row's touched
  columns in closed form, so the rect's corner tiles drop out of the
  lists.  Conservative as before (same 1 px margin), so frames are
  pixel-identical; on 4K `buildtest` list entries fall 18% and zoom 1
  GPU time from 11.7 to 11.3 ms a frame.  With `CLASSIC_PERF` the grid
  reports `grid_tiles`, `grid_lit_tiles`, `grid_entries`, `grid_max` and
  `grid_overflow` (#150).

### Fixed

- Fewer holes in the tilt warp.  A pixel neither direction neighbour
  covers now tries a third source: at a large tilt residual (past a
  quarter of a level step) the adjacent tilt level's frame of the nearer
  yaw neighbour, warped by its own residual (`vehicle_def::third_tilt`,
  `DirBlend.third`, a per-frame `blend_thirds` row texture).  It is
  searched only on holes near content.  Failing that, `best2` takes the
  opaque search end that reprojects within 2 px.  On the container
  sheet's eval poses, holes go from 2291 to 13 and IoU from 0.978 to
  0.990.  Sprite passes cost 36% more GPU time when every sprite triggers,
  paid only by triggered pixels (#168).
- No more dark band over a sprite's lower faces.  The sun-shadow receiver
  was the flat card through the ground anchor, whose lower part sits
  under sloped terrain and read the terrain's shadow.  Anchor-depth
  sprites now look shadows up at the surface point the pixel shows (slid
  along the view ray to the fragment's depth) when it is on the sun side
  of the card, else at the card point offset along the card normal; other
  sprites are unchanged, and sprite-on-sprite and model shadows still
  land (#167).
- Iso sprites draw at their true sheet height.  `compute_iso_sprite_model`
  ran the quad down world −Z at `1 / PPM_TARGET` m per pixel, which the
  30° camera foreshortened a second time (sheets are already camera
  images): every sprite drew at 0.866 of its height.  The quad now stands
  `1 / (PPM_TARGET · cos30)` m per pixel, so a sheet maps 1:1 to the
  screen and the shadow card is the object's true height (sun shadows
  ~15% longer).  Legacy global-depth sheets keep their old depth
  reference.  The demo, lighttest and lunar trace goldens change only in
  IsoSprite model heights (#166).
- `vehicle_socket_fx` orients the one-shot effect by the full socket
  frame (heading, pitch / roll, turret yaw, gun pitch) through the new
  `Engine::fx_emit_oriented`.  It used to go through `fx_emit`, whose
  frame is the world axes: only the particle velocity cone followed the
  socket, while ribbon `end`s, light `offset`s and spawn shapes stayed
  world-aligned, so the EMRLV's rail tracer always fired East.
  `fx_emit` itself is unchanged (#163).
- Layered (v2) vehicles: `vehicle_teleport` read the v1 wheel and anchor
  tables at the quantized direction, which a 16-direction v2 vehicle
  sized at 8, panicking past direction 7 (the mule's scripted reverse
  teleports every frame).  v2 spawns size every per-direction table to
  the def's count and the teleport reads them by their own length.
  Spot lights on a vehicle socket now turn their beam with the socket
  frame instead of keeping the authored world direction.  The demo's
  light-marker overlay moved from L to Shift+L, so a guest binding L no
  longer draws a yellow cross at every light (#162).
- Anchor-relative depth sheets re-anchor against the frame's own ground
  anchor depth instead of the quad's top-left corner (which sat
  `0.5 · anchor_y · height` metres nearer and skewed every layer by a
  different amount); legacy sheets keep the corner base (#157).
- A model no longer vanishes for one frame when a guest swaps it to a
  glb with a different node set.  `start_model_clip` runs after that
  frame's `update_models`, so the pose was still sized for the old glb:
  the draw skipped the model and node-attached lights, emitters and the
  camera pin resolved against the old nodes.  It now re-poses the entity
  at once through the new `Engine::resample_model` (clip time 0, with
  `frame_offset`), and `model_draws` falls back to the rest pose rather
  than skipping a pose that does not fit its glb (#134).
- Scenario `keyPress` steps reach the guest.  The test runner wrote the
  key into `engine.input` after every update closure had run, and the
  next `frame` replaced `input` with the platform snapshot, so no guest
  ever saw a scripted key.  The step now queues it on the new
  `Engine::scripted_input` (a one-shot pressed queue plus a held set,
  released by `pressed: false`), merged into `input` right after the
  platform snapshot: a press at frame N is seen by the update closures
  at N + 1.  Mouse steps are unchanged (#135).
- Hidden effects go with what hides them.  When an `Emitter`'s entity
  or its parent (a building's `Model`) is `Disabled`, its live particles
  now fade out over `HIDDEN_FADE_S` (0.25 s, multiplying the layer's
  alpha) and its rings are then cleared, its ribbons stop and its light
  layers are released at once; before, the smoke, sparks and burst lights
  of a hidden building played out their full lifetimes over empty ground.
  Showing it again resets the fade.  `Emitter.enabled = false` is
  unchanged: spawning stops and live particles die naturally.  New
  `ParticleRing::clear` (#136).
- Small particles no longer vanish when zoomed out.  A quad narrower
  than about a texel of the (half-res) particle target covered no pixel
  centre and dropped out of the image: ice crystals (0.04-0.07 m, ~0.2
  half-res texels at zoom 0.15) kept ~1/12 of their area-expected light.
  `particle.vert` now floors the quad width at `PARTICLE_MIN_TEXELS` (1.5
  target texels, from the new `u_target_px`) and scales its alpha by the
  area ratio, so a particle keeps its average light at any zoom
  (`particle_size_floor` is the CPU mirror) (#139).
- Particles past the map edge no longer draw over the void.  The primary
  tilemap's world-xy bounds reach `particle.vert` as `u_map`
  (`ParticleLayerUniforms::map`), and a particle fades out linearly over
  `MAP_FADE_M` (1 m) beyond them; without a tilemap nothing changes
  (`NO_MAP`).  A destroyed building's debris dust used to fall past the
  edge with no ground under it and drift over the black void (#140).

## [0.3.3] - 2026-10-17

### Added

- Opt-in terrain material blending, `Tilemap.blend` (serde default off, so
  every scene without it renders as before).  The tilemap shader warps the
  tile-id lookup with a deterministic low-frequency noise, fetches the 2x2
  ids around the half-tile-shifted lattice point and resolves their atlas
  cells: per-class bilinear membership gives material boundaries a smooth,
  marching-squares contour instead of the tile staircase, sharpened to a
  narrow `width` band; variants of one material resolve on smooth weights,
  hiding the tile grid.  With `height_strength > 0` the tileset alpha is
  per-texel height and decides transitions with a per-class `priority`; a
  class may be `hard`.  Classes come from the scene (`materials[]`), uploaded
  as a 256x1 lookup table.  Blended tilesets get a capped mip chain sampled
  at an explicit LOD, so zoomed-out terrain stops aliasing into moire;
  magnification stays `NEAREST`.  Flat single-id areas cost one atlas
  sample.  The nav overlay never blends.
- Analytic GPU particles (M1): CPU-spawned immutable 48-byte records in
  per-(effect instance, layer) ring buffers, uploaded as at most two
  `buffer_sub_data` ranges per frame and drawn with one instanced
  `particle.vert` call that evaluates ballistic + linear-drag motion, a
  turbulence wobble and a ground clamp/bounce/die in closed form at
  `u_time`.  Rendered as phase 3c (after the model ghost, before the UI),
  depth-tested against the scene with premultiplied blending; one
  `Particles` trace item per drawn layer.  Spawning is SplitMix64-keyed per
  instance/frame/layer, so a fixed timestep reproduces every particle (#126).
- Effect libraries and emitters (M2): `classic_core::effects` (a JSON
  `{"effects": [...]}` library shipped as a ROM `data[]` artifact and
  registered under the ROM namespace), a registered `Emitter` component
  (`effect`, `position`/`parent`/`node`, drive multipliers `rate`, `speed`,
  `scale`, `color`, `intensity`, a cumulative `burst` counter and
  `enabled`), an `Animator { target: "x.Emitter" }` arm sampling
  `emitter.*` channels, `Engine::fx_emit` one-shots, and lit particles
  sharing the byte-identical lighting block (`particle.vert` joins
  `lit_shaders_share_the_lighting_block`).  `Light` gains the same
  optional `node`: a light or emitter attached to a `Model` parent's glTF
  node follows its full transform (#126).
- Particle looks (M3, #126): velocity streaks, flipbook textures, procedural
  soft/smoke/glow shapes, the closed-form ground-vortex swirl (a radial
  ground blast captured into rising counter-rotating spirals), ground
  bounce and die with deterministic sub-effects on landing, procedural
  arc and lightning ribbons (midpoint displacement with branches,
  `ribbon.vert`/`ribbon.frag`) with strike flash lights, and companion
  light layers (flashes, flicker).  Glowing (additive or emissive) layers
  draw after every alpha layer, so a rocket jet stays visible through the
  dust it raises, and lit particles roll off their highlights instead of
  clipping to white under strong nearby lights.  Particle textures sample
  LINEAR through their own sampler object (each flipbook frame inset half a
  texel), while the same ROM texture stays NEAREST for sprites (#129).
- Bloom (M4): a soft-knee bright-pass, half- and quarter-resolution
  separable Gaussian blur and an additive composite on RGBA8 targets (no
  float render targets), run only on frames with something emissive on
  screen (#126).
- Particle fill cost (M4): particles draw into a half-resolution target
  (scene depth blitted in, premultiplied "over" composite back; full-res
  fallback only on multisampled or mismatched framebuffers), the smoke
  shape samples one baked tileable noise texture instead of per-pixel
  value noise, and procedural shapes skip their transparent quad corners
  (#126).
- `CLASSIC_GOLDEN_PNG` also takes a frame list (`10,30,60`): PNG captures
  of the offscreen target at those frames into `CLASSIC_GOLDEN_DIR`, for
  visual review of motion; `1` keeps the pixel golden.  A reference effect
  library test fixture (`classic-core` `tests/fixtures/effects.json`)
  covers every layer kind, spawn shape, ground and render mode (#126).
- A `camera` step for `CLASSIC_TEST` scenarios:
  `{"camera": {"follow": "rocket", "zoom": 0.3}}` follows a named entity
  every frame (an exact name, else the bare part of `ns::name`; emitters
  and lights frame their resolved attachment via
  `Engine::named_world_point`), `{"tile": [x, y]}` holds an iso tile and
  `{}` releases the pin.  With `CLASSIC_GOLDEN_PNG=<frames>` it frames
  headless review captures of ROM-authored effects (#129).
- Guest particle ABI (M5): `fx_emit(effect, x, y, z, dx, dy, dz, scale)`
  one-shots and the handle-based `emitter_spawn`/`emitter_set`/
  `emitter_release` (#127), mirroring the light pool (world metres, `+Z` up;
  `[native web worker]`, trapping in the Tier-3 worker).  A guest emitter is
  an ordinary `Emitter` entity in a 256-slot pool, so it attaches, traces and
  finishes its particles on release like a scene emitter.  The light pool's
  slot allocator moved to a shared `classic_engine::handles::EntityHandles`.
- Vehicle wheel dust and touchdown puffs (M10, #128): optional
  `VehicleDef`/`IsoVehicle` `dust_effect` and `touchdown_effect` (also
  settable per scene through `vehicle_overrides`).  Each grounded wheel gets
  a body-parented `Emitter` whose rate and launch speed follow ground speed,
  with bursts on every speed change, aimed up and back against the travel
  direction; landing from an airborne arc fires the touchdown effect, scaled
  by the impact speed.  Vehicles without the keys spawn nothing and dump
  unchanged.
- `rom:fxtest` names the particle test-bench ROM (desktop `rom_lookup`, web
  `ROM_URLS`), fetched by `fetch-roms` and pinned in `roms.lock.json` with
  the other published ROMs (#129, #130).
- Multi-node emitters: an `Emitter.node` ending in `*` (`"fx_nozzle_*"`)
  attaches the one effect instance to every node of the parent `Model`'s
  current glb whose name has that prefix, each node a spawn site with its
  own full TRS.  An effect describes one plume, so each node runs a
  layer's full `rate` and `burst`; births are dealt to the nodes
  round-robin (an even, deterministic split) into one shared ring per
  layer (`max` caps the whole cluster), and one `Animator` drives it all.
  Lights, ribbons and review framing use the nodes' centroid.  The set
  re-resolves every frame, following a clip swap to a glb with a
  different node set.  Exact names attach as before; a pattern matching
  nothing falls back to the parent's ground point, like a missed exact
  name.  `Light.node` stays single-node (#130).

### Fixed

- Guest light and emitter handles are generational (#127): a handle packs
  `(generation << 16) | slot` into a non-negative `i32`, and a release bumps
  the slot's generation.  A guest holding a handle past its release (or past
  a light's TTL) now gets `0` from `light_set`/`light_release`/`emitter_set`/
  `emitter_release` instead of driving whatever reused the slot.  The first
  handles minted are unchanged (`0, 1, 2, …`); `-1` stays the failure value.
- Tilemap map coordinates are `highp` end to end (varying, `map_size`) and
  the tile id is a `texelFetch` of the integer cell.  At `mediump` (fp16 on
  real mobile GPUs, and in Mesa's llvmpipe) the normalized map coordinate of a
  400-tile map resolves only ~0.2 tile near its far edge.  The demo frame
  moves on ~0.3% of pixels, along texel and tile edges, where the old
  lookup rounded.
- The web canvas is created with `antialias: false`, like the native
  window.  The browser default made it multisampled, which the half-res
  particle pass cannot blit depth from, so web particles always drew at
  full resolution: a zoomed-in rocket landing halved the frame rate.
  Terrain edges are now aliased exactly as on native (#130).
- Vehicle springs no longer blow up on long frames.  Their
  semi-implicit Euler steps are stable only below ~0.13 s; the web loop
  passed raw frame gaps (no 0.1 s cap, unlike native/headless), so a
  slow page took 0.7 s steps and the fx gallery's rover went `NaN` and
  panicked in the wheel travel clamp.  The springs now sub-step at
  0.05 s (one step, bit-identical, at or below it), every backend caps
  the frame delta through `classic_platform::frame_delta` (0.1 s), and
  the wheel clamp cannot panic (#130).

## [0.3.2] - 2026-10-07

### Added

- A `classic-rom` agent skill for the engine side of the ROM layer: the
  crate map, every `RomManifest` field with its serde default and reader, the
  `LoadedRoms` dependency DAG, the namespace resolution rule (a bare name
  resolves in its own namespace, then the global one, never in a
  dependency), the exact fields the boot rewrite passes qualify, the
  `frame_name` case behind the #117 fix, boot hydration order, and the tests
  plus the ROM-lock lockstep.  Authoring rules stay in classic-roms'
  `rom-authoring` skill (#123).
- A `lighttest` golden: classic-roms' new lighting-reference ROM joins the
  ROM tables (`fetch-roms`, desktop, web), with a `lighttest` scenario and a
  trace-only golden in CI.  It asserts three colours, each shown to fail
  with its subject removed: the red container's top (static packed sprites,
  #117), a spot pool on the ground (#118) and the rocket's sun shadow.  The
  `lunar` trace is re-baselined for the rocket moving into `lunar-common`
  (texture names only) (#119).

### Changed

- The `classic-testing` skill and the golden job's comments record what
  actually blocked the three unwired e2e scenarios, re-measured against a
  freshly fetched ROM set: `rocket` sampled a pad the lunar guest relocates
  to a runtime landing zone, `render_order` sampled alpha on sprite
  silhouette edges, and `container_ghost` targeted a scene that was never
  published.  They also record why the `basetest-lit` golden guarded
  neither lighting nor its containers: the nav-mesh overlay stayed drawn
  under `CLASSIC_NO_UI=1`, and statically framed sprites missed their
  namespaced frames.  The fixes are under *Fixed* and *Removed* (#113, #114).
- Every handler test in the workspace was swept for the `Cell::clone` trap (a
  cloned `Cell` is a detached copy, so an assertion on it can never fail).
  The one instance was already fixed in `e3cf553`; the sweep adds
  `click_fires_with_mouse_clicked`, the positive twin that gives
  `click_does_not_fire_without_mouse_clicked` something to fail against, and
  records the rule in the `classic-physics` skill (#122).

### Removed

- The `basetest` scene: its `basetest` and `basetest-lit` goldens, the
  `basetest-lit` CI step, its `roms.lock.json` entry and its
  `fetch-roms`/desktop/web ROM table entries.  Its two containers render
  no pixels against the current ROM set and its CI capture showed the
  nav-mesh overlay rather than the lit scene, so it guarded neither the
  sprites nor the lighting it was credited with (#115).
- The `container_ghost` e2e scenario.  It targeted the `container` entity
  of classic-roms' one-off `container` test scene, which was never published
  and is being deleted (classic-roms #44); its only runnable retarget, the
  retired `basetest`, passed vacuously (#116).

### Fixed

- The `render_order` and `rocket` e2e scenarios pass again and run in the
  CI golden job.  `render_order`'s alpha-only samples sat on sprite
  silhouette edges (alpha 0.76 under llvmpipe, 1.0 on hardware GL, and 1.0
  over bare terrain had the LRV vanished); it now asserts colour inside the
  chassis and the front-left fender.  `rocket` now frames the guest's
  runtime landing zone (317.5, 277.5) at frame 220 under
  `CLASSIC_FIXED_DT=0.05`, after touchdown, and asserts the base band and
  body panel colours.  Each assertion was checked to fail with its subject
  absent (#114).
- `CLASSIC_NO_UI=1` no longer paints the map in nav-tile colours.  The
  nav-mesh overlay is an editor view that only the editor's mode control
  ever hid, so skipping the editor left it drawn over the lit terrain on
  every ROM with a nav mesh; it now starts hidden when the editor is not
  installed (#115).
- Statically declared packed-atlas sprites render again.  Since ROM
  namespacing (`7e5764b`), frame tables key their frames under the owning
  ROM's namespace (`lunar-common::shippingContainerBody_56`) while
  `scene.json` authors bare frame names, and the hydration pass qualified
  `IsoSprite.texture` but never `frame_name`.  Every sprite nothing re-framed
  at runtime missed its frame: `basetest`'s containers drew nothing, and the
  demo's house, tree and semaphores fell back to their whole standalone
  textures.  Both goldens were re-baselined over it; the demo trace is
  re-baselined back to the atlas frames (#117).
- Spot lights light what they point at.  All three lit shaders computed the
  cone as `dot(L, dir)` with `L` the surface-to-light vector, so a spot only
  lit what sat *behind* it (a downward spot lit nothing on the ground), and
  softened its edge out to `acos(0.6 cos a)` -- ~56 deg for a 20 deg cone.
  `dir` is now the beam direction and the edge softens to `1.25x` the
  half-angle.  Found by the new `lighttest` ROM, the first scene to declare a
  spot (#118).
- A world-space circle collider is hit where it is drawn.  `project_shape`
  scaled a circle's diameter into screen space but dropped its position, so
  every `ColliderSpace::World` circle projected to screen origin.  The
  projection now carries a screen position beside the shape (#122).

## [0.3.1] - 2026-09-28

### Added

- `CLASSIC_TEST=<name>` selects an e2e scenario.  `build_test_scenario` took a
  `name` and discarded it, so `tests/scenarios/render_order`,
  `container_ghost` and `rocket` were unreachable and only `default` ever ran.
  Scenarios now come from a `SCENARIOS` table in `classic-demo/src/testing.rs`
  that `include_str!`s each file, an unknown name panics with the known list
  instead of silently falling back to `default`, and `1`/`true`/`all` stay
  aliases for `default`.  Two unit tests keep the table in lockstep with
  `tests/scenarios/` and parse every entry, which is the gate that was missing
  (#111).

### Fixed

- GJK no longer reports "no collision" for concentric shapes.  A zero
  centre-delta left the first search direction at `Vec3::ZERO`, and
  `Shape::support` normalizes `dir` for a circle, so the support point came
  back NaN, `dir.dot(diff) >= 0.0` was false, and two fully overlapping
  same-centre colliders read as disjoint.  `evolve_simplex` now falls back to a
  fixed axis when the centres coincide and terminates with `Intersection` when
  the 2-simplex perpendicular degenerates (the origin lying *on* segment CB);
  `Shape::support` uses `normalize_or_zero` so a zero direction can never
  produce a NaN support point at all (#109).
- Enter/Exit collision handlers fire again for `ColliderSpace::World`
  colliders.  `begin_frame` inserts a world collider into the *screen* quadtree
  by its projected rect, but `handle_for` handed back the un-projected *world*
  rect, so `perform_calls` built its candidate set from the wrong region of the
  tree.  `handle_for` now derives from `shape_of`, which already projects, so
  the two cannot drift apart again (#110).
- Repair the ROM-namespaced entity names in the `render_order` and `rocket`
  scenarios (`lunar-common::lrv`, `lunar-common::lrvWheelFl`,
  `lunar::rocket`).  They were authored before namespacing and never ran
  afterwards, so their assertions could not resolve an entity at all.  Both
  still fail on substance and remain out of CI; `classic-testing` section 9
  records each one's blocker (#111).

## [0.3.0] - 2026-09-27

### Added

- 3D glTF model rendering: a `Model` component (registered + dumpable, subsumes
  `Transform`) draws a node-parented `.glb` (`classic_core::model`: parse,
  LINEAR/STEP/CUBICSPLINE clip sampling, node world transforms, rig-origin
  translation) as real geometry through a new world-lit `mesh` shader
  (`Gfx::draw_model` / `draw_shadow_model`, `DrawKind::Model`) that writes true
  camera view depth and casts sun shadows.  ROMs declare models in `models[]`
  (`{name, src}`, `src` = `/models/<name>.glb`; `ResourceKind::Model` in
  `classic-rom`), loaded by a `BootStep::LoadModel`; `Model.model` and
  `Model.tilemap` resolve through the namespace rewrite passes.  The clip is
  played by the `start_model_clip` host import and advanced by
  `Engine::update_models`; a `Light` parented to a model tracks its animated rig
  origin, and `has_resource` kind 3 queries models (#100).
- `rocket_motion` gate test: parses the exported US Rocket glbs through
  `classic_core::model` (from `CLASSIC_ROCKET_GLB_DIR`, skipped when unset) and
  checks per-key |Δv|, touchdown speed, foot contact and the
  landing-end/launch-start pose, writing a per-frame z/v/tilt CSV (#100).
- `lit_shaders_share_the_lighting_block` pins the shared dynamic-light block
  across `sheet.frag`, `iso_tilemap.frag` and `mesh.frag` (#100).
- 3D models keep the sprite pixel look: they render into a `NEAREST` pixelation
  target sized to the sprite texel (`viewport × min(1, 1/zoom)`,
  `classic_gfx::model_target_size`) and a `modelComposite` pass writes them back
  with their depth (`gl_FragDepth`); a second composite ghosts a model at 40%
  behind sprites and terrain, like sprites, on a reserved stencil group
  (`MODEL_GHOST_GROUP` = 255; vehicle groups now cycle 1..254) (#100).
- `cargo xtask check-patterns`: a CI-gated guard for the codified architecture
  patterns (no raw thread/`Worker` spawns outside `classic-worker`, no
  hand-numbered `OP_*` tables), with a per-line `xtask-allow` opt-out (#98).
- `AGENTS.md` "Patterns": the eight rules this codebase keeps itself to, with
  `# Architecture` rustdoc notes on `classic-worker`, `classic-guest` and
  `classic-engine::boot`, and a refreshed `docs/architecture.d2`/`.svg` (#98).
- Declarative host-import ABI table (`classic_core::abi_manifest`): one entry
  per import records its typed params, return kind and backend set, with a
  runtime `HOST_IMPORTS` descriptor.  The wasmi/wasmtime linker layer and the
  Tier-3 worker surface are now generated from it, and new tests check that
  every table import links on each native backend (#98).
- Headless-browser test harness for the wasm-only guest backends
  (`wasm-bindgen-test` in Chromium, `crates/classic-guest/tests/web.rs`; the
  flake pins the matching `wasm-bindgen-cli` and provides Chromium +
  chromedriver) and a `web tests` CI job (#98).
- `GuestRuntime::is_ready()` readiness handshake: the untrusted web `Worker`
  runtime reports ready once its `Worker` has booted, and the demo defers a
  not-yet-ready guest's `init` to its first ready frame.  `trunk serve` now
  sends COOP/COEP (cross-origin isolation), so `SharedArrayBuffer` and that
  runtime are available in development; the GitHub Pages deploy stays
  non-isolated (#98).

### Changed

- `Light.radius` is world metres end to end: `gather_lights` no longer divides
  it by `PPM_TARGET` and the default is `3.125` m (was `200` px).  Lights
  authored in px (published ROMs before the metre re-author) render with a 64x
  radius until re-published (#100).
- `gather_lights` skips a `Light` while it or its parent is hidden (`Disabled`):
  a hidden rocket's burn light goes dark instead of lingering at its last
  animated values (#100).
- Dump every registered component through one generic
  `classic_core::registry::dump_as::<T>` instead of 15 hand-written dumpers;
  `ComponentReg::dump_value` adds the `"type"` key, so a `Dumper` now returns
  the component body only (#98).
- Collapse the app boot drivers onto `classic_engine::boot`'s `run_sync`
  (headless) and `InterleavedBoot` (per frame while the loader renders; the
  windowed desktop's `InterleavedBoot::threaded` owns the boot thread, caps
  hand-off and event forwarding), removing the desktop's `BootMsg` and
  `ChannelBootSink` and the web app's hand-rolled boot loop (#98).
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
  `classic_demo::{finish_init_engine, compile_guest_modules}` (#98).
- Fan boot-time texture decode and native `.basis` transcode out on a pooled
  `JobQueue` (`JobQueue::pooled` + ordered `run_all`, loader threads now named
  `classic-decode-N`/`classic-basis-N`) instead of a separate thread pool; the
  web basis transcode `Worker` moves into `classic_worker::transcoder_worker`
  (#98).
- Web workers exchange bytes as transferred buffers instead of structured
  clones: nav snapshots, guest/transcoder modules, task arguments and transcode
  inputs going in, and path, task and transcode results coming out.  The
  pathfinder result no longer clones the whole Worker wasm heap per search
  (#98).
- Run background work on a generic `classic_worker::JobQueue<J>` (threaded or
  synchronous): `PathfinderWorker` and `GuestWorker` share its thread, FIFO
  update, flush barrier and result map, and `Engine::set_synchronous_workers`
  is now the single determinism switch — the engine always submits to the
  pathfinder instead of branching into separate inline paths (synchronous
  vehicle searches still use a snapshot of the live world).
  `GuestLimits::synchronous_workers` and the `synchronous` argument of
  `Engine::install_guest_worker{,_compiled}` are removed (#98).
- Spawn every background thread and web `Worker` through shared helpers
  (`classic_worker::spawn_thread`, `classic_worker::spawn_web_worker`) instead
  of five copies of the thread setup and four copies of the Blob/URL/`onmessage`
  worker setup; native background threads are now named (#98).
- Generate the trusted browser `WebAssembly` guest backend's host imports from
  the ABI table, replacing ~1,500 hand-written closures and its hand-numbered
  `OP_*` dispatcher; imports with more than 8 wasm params are now one
  self-contained shim each instead of a shared global dispatcher (#98).
- Generate the untrusted Worker guest backend from the ABI table: the
  main-thread dispatch is a table-built registry and `worker.js` builds its
  import stubs from a descriptor, so both hand-numbered `OP_*` tables are gone.
  The guest module is posted to the Worker instead of being copied into its
  1 MiB `SharedArrayBuffer` (#98).
- Split the oversized source files into focused modules with no behaviour
  change: `classic-engine`'s `lib.rs` (`lifecycle`, `hooks`, `boot_api`,
  `render`) and `vehicle.rs` (`vehicle/`), `classic-gfx`'s `lib.rs`, and
  `classic-guest`'s `runtime_web.rs` (`runtime_web/`) (#98).

### Removed

- Unused `classic-gfx` dependency from `classic-platform` (#98).
- `classic_worker::ThreadPool`, superseded by `JobQueue::pooled` (#98).

### Fixed

- `Rom::pack` writes each archive path once: every frame-table texture of a
  packed atlas aliases the same sheet `src`, so the sheet was appended to the
  archive once per alias.  zstd hid it for small ETC1S sheets; with UASTC sheets
  `lunar-common.rom` packed to 16.5 MB instead of 4.95 MB (#100).
- Terrain and 3D models no longer wobble along screen X while zooming on native
  GPUs: `iso_tilemap.vert`, `mesh.vert`, `shadow_depth.vert` and
  `shadow_sprite.vert` declared `precision mediump float`, which Mesa radeonsi
  (and other GLES drivers) really evaluates as 16-bit float, so pixel-space
  positions in the tens of thousands snapped to 2-4 px steps.  They now default
  to `highp` (sprites and the web target already computed these in full
  precision) (#100).
- Container inventory tooltips show item icons again: a frame table loads with
  its key **and** frame names qualified by its ROM namespace (`lunar::icons` /
  `lunar::regolith`), but the tooltip looked up the bare `icons` sheet and bare
  item names, so every icon was silently skipped and only the counts rendered
  (#100).
- Namespace-qualify `Light.parent` in the ROM cross-reference pass: a parented
  light in a namespaced ROM (the lunar rocket burn light, basetest's
  `controlLight`) looked up its bare parent name, never found it, and was
  skipped every frame (#100).
- Web (trusted) guests: a failed background task no longer panics with a
  `RefCell` double borrow in `poll_task`, and a second guest runtime no longer
  re-points every earlier runtime's wide host imports at itself (#98).
- Web (untrusted Worker) guests: expose the 16 host imports the backend was
  missing (the field/kernel registry, `spawn_task`/`poll_task`,
  `vehicle_goto_poll`), stream host-import payloads of any size instead of
  failing above 6 KiB in / 64 KiB out, match the native return values (e.g.
  `set_camera`), and report guest traps and link errors as `GuestError::Trap`
  instead of timing out (#98).

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
