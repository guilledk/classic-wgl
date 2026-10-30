//! Live perf graph: `Shift+KeyG` opens a bottom-right panel plotting the
//! frame profiler's live samples (`classic_gfx::perf` live mode): wall and
//! CPU frame time, the GPU total when timer queries are available, and the
//! six heaviest CPU phases (ranked on a decaying mean after [`WARMUP`]
//! frames and every [`RERANK_FRAMES`] after; a phase that stays in the top
//! six keeps its slot and colour).  Again closes it.
//!
//! Installed with the dev HUD (`host_features && !CLASSIC_NO_UI`) and closed
//! by default: nothing is spawned and the profiler stays off until the first
//! open, so a run that never presses the key draws, traces and costs
//! nothing new.  While open, live mode runs (with GPU timing); closing stops
//! it at the next frame boundary.
//!
//! Everything runs in one pre-update hook, never in `on_sim_update`, so the
//! graph keeps advancing under the inspector's simulation pause.  It sits in
//! a transparent padding holder anchored bottom-right; while the inspector
//! is open the holder's right inset grows past the inspector's detail panel,
//! so both fit side by side (at 1280 points wide and up; narrower, the
//! graph overlaps the inspector's tree panel).  Wheel zoom, drag pan and
//! double-click reset come from `Engine::spawn_graph` (interactive).

use classic_core::components::{
    GraphAxes, GraphRender, GraphSeriesRef, GraphStyle, GraphView, UiAnchor, UiKind, UiNode,
};
use classic_engine::ui::{GraphElem, GraphOptions};
use classic_engine::Engine;
use classic_gfx::perf::{self, LiveConfig};

use crate::state::DemoStateRef;

/// With Shift held (a bare `G` is left to guests).
pub const TOGGLE_KEY: &str = "KeyG";
/// Samples kept per series.
pub const RING: usize = 8192;
/// The followed x window, in frames.
pub const WINDOW: f64 = 600.0;
/// Frames ranked before the phase series are first picked.
pub const WARMUP: u32 = 60;
/// Frames between re-picks of the phase series.
pub const RERANK_FRAMES: u32 = 120;
/// Per-frame decay of the ranking sums (a memory of ~100 frames).
const RANK_DECAY: f64 = 0.99;
/// CPU phases plotted.
pub const TOP_PHASES: usize = 6;
/// Frame size and the plot's insets `[top, right, bottom, left]` (title,
/// legend, x ticks, y ticks).
pub const SIZE: [f32; 2] = [460.0, 220.0];
const MARGINS: [f32; 4] = [22.0, 124.0, 18.0, 40.0];
/// Gap to the viewport edge (and to the inspector's detail panel).
const EDGE: f32 = 8.0;
/// Over the HUD (-1000 … -1500), under the inspector (-1600).
const Z: f32 = -1550.0;

pub const WALL: &str = "perf.wall";
pub const CPU: &str = "perf.cpu";
pub const GPU: &str = "perf.gpu";
pub const PHASES: [&str; TOP_PHASES] =
    ["perf.phase0", "perf.phase1", "perf.phase2", "perf.phase3", "perf.phase4", "perf.phase5"];
/// Index of the first phase series in the graph's series list.
const FIRST_PHASE: usize = 3;

const COLORS: [[f32; 4]; 3 + TOP_PHASES] = [
    [0.95, 0.95, 0.95, 1.0],
    [1.0, 0.55, 0.15, 1.0],
    [0.3, 0.8, 1.0, 1.0],
    [0.4, 0.9, 0.4, 1.0],
    [0.95, 0.4, 0.85, 1.0],
    [0.95, 0.9, 0.3, 1.0],
    [0.55, 0.6, 1.0, 1.0],
    [1.0, 0.35, 0.35, 1.0],
    [0.3, 0.85, 0.75, 1.0],
];

/// The panel's state (`DemoState::perf_graph`).
#[derive(Debug, Default)]
pub struct PerfGraphState {
    pub open: bool,
    /// The graph, once first opened.
    pub elem: Option<GraphElem>,
    /// Its bottom-right padding holder.
    pub holder: Option<hecs::Entity>,
    /// The profiler frame index of x = 0 (the first sample since opening).
    f0: Option<u64>,
    last_f: Option<u64>,
    last_gpu_f: Option<u64>,
    warm: u32,
    sums: Vec<(&'static str, f64)>,
    /// The phase plotted in each slot (series `PHASES[i]`), once ranked.
    pub phases: [Option<&'static str>; TOP_PHASES],
    /// The holder's right inset as last applied.
    right: f32,
}

/// Install the toggle + feed hook.  Spawns nothing (see the module docs).
pub fn install(engine: &mut Engine, state: &DemoStateRef) {
    let s = state.clone();
    engine.on_pre_update(move |e| pre_update(e, &s));
}

fn pre_update(engine: &mut Engine, state: &DemoStateRef) {
    let shift = engine.input.is_key_down("ShiftLeft") || engine.input.is_key_down("ShiftRight");
    if shift && engine.input.was_key_pressed(TOGGLE_KEY) {
        let open = !state.borrow().perf_graph.open;
        set_open(engine, state, open);
    }
    let mut s = state.borrow_mut();
    if !s.perf_graph.open {
        return;
    }
    let inspector = s.inspector.open;
    let p = &mut s.perf_graph;
    dodge(engine, p, inspector);
    feed(engine, p);
}

fn set_open(engine: &mut Engine, state: &DemoStateRef, open: bool) {
    let mut s = state.borrow_mut();
    let p = &mut s.perf_graph;
    p.open = open;
    if !open {
        perf::stop_live();
        if let Some(h) = p.holder {
            engine.set_enabled(h, false);
        }
        return;
    }
    // Fresh rings and ranking on every open: x restarts at 0.
    for name in [WALL, CPU, GPU].iter().chain(&PHASES) {
        engine.create_graph_series(name, RING);
    }
    *p = PerfGraphState { open: true, elem: p.elem, holder: p.holder, ..Default::default() };
    perf::start_live(LiveConfig { gpu: true });
    match (p.elem, p.holder) {
        (Some(elem), Some(holder)) => {
            if let Ok(mut g) = engine.world.get::<&mut GraphRender>(elem.plot) {
                g.view = home_view();
                for (i, r) in g.series.iter_mut().enumerate().skip(FIRST_PHASE) {
                    r.label = phase_placeholder(i - FIRST_PHASE);
                }
            }
            engine.set_enabled(holder, true);
        }
        _ => {
            let (elem, holder) = spawn(engine);
            p.elem = Some(elem);
            p.holder = Some(holder);
        }
    }
}

fn home_view() -> GraphView {
    GraphView { x_min: 0.0, x_max: WINDOW, y_min: 0.0, y_max: 1.0, auto_y: true, follow_tail: true }
}

fn phase_placeholder(i: usize) -> String {
    if i == 0 {
        "ranking…".into()
    } else {
        "-".into()
    }
}

/// The graph in its padding holder, bottom-right of the game UI root.
fn spawn(engine: &mut Engine) -> (GraphElem, hecs::Entity) {
    let style = |i: usize| GraphStyle { color: COLORS[i], line_width: 1.25, envelope_alpha: 0.35 };
    let mut series = vec![
        GraphSeriesRef { label: "wall".into(), ..GraphSeriesRef::new(WALL, style(0)) },
        GraphSeriesRef { label: "cpu".into(), ..GraphSeriesRef::new(CPU, style(1)) },
        GraphSeriesRef { label: "gpu".into(), ..GraphSeriesRef::new(GPU, style(2)) },
    ];
    for (i, name) in PHASES.iter().enumerate() {
        series.push(GraphSeriesRef {
            label: phase_placeholder(i),
            ..GraphSeriesRef::new(*name, style(FIRST_PHASE + i))
        });
    }
    let opts = GraphOptions {
        width: SIZE[0],
        height: SIZE[1],
        margins: Some(MARGINS),
        background: [0.04, 0.05, 0.08, 0.9],
        series,
        view: home_view(),
        axes: GraphAxes {
            ticks: 4,
            title: "frame ms  (Shift+G closes)".into(),
            ..Default::default()
        },
        interactive: true,
        legend: true,
        z: Some(Z),
        ..Default::default()
    };
    let elem = engine.spawn_graph(opts);
    let ui = engine.ui.as_mut().expect("spawn_graph created the manager");
    let holder = ui.spawn_padding(&mut engine.world, 0.0, EDGE, EDGE, 0.0, [0.0; 4]);
    ui.container_add_child(
        &mut engine.world,
        holder,
        elem.frame,
        UiAnchor::TopLeft,
        UiAnchor::TopLeft,
    );
    ui.root_add_child(&mut engine.world, holder, UiAnchor::BotRight, UiAnchor::BotRight);
    (elem, holder)
}

/// Keep the panel left of the inspector's detail panel while it is open.
fn dodge(engine: &mut Engine, p: &mut PerfGraphState, inspector: bool) {
    let Some(holder) = p.holder else { return };
    let right = if inspector {
        let (vw, vh) = engine.viewport_size();
        let (vw, vh) = if vw > 0.5 && vh > 0.5 { (vw, vh) } else { (1280.0, 720.0) };
        let detail = crate::inspector::compute_layout(vw, vh, false).detail.panel;
        vw - detail.x + EDGE
    } else {
        EDGE
    };
    if right == p.right {
        return;
    }
    p.right = right;
    if let Ok(mut node) = engine.world.get::<&mut UiNode>(holder) {
        node.kind = UiKind::Padding { top: 0.0, right, bottom: EDGE, left: 0.0 };
    }
    if let Some(ui) = engine.ui.as_mut() {
        ui.mark_dirty();
    }
}

/// Push the profiler's newest frame (and GPU set) into the series.
fn feed(engine: &mut Engine, p: &mut PerfGraphState) {
    let Some(s) = perf::last_frame() else { return };
    if p.last_f == Some(s.f) {
        return;
    }
    p.last_f = Some(s.f);
    let f0 = *p.f0.get_or_insert(s.f);
    let x = (s.f - f0) as f32;
    // The first profiled frame has no previous one to measure wall from.
    if s.wall > 0.0 {
        engine.graph_push(WALL, x, s.wall as f32);
    }
    engine.graph_push(CPU, x, s.cpu as f32);

    // Rank on a decaying sum (a ~RANK_MEMORY-frame memory), so a boot-time
    // phase fades out of the pick once the game runs.
    for e in &mut p.sums {
        e.1 *= RANK_DECAY;
    }
    for &(name, ms) in &s.phases {
        match p.sums.iter_mut().find(|(n, _)| *n == name) {
            Some(e) => e.1 += ms,
            None => p.sums.push((name, ms)),
        }
    }
    p.warm += 1;
    if p.warm == WARMUP || (p.warm > WARMUP && (p.warm - WARMUP).is_multiple_of(RERANK_FRAMES)) {
        rerank(engine, p);
    }
    for (i, name) in p.phases.iter().enumerate() {
        let Some(name) = name else { continue };
        let ms = s.phases.iter().find(|(n, _)| n == name).map_or(0.0, |(_, v)| *v);
        engine.graph_push(PHASES[i], x, ms as f32);
    }

    if let Some(g) = perf::last_gpu() {
        if g.f >= f0 && p.last_gpu_f.is_none_or(|l| g.f > l) {
            p.last_gpu_f = Some(g.f);
            engine.graph_push(GPU, (g.f - f0) as f32, g.total as f32);
        }
    }
}

/// Re-pick the plotted phases: a phase still in the top six keeps its slot
/// (series, colour), a newcomer takes a freed slot on a fresh ring.
fn rerank(engine: &mut Engine, p: &mut PerfGraphState) {
    let slots = assign_slots(&p.phases, &top_phases(&p.sums, TOP_PHASES));
    for (i, (old, new)) in p.phases.iter().zip(&slots).enumerate() {
        if old != new {
            engine.create_graph_series(PHASES[i], RING);
        }
    }
    p.phases = slots;
    let Some(elem) = p.elem else { return };
    if let Ok(mut g) = engine.world.get::<&mut GraphRender>(elem.plot) {
        for (i, r) in g.series.iter_mut().enumerate().skip(FIRST_PHASE) {
            let name = p.phases.get(i - FIRST_PHASE).copied().flatten().unwrap_or("-");
            if r.label != name {
                r.label = name.to_string();
            }
        }
    }
}

/// Slot assignment for a new top list: names already in a slot stay there,
/// the rest fill the free slots in rank order.
pub fn assign_slots(
    slots: &[Option<&'static str>; TOP_PHASES],
    top: &[&'static str],
) -> [Option<&'static str>; TOP_PHASES] {
    let mut out = slots.map(|s| s.filter(|n| top.contains(n)));
    let kept = out;
    let mut fresh = top.iter().filter(|n| !kept.contains(&Some(**n)));
    for slot in out.iter_mut().filter(|s| s.is_none()) {
        *slot = fresh.next().copied();
    }
    out
}

/// The `n` names with the largest totals, heaviest first (ties by name, so
/// the pick is stable).
pub fn top_phases(sums: &[(&'static str, f64)], n: usize) -> Vec<&'static str> {
    let mut v = sums.to_vec();
    v.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    v.into_iter().take(n).map(|(name, _)| name).collect()
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use classic_core::components::Disabled;
    use classic_engine::Engine;

    use super::*;
    use crate::state::DemoState;

    /// One frame with `pressed` just pressed (and Shift held with `shift`).
    fn frame_keys(engine: &mut Engine, pressed: Option<&str>, shift: bool) {
        let mut input = engine.input.clone();
        input.keys_pressed.clear();
        input.keys_down.clear();
        if let Some(k) = pressed {
            input.keys_pressed.insert(k.into(), true);
        }
        if shift {
            input.keys_down.insert("ShiftLeft".into(), true);
        }
        engine.frame(&mut input, 1280.0, 720.0, 1.0 / 60.0);
    }

    fn frame(engine: &mut Engine) {
        frame_keys(engine, None, false);
    }

    fn shift_g(engine: &mut Engine) {
        frame_keys(engine, Some(TOGGLE_KEY), true);
    }

    #[test]
    fn a_rerank_keeps_surviving_phases_in_their_slots() {
        let first = assign_slots(&[None; TOP_PHASES], &["a", "b", "c"]);
        assert_eq!(first, [Some("a"), Some("b"), Some("c"), None, None, None]);
        // "a" drops out, "d" and "e" come in: b and c stay put.
        let next = assign_slots(&first, &["c", "d", "b", "e"]);
        assert_eq!(next, [Some("d"), Some("b"), Some("c"), Some("e"), None, None]);
    }

    #[test]
    fn top_phases_rank_by_total_then_name() {
        let sums = [("ui", 1.0), ("update", 5.0), ("bloom", 1.0), ("terrain", 3.0)];
        assert_eq!(top_phases(&sums, 3), ["update", "terrain", "bloom"]);
        assert_eq!(top_phases(&sums, 9).len(), 4);
    }

    #[test]
    fn shift_g_opens_feeds_and_closes_the_graph() {
        let mut engine = Engine::new_for_test();
        let state: DemoStateRef = Rc::new(RefCell::new(DemoState::default()));
        let before = engine.world.len();
        install(&mut engine, &state);
        for _ in 0..3 {
            frame(&mut engine);
        }
        assert_eq!(engine.world.len(), before, "closed: nothing spawned");
        assert!(!perf::enabled(), "closed: the profiler stays off");

        // A bare G does nothing.
        frame_keys(&mut engine, Some(TOGGLE_KEY), false);
        assert!(!state.borrow().perf_graph.open);

        shift_g(&mut engine);
        assert!(state.borrow().perf_graph.open);
        assert!(perf::live(), "live from the frame boundary on");
        for _ in 0..(WARMUP + 5) {
            frame(&mut engine);
        }
        let n = engine.graph_series(CPU).map_or(0, |s| s.len());
        assert!(n >= WARMUP as usize, "cpu samples fed: {n}");
        assert!(state.borrow().perf_graph.phases.iter().any(Option::is_some), "phases ranked");
        let elem = state.borrow().perf_graph.elem.unwrap();
        assert!(!engine.is_disabled(elem.plot));
        assert!(engine.graph_plot_rect(elem.plot).is_some());

        shift_g(&mut engine);
        assert!(!state.borrow().perf_graph.open);
        assert!(!perf::live() && !perf::enabled(), "live mode stopped");
        assert!(engine.world.get::<&Disabled>(state.borrow().perf_graph.holder.unwrap()).is_ok());
        assert!(engine.is_disabled(elem.plot));

        // Reopening restarts x at 0 on fresh rings.
        shift_g(&mut engine);
        frame(&mut engine);
        frame(&mut engine);
        let s = engine.graph_series(CPU).unwrap();
        assert!(s.len() <= 2 && s.x_range().unwrap().0 == 0.0);
    }
}
