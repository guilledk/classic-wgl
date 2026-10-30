---
name: classic-graph
description: >
    The GPU graph UI element for large (x, y) series in classic-wgl.
    Covers the SeriesBuf sample ring + min/max pyramid (classic-core),
    the level VBOs with seam pads and the instanced polyline / envelope
    shaders (classic-gfx), the engine series store, view resolution
    (follow_tail, auto_y) and DrawKind::Graph (classic-engine),
    UIManager::spawn_graph, and Engine::spawn_graph's chrome (grid, tick
    labels, title, legend) and interaction (wheel zoom, drag pan,
    double-click reset), plus the live perf graph that feeds on it.
    Trigger phrases: "graph", "plot", "GraphRender", "SeriesBuf",
    "pyramid", "envelope", "polyline", "spawn_graph", "graph_push",
    "set_graph_data", "create_graph_series", "follow_tail", "auto_y",
    "graph_verts", "SeriesGpu", "draw_graph_series", "legend",
    "tick labels", "zoom_view", "pan_view", "perf graph", "Shift+KeyG".
---

# Classic Graph — GPU series plots

A graph is a UI element that draws one or more `(x, y)` series (1M+ points)
at interactive zoom/pan on GLES 3.0 / WebGL2 alone: no compute, SSBOs or
geometry shaders.  It costs `O(plot width)` vertices whatever the series
size, because the CPU keeps a **min/max pyramid** and the GPU draws only the
level whose bucket count matches the plot width.

| Layer | Where | What |
|---|---|---|
| Data | `classic-core/src/graph.rs` | `SeriesBuf` ring + pyramid, `select`, `y_range`, `take_dirty`, `nice_ticks`, `view_to_px` |
| Component | `classic-core/src/components/graph.rs` | `GraphRender { series, view, axes }`, registered as `"Graph"` |
| GPU | `classic-gfx/src/graph.rs` + `shaders/graph_*.{vert,frag}` | `SeriesGpu` level VBOs, `Gfx::draw_graph_series` |
| Engine | `classic-engine/src/graph.rs` | series store, `resolve_view`, `DrawKind::Graph` (+ grid, legend swatches) |
| Chrome | `classic-engine/src/graph_ui.rs` | `Engine::spawn_graph`: tick labels, title, legend, zoom / pan / reset |
| UI | `classic-engine/src/ui.rs` | `UIManager::spawn_graph` → `GraphElem { frame, plot }` (bare) |
| Consumer | `classic-demo/src/perf_graph.rs` | the `Shift+KeyG` live perf graph |

## 1. Spawning and feeding a graph

```rust
use classic_core::components::{GraphAxes, GraphSeriesRef, GraphStyle, GraphView, UiAnchor};
use classic_engine::ui::GraphOptions;

engine.create_graph_series("frame_ms", 8192);        // streaming ring (pow2)
// or: engine.set_graph_data("sine", &xs, &ys);      // static, cap = next pow2
let style = GraphStyle { color: [1.0, 0.6, 0.1, 1.0], line_width: 1.5, envelope_alpha: 0.35 };
let opts = GraphOptions {
    width: 460.0, height: 220.0,                     // logical points
    margins: Some([22.0, 124.0, 18.0, 40.0]),         // top (title), right (legend),
                                                     // bottom (x ticks), left (y ticks)
    series: vec![GraphSeriesRef { label: "frame".into(), ..GraphSeriesRef::new("frame_ms", style) }],
    view: GraphView { x_min: 0.0, x_max: 600.0, auto_y: true, follow_tail: true, ..Default::default() },
    axes: GraphAxes { ticks: 4, title: "frame ms".into(), ..Default::default() },
    interactive: true,                               // wheel zoom, drag pan, double-click reset
    legend: true,
    z: Some(-1550.0),                                // optional: draw order (lower = on top)
    ..Default::default()
};
let g = engine.spawn_graph(opts);                    // chrome + interaction
let ui = engine.ui.as_mut().unwrap();                 // disjoint from engine.world
ui.root_add_child(&mut engine.world, g.frame, UiAnchor::BotRight, UiAnchor::BotRight);

// every frame (a host hook, never on_sim_update):
engine.graph_push("frame_ms", frame as f32, ms);
```

- **Two spawns.**  `UIManager::spawn_graph(world, opts)` is the bare element:
  frame + plot (+ the grid, drawn by `draw_graph`).  `Engine::spawn_graph(opts)`
  builds on it and adds the tick labels, title and legend, and with
  `interactive` the zoom / pan / reset and a click-eating collider (see §5).
  `interactive` and `legend` mean nothing to the bare spawn.
- `GraphElem.frame` is a **padding container** (`UiKind::Padding` with
  `GraphOptions::insets()`: `margins`, else `padding` on every side) in the
  `background` colour: anchor, attach, `set_enabled` it like any other.
  `GraphElem.plot` is its `GraphRender` child at the insets' top-left, at
  `z - 0.5` (`z` = `GraphOptions::z`, else the UI layer) so it draws after
  the frame (the render list sorts by **descending** z); labels sit at
  `z - 0.75`.
- `GraphSeriesRef::new(data, style)`; `label` names it in the legend
  (`display_name()`: the label, else `data`).  Changing `label` (or a
  series list's labels) at runtime is fine: the legend reads it every frame.
- `Engine::graph_plot_rect(plot)` is the plot rect `(x, y, w, h)` in logical
  points as last laid out (interaction hit-tests against it).
- `Engine::graph_series(name)` reads a series; `remove_graph_series` drops
  it; `graph_vertex_count()` is the last frame's vertex total.
- x must be **non-decreasing** per series (`push` debug-asserts it): to
  restart x, recreate the series (`create_graph_series` replaces it).
- Series data is f32; the view is f64.  Keep x small (frame index, seconds
  since start) rather than epoch timestamps: f32 has 24 bits of mantissa.

## 2. Data layout (classic-core)

- **Level 0** is the raw ring: `cap` (a power of two) slots of `[x, y]`,
  sample `abs` (its absolute index, counting forever) in slot
  `abs & (cap - 1)`.
- **Level k** (`1..=log2(cap)`) holds `Bucket { x0, x1, y_min, y_max }`
  (`#[repr(C)]`, 16 B) of `2^k` aligned samples in
  **`slots(k) = (2 * cap) >> k`** slots, not `cap >> k`: an unaligned live
  window touches `(cap >> k) + 1` buckets (the partly evicted oldest one and
  the partly filled newest one), so the oldest needs its own slot.  Size GPU
  buffers from `slots(k) * slot_stride(k)` / `level_bytes(k)`, never from
  `cap >> k`.
- A bucket holds its **surviving** samples only: a push rebuilds the new and
  the evicted bucket per level (O(log cap)), so an evicted extreme never
  lingers.
- `select(view_x0, view_x1, plot_w_px)` → `Selection { level, mode, start,
  count, ranges }`: level 0 `Polyline` below 2 points per pixel, else the
  first level with `<= plot_w` buckets as an `Envelope`; `ranges` is at most
  two `(first_slot, count)` (the ring wraps).  The engine passes the plot
  width in **physical** pixels.
- `take_dirty()` → per level, 1–4 slot ranges changed since the last call.

## 3. GPU (classic-gfx)

- `SeriesGpu` (one per series, built lazily on first draw): a VAO (instance
  divisors live there, never on the shared `Gfx` VAO) and one VBO per level,
  **uploaded whole on first use**, then only its dirty slots
  (`SeriesGpu::sync` → `buffer_sub_data`).  Never-drawn levels cost nothing.
- **Seam pads**: a level VBO is `[slot last][slots…][slot 0][slot 1]`
  (`PAD_LEAD = 1`, `PAD_TRAIL = 2`), re-uploaded whenever the level changes.
  An instance reads its neighbours at consecutive offsets, so the instance
  that straddles the wrap seam reads continuous memory: no gap, joins intact.
- `plan_range_draws(sel)` splits a selection into ≤ 2 instanced draws; an
  instance joins entry `j` to `j + 1` (`count - 1` per selection; the first
  draw's last instance crosses the seam into the trailing pad).
- **Polyline** (`graphLine`, `graph_line.{vert,frag}`): one instance per
  segment, attributes `a_p0..a_p3` = the same VBO at offsets −1..+2 samples
  (previous, start, end, next).  Nine vertices (`TRIANGLES`): the segment
  quad plus a bevel triangle at the end.  Joins mitre while
  `dot(d_in, d_out) >= 2 / 4² − 1` (a miter limit of 4 half-widths, decided
  from the two directions alone so both segments agree); sharper joins fall
  back to a bevel (the triangle, otherwise collapsed to a point).  The line
  ends get straight extensions (`u_first` / `u_last`).  Coverage: signed
  distance across the line, `fwidth` AA, a one-pixel fringe.
- **Envelope** (`graphEnvelope`, `graph_envelope.{vert,frag}`): one instance
  per pair of consecutive buckets, a 4-vertex `TRIANGLE_STRIP` trapezoid
  between the bucket centres, widened by the line half-width (so a flat
  signal still reads as a line) plus the AA fringe.  A band no taller than
  the line draws at full alpha; one well past it fades to
  `style.envelope_alpha`, so the zoom-out switch from polyline to envelope
  does not flash.
- Never `gl.LINES`.  Everything is in **physical** pixels inside the shader:
  `px = u_offset + (p − u_origin) · u_scale` (`plot_transform`), with the
  logical `line_width` × `pixel_ratio`; a line under one pixel draws one
  pixel wide at reduced alpha.
- `Gfx::draw_graph_series` returns its vertex count and bumps the
  `graph_verts` perf counter (`classic_gfx::perf::count`).
- ESSL 3.00 / WebGL rules apply (no `?:` on structs); both programs are in
  `builtin_shaders()`, so the web `builtin_shaders_compile_under_webgl` test
  covers them.


## 4. Engine (classic-engine)

- `Engine.graphs` (`GraphStore`): name → `SeriesBuf` + `Option<SeriesGpu>`.
  Replacing (`create_graph_series` / `set_graph_data`) or removing a series
  retires its mirror; retired mirrors are freed at the next frame's
  render-list build.  It also holds the chrome state of every
  `Engine::spawn_graph` graph (`ui`, by plot entity).
- `collect_graph_items` adds `(Transform.z, e, DrawKind::Graph)` for every
  enabled `GraphRender` (also drawn while the world is hidden: it is UI).
- `draw_graph` (UI phase, depth test off): plot rect = `Transform.xy` +
  `UiNode.size` ∩ a non-zero `UiNode.clip_rect`; sync the mirrors;
  `resolve_view` and **write the resolved view back** into
  `GraphRender.view`; trace; `scissor_logical(rect)`; the **grid** (one
  physical pixel wide, snapped, at `nice_ticks(view, axes.ticks)` in
  `axes.grid_color`, when `axes.show`); one `draw_graph_series` per series;
  disable the scissor; the legend's colour swatches (unclipped).
- `resolve_view(view, series, plot_w_px)` (pure, unit-tested):
  `follow_tail` slides the x window (same width) to end at the newest sample
  of any series, **or**, while the data is shorter than the window, to start
  at the oldest (a live graph fills from the left, then scrolls); `auto_y`
  fits y to `SeriesBuf::y_range` in that window plus 5% (a flat window gets
  a unit band).  No data → view unchanged.

## 5. Chrome and interaction (`graph_ui.rs`)

All of it runs in **one engine pre-update hook**, registered by the first
`Engine::spawn_graph` (no `lifecycle.rs` change): it runs while the
simulation is paused (never `on_sim_update`), and before the `on_update`
closures, so the camera never sees a wheel the graph took.  Per graph and
frame:

1. **Visibility.**  The labels are bare `Transform` + `SdfTextRender`
   entities (no `UiNode`: the layout never moves them, a text change never
   dirties the layout, and the draw applies their justify).  So they do not
   inherit the frame's `Disabled`: the hook mirrors `is_disabled(plot)` onto
   them (and onto the collider) when it changes.  A despawned plot frees its
   labels and collider.
2. **Interaction** (`interactive`):
   - **Wheel** over the frame is consumed (`input.mouse_wheel = 0`, the
     `hud::route_text_scroll` pattern); over the plot it zooms by
     `0.5^wheel` (a native notch = 2x) about the cursor: `zoom_view` keeps
     the data point under the cursor fixed, x always, y unless `auto_y`.
   - **Press** on the plot starts a drag; `pan_view` moves the data with the
     pointer (x always, y unless `auto_y`) while button 0 is down.
   - **Double-click** (two presses ≤ 0.4 s of frame time and ≤ 6 pt apart)
     → `reset_view`: `follow_tail` + `auto_y`, the spawn-time x width.
   - Zoom and pan clear `follow_tail`; `clamp_view_x` keeps the x window on
     the data (inside it, or covering it when wider), at most
     `max(data span, home width)` wide.
   - **Clicks**: the frame gets a plain screen collider (no UI hover tint)
     that consumes clicks (`ui_consumed_click`, so the world, the RTS
     selection and the guests never see them) at
     `GRAPH_CLICK_PRIORITY` = 90: over game UI and the inspector's world
     backdrop, under its panels (100).  Its click handler only sets a flag;
     the hook reads it, so a press the inspector took never pans.  The
     collider rect is synced from the frame each frame (one frame behind a
     layout move).
3. **View**: `resolve_view` once more and write it back, so the labels
   match what `draw_graph` draws (it resolves again; feed series in a hook
   registered *before* the graph, i.e. before the first `spawn_graph`, and
   the two agree exactly).
4. **Labels**: x ticks centred under the plot, y ticks right-aligned left of
   it (vertically centred on the line, from the font's digit height), the
   title centred in the top margin, the legend in the right margin (one
   `LEGEND_ROW_H` = 15 pt row per series: swatch, name, value
   right-aligned).  Texts are compared before being written, so a glyph
   rebuild only happens on a real change; legend values refresh every
   `LEGEND_REFRESH_FRAMES` = 10 frames.  `format_tick(v, step)` prints as
   many decimals as the step needs (no `-0`); `format_value` about three
   significant digits (`123`, `16.7`, `0.46`).

The pure pieces (`zoom_view`, `pan_view`, `clamp_view_x`, `reset_view`,
`format_tick`, `format_value`) are re-exported from `classic_engine::graph`.

## 6. The live perf graph (`classic-demo/src/perf_graph.rs`)

The first consumer: `Shift+KeyG` (a bare `G` is left to guests).  See
`classic-debugging` §11 for what it shows; how it is built:

- Installed with the dev HUD (`host_features && !no_ui`) as **one pre-update
  hook**, after the inspector.  It spawns nothing until the first open, so a
  run that never presses the key traces exactly as before.
- On open: (re)create the series (`perf.wall`, `perf.cpu`, `perf.gpu`,
  `perf.phase0..5`, 8192 each, so x restarts at 0), `perf::start_live`
  (GPU timing on), and spawn the graph once (`Engine::spawn_graph`,
  interactive + legend, z -1550: over the HUD, under the inspector) in a
  transparent padding **holder** anchored bottom-right; later opens just
  re-enable the holder.  On close: `perf::stop_live`, disable the holder.
- Each frame: push `perf::last_frame()` (the previous frame, once per `f`;
  x = `f - f0`) and, when its `f` is new, `perf::last_gpu()` at its own
  frame's x.  The holder's right inset grows past the inspector's detail
  panel (`inspector::compute_layout`) while the inspector is open.
- Phase slots: a decaying sum per phase (`× 0.99` a frame), re-ranked at 60
  frames and every 120 after; `assign_slots` keeps a surviving phase in its
  slot (series and colour) and gives newcomers the freed slots on fresh
  rings; the series `label` carries the phase name into the legend.

## 7. Golden trace

A graph traces as one `"Graph"` item per plot: `model` = the plot rect,
`camera_ignored: true`, and a `graph` object `{ view: [x_min, x_max, y_min,
y_max], series: [colour, …] }`.  The field is absent on every other kind.
The labels trace as ordinary `"SdfText"` items; the grid and swatches are
part of the graph's draw (untraced).  No shipped scene spawns a graph and
the perf graph spawns on its key only, so the demo / lunar traces are
unchanged.

## 8. Tests

- `classic-core/tests/graph.rs`: pyramid vs brute force, ring wrap,
  `select`, dirty ranges, ticks.
- `classic-gfx/src/graph.rs`: draw planning across the seam, seam pads,
  vertex counts, the plot transform.
- `classic-engine/src/graph.rs`: `resolve_view` (incl. the fill-from-left
  tail), the series store.
- `classic-engine/src/graph_ui.rs`: zoom keeps the point under the cursor,
  pan + clamp, reset, label formatting, and the hook end to end on
  `Engine::new_for_test` (follow + labels, wheel zoom + consumption, drag
  + double-click, hide / despawn).
- `classic-engine/src/graph_tests.rs` (headless EGL readback): polyline and
  envelope sine at ratio 1 and 2, plot-rect clipping, 1M points →
  `<= 4 · (plot_w + 2)` vertices, and a ring streamed past its wrap (GPU
  mirror built before the wrap) lit in every column.
- `classic-demo/src/perf_graph.rs`: `Shift+G` opens (and a bare `G` does
  not), feeds, ranks and closes; slot assignment.
- Captures: open it from a scenario with `keyPress ShiftLeft` + `keyPress
  KeyG` in one step (native), or `key-down Shift ShiftLeft` / `key G KeyG
  2` / `key-up Shift ShiftLeft` (web script).

## 9. Not yet

LTTB decimation; a GPU min/max pyramid; the guest API; hover read-out (a
cursor line with values); per-axis zoom modifiers.
