//! # Skill: `classic-graph`
//!
//! **Read `.agents/skills/classic-graph/SKILL.md` before working on this module.**
//!
//! The graph element's engine half: the named series store (a
//! [`SeriesBuf`] plus its lazily built GPU mirror per series), the per-frame
//! view resolution (`follow_tail`, `auto_y`), and the `DrawKind::Graph`
//! draw.  A `GraphRender` entity only *names* its series
//! (`GraphSeriesRef::data`); the samples live here, like an
//! `SdfTextRender`'s font lives in `Engine::sdf_fonts`.
//!
//! The plot rect is the entity's `Transform.position.xy` and `UiNode.size`
//! (logical points, top-left origin), clipped to `UiNode.clip_rect` when a
//! clipping parent set one.  Draws run in the UI phase (depth test off,
//! draw-order layering), scissored to the plot rect.

use std::collections::HashMap;

use classic_core::components::{GraphAxes, GraphRender, GraphView, Transform, UiNode};
use classic_core::graph::{nice_ticks, view_to_px, SeriesBuf};
use classic_gfx::{Gfx, GraphSeriesParams, SeriesGpu};
use glam::{Mat4, Vec3, Vec4};
use glow::HasContext;

use crate::golden::{self, GraphTrace, TraceCollector, TraceItemParams};
use crate::graph_ui::GraphUi;
pub use crate::graph_ui::{
    clamp_view_x, format_tick, format_value, pan_view, reset_view, zoom_view, GRAPH_CLICK_PRIORITY,
    LEGEND_REFRESH_FRAMES,
};
use crate::{DrawKind, Engine};

struct GraphSeries {
    buf: SeriesBuf,
    gpu: Option<SeriesGpu>,
}

/// Engine-owned graph series, keyed by name.
#[derive(Default)]
pub(crate) struct GraphStore {
    series: HashMap<String, GraphSeries>,
    /// Mirrors of replaced or removed series, freed at the next frame.
    retired: Vec<SeriesGpu>,
    /// Vertices the graph draws emitted this (or the last) frame.
    frame_verts: u64,
    /// Chrome + interaction of each `Engine::spawn_graph` graph, by plot.
    pub(crate) ui: HashMap<hecs::Entity, GraphUi>,
    /// Whether the `graph_ui` pre-update hook is registered.
    pub(crate) ui_hook: bool,
}

impl GraphStore {
    fn insert(&mut self, name: &str, buf: SeriesBuf) {
        if let Some(old) = self.series.insert(name.to_string(), GraphSeries { buf, gpu: None }) {
            self.retired.extend(old.gpu);
        }
    }
}

impl Engine {
    /// Create (or replace) the streaming series `name`: an empty ring of the
    /// newest `capacity` samples (rounded up to a power of two), fed by
    /// [`Engine::graph_push`].
    pub fn create_graph_series(&mut self, name: &str, capacity: usize) {
        self.graphs.insert(name, SeriesBuf::with_capacity(capacity));
    }

    /// Replace series `name` with exactly these samples (`x` non-decreasing;
    /// capacity = the next power of two `>= x.len()`).
    pub fn set_graph_data(&mut self, name: &str, x: &[f32], y: &[f32]) {
        self.graphs.insert(name, SeriesBuf::from_points(x, y));
    }

    /// Append a sample to series `name` (evicting the oldest when full);
    /// `false` when there is no such series.  `x` must not decrease.
    pub fn graph_push(&mut self, name: &str, x: f32, y: f32) -> bool {
        match self.graphs.series.get_mut(name) {
            Some(s) => {
                s.buf.push(x, y);
                true
            }
            None => false,
        }
    }

    /// The samples of series `name`.
    pub fn graph_series(&self, name: &str) -> Option<&SeriesBuf> {
        self.graphs.series.get(name).map(|s| &s.buf)
    }

    /// Drop series `name` (a graph naming it then draws nothing for it).
    pub fn remove_graph_series(&mut self, name: &str) -> bool {
        match self.graphs.series.remove(name) {
            Some(old) => {
                self.graphs.retired.extend(old.gpu);
                true
            }
            None => false,
        }
    }

    /// Vertices every graph drew in the last frame (also the
    /// `graph_verts` perf counter).
    pub fn graph_vertex_count(&self) -> u64 {
        self.graphs.frame_verts
    }

    /// The plot rect `(x, y, w, h)` of a `GraphRender` entity in logical
    /// points, as last laid out (interaction hit-tests against it).
    pub fn graph_plot_rect(&self, entity: hecs::Entity) -> Option<Vec4> {
        plot_rect(&self.world, entity)
    }

    /// Render-list collection for `DrawKind::Graph` (called from `frame`):
    /// frees retired mirrors and resets the frame's vertex count too.
    pub(crate) fn collect_graph_items(&mut self, items: &mut Vec<(f32, hecs::Entity, DrawKind)>) {
        self.graphs.frame_verts = 0;
        if let Some(gfx) = self.gfx.as_ref() {
            for gpu in self.graphs.retired.drain(..) {
                gpu.delete(&gfx.gl);
            }
        }
        for (e, (tf, _)) in self.world.query::<(&Transform, &GraphRender)>().iter() {
            if !self.is_disabled(e) {
                items.push((tf.position.z, e, DrawKind::Graph));
            }
        }
    }
}

/// The plot rect of `entity`: `Transform.position.xy` + `UiNode.size`,
/// intersected with a non-zero `UiNode.clip_rect`.  `None` when empty.
pub(crate) fn plot_rect(world: &hecs::World, entity: hecs::Entity) -> Option<Vec4> {
    let tf = world.get::<&Transform>(entity).ok()?;
    let node = world.get::<&UiNode>(entity).ok()?;
    let mut r = Vec4::new(tf.position.x, tf.position.y, node.size.x, node.size.y);
    if node.clip_rect != Vec4::ZERO {
        let c = node.clip_rect;
        let x0 = r.x.max(c.x);
        let y0 = r.y.max(c.y);
        let x1 = (r.x + r.z).min(c.x + c.z);
        let y1 = (r.y + r.w).min(c.y + c.w);
        r = Vec4::new(x0, y0, x1 - x0, y1 - y0);
    }
    (r.z >= 1.0 && r.w >= 1.0).then_some(r)
}

/// Grid lines at the view's nice ticks (`GraphAxes::ticks`), one physical
/// pixel wide and snapped to the pixel grid, under the series.
fn draw_grid(gfx: &Gfx, rect: Vec4, view: &GraphView, axes: &GraphAxes) {
    if !axes.show || axes.grid_color[3] <= 0.0 {
        return;
    }
    let ratio = gfx.pixel_ratio.max(f32::MIN_POSITIVE);
    let px = 1.0 / ratio;
    let snap = |v: f64| ((v as f32 * ratio).floor()) / ratio;
    let (x0, x1) = (rect.x as f64, (rect.x + rect.z) as f64);
    for x in nice_ticks(view.x_min, view.x_max, axes.ticks.max(1)) {
        let sx = snap(view_to_px(x, view.x_min, view.x_max, x0, x1));
        draw_ui_rect(gfx, Vec4::new(sx, rect.y, px, rect.w), &axes.grid_color);
    }
    let (y0, y1) = ((rect.y + rect.w) as f64, rect.y as f64);
    for y in nice_ticks(view.y_min, view.y_max, axes.ticks.max(1)) {
        let sy = snap(view_to_px(y, view.y_min, view.y_max, y0, y1));
        draw_ui_rect(gfx, Vec4::new(rect.x, sy, rect.z, px), &axes.grid_color);
    }
}

/// A screen-space solid rect `(x, y, w, h)` in logical points.
fn draw_ui_rect(gfx: &Gfx, r: Vec4, color: &[f32; 4]) {
    let model = Mat4::from_translation(Vec3::new(r.x, r.y, 0.0))
        * Mat4::from_scale(Vec3::new(r.z, r.w, 1.0));
    gfx.draw_rect(&model, &Mat4::IDENTITY, color, true);
}

/// The view a graph draws this frame (pure).  `follow_tail` slides the x
/// window (keeping its width) so its right edge is the newest sample of any
/// series (or, while the data is shorter than the window, its left edge the
/// oldest); `auto_y` fits `y` to what the series draw in that window
/// (`SeriesBuf::y_range`) plus a 5% margin, a flat signal getting a unit
/// band.  Without data the view is returned unchanged.
pub fn resolve_view(view: &GraphView, series: &[&SeriesBuf], plot_w_px: f32) -> GraphView {
    let mut v = *view;
    if v.follow_tail {
        let range = series
            .iter()
            .filter_map(|s| s.x_range())
            .map(|(a, b)| (a as f64, b as f64))
            .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)));
        if let Some((oldest, newest)) = range {
            let width = if v.x_max > v.x_min { v.x_max - v.x_min } else { 1.0 };
            // Until the data fills the window it grows from the left edge.
            v.x_min = (newest - width).max(oldest);
            v.x_max = v.x_min + width;
        }
    }
    if v.auto_y {
        let range = series
            .iter()
            .filter_map(|s| s.y_range(v.x_min, v.x_max, plot_w_px))
            .map(|(lo, hi)| (lo as f64, hi as f64))
            .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)));
        if let Some((lo, hi)) = range {
            let span = hi - lo;
            let pad = if span > 0.0 { 0.05 * span } else { 0.5 * lo.abs().max(1.0) };
            v.y_min = lo - pad;
            v.y_max = hi + pad;
        }
    }
    v
}

/// Draw one `GraphRender` entity (the `DrawKind::Graph` dispatch): sync its
/// series' GPU mirrors, resolve and write back the view, trace it, then
/// draw each series scissored to the plot rect.
pub(crate) fn draw_graph(
    gfx: &Gfx,
    world: &hecs::World,
    store: &mut GraphStore,
    entity: hecs::Entity,
    order: f32,
    name: &str,
    trace: Option<&mut TraceCollector>,
) {
    let Some(rect) = plot_rect(world, entity) else { return };
    let Ok(mut graph) = world.get::<&mut GraphRender>(entity) else { return };
    let z = world.get::<&Transform>(entity).map(|t| t.position.z).unwrap_or(order);
    let plot_w_px = rect.z * gfx.pixel_ratio;

    for sref in &graph.series {
        let Some(s) = store.series.get_mut(&sref.data) else { continue };
        if s.gpu.as_ref().is_some_and(|g| g.capacity() != s.buf.capacity()) {
            store.retired.extend(s.gpu.take());
        }
        let gpu = s.gpu.get_or_insert_with(|| SeriesGpu::new(&gfx.gl, &mut s.buf));
        gpu.sync(&gfx.gl, &mut s.buf);
    }
    let bufs: Vec<&SeriesBuf> =
        graph.series.iter().filter_map(|r| store.series.get(&r.data)).map(|s| &s.buf).collect();
    let view = resolve_view(&graph.view, &bufs, plot_w_px);
    graph.view = view;

    if let Some(t) = trace {
        let model = Mat4::from_translation(Vec3::new(rect.x, rect.y, z))
            * Mat4::from_scale(Vec3::new(rect.z, rect.w, 1.0));
        t.push_graph(
            TraceItemParams {
                order,
                kind: "Graph",
                name,
                model: &model,
                camera_ignored: true,
                texture: None,
                frame: None,
                color: None,
                depth: None,
                normal: None,
                screen: Some(golden::project_rect(&Mat4::IDENTITY, &model, true)),
            },
            GraphTrace {
                view: [view.x_min, view.x_max, view.y_min, view.y_max],
                series: graph.series.iter().map(|s| s.style.color).collect(),
            },
        );
    }

    gfx.scissor_logical(rect);
    draw_grid(gfx, rect, &view, &graph.axes);
    for sref in &graph.series {
        let Some(GraphSeries { buf, gpu: Some(gpu) }) = store.series.get_mut(&sref.data) else {
            continue;
        };
        let sel = buf.select(view.x_min, view.x_max, plot_w_px);
        let params = GraphSeriesParams {
            rect,
            view: [view.x_min, view.x_max, view.y_min, view.y_max],
            color: sref.style.color,
            line_width: sref.style.line_width,
            envelope_alpha: sref.style.envelope_alpha,
        };
        store.frame_verts += gfx.draw_graph_series(gpu, buf, &sel, &params);
    }
    unsafe { gfx.gl.disable(glow::SCISSOR_TEST) };
    // The legend's colour swatches (`Engine::spawn_graph`), unclipped.
    if let Some(ui) = store.ui.get(&entity) {
        for (r, color) in &ui.swatches {
            draw_ui_rect(gfx, *r, color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(n: usize, x0: f32) -> SeriesBuf {
        let x: Vec<f32> = (0..n).map(|i| x0 + i as f32).collect();
        let y: Vec<f32> = x.iter().map(|v| (v * 0.1).sin()).collect();
        SeriesBuf::from_points(&x, &y)
    }

    #[test]
    fn follow_tail_keeps_the_width_and_ends_at_the_newest_sample() {
        let a = line(100, 0.0);
        let b = line(50, 200.0);
        let view = GraphView {
            x_min: 0.0,
            x_max: 30.0,
            auto_y: false,
            follow_tail: true,
            ..Default::default()
        };
        let v = resolve_view(&view, &[&a, &b], 300.0);
        assert_eq!((v.x_min, v.x_max), (219.0, 249.0));
        assert_eq!((v.y_min, v.y_max), (view.y_min, view.y_max), "y untouched");
    }

    #[test]
    fn follow_tail_grows_from_the_oldest_sample_until_the_window_fills() {
        let a = line(10, 5.0);
        let view = GraphView { x_min: 0.0, x_max: 600.0, follow_tail: true, ..Default::default() };
        let v = resolve_view(&view, &[&a], 300.0);
        assert_eq!((v.x_min, v.x_max), (5.0, 605.0));
    }

    #[test]
    fn auto_y_fits_the_window_with_a_margin() {
        let x: Vec<f32> = (0..10).map(|i| i as f32).collect();
        let y: Vec<f32> = x.iter().map(|v| if *v < 5.0 { 0.0 } else { 10.0 }).collect();
        let s = SeriesBuf::from_points(&x, &y);
        let view = GraphView { x_min: 0.0, x_max: 9.0, auto_y: true, ..Default::default() };
        let v = resolve_view(&view, &[&s], 100.0);
        assert_eq!((v.y_min, v.y_max), (-0.5, 10.5));
        // Only the window counts.
        let view = GraphView { x_min: 0.0, x_max: 3.0, ..view };
        let v = resolve_view(&view, &[&s], 100.0);
        assert_eq!((v.y_min, v.y_max), (-0.5, 0.5), "a flat window gets a unit band");
    }

    #[test]
    fn no_data_leaves_the_view_alone() {
        let empty = SeriesBuf::with_capacity(16);
        let view = GraphView { follow_tail: true, auto_y: true, ..Default::default() };
        assert_eq!(resolve_view(&view, &[&empty], 100.0), view);
        assert_eq!(resolve_view(&view, &[], 100.0), view);
    }

    #[test]
    fn the_series_store_creates_feeds_and_replaces() {
        let mut e = Engine::new_for_test();
        assert!(!e.graph_push("fps", 0.0, 1.0), "no series yet");
        e.create_graph_series("fps", 100);
        assert_eq!(e.graph_series("fps").unwrap().capacity(), 128);
        for i in 0..200 {
            assert!(e.graph_push("fps", i as f32, 1.0));
        }
        let s = e.graph_series("fps").unwrap();
        assert_eq!((s.len(), s.first_abs()), (128, 72), "the ring keeps the newest");
        e.set_graph_data("fps", &[0.0, 1.0, 2.0], &[3.0, 4.0, 5.0]);
        assert_eq!(e.graph_series("fps").unwrap().len(), 3);
        assert!(e.remove_graph_series("fps"));
        assert!(e.graph_series("fps").is_none());
    }
}
