//! # Skill: `classic-graph`
//!
//! **Read `.agents/skills/classic-graph/SKILL.md` before working on this module.**
//!
//! The graph element's chrome and interaction, added by
//! [`Engine::spawn_graph`] on top of the bare `UIManager::spawn_graph`
//! element: tick labels, the title and a legend as pooled SDF text, and
//! (opt-in, [`GraphOptions::interactive`]) wheel zoom anchored at the
//! cursor, drag pan and double-click reset.
//!
//! Everything runs in one engine pre-update hook ([`update`], installed with
//! the first such graph), never in `on_sim_update`, so a graph keeps
//! updating while the simulation is paused.  It runs before the `on_update`
//! closures, so a wheel over the graph is consumed before the camera zoom
//! sees it (the `route_text_scroll` pattern); a click on the frame is taken
//! by a plain screen collider that consumes it (`ui_consumed_click`), so it
//! never reaches the world.
//!
//! The labels are bare `Transform` + `SdfTextRender` entities (no `UiNode`):
//! the UI layout never moves them, a text change does not dirty the layout,
//! and the draw applies their justify.  The hook places them every frame
//! (follow-tail ticks slide) and rewrites a text only when it changes; the
//! legend values refresh every [`LEGEND_REFRESH_FRAMES`] frames.

use std::cell::Cell;
use std::rc::Rc;

use classic_core::collision::{polygon_from_verts, HandlerKind};
use classic_core::components::{
    ColliderData, Disabled, GraphRender, GraphView, SdfTextRender, TextJustify, Transform, UiNode,
    DEFAULT_SDF_FONT,
};
use classic_core::graph::{nice_ticks, px_to_view, view_to_px, SeriesBuf};
use glam::{Vec2, Vec3, Vec4};

use crate::graph::{plot_rect, resolve_view};
use crate::ui::{GraphElem, GraphOptions};
use crate::Engine;

/// Click priority of an interactive graph's frame: over game UI and the
/// inspector's world backdrop, under the inspector's panels.
pub const GRAPH_CLICK_PRIORITY: i32 = 90;
/// Frames between legend value rewrites.
pub const LEGEND_REFRESH_FRAMES: u32 = 10;
/// Two presses closer than this (seconds of frame time, and points) reset
/// the view.
const DOUBLE_CLICK_S: f64 = 0.4;
const DOUBLE_CLICK_PT: f32 = 6.0;
/// One wheel notch (native `1.0`) zooms by this factor.
const WHEEL_ZOOM: f64 = 0.5;
const TICK_SCALE: f32 = 0.34;
const LEGEND_SCALE: f32 = 0.36;
const TITLE_SCALE: f32 = 0.42;
const LEGEND_ROW_H: f32 = 15.0;
const SWATCH: f32 = 9.0;
const LABEL_COLOR: [f32; 4] = [0.78, 0.8, 0.86, 1.0];
const TITLE_COLOR: [f32; 4] = [1.0, 0.6, 0.3, 1.0];
/// Text height per unit of SDF scale when the font is not loaded yet.
const FALLBACK_EM: f32 = 36.0;

/// One graph's chrome and interaction state (`GraphStore::ui`, keyed by the
/// plot entity).
pub(crate) struct GraphUi {
    frame: hecs::Entity,
    interactive: bool,
    legend: bool,
    collider: Option<u32>,
    /// Set by the frame collider's click handler.
    hit: Rc<Cell<bool>>,
    shown: bool,
    /// The x window width a double-click restores.
    home_width: f64,
    drag: Option<Vec2>,
    last_press: Option<(f64, Vec2)>,
    /// Frame time accumulated by the hook (double-click timing).
    clock: f64,
    x_labels: Vec<hecs::Entity>,
    y_labels: Vec<hecs::Entity>,
    title: Option<hecs::Entity>,
    legend_names: Vec<hecs::Entity>,
    legend_values: Vec<hecs::Entity>,
    legend_age: u32,
    /// Legend colour swatches `(rect, colour)` for `draw_graph`.
    pub(crate) swatches: Vec<(Vec4, [f32; 4])>,
}

impl GraphUi {
    fn labels(&self) -> impl Iterator<Item = hecs::Entity> + '_ {
        self.x_labels
            .iter()
            .chain(&self.y_labels)
            .chain(&self.title)
            .chain(&self.legend_names)
            .chain(&self.legend_values)
            .copied()
    }
}

impl Engine {
    /// Spawn a complete graph element on the game UI: the bare element
    /// (`UIManager::spawn_graph`: frame, plot, grid) plus its tick labels
    /// and title (`GraphAxes`), a legend (`GraphOptions::legend`) and, with
    /// `GraphOptions::interactive`, wheel zoom / drag pan / double-click
    /// reset.  Attach `frame` to a parent (`root_add_child`) like any UI
    /// element; `set_enabled(frame, ..)` shows / hides all of it.
    pub fn spawn_graph(&mut self, opts: GraphOptions) -> GraphElem {
        let interactive = opts.interactive;
        let legend = opts.legend;
        let axes = opts.axes.show;
        let ticks = opts.axes.ticks.max(1) as usize + 2;
        let title = !opts.axes.title.is_empty();
        let series = opts.series.len();
        let home_width = opts.view.x_max - opts.view.x_min;
        self.game_ui();
        let ui = self.ui.as_mut().expect("game_ui created the manager");
        let elem = ui.spawn_graph(&mut self.world, opts);
        let z = self.world.get::<&Transform>(elem.plot).map_or(0.0, |t| t.position.z) - 0.25;

        let world = &mut self.world;
        let mut text = |n: usize, scale: f32, color: [f32; 4], justify: TextJustify| {
            (0..n).map(|_| spawn_label(world, z, scale, color, justify)).collect::<Vec<_>>()
        };
        let (x_n, y_n) = if axes { (ticks, ticks) } else { (0, 0) };
        let x_labels = text(x_n, TICK_SCALE, LABEL_COLOR, TextJustify::Center);
        let y_labels = text(y_n, TICK_SCALE, LABEL_COLOR, TextJustify::Right);
        let title =
            text(usize::from(axes && title), TITLE_SCALE, TITLE_COLOR, TextJustify::Center).pop();
        let l_n = if legend { series } else { 0 };
        let legend_names = text(l_n, LEGEND_SCALE, LABEL_COLOR, TextJustify::Left);
        let legend_values = text(l_n, LEGEND_SCALE, LABEL_COLOR, TextJustify::Right);

        let hit = Rc::new(Cell::new(false));
        let collider = interactive.then(|| {
            let shape = polygon_from_verts(vec![
                Vec3::ZERO,
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(1.0, 1.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
            ]);
            let pid = self.physics.register_collider(ColliderData::new(shape));
            self.physics.set_collider_consumes_click(pid, true);
            self.physics.set_collider_click_priority(pid, GRAPH_CLICK_PRIORITY);
            let h = hit.clone();
            self.physics.add_collider_handler(pid, HandlerKind::Click, move || {
                h.set(true);
                true
            });
            pid
        });
        self.graphs.ui.insert(
            elem.plot,
            GraphUi {
                frame: elem.frame,
                interactive,
                legend,
                collider,
                hit,
                shown: true,
                home_width: if home_width > 0.0 { home_width } else { 1.0 },
                drag: None,
                last_press: None,
                clock: 0.0,
                x_labels,
                y_labels,
                title,
                legend_names,
                legend_values,
                legend_age: u32::MAX,
                swatches: Vec::new(),
            },
        );
        if !self.graphs.ui_hook {
            self.graphs.ui_hook = true;
            self.on_pre_update(update);
        }
        elem
    }
}

fn spawn_label(
    world: &mut hecs::World,
    z: f32,
    scale: f32,
    color: [f32; 4],
    justify: TextJustify,
) -> hecs::Entity {
    world.spawn((
        Transform::new(Vec3::new(0.0, 0.0, z), Vec3::new(scale, scale, 1.0)),
        SdfTextRender {
            atlas_name: DEFAULT_SDF_FONT.into(),
            color,
            outline_color: [0.0, 0.0, 0.0, 0.0],
            outline_width: 0.0,
            ignore_cam: true,
            text: String::new(),
            justify,
            weight: 0.0,
            gamma: 1.0,
        },
    ))
}

/// The pre-update hook: every [`Engine::spawn_graph`] graph's visibility,
/// interaction and labels.
fn update(engine: &mut Engine) {
    let plots: Vec<hecs::Entity> = engine.graphs.ui.keys().copied().collect();
    for plot in plots {
        let Some(mut g) = engine.graphs.ui.remove(&plot) else { continue };
        if !engine.world.contains(plot) {
            release(engine, g);
            continue;
        }
        step(engine, plot, &mut g);
        engine.graphs.ui.insert(plot, g);
    }
}

/// Free a despawned graph's collider and labels.
fn release(engine: &mut Engine, g: GraphUi) {
    if let Some(pid) = g.collider {
        engine.physics.unregister_collider(pid);
    }
    for e in g.labels() {
        let _ = engine.world.despawn(e);
    }
}

fn step(engine: &mut Engine, plot: hecs::Entity, g: &mut GraphUi) {
    let shown = !engine.is_disabled(plot);
    if shown != g.shown {
        g.shown = shown;
        for e in g.labels().collect::<Vec<_>>() {
            if shown {
                let _ = engine.world.remove_one::<Disabled>(e);
            } else {
                let _ = engine.world.insert_one(e, Disabled);
            }
        }
        if let Some(pid) = g.collider {
            engine.physics.set_collider_enabled(pid, shown);
        }
        g.drag = None;
        g.hit.set(false);
        g.legend_age = u32::MAX;
    }
    if !shown {
        return;
    }
    let (Some(rect), Some(frame)) = (plot_rect(&engine.world, plot), ui_rect(engine, g.frame))
    else {
        return;
    };
    g.clock += f64::from(engine.time.delta);
    if let Some(pid) = g.collider {
        engine.physics.sync_collider_rect(pid, frame.x, frame.y, frame.z, frame.w);
    }

    let Ok(graph) = engine.world.get::<&GraphRender>(plot).map(|g| (*g).clone()) else { return };
    let mut view = graph.view;
    if g.interactive {
        view = interact(engine, g, &graph, rect, frame);
    }
    let bufs: Vec<&SeriesBuf> =
        graph.series.iter().filter_map(|s| engine.graph_series(&s.data)).collect();
    let view = resolve_view(&view, &bufs, rect.z * engine.pixel_ratio());
    if let Ok(mut gr) = engine.world.get::<&mut GraphRender>(plot) {
        gr.view = view;
    }
    place_labels(engine, g, &graph, &view, rect, frame);
}

/// Wheel, press and drag over an interactive graph; returns the new view.
fn interact(
    engine: &mut Engine,
    g: &mut GraphUi,
    graph: &GraphRender,
    rect: Vec4,
    frame: Vec4,
) -> GraphView {
    let mut view = graph.view;
    let mouse = engine.input.mouse_pos;
    let data = data_x_range(engine, graph);
    let wheel = engine.input.mouse_wheel;
    if wheel.abs() > 0.01 && contains(frame, mouse) {
        engine.input.mouse_wheel = 0.0;
        if contains(rect, mouse) {
            let factor = WHEEL_ZOOM.powf(f64::from(wheel));
            view = zoom_view(&view, rect, mouse, factor);
            if let Some(d) = data {
                view = clamp_view_x(&view, d, g.home_width);
            }
        }
    }
    if g.hit.replace(false) && contains(rect, mouse) {
        let double = g.last_press.is_some_and(|(t, p)| {
            g.clock - t <= DOUBLE_CLICK_S && (p - mouse).length() <= DOUBLE_CLICK_PT
        });
        if double {
            g.last_press = None;
            g.drag = None;
            view = reset_view(&view, g.home_width);
        } else {
            g.last_press = Some((g.clock, mouse));
            g.drag = Some(mouse);
        }
    }
    if let Some(last) = g.drag {
        if engine.input.is_mouse_down(0) {
            let d = mouse - last;
            if d != Vec2::ZERO {
                view = pan_view(&view, rect, d);
                if let Some(d) = data {
                    view = clamp_view_x(&view, d, g.home_width);
                }
            }
            g.drag = Some(mouse);
        } else {
            g.drag = None;
        }
    }
    view
}

fn place_labels(
    engine: &mut Engine,
    g: &mut GraphUi,
    graph: &GraphRender,
    view: &GraphView,
    rect: Vec4,
    frame: Vec4,
) {
    let em = em(engine);
    let target = graph.axes.ticks.max(1);
    // x ticks: centred under the plot.
    let xs = nice_ticks(view.x_min, view.x_max, target);
    let x_step = tick_step(&xs);
    let y_top = rect.y + rect.w + 3.0;
    for (i, &e) in g.x_labels.iter().enumerate() {
        let (text, at) = match xs.get(i) {
            Some(&x) => {
                let sx =
                    view_to_px(x, view.x_min, view.x_max, rect.x as f64, (rect.x + rect.z) as f64);
                (format_tick(x, x_step), Vec2::new(sx as f32, y_top))
            }
            None => (String::new(), Vec2::ZERO),
        };
        set_label(&mut engine.world, e, &text, at);
    }
    // y ticks: right-aligned left of the plot, centred on the tick.
    let ys = nice_ticks(view.y_min, view.y_max, target);
    let y_step = tick_step(&ys);
    let h = em * TICK_SCALE;
    for (i, &e) in g.y_labels.iter().enumerate() {
        let (text, at) = match ys.get(i) {
            Some(&y) => {
                let sy =
                    view_to_px(y, view.y_min, view.y_max, (rect.y + rect.w) as f64, rect.y as f64);
                (format_tick(y, y_step), Vec2::new(rect.x - 4.0, sy as f32 - 0.5 * h))
            }
            None => (String::new(), Vec2::ZERO),
        };
        set_label(&mut engine.world, e, &text, at);
    }
    if let Some(e) = g.title {
        let th = em * TITLE_SCALE;
        let at = Vec2::new(rect.x + 0.5 * rect.z, frame.y + (rect.y - frame.y - th).max(0.0) * 0.5);
        set_label(&mut engine.world, e, &graph.axes.title, at);
    }

    // Legend: a column right of the plot.
    if !g.legend {
        return;
    }
    let x0 = rect.x + rect.z + 10.0;
    let x1 = frame.x + frame.z - 6.0;
    let lh = em * LEGEND_SCALE;
    g.swatches.clear();
    g.legend_age = g.legend_age.saturating_add(1);
    let refresh = g.legend_age > LEGEND_REFRESH_FRAMES;
    if refresh {
        g.legend_age = 0;
    }
    for (i, s) in graph.series.iter().enumerate() {
        let (Some(&name), Some(&value)) = (g.legend_names.get(i), g.legend_values.get(i)) else {
            break;
        };
        let y = rect.y + i as f32 * LEGEND_ROW_H;
        let mid = y + 0.5 * LEGEND_ROW_H;
        g.swatches.push((Vec4::new(x0, mid - 0.5 * SWATCH, SWATCH, SWATCH), s.style.color));
        set_label(
            &mut engine.world,
            name,
            s.display_name(),
            Vec2::new(x0 + SWATCH + 5.0, mid - 0.5 * lh),
        );
        let at = Vec2::new(x1, mid - 0.5 * lh);
        if refresh {
            let last = engine.graph_series(&s.data).and_then(last_y);
            let text = last.map(format_value).unwrap_or_default();
            set_label(&mut engine.world, value, &text, at);
        } else {
            set_label_pos(&mut engine.world, value, at);
        }
    }
}

/// Text height per unit of SDF scale (the font's digit height).
fn em(engine: &Engine) -> f32 {
    engine.sdf_fonts.get(DEFAULT_SDF_FONT).map_or(FALLBACK_EM, |font| {
        classic_core::sdf_builder::build_sdf_glyph_buffer(font, "0", 1.0, TextJustify::Left, 0.0)
            .text_height
    })
}

fn set_label(world: &mut hecs::World, e: hecs::Entity, text: &str, at: Vec2) {
    if let Ok(mut sdf) = world.get::<&mut SdfTextRender>(e) {
        if sdf.text != text {
            text.clone_into(&mut sdf.text);
        }
    }
    set_label_pos(world, e, at);
}

fn set_label_pos(world: &mut hecs::World, e: hecs::Entity, at: Vec2) {
    if let Ok(mut tf) = world.get::<&mut Transform>(e) {
        tf.position.x = at.x.round();
        tf.position.y = at.y.round();
    }
}

fn ui_rect(engine: &Engine, e: hecs::Entity) -> Option<Vec4> {
    let tf = engine.world.get::<&Transform>(e).ok()?;
    let node = engine.world.get::<&UiNode>(e).ok()?;
    Some(Vec4::new(tf.position.x, tf.position.y, node.size.x, node.size.y))
}

fn contains(r: Vec4, p: Vec2) -> bool {
    p.x >= r.x && p.x <= r.x + r.z && p.y >= r.y && p.y <= r.y + r.w
}

fn data_x_range(engine: &Engine, graph: &GraphRender) -> Option<(f64, f64)> {
    graph
        .series
        .iter()
        .filter_map(|s| engine.graph_series(&s.data)?.x_range())
        .map(|(a, b)| (f64::from(a), f64::from(b)))
        .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))
}

fn last_y(s: &SeriesBuf) -> Option<f64> {
    (!s.is_empty()).then(|| f64::from(s.sample(s.len() - 1)[1]))
}

fn tick_step(ticks: &[f64]) -> f64 {
    match ticks {
        [a, b, ..] => b - a,
        _ => 1.0,
    }
}

/// Zoom `view` by `factor` (`< 1` zooms in) about the data point under
/// `cursor` (logical points) in the plot `rect`, so that point stays under
/// the cursor: x always, y unless `auto_y` fits it.  Clears `follow_tail`.
pub fn zoom_view(view: &GraphView, rect: Vec4, cursor: Vec2, factor: f64) -> GraphView {
    let mut v = *view;
    v.follow_tail = false;
    let (x0, x1) = (rect.x as f64, (rect.x + rect.z) as f64);
    let xc = px_to_view(cursor.x as f64, v.x_min, v.x_max, x0, x1);
    v.x_min = xc - (xc - v.x_min) * factor;
    v.x_max = xc + (v.x_max - xc) * factor;
    if !v.auto_y {
        let (y0, y1) = ((rect.y + rect.w) as f64, rect.y as f64);
        let yc = px_to_view(cursor.y as f64, v.y_min, v.y_max, y0, y1);
        v.y_min = yc - (yc - v.y_min) * factor;
        v.y_max = yc + (v.y_max - yc) * factor;
    }
    v
}

/// Pan `view` so the data follows a pointer drag of `delta` points across
/// the plot `rect`: x always, y unless `auto_y`.  Clears `follow_tail`.
pub fn pan_view(view: &GraphView, rect: Vec4, delta: Vec2) -> GraphView {
    let mut v = *view;
    v.follow_tail = false;
    let dx = -(delta.x as f64) * (v.x_max - v.x_min) / (rect.z as f64).max(1.0);
    v.x_min += dx;
    v.x_max += dx;
    if !v.auto_y {
        let dy = (delta.y as f64) * (v.y_max - v.y_min) / (rect.w as f64).max(1.0);
        v.y_min += dy;
        v.y_max += dy;
    }
    v
}

/// Keep the x window on the data `(lo, hi)`: its width between a minimum
/// and `max(hi - lo, home_width)`, and inside the data (or, when wider than
/// the data, covering all of it).
pub fn clamp_view_x(view: &GraphView, data: (f64, f64), home_width: f64) -> GraphView {
    let mut v = *view;
    let (lo, hi) = data;
    let span = (hi - lo).max(0.0);
    let max_w = span.max(home_width).max(f64::MIN_POSITIVE);
    let w = (v.x_max - v.x_min).clamp(max_w * 1e-4, max_w);
    let (a, b) = if w >= span { (hi - w, lo) } else { (lo, hi - w) };
    v.x_min = v.x_min.clamp(a, b);
    v.x_max = v.x_min + w;
    v
}

/// The double-click reset: follow the tail with a `home_width` window and
/// fit y again.
pub fn reset_view(view: &GraphView, home_width: f64) -> GraphView {
    GraphView {
        x_min: view.x_max - home_width,
        x_max: view.x_max,
        follow_tail: true,
        auto_y: true,
        ..*view
    }
}

/// A tick label: as many decimals as the tick `step` needs.
pub fn format_tick(v: f64, step: f64) -> String {
    let decimals = if step > 0.0 && step.is_finite() {
        (-step.log10() - 1e-6).ceil().max(0.0) as usize
    } else {
        0
    };
    let s = format!("{v:.decimals$}");
    if s.trim_start_matches('-').chars().all(|c| c == '0' || c == '.') {
        s.trim_start_matches('-').to_string()
    } else {
        s
    }
}

/// A legend value: three significant digits or so (`123`, `16.7`, `0.46`).
pub fn format_value(v: f64) -> String {
    let a = v.abs();
    if !v.is_finite() {
        "-".into()
    } else if a >= 100.0 {
        format!("{v:.0}")
    } else if a >= 10.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.2}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECT: Vec4 = Vec4::new(100.0, 50.0, 400.0, 200.0);

    fn view() -> GraphView {
        GraphView {
            x_min: 0.0,
            x_max: 100.0,
            y_min: -1.0,
            y_max: 1.0,
            auto_y: false,
            follow_tail: true,
        }
    }

    fn under(v: &GraphView, p: Vec2) -> (f64, f64) {
        let x = px_to_view(p.x as f64, v.x_min, v.x_max, RECT.x as f64, (RECT.x + RECT.z) as f64);
        let y = px_to_view(p.y as f64, v.y_min, v.y_max, (RECT.y + RECT.w) as f64, RECT.y as f64);
        (x, y)
    }

    #[test]
    fn zoom_keeps_the_point_under_the_cursor() {
        let v = view();
        let cursor = Vec2::new(180.0, 90.0);
        let before = under(&v, cursor);
        for factor in [0.5, 0.8, 1.25, 2.0] {
            let z = zoom_view(&v, RECT, cursor, factor);
            let after = under(&z, cursor);
            assert!((after.0 - before.0).abs() < 1e-9, "x {factor}: {before:?} -> {after:?}");
            assert!((after.1 - before.1).abs() < 1e-9, "y {factor}");
            assert!(((z.x_max - z.x_min) - 100.0 * factor).abs() < 1e-9);
            assert!(!z.follow_tail, "zooming stops following the tail");
        }
        // auto_y: y is left to the fit.
        let auto = GraphView { auto_y: true, ..v };
        let z = zoom_view(&auto, RECT, cursor, 0.5);
        assert_eq!((z.y_min, z.y_max), (auto.y_min, auto.y_max));
    }

    #[test]
    fn pan_follows_the_drag_and_clamps_to_the_data() {
        let v = GraphView { x_min: 40.0, x_max: 60.0, ..view() };
        // 400 pt plot, 20-unit window: 20 pt right = 1 unit earlier.
        let p = pan_view(&v, RECT, Vec2::new(20.0, 0.0));
        assert!((p.x_min - 39.0).abs() < 1e-9 && (p.x_max - 59.0).abs() < 1e-9);
        assert!(!p.follow_tail);
        let p = pan_view(&v, RECT, Vec2::new(0.0, 100.0));
        assert!((p.y_min - 0.0).abs() < 1e-9, "dragging down shows lower values: {p:?}");
        // Far past either end: pinned to the data.
        let data = (0.0, 100.0);
        let p = clamp_view_x(&pan_view(&v, RECT, Vec2::new(10_000.0, 0.0)), data, 20.0);
        assert_eq!((p.x_min, p.x_max), (0.0, 20.0));
        let p = clamp_view_x(&pan_view(&v, RECT, Vec2::new(-10_000.0, 0.0)), data, 20.0);
        assert_eq!((p.x_min, p.x_max), (80.0, 100.0));
        // A window wider than the data keeps covering all of it.
        let wide = GraphView { x_min: -500.0, x_max: 100.0, ..v };
        let p = clamp_view_x(&wide, (0.0, 50.0), 600.0);
        assert_eq!((p.x_min, p.x_max), (-500.0, 100.0));
        let p = clamp_view_x(&GraphView { x_min: 10.0, x_max: 610.0, ..v }, (0.0, 50.0), 600.0);
        assert_eq!((p.x_min, p.x_max), (0.0, 600.0));
        // Zooming out stops at all of the data (or the home window).
        let p = clamp_view_x(&GraphView { x_min: -1e6, x_max: 1e6, ..v }, data, 20.0);
        assert_eq!((p.x_min, p.x_max), (0.0, 100.0));
    }

    #[test]
    fn reset_follows_the_tail_with_the_home_window() {
        let v = GraphView { x_min: 3.0, x_max: 7.0, auto_y: false, follow_tail: false, ..view() };
        let r = reset_view(&v, 600.0);
        assert!(r.follow_tail && r.auto_y);
        assert_eq!(r.x_max - r.x_min, 600.0);
    }

    use classic_core::components::{GraphSeriesRef, GraphStyle, UiAnchor};

    /// A 400 x 200 interactive graph at the origin: plot (40, 20) 260 x 160,
    /// over 1000 samples, following a 100-wide tail.
    fn engine_with_graph() -> (Engine, GraphElem) {
        let mut e = Engine::new_for_test();
        e.create_graph_series("s", 1024);
        for i in 0..1000 {
            e.graph_push("s", i as f32, (i as f32 * 0.1).sin());
        }
        let opts = GraphOptions {
            width: 400.0,
            height: 200.0,
            margins: Some([20.0, 100.0, 20.0, 40.0]),
            series: vec![GraphSeriesRef::new("s", GraphStyle::default())],
            view: GraphView { x_min: 0.0, x_max: 100.0, auto_y: true, follow_tail: true, ..view() },
            interactive: true,
            legend: true,
            ..Default::default()
        };
        let g = e.spawn_graph(opts);
        let ui = e.ui.as_mut().unwrap();
        ui.root_add_child(&mut e.world, g.frame, UiAnchor::TopLeft, UiAnchor::TopLeft);
        ui.refresh_layout(&mut e.world);
        e.time.delta = 1.0 / 60.0;
        (e, g)
    }

    fn graph_view(e: &Engine, g: GraphElem) -> GraphView {
        e.world.get::<&GraphRender>(g.plot).unwrap().view
    }

    fn text(e: &Engine, l: hecs::Entity) -> String {
        e.world.get::<&SdfTextRender>(l).unwrap().text.clone()
    }

    #[test]
    fn the_hook_follows_the_tail_and_writes_the_labels() {
        let (mut e, g) = engine_with_graph();
        assert_eq!(e.graph_plot_rect(g.plot), Some(Vec4::new(40.0, 20.0, 260.0, 160.0)));
        update(&mut e);
        let v = graph_view(&e, g);
        assert_eq!((v.x_min, v.x_max), (899.0, 999.0));
        let ui = &e.graphs.ui[&g.plot];
        assert_eq!(text(&e, ui.x_labels[0]), "900");
        assert_eq!(text(&e, ui.x_labels[1]), "920");
        assert_eq!(text(&e, ui.legend_names[0]), "s");
        assert_eq!(text(&e, ui.legend_values[0]), format_value((999.0f32 * 0.1).sin() as f64));
        assert_eq!(ui.swatches.len(), 1);
        let x = e.world.get::<&Transform>(ui.x_labels[0]).unwrap().position.x;
        assert_eq!(x, (40.0 + 260.0 * 0.01f32).round(), "the 900 tick is 1% into the plot");
    }

    #[test]
    fn the_wheel_zooms_over_the_plot_and_is_consumed() {
        let (mut e, g) = engine_with_graph();
        update(&mut e);
        // Outside the graph: the camera keeps the wheel.
        e.input.mouse_pos = Vec2::new(800.0, 400.0);
        e.input.mouse_wheel = 1.0;
        update(&mut e);
        assert_eq!(e.input.mouse_wheel, 1.0);
        assert!(graph_view(&e, g).follow_tail);
        // Over the plot: zoomed in 2x about the cursor, consumed.
        e.input.mouse_pos = Vec2::new(170.0, 100.0);
        let before = graph_view(&e, g);
        update(&mut e);
        assert_eq!(e.input.mouse_wheel, 0.0);
        let v = graph_view(&e, g);
        assert!(!v.follow_tail);
        assert!(((v.x_max - v.x_min) - 50.0).abs() < 1e-9);
        let xc = |v: &GraphView| px_to_view(170.0, v.x_min, v.x_max, 40.0, 300.0);
        assert!((xc(&v) - xc(&before)).abs() < 1e-9, "the point under the cursor stays");
        // Over the frame's margin: consumed, not zoomed.
        e.input.mouse_pos = Vec2::new(350.0, 100.0);
        e.input.mouse_wheel = 1.0;
        update(&mut e);
        assert_eq!(e.input.mouse_wheel, 0.0);
        assert_eq!(graph_view(&e, g).x_max - graph_view(&e, g).x_min, v.x_max - v.x_min);
    }

    #[test]
    fn drag_pans_and_a_double_click_resets() {
        let (mut e, g) = engine_with_graph();
        update(&mut e);
        let press = |e: &mut Engine, at: Vec2| {
            e.graphs.ui[&g.plot].hit.set(true);
            e.input.mouse_pos = at;
            e.input.mouse_down[0] = true;
            update(e);
        };
        press(&mut e, Vec2::new(100.0, 100.0));
        // 52 pt right on a 260 pt, 100-wide window: 20 units earlier.
        e.input.mouse_pos = Vec2::new(152.0, 100.0);
        update(&mut e);
        let v = graph_view(&e, g);
        assert!(!v.follow_tail);
        assert!((v.x_min - 879.0).abs() < 1e-6 && (v.x_max - 979.0).abs() < 1e-6, "{v:?}");
        e.input.mouse_down[0] = false;
        update(&mut e);
        // Two presses within the double-click window: back to the tail.
        press(&mut e, Vec2::new(200.0, 90.0));
        e.input.mouse_down[0] = false;
        update(&mut e);
        press(&mut e, Vec2::new(201.0, 90.0));
        let v = graph_view(&e, g);
        assert!(v.follow_tail && v.auto_y, "{v:?}");
        assert_eq!((v.x_min, v.x_max), (899.0, 999.0));
    }

    #[test]
    fn hiding_the_frame_hides_the_labels() {
        let (mut e, g) = engine_with_graph();
        update(&mut e);
        e.set_enabled(g.frame, false);
        update(&mut e);
        let ui = &e.graphs.ui[&g.plot];
        assert!(ui.labels().all(|l| e.world.get::<&Disabled>(l).is_ok()));
        e.set_enabled(g.frame, true);
        update(&mut e);
        let ui = &e.graphs.ui[&g.plot];
        assert!(ui.labels().all(|l| e.world.get::<&Disabled>(l).is_err()));
        // Despawned: the chrome goes with it.
        let labels: Vec<_> = ui.labels().collect();
        e.world.despawn(g.plot).unwrap();
        update(&mut e);
        assert!(e.graphs.ui.is_empty());
        assert!(labels.iter().all(|&l| !e.world.contains(l)));
    }

    #[test]
    fn labels_format_to_their_precision() {
        assert_eq!(format_tick(1200.0, 200.0), "1200");
        assert_eq!(format_tick(2.5, 0.5), "2.5");
        assert_eq!(format_tick(0.30000000000000004, 0.1), "0.3");
        assert_eq!(format_tick(-0.0000001, 0.05), "0.00");
        assert_eq!(format_tick(-4.0, 2.0), "-4");
        assert_eq!(format_value(16.666), "16.7");
        assert_eq!(format_value(0.4567), "0.46");
        assert_eq!(format_value(123.4), "123");
        assert_eq!(format_value(-2.0), "-2.00");
        assert_eq!(format_value(f64::NAN), "-");
    }
}
