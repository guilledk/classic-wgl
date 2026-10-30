---
name: classic-graph
description: >
    The GPU graph UI element for large (x, y) series in classic-wgl.
    Covers the SeriesBuf sample ring + min/max pyramid (classic-core),
    the level VBOs with seam pads and the instanced polyline / envelope
    shaders (classic-gfx), the engine series store, view resolution
    (follow_tail, auto_y) and DrawKind::Graph (classic-engine), and
    UIManager::spawn_graph.
    Trigger phrases: "graph", "plot", "GraphRender", "SeriesBuf",
    "pyramid", "envelope", "polyline", "spawn_graph", "graph_push",
    "set_graph_data", "create_graph_series", "follow_tail", "auto_y",
    "graph_verts", "SeriesGpu", "draw_graph_series".
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
| Engine | `classic-engine/src/graph.rs` | series store, `resolve_view`, `DrawKind::Graph` |
| UI | `classic-engine/src/ui.rs` | `UIManager::spawn_graph` → `GraphElem { frame, plot }` |

## 1. Spawning and feeding a graph

```rust
use classic_core::components::{GraphSeriesRef, GraphStyle, GraphView, UiAnchor};
use classic_engine::ui::GraphOptions;

engine.create_graph_series("frame_ms", 8192);        // streaming ring (pow2)
// or: engine.set_graph_data("sine", &xs, &ys);      // static, cap = next pow2
let opts = GraphOptions {
    width: 360.0, height: 160.0, padding: 8.0,       // logical points
    background: [0.05, 0.05, 0.08, 0.85],
    series: vec![GraphSeriesRef {
        data: "frame_ms".into(),
        style: GraphStyle { color: [1.0, 0.6, 0.1, 1.0], line_width: 1.5, envelope_alpha: 0.35 },
    }],
    view: GraphView { x_min: 0.0, x_max: 600.0, auto_y: true, follow_tail: true, ..Default::default() },
    ..Default::default()
};
engine.game_ui();
let ui = engine.ui.as_mut().unwrap();                 // disjoint from engine.world
let g = ui.spawn_graph(&mut engine.world, opts);
ui.root_add_child(&mut engine.world, g.frame, UiAnchor::BotRight, UiAnchor::BotRight);

// every frame (a host on_update, never on_sim_update):
engine.graph_push("frame_ms", frame as f32, ms);
```

- `GraphElem.frame` is a plain `RectRender` container: anchor, attach,
  `set_enabled` it like any other.  `GraphElem.plot` is its `GraphRender`
  child, inset by `padding` (room for axes and labels), at `zlayer - 0.5` so
  it draws after the frame (the render list sorts by **descending** z).
- `Engine::graph_plot_rect(plot)` is the plot rect `(x, y, w, h)` in logical
  points as last laid out (interaction hit-tests against it).
- `Engine::graph_series(name)` reads a series; `remove_graph_series` drops
  it; `graph_vertex_count()` is the last frame's vertex total.
- x must be **non-decreasing** per series (`push` debug-asserts it).
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
  render-list build.
- `collect_graph_items` adds `(Transform.z, e, DrawKind::Graph)` for every
  enabled `GraphRender` (also drawn while the world is hidden: it is UI).
- `draw_graph` (UI phase, depth test off): plot rect = `Transform.xy` +
  `UiNode.size` ∩ a non-zero `UiNode.clip_rect`; sync the mirrors;
  `resolve_view` and **write the resolved view back** into
  `GraphRender.view`; trace; `scissor_logical(rect)`; one
  `draw_graph_series` per series; disable the scissor.
- `resolve_view(view, series, plot_w_px)` (pure, unit-tested):
  `follow_tail` slides the x window (same width) to end at the newest sample
  of any series; `auto_y` fits y to `SeriesBuf::y_range` in that window plus
  5% (a flat window gets a unit band).  No data → view unchanged.

## 5. Golden trace

A graph traces as one `"Graph"` item per plot: `model` = the plot rect,
`camera_ignored: true`, and a `graph` object `{ view: [x_min, x_max, y_min,
y_max], series: [colour, …] }`.  The field is absent on every other kind, and
no shipped scene spawns a graph, so the demo / lunar traces are unchanged.

## 6. Tests

- `classic-core/tests/graph.rs`: pyramid vs brute force, ring wrap,
  `select`, dirty ranges, ticks.
- `classic-gfx/src/graph.rs`: draw planning across the seam, seam pads,
  vertex counts, the plot transform.
- `classic-engine/src/graph.rs`: `resolve_view`, the series store.
- `classic-engine/src/graph_tests.rs` (headless EGL readback): polyline and
  envelope sine at ratio 1 and 2, plot-rect clipping, 1M points →
  `<= 4 · (plot_w + 2)` vertices, and a ring streamed past its wrap (GPU
  mirror built before the wrap) lit in every column.

## 7. Not yet (layer 3 and later)

Axes, grid and tick labels (`GraphAxes` is carried but not drawn yet),
legend, wheel zoom / drag pan, the live perf graph; LTTB decimation; a GPU
min/max pyramid; the guest API.
