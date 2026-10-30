//! Graph element pixels on a real GL context (headless EGL; skips when
//! libEGL is absent): polyline and envelope modes at pixel ratio 1 and 2,
//! plot-rect clipping, the `O(plot width)` vertex bound on a 1M-point
//! series, and continuity across the ring's wrap seam.

use classic_core::components::{GraphRender, GraphSeriesRef, GraphStyle, GraphView, UiAnchor};
use classic_core::graph::{GraphMode, SeriesBuf};
use classic_platform::headless::HeadlessPlatform;
use classic_platform::{InputState, Platform};

use crate::boot::InterleavedBoot;
use crate::ui::{GraphElem, GraphOptions};
use crate::Engine;

const FRAME_BG: [f32; 4] = [0.0, 0.0, 0.5, 1.0];
const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const PAD: f32 = 20.0;

/// A headless engine with a `w` x `h` **logical** view at `ratio`; `None`
/// without libEGL.
fn gl_engine(w: u32, h: u32, ratio: f32) -> Option<(HeadlessPlatform, Engine)> {
    let (pw, ph) = ((w as f32 * ratio) as u32, (h as f32 * ratio) as u32);
    let platform = match HeadlessPlatform::new(pw, ph) {
        Ok(p) => p.with_pixel_ratio(ratio),
        Err(e) => {
            eprintln!("skipping: no headless EGL ({e})");
            return None;
        }
    };
    let mut engine = InterleavedBoot::new(None).engine(platform.gl());
    let manifest: classic_rom::RomManifest =
        serde_json::from_value(serde_json::json!({ "textures": [] })).expect("an empty manifest");
    engine.ensure_gfx(platform.gl(), &manifest, &classic_rom::NullBootSink);
    engine.gfx.as_mut().unwrap().set_render_target(pw, ph);
    Some((platform, engine))
}

/// A graph filling the `w` x `h` view (frame at the origin, plot inset by
/// `PAD`) over series `name`.
fn spawn(engine: &mut Engine, w: f32, h: f32, name: &str, view: GraphView) -> GraphElem {
    let opts = GraphOptions {
        width: w,
        height: h,
        padding: PAD,
        background: FRAME_BG,
        series: vec![GraphSeriesRef {
            data: name.into(),
            style: GraphStyle { color: RED, line_width: 3.0, envelope_alpha: 0.35 },
        }],
        view,
        ..Default::default()
    };
    engine.game_ui();
    let ui = engine.ui.as_mut().unwrap();
    let elem = ui.spawn_graph(&mut engine.world, opts);
    ui.root_add_child(&mut engine.world, elem.frame, UiAnchor::TopLeft, UiAnchor::TopLeft);
    elem
}

/// Render one frame and read the whole target back.
struct Frame {
    rgba: Vec<u8>,
    w: i32,
    h: i32,
    ratio: f32,
}

impl Frame {
    fn render(platform: &HeadlessPlatform, engine: &mut Engine) -> Self {
        let mut input = InputState::new();
        engine.frame_sized(&mut input, platform.viewport(), 1.0 / 60.0);
        let gfx = engine.gfx.as_ref().unwrap();
        let rt = gfx.render_target.as_ref().unwrap();
        Self {
            rgba: rt.read_pixels_rgba(&gfx.gl),
            w: rt.width as i32,
            h: rt.height as i32,
            ratio: gfx.pixel_ratio,
        }
    }

    /// The physical pixel at the centre of logical point `(x, y)`.
    fn at(&self, x: f32, y: f32) -> [u8; 4] {
        let px = ((x + 0.5) * self.ratio).floor() as i32;
        let py = ((y + 0.5) * self.ratio).floor() as i32;
        assert!(px >= 0 && py >= 0 && px < self.w && py < self.h, "({x}, {y}) off target");
        let i = (((self.h - 1 - py) * self.w + px) * 4) as usize;
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2], self.rgba[i + 3]]
    }

    fn lit(&self, x: f32, y: f32) -> bool {
        let [r, _, b, _] = self.at(x, y);
        r > 128 && r > b
    }

    fn background(&self, x: f32, y: f32) -> bool {
        let [r, g, b, _] = self.at(x, y);
        r < 8 && g < 8 && (b as i32 - 128).abs() < 8
    }
}

fn sine(n: usize) -> (Vec<f32>, Vec<f32>) {
    let x: Vec<f32> = (0..n).map(|i| i as f32 / (n - 1) as f32 * std::f32::consts::TAU).collect();
    let y = x.iter().map(|v| v.sin()).collect();
    (x, y)
}

/// The logical y of data `y` in a plot `(PAD, PAD, _, ph)` over `[y0, y1]`.
fn y_px(y: f64, y0: f64, y1: f64, ph: f32) -> f32 {
    PAD + ph * (1.0 - ((y - y0) / (y1 - y0)) as f32)
}

/// Draw a sine in `mode`, then check pixels on and off the curve.
fn sine_on_and_off_the_curve(n: usize, ratio: f32, mode: GraphMode) {
    let (w, h) = (400.0, 200.0);
    let Some((platform, mut engine)) = gl_engine(w as u32, h as u32, ratio) else { return };
    let (x, y) = sine(n);
    engine.set_graph_data("sine", &x, &y);
    let view = GraphView {
        x_min: 0.0,
        x_max: std::f64::consts::TAU,
        y_min: -1.25,
        y_max: 1.25,
        auto_y: false,
        follow_tail: false,
    };
    let elem = spawn(&mut engine, w, h, "sine", view);
    let f = Frame::render(&platform, &mut engine);

    let rect = engine.graph_plot_rect(elem.plot).expect("a laid-out plot");
    assert_eq!(rect.to_array(), [PAD, PAD, w - 2.0 * PAD, h - 2.0 * PAD]);
    let sel = engine.graph_series("sine").unwrap().select(view.x_min, view.x_max, rect.z * ratio);
    assert_eq!(sel.mode, mode, "{n} points at ratio {ratio}");

    let (pw, ph) = (rect.z, rect.w);
    for col in [30.0f32, 75.0, 140.0, 200.0, 260.0, 330.0, 370.0] {
        let xd = ((col + 0.5 - PAD) / pw) as f64 * std::f64::consts::TAU;
        let cy = y_px(xd.sin(), view.y_min, view.y_max, ph);
        assert!(
            f.lit(col, cy.floor()),
            "{mode:?} @{ratio}: ({col}, {cy}) not lit: {:?}",
            f.at(col, cy)
        );
        // Far off the curve: the frame background shows through.
        let off = if cy > PAD + ph / 2.0 { cy - 50.0 } else { cy + 50.0 };
        assert!(f.background(col, off), "{mode:?} @{ratio}: ({col}, {off}) {:?}", f.at(col, off));
    }
    assert!(engine.graph_vertex_count() > 0);
}

#[test]
fn a_polyline_sine_lights_the_curve_at_ratio_1_and_2() {
    for ratio in [1.0, 2.0] {
        sine_on_and_off_the_curve(200, ratio, GraphMode::Polyline);
    }
}

#[test]
fn an_envelope_sine_lights_the_curve_at_ratio_1_and_2() {
    for ratio in [1.0, 2.0] {
        sine_on_and_off_the_curve(200_000, ratio, GraphMode::Envelope);
    }
}

#[test]
fn the_plot_rect_clips_the_series() {
    for (n, ratio) in [(200, 1.0), (200_000, 1.0), (200, 2.0)] {
        let (w, h) = (400.0, 200.0);
        let Some((platform, mut engine)) = gl_engine(w as u32, h as u32, ratio) else { return };
        let (x, y) = sine(n);
        engine.set_graph_data("sine", &x, &y);
        // Data on both sides of the x window and past the y window.
        let view = GraphView {
            x_min: 1.0,
            x_max: 5.0,
            y_min: -0.5,
            y_max: 0.5,
            auto_y: false,
            follow_tail: false,
        };
        spawn(&mut engine, w, h, "sine", view);
        let f = Frame::render(&platform, &mut engine);
        let ph = h - 2.0 * PAD;
        // Inside: the curve crosses y = 0 at x = pi.
        let col = PAD + ((std::f64::consts::PI - 1.0) / 4.0) as f32 * (w - 2.0 * PAD);
        assert!(f.lit(col.floor(), y_px(0.0, -0.5, 0.5, ph).floor()), "n {n} @{ratio}");
        // The frame padding all around the plot stays untouched.
        for i in 0..(w as i32) {
            for row in [2.0, PAD - 1.0, h - PAD, h - 3.0] {
                let p = (i as f32, row);
                assert!(f.background(p.0, p.1), "n {n} @{ratio}: {p:?} {:?}", f.at(p.0, p.1));
            }
        }
        for j in 0..(h as i32) {
            for c in [2.0, PAD - 1.0, w - PAD, w - 3.0] {
                let p = (c, j as f32);
                assert!(f.background(p.0, p.1), "n {n} @{ratio}: {p:?} {:?}", f.at(p.0, p.1));
            }
        }
    }
}

#[test]
fn a_million_points_draw_o_plot_width_vertices() {
    let (w, h) = (400.0, 200.0);
    let Some((platform, mut engine)) = gl_engine(w as u32, h as u32, 1.0) else { return };
    let (x, y) = sine(1 << 20);
    engine.set_graph_data("big", &x, &y);
    let tau = std::f64::consts::TAU;
    let view = GraphView { x_min: 0.0, x_max: tau, auto_y: true, ..Default::default() };
    let elem = spawn(&mut engine, w, h, "big", view);
    let f = Frame::render(&platform, &mut engine);
    let plot_w = (w - 2.0 * PAD) as u64;
    let verts = engine.graph_vertex_count();
    assert!(verts > 0 && verts <= 4 * (plot_w + 2), "zoomed out: {verts} vertices");
    // Auto-Y framed the sine with a 5% margin; its crest is lit.
    let v = engine.world.get::<&GraphRender>(elem.plot).unwrap().view;
    assert!((v.y_max - 1.1).abs() < 1e-3 && (v.y_min + 1.1).abs() < 1e-3, "{v:?}");
    let crest_x = PAD + (w - 2.0 * PAD) / 4.0;
    assert!(f.lit(crest_x.floor(), y_px(1.0, v.y_min, v.y_max, h - 2.0 * PAD).floor()));

    // Zoomed in to ~300 samples: a polyline, still O(plot width).
    let span = 300.0 * tau / (1 << 20) as f64;
    engine.world.get::<&mut GraphRender>(elem.plot).unwrap().view.x_min = 3.0;
    engine.world.get::<&mut GraphRender>(elem.plot).unwrap().view.x_max = 3.0 + span;
    Frame::render(&platform, &mut engine);
    let verts = engine.graph_vertex_count();
    assert!(verts > 0 && verts <= 9 * 2 * plot_w, "zoomed in: {verts} vertices");
}

/// Stream a ramp `y = x` through a ring until it wraps, then check that every
/// plot column has the line lit where it should be (a seam gap would leave
/// one or more columns dark).  The GPU mirror is built after the first
/// `cap` samples, so the wrap itself arrives as dirty-slot updates.
fn streamed_ramp_is_continuous(cap: usize, pushes: usize, mode: GraphMode) {
    let (w, h) = (600.0, 200.0);
    let Some((platform, mut engine)) = gl_engine(w as u32, h as u32, 1.0) else { return };
    engine.create_graph_series("ramp", cap);
    let cap = engine.graph_series("ramp").unwrap().capacity();
    for i in 0..cap {
        engine.graph_push("ramp", i as f32, i as f32);
    }
    // follow_tail keeps the window width (the ring's span); auto_y fits y.
    let width = (cap - 1) as f64;
    let view = GraphView { x_min: 0.0, x_max: width, auto_y: true, follow_tail: true, ..view0() };
    let elem = spawn(&mut engine, w, h, "ramp", view);
    Frame::render(&platform, &mut engine);
    for i in cap..pushes {
        engine.graph_push("ramp", i as f32, i as f32);
    }
    let (first, last) = engine.graph_series("ramp").unwrap().x_range().unwrap();
    let f = Frame::render(&platform, &mut engine);
    let v = engine.world.get::<&GraphRender>(elem.plot).unwrap().view;
    assert_eq!((v.x_min, v.x_max), (first as f64, last as f64));
    let s: &SeriesBuf = engine.graph_series("ramp").unwrap();
    let pw = w - 2.0 * PAD;
    let sel = s.select(v.x_min, v.x_max, pw);
    assert_eq!(sel.mode, mode);
    assert!(sel.ranges[1].1 > 0, "the selection wraps the ring: {sel:?}");
    let ph = h - 2.0 * PAD;
    for col in (PAD as i32 + 3)..((w - PAD) as i32 - 3) {
        let xd = v.x_min + ((col as f32 + 0.5 - PAD) / pw) as f64 * (v.x_max - v.x_min);
        let cy = y_px(xd, v.y_min, v.y_max, ph);
        let c = col as f32;
        let lit = (-1..=1).any(|d| f.lit(c, cy.floor() + d as f32));
        assert!(lit, "{mode:?}: column {col} dark at y {cy}: {:?}", f.at(c, cy));
    }
}

fn view0() -> GraphView {
    GraphView::default()
}

#[test]
fn a_wrapped_polyline_has_no_seam_gap() {
    // ~9 px per segment, so a missing seam segment leaves a visible gap.
    streamed_ramp_is_continuous(64, 100, GraphMode::Polyline);
}

#[test]
fn a_wrapped_envelope_has_no_seam_gap() {
    streamed_ramp_is_continuous(1 << 14, 40_000, GraphMode::Envelope);
}
