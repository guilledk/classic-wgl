//! Boot loading screen: a retained-mode UI overlay driven by the [`BootSink`]
//! stream.
//!
//! [`VisualBootSink`] records the boot event stream into a small mutable state
//! (DAG node phases, per-sheet resource chips, a log scroller, live CPU/RSS)
//! and renders it through the normal [`crate::Engine::frame`] pipeline: the
//! loader spawns plain UI entities (`RectRender` / `SdfTextRender` /
//! `SpriteRender`, all `ignore_cam`) and mutates their transforms/text/colors
//! from the boot state on every frame via [`VisualBootSink::sync`], with the
//! DAG connector edges drawn in a post-pass overlay hook.  The embedded DejaVu
//! Sans SDF atlas (loaded by [`crate::Engine::init_gfx`]) means text renders
//! from frame 0.

use classic_platform::BootTimer;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use classic_core::components::{
    Disabled, RectRender, SdfTextRender, SpriteRender, TextJustify, Transform, UiKind, UiNode,
    DEFAULT_SDF_FONT,
};
use classic_gfx::{Gfx, GlBuffer};
use classic_rom::{BootEvent, BootSink, LoadedRom, LoadedRoms};
use glam::{Mat4, Vec2, Vec3};

use crate::boot::BootStep;
use crate::Engine;

/// How many trailing event descriptions the footer log scroller keeps.
const LOG_LINES: usize = 4;

/// Viewport width above which the two-column "tree" layout is used (narrow
/// windows fall back to a simple single-column stack).
const WIDE_BREAKPOINT: f32 = 1024.0;
/// Fraction of the body width the left DAG panel occupies in the wide layout.
const DAG_FRACTION: f32 = 0.42;

// Layout constants (screen pixels at a 1280px reference viewport, top-left
// origin).  Scaled by the responsive factor `s = vw / 1280`.
const HEADER_H: f32 = 34.0;
const PAD: f32 = 12.0;
const NODE_W: f32 = 220.0;
const NODE_H: f32 = 46.0;
const CHIP_W: f32 = 9.0;
const CHIP_GAP: f32 = 3.0;
const CHIP_ROW_H: f32 = 18.0;
const FOOTER_H: f32 = 96.0;

// Palette.
const BG: [f32; 4] = [0.03, 0.03, 0.06, 1.0];
const HEADER_BG: [f32; 4] = [0.07, 0.07, 0.12, 1.0];
const NODE_BG: [f32; 4] = [0.10, 0.10, 0.16, 1.0];
const BAR_BG: [f32; 4] = [0.14, 0.14, 0.20, 1.0];
const TEXT_COLOR: [f32; 4] = [0.81, 0.78, 0.94, 1.0];
const DIM_TEXT: [f32; 4] = [0.55, 0.53, 0.66, 1.0];
const EDGE_COLOR: [f32; 4] = [0.30, 0.30, 0.42, 1.0];

// Z layering (the UI phase is pure draw-order).  The render list sorts by
// `Transform.position.z` *descending* and draws in that order, so the highest
// z draws first (bottom) and the lowest draws last (top).  Assign the
// background the highest z and later layers progressively lower z, so the
// background sits behind the header/body/footer.  Values only order relative
// to each other; they stay well inside the ortho clip range.
const Z_BG: f32 = -100.0;
const Z_HEADER: f32 = -200.0;
const Z_HEADER_TEXT: f32 = -210.0;
const Z_BODY: f32 = -300.0;
const Z_BODY_DETAIL: f32 = -310.0;
const Z_BODY_TEXT: f32 = -320.0;
const Z_FOOTER: f32 = -400.0;
const Z_FOOTER_DETAIL: f32 = -410.0;
const Z_FOOTER_TEXT: f32 = -420.0;

/// The phase a ROM node is in, derived from the boot event stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RomPhase {
    Queued,
    Fetching,
    Fetched,
    Decompressed,
    Parsed,
    Hydrating,
    Done,
}

impl RomPhase {
    fn label(self) -> &'static str {
        match self {
            RomPhase::Queued => "queued",
            RomPhase::Fetching => "fetching",
            RomPhase::Fetched => "fetched",
            RomPhase::Decompressed => "decompressed",
            RomPhase::Parsed => "parsed",
            RomPhase::Hydrating => "hydrating",
            RomPhase::Done => "done",
        }
    }

    fn color(self) -> [f32; 4] {
        match self {
            RomPhase::Queued => [0.40, 0.40, 0.45, 1.0],
            RomPhase::Fetching => [0.35, 0.75, 0.90, 1.0],
            RomPhase::Fetched => [0.40, 0.55, 0.90, 1.0],
            RomPhase::Decompressed => [0.70, 0.50, 0.90, 1.0],
            RomPhase::Parsed => [0.90, 0.80, 0.35, 1.0],
            RomPhase::Hydrating => [0.95, 0.65, 0.30, 1.0],
            RomPhase::Done => [0.40, 0.85, 0.45, 1.0],
        }
    }
}

/// The decode/upload state of one resource sheet (a chip).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChipState {
    Queued,
    Decoded,
    Uploaded,
}

/// One resource sheet (a unique texture source or `.basis` job).
#[derive(Clone, Debug)]
struct Chip {
    name: String,
    state: ChipState,
}

/// One ROM node in the dependency DAG.
#[derive(Clone, Debug)]
struct RomNode {
    name: String,
    phase: RomPhase,
    chips: Vec<Chip>,
    received: u64,
    total: u64,
    /// Resolver names this ROM depends on (its `manifest.deps`), fanning into it.
    deps: Vec<String>,
}

/// The UI entities backing one DAG node.
struct NodeUi {
    rect: hecs::Entity,
    phase_bar: hecs::Entity,
    name: hecs::Entity,
    phase: hecs::Entity,
    legend_swatch: hecs::Entity,
    legend_name: hecs::Entity,
}

/// The entity set the loader renders through (spawned by
/// [`VisualBootSink::install`], driven each frame by [`VisualBootSink::sync`]).
struct LoaderUi {
    bg: hecs::Entity,
    header_bg: hecs::Entity,
    title: hecs::Entity,
    metrics: hecs::Entity,
    footer_bg: hecs::Entity,
    progress_bg: hecs::Entity,
    progress_fill: hecs::Entity,
    pct: hecs::Entity,
    latest: hecs::Entity,
    log: Vec<hecs::Entity>,
    preview_bg: hecs::Entity,
    preview_placeholder: hecs::Entity,
    preview_sprite: hecs::Entity,
    preview_label: hecs::Entity,
    dots_bg: hecs::Entity,
    dots_title: hecs::Entity,
    nodes: HashMap<String, NodeUi>,
    chips: HashMap<String, hecs::Entity>,
    /// Cached layout for the edge overlay (top-left of each node, indexed like
    /// the model's `nodes`).
    positions: Vec<(f32, f32)>,
    /// `(dep_idx, dependent_idx)` connector edges.
    deps: Vec<(usize, usize)>,
    tree_ready: bool,
    wide: bool,
    s: f32,
    node_w: f32,
    node_h: f32,
}

/// The loader's accumulated state (shared with the renderer through the sink).
struct LoaderState {
    spec: String,
    nodes: Vec<RomNode>,
    node_index: HashMap<String, usize>,
    chip_index: HashMap<String, (usize, usize)>,
    latest: String,
    log: Vec<String>,
    cpu_percent: u32,
    rss_bytes: u64,
    /// The most recently uploaded texture (drawn in the preview panel).
    last_texture: Option<String>,
    /// Whether the resolved DAG topology (deps) is known yet.  Until
    /// [`VisualBootSink::set_dag`] runs, the tree layout can't place dependents
    /// under their parents, so the renderer falls back to a simple stack.
    dag_set: bool,
    started: BootTimer,
    done: bool,
}

impl LoaderState {
    fn new() -> Self {
        Self {
            spec: String::new(),
            nodes: Vec::new(),
            node_index: HashMap::new(),
            chip_index: HashMap::new(),
            latest: String::new(),
            log: Vec::new(),
            cpu_percent: 0,
            rss_bytes: 0,
            last_texture: None,
            dag_set: false,
            started: BootTimer::start(),
            done: false,
        }
    }

    /// Update node phases + chip states from one event.  Node-scoped events
    /// lazily create a minimal node (no chips) so the header/DAG can show a
    /// ROM as soon as it is first observed, before `set_dag` fills in chips.
    fn apply(&mut self, event: &BootEvent) {
        match event {
            BootEvent::ResolveStarted { spec } => self.spec = spec.clone(),
            BootEvent::RomFetchStarted { name, total } => {
                if let Some(node) = self.node_mut(name) {
                    node.phase = RomPhase::Fetching;
                    node.received = 0;
                    node.total = total.unwrap_or(0);
                }
            }
            BootEvent::RomFetchProgress { name, received, total } => {
                if let Some(node) = self.node_mut(name) {
                    node.phase = RomPhase::Fetching;
                    node.received = *received;
                    node.total = *total;
                }
            }
            BootEvent::RomFetched { name, .. } => {
                if let Some(node) = self.node_mut(name) {
                    node.phase = RomPhase::Fetched;
                }
            }
            BootEvent::RomDecompressed { name, .. } => {
                if let Some(node) = self.node_mut(name) {
                    node.phase = RomPhase::Decompressed;
                }
            }
            BootEvent::RomParsed { name, deps, .. } => {
                if let Some(node) = self.node_mut(name) {
                    node.phase = RomPhase::Parsed;
                    node.deps = deps.clone();
                }
            }
            BootEvent::ResourceDecoded { name, .. } => self.chip_set(name, ChipState::Decoded),
            BootEvent::TextureUploaded { name } => {
                self.last_texture = Some(name.clone());
                self.chip_set(name, ChipState::Uploaded);
            }
            BootEvent::GuestCompiling { rom } => {
                if let Some(node) = self.node_mut(rom) {
                    node.phase = RomPhase::Hydrating;
                }
            }
            BootEvent::GuestInstantiated { rom } | BootEvent::StateSpawned { rom, .. } => {
                if let Some(node) = self.node_mut(rom) {
                    node.phase = RomPhase::Done;
                }
            }
            BootEvent::ResourceUsage { cpu_percent, rss_bytes } => {
                self.cpu_percent = *cpu_percent;
                self.rss_bytes = *rss_bytes;
            }
            BootEvent::BootComplete { .. } => self.done = true,
            BootEvent::BootFailed { .. } | BootEvent::ShaderCompiled { .. } => {}
        }
    }

    fn node_mut(&mut self, name: &str) -> Option<&mut RomNode> {
        if !self.node_index.contains_key(name) {
            let idx = self.nodes.len();
            self.node_index.insert(name.to_string(), idx);
            self.nodes.push(RomNode {
                name: name.to_string(),
                phase: RomPhase::Queued,
                chips: Vec::new(),
                received: 0,
                total: 0,
                deps: Vec::new(),
            });
        }
        let idx = *self.node_index.get(name)?;
        self.nodes.get_mut(idx)
    }

    fn chip_set(&mut self, name: &str, state: ChipState) {
        let Some(&(node_idx, chip_idx)) = self.chip_index.get(name) else { return };
        if let Some(chip) = self.nodes.get_mut(node_idx).and_then(|n| n.chips.get_mut(chip_idx)) {
            chip.state = state;
        }
        if let Some(node) = self.nodes.get_mut(node_idx) {
            node.phase = RomPhase::Hydrating;
        }
    }

    /// Overall boot progress `0.0..=1.0`, from the fraction of uploaded sheets
    /// (falls back to done-node fraction when there are no sheets).
    fn progress(&self) -> f32 {
        let (total, done) =
            self.nodes.iter().flat_map(|n| n.chips.iter()).fold((0usize, 0usize), |(t, d), c| {
                (t + 1, d + usize::from(c.state == ChipState::Uploaded))
            });
        if total > 0 {
            return done as f32 / total as f32;
        }
        if self.nodes.is_empty() {
            return 0.0;
        }
        let done_nodes = self.nodes.iter().filter(|n| n.phase == RomPhase::Done).count();
        done_nodes as f32 / self.nodes.len() as f32
    }
}

/// A [`BootSink`] that renders the boot loading screen.  Install its UI with
/// [`VisualBootSink::install`], drive it each boot-loop frame with
/// [`VisualBootSink::sync`], and tear it down with [`VisualBootSink::uninstall`].
pub struct VisualBootSink {
    state: Mutex<LoaderState>,
    /// The entity set + layout cache (only touched on the GL/run-loop thread).
    ui: Mutex<Option<LoaderUi>>,
}

impl VisualBootSink {
    pub fn new() -> Self {
        Self { state: Mutex::new(LoaderState::new()), ui: Mutex::new(None) }
    }

    /// Install the resolved DAG topology (topological `LoadedRoms.order`) and
    /// derive the per-ROM chip list.  Node phases already observed from the
    /// event stream (before this point) are preserved, so the DAG reflects the
    /// fetch/parse progress that happened while the DAG topology wasn't yet
    /// known.
    pub fn set_dag(&self, loaded: &LoadedRoms) {
        let chips_by_rom = collect_chips(loaded);
        let mut state = self.state.lock().expect("boot loader poisoned");

        let mut nodes = Vec::with_capacity(loaded.order.len());
        let mut node_index = HashMap::new();
        let mut chip_index = HashMap::new();
        for entry in &loaded.order {
            let name = entry.name.clone();
            let phase = state
                .node_index
                .get(&name)
                .and_then(|&i| state.nodes.get(i))
                .map(|n| n.phase)
                .unwrap_or(RomPhase::Queued);
            let chips = chips_by_rom.get(&rom_key(entry)).cloned().unwrap_or_default();
            let idx = nodes.len();
            for (chip_idx, chip) in chips.iter().enumerate() {
                chip_index.insert(chip.name.clone(), (idx, chip_idx));
            }
            node_index.insert(name.clone(), idx);
            nodes.push(RomNode {
                name,
                phase,
                chips,
                received: 0,
                total: 0,
                deps: entry.rom.manifest.deps.clone(),
            });
        }
        state.nodes = nodes;
        state.node_index = node_index;
        state.chip_index = chip_index;
        state.dag_set = true;
    }

    /// Spawn the loader's UI entities and register the edge-drawing overlay
    /// hook.  Idempotent.  Called on frame 0 of the boot loop, after
    /// [`crate::Engine::init_gfx`] has loaded the embedded font.
    pub fn install(self: &Arc<Self>, engine: &mut Engine) {
        let mut ui_guard = self.ui.lock().expect("boot loader poisoned");
        if ui_guard.is_some() {
            return;
        }

        let world = &mut engine.world;
        let log = (0..LOG_LINES)
            .map(|_| spawn_text(world, "", 0.55, DIM_TEXT, TextJustify::Left, Z_FOOTER_TEXT))
            .collect();
        let ui = LoaderUi {
            bg: spawn_rect(world, BG, Z_BG),
            header_bg: spawn_rect(world, HEADER_BG, Z_HEADER),
            title: spawn_text(world, "CLASSIC", 0.8, TEXT_COLOR, TextJustify::Left, Z_HEADER_TEXT),
            metrics: spawn_text(world, "", 0.6, DIM_TEXT, TextJustify::Right, Z_HEADER_TEXT),
            footer_bg: spawn_rect(world, HEADER_BG, Z_FOOTER),
            progress_bg: spawn_rect(world, BAR_BG, Z_FOOTER_DETAIL),
            progress_fill: spawn_rect(world, [0.40, 0.85, 0.45, 1.0], Z_FOOTER_DETAIL),
            pct: spawn_text(world, "0%", 0.7, TEXT_COLOR, TextJustify::Left, Z_FOOTER_TEXT),
            latest: spawn_text(world, "", 0.7, TEXT_COLOR, TextJustify::Left, Z_FOOTER_TEXT),
            log,
            preview_bg: spawn_rect(world, NODE_BG, Z_BODY),
            preview_placeholder: spawn_text(
                world,
                "no asset yet",
                0.55,
                DIM_TEXT,
                TextJustify::Left,
                Z_BODY_DETAIL,
            ),
            preview_sprite: spawn_sprite(world, Z_BODY_DETAIL),
            preview_label: spawn_text(
                world,
                "preview",
                0.5,
                DIM_TEXT,
                TextJustify::Left,
                Z_BODY_TEXT,
            ),
            dots_bg: spawn_rect(world, NODE_BG, Z_BODY),
            dots_title: spawn_text(
                world,
                "resources",
                0.55,
                DIM_TEXT,
                TextJustify::Left,
                Z_BODY_TEXT,
            ),
            nodes: HashMap::new(),
            chips: HashMap::new(),
            positions: Vec::new(),
            deps: Vec::new(),
            tree_ready: false,
            wide: true,
            s: 1.0,
            node_w: NODE_W,
            node_h: NODE_H,
        };
        *ui_guard = Some(ui);

        // Draw the DAG connector edges after the render list (the loader's
        // entities are pure UI entities; lines have no entity equivalent).
        let sink = self.clone();
        engine.add_overlay(move |engine| {
            let ui_guard = sink.ui.lock().expect("boot loader poisoned");
            let Some(ui) = ui_guard.as_ref() else { return };
            let Some(gfx) = engine.gfx.as_ref() else { return };
            for (from, to) in &ui.deps {
                let (x0, y0) = ui.positions[*from];
                let (x1, y1) = ui.positions[*to];
                let fx = x0 + ui.node_w / 2.0;
                let fy = y0 + ui.node_h;
                let tx = x1 + ui.node_w / 2.0;
                let ty = y1;
                if ui.wide {
                    if ui.tree_ready {
                        elbow(gfx, fx, fy, tx, ty, EDGE_COLOR);
                    }
                } else {
                    line(gfx, fx, fy, tx, ty, EDGE_COLOR);
                }
            }
        });
    }

    /// Synchronise the loader's UI entities from the current boot state.
    /// Called before [`crate::Engine::frame`] each boot-loop frame.
    pub fn sync(&self, engine: &mut Engine, vw: f32, vh: f32) {
        let state = self.state.lock().expect("boot loader poisoned");
        let mut ui_guard = self.ui.lock().expect("boot loader poisoned");
        let Some(ui) = ui_guard.as_mut() else { return };

        let s = (vw / 1280.0).clamp(0.6, 2.0);
        let pad = PAD * s;
        let header_h = HEADER_H * s;
        let node_w = NODE_W * s;
        let node_h = NODE_H * s;
        let chip_w = CHIP_W * s;
        let chip_gap = CHIP_GAP * s;
        let chip_row_h = CHIP_ROW_H * s;
        let footer_h = FOOTER_H * s;

        // ---- Chrome (background, header, footer, progress, log). ----
        set_rect(engine, ui.bg, 0.0, 0.0, vw, vh);
        set_rect(engine, ui.header_bg, 0.0, 0.0, vw, header_h);
        let title = if state.spec.is_empty() {
            "CLASSIC".to_string()
        } else {
            format!("CLASSIC / {}", state.spec.to_uppercase())
        };
        set_text(engine, ui.title, pad, 8.0 * s, 0.8 * s, &title, TEXT_COLOR);
        let metrics = format!(
            "cpu {}%   rss {:.1} MiB   {:.1}s",
            state.cpu_percent,
            state.rss_bytes as f64 / (1024.0 * 1024.0),
            state.started.elapsed_secs(),
        );
        set_text(engine, ui.metrics, vw - pad, 8.0 * s, 0.6 * s, &metrics, DIM_TEXT);

        let footer_top = vh - footer_h;
        set_rect(engine, ui.footer_bg, 0.0, footer_top, vw, footer_h);
        let bar_x = pad;
        let bar_w = vw - pad * 2.0 - 56.0 * s;
        let bar_y = footer_top + pad;
        let bar_h = 8.0 * s;
        set_rect(engine, ui.progress_bg, bar_x, bar_y, bar_w, bar_h);
        let frac = state.progress().clamp(0.0, 1.0);
        set_rect(engine, ui.progress_fill, bar_x, bar_y, bar_w * frac, bar_h);
        set_text(
            engine,
            ui.pct,
            bar_x + bar_w + 8.0 * s,
            bar_y - 3.0 * s,
            0.7 * s,
            &format!("{:.0}%", frac * 100.0),
            TEXT_COLOR,
        );

        let mut ly = footer_top + pad + bar_h + 8.0 * s;
        set_text(engine, ui.latest, pad, ly, 0.7 * s, &state.latest, TEXT_COLOR);
        ly += 20.0 * s;
        let mut log_i = 0;
        for line in state.log.iter().rev().take(LOG_LINES - 1).rev() {
            if log_i < ui.log.len() {
                set_text(engine, ui.log[log_i], pad, ly, 0.55 * s, line, DIM_TEXT);
            }
            log_i += 1;
            ly += 15.0 * s;
        }
        while log_i < ui.log.len() {
            set_text(engine, ui.log[log_i], 0.0, 0.0, 0.55 * s, "", DIM_TEXT);
            log_i += 1;
        }

        // ---- Body layout. ----
        let body_top = header_h + pad;
        let body_bottom = footer_top;
        let body_h = (body_bottom - body_top).max(60.0);
        let wide = vw >= WIDE_BREAKPOINT;
        let tree_ready = state.dag_set || state.nodes.iter().any(|n| !n.deps.is_empty());

        ui.s = s;
        ui.node_w = node_w;
        ui.node_h = node_h;
        ui.wide = wide;
        ui.tree_ready = tree_ready;

        let positions: Vec<(f32, f32)> = if wide {
            let dag_w = vw * DAG_FRACTION - pad;
            if tree_ready {
                tree_layout(&state, pad, body_top, dag_w, body_h, node_w, node_h)
            } else {
                stacked_layout(&state, pad, body_top, dag_w, body_h, node_w, node_h, s)
            }
        } else {
            narrow_stacked(&state, body_top, body_h, node_w, node_h, vw, s)
        };

        // Reconcile the DAG node entities and position them.
        for (i, node) in state.nodes.iter().enumerate() {
            if !ui.nodes.contains_key(&node.name) {
                ui.nodes.insert(node.name.clone(), spawn_node(engine));
            }
            let nu = ui.nodes.get_mut(&node.name).expect("node spawned");
            let (x, y) = positions[i];
            set_rect(engine, nu.rect, x, y, node_w, node_h);
            set_rect(engine, nu.phase_bar, x, y, node_w, 3.0 * s);
            set_rect_color(engine, nu.phase_bar, node.phase.color());
            set_text(engine, nu.name, x + PAD * s, y + 8.0 * s, 0.7 * s, &node.name, TEXT_COLOR);
            let fetch = if node.phase == RomPhase::Fetching {
                if node.total > 0 {
                    format!("  {} / {} B", node.received, node.total)
                } else {
                    format!("  {} B", node.received)
                }
            } else {
                String::new()
            };
            set_text(
                engine,
                nu.phase,
                x + PAD * s,
                y + 26.0 * s,
                0.55 * s,
                &format!("{}{}", node.phase.label(), fetch),
                node.phase.color(),
            );
        }
        ui.nodes.retain(|name, nu| {
            if state.node_index.contains_key(name) {
                true
            } else {
                despawn_node(engine, nu);
                false
            }
        });

        // Reconcile the chip entities (spawn/remove), coloured by state.
        for node in state.nodes.iter() {
            let rc = rom_color(&node.name);
            for chip in &node.chips {
                if !ui.chips.contains_key(&chip.name) {
                    ui.chips.insert(
                        chip.name.clone(),
                        spawn_rect(&mut engine.world, chip_color(rc, chip.state), Z_BODY_DETAIL),
                    );
                }
                if let Some(&ce) = ui.chips.get(&chip.name) {
                    set_rect_color(engine, ce, chip_color(rc, chip.state));
                }
            }
        }
        ui.chips.retain(|name, e| {
            if state.chip_index.contains_key(name) {
                true
            } else {
                despawn(engine, *e);
                false
            }
        });

        // ---- Wide vs narrow body. ----
        if wide {
            let dag_right = vw * DAG_FRACTION;
            let right_x = dag_right + pad;
            let right_w = vw - pad - right_x;
            let mid_y = body_top + body_h * 0.5;

            // Preview (top-right).
            let preview_h = (mid_y - body_top - pad).max(40.0);
            set_rect(engine, ui.preview_bg, right_x, body_top, right_w, preview_h);
            set_enabled(engine, ui.preview_bg, true);
            set_enabled(engine, ui.preview_label, true);
            let preview_label = state.last_texture.as_deref().unwrap_or("preview").to_string();
            set_text(
                engine,
                ui.preview_label,
                right_x + PAD * s,
                body_top + preview_h - 20.0 * s,
                0.5 * s,
                &preview_label,
                DIM_TEXT,
            );
            let tex = state.last_texture.as_deref().and_then(|n| {
                engine.gfx.as_ref().and_then(|g| g.textures.get(n).map(|t| (n, t.size)))
            });
            match tex {
                Some((name, (tw, th))) => {
                    let scale = (right_w / tw.max(1) as f32).min(preview_h / th.max(1) as f32);
                    let dw = tw as f32 * scale;
                    let dh = th as f32 * scale;
                    let px = right_x + (right_w - dw) / 2.0;
                    let py = body_top + (preview_h - dh) / 2.0;
                    set_sprite(engine, ui.preview_sprite, px, py, dw, dh, name);
                    set_enabled(engine, ui.preview_sprite, true);
                    set_enabled(engine, ui.preview_placeholder, false);
                }
                None => {
                    set_text(
                        engine,
                        ui.preview_placeholder,
                        right_x + PAD * s,
                        body_top + PAD * s,
                        0.55 * s,
                        "no asset yet",
                        DIM_TEXT,
                    );
                    set_enabled(engine, ui.preview_placeholder, true);
                    set_enabled(engine, ui.preview_sprite, false);
                }
            }

            // Dots (bottom-right): panel + legend + chip grid.
            let dots_top = mid_y + pad;
            let dots_h = (body_bottom - dots_top).max(40.0);
            set_rect(engine, ui.dots_bg, right_x, dots_top, right_w, dots_h);
            set_enabled(engine, ui.dots_bg, true);
            set_text(
                engine,
                ui.dots_title,
                right_x + PAD * s,
                dots_top + PAD * s,
                0.55 * s,
                "resources",
                DIM_TEXT,
            );
            set_enabled(engine, ui.dots_title, true);

            let legend_y = dots_top + PAD * s + 20.0 * s;
            let mut lx = right_x + PAD * s;
            for node in state.nodes.iter() {
                if let Some(nu) = ui.nodes.get(&node.name) {
                    let rc = rom_color(&node.name);
                    set_rect(engine, nu.legend_swatch, lx, legend_y + 2.0 * s, chip_w, chip_w);
                    set_rect_color(engine, nu.legend_swatch, rc);
                    set_text(engine, nu.legend_name, lx, legend_y, 0.5 * s, &node.name, DIM_TEXT);
                    set_enabled(engine, nu.legend_swatch, true);
                    set_enabled(engine, nu.legend_name, true);
                    lx += chip_w + chip_gap;
                    lx += measure_legend(engine, &node.name, 0.5 * s) + chip_gap * 4.0;
                }
            }

            let mut cx = right_x + PAD * s;
            let mut cy = legend_y + 24.0 * s;
            let max_x = right_x + right_w - PAD * s;
            for node in state.nodes.iter() {
                for chip in &node.chips {
                    if let Some(&ce) = ui.chips.get(&chip.name) {
                        set_rect(engine, ce, cx, cy, chip_w, chip_w);
                        cx += chip_w + chip_gap;
                        if cx + chip_w > max_x {
                            cx = right_x + PAD * s;
                            cy += chip_w + chip_gap;
                        }
                    }
                }
                cx += chip_gap * 2.0;
            }
        } else {
            // Narrow: hide the preview/dots panels, show chip rows under the
            // stacked DAG.
            set_enabled(engine, ui.preview_bg, false);
            set_enabled(engine, ui.preview_placeholder, false);
            set_enabled(engine, ui.preview_sprite, false);
            set_enabled(engine, ui.preview_label, false);
            set_enabled(engine, ui.dots_bg, false);
            set_enabled(engine, ui.dots_title, false);

            let n = state.nodes.len().max(1) as f32;
            let node_gap =
                ((body_h - n * node_h) / (n - 1.0).max(1.0)).clamp(6.0 * s, (pad + 8.0) * s);
            let chip_y0 = body_top + node_gap * (state.nodes.len() as f32 + 1.0) + pad;
            let mut cy = chip_y0;
            for node in state.nodes.iter() {
                if let Some(nu) = ui.nodes.get(&node.name) {
                    if node.chips.is_empty() {
                        set_enabled(engine, nu.legend_swatch, false);
                        set_enabled(engine, nu.legend_name, false);
                        continue;
                    }
                    set_enabled(engine, nu.legend_swatch, false);
                    set_enabled(engine, nu.legend_name, true);
                    set_text(
                        engine,
                        nu.legend_name,
                        pad,
                        cy + 1.0,
                        0.6 * s,
                        &node.name,
                        TEXT_COLOR,
                    );
                    let mut cx = pad + measure_legend(engine, &node.name, 0.6 * s) + pad;
                    for chip in &node.chips {
                        if let Some(&ce) = ui.chips.get(&chip.name) {
                            set_rect(engine, ce, cx, cy + 3.0 * s, chip_w, chip_w);
                            cx += chip_w + chip_gap;
                        }
                    }
                    cy += chip_row_h + 6.0 * s;
                }
            }
        }

        ui.positions = positions;
        ui.deps.clear();
        for (i, node) in state.nodes.iter().enumerate() {
            for dep_name in &node.deps {
                if let Some(&di) = state.node_index.get(dep_name) {
                    ui.deps.push((di, i));
                }
            }
        }
    }

    /// Despawn the loader's UI entities (once boot finishes, before the
    /// editor's full HUD installs).  The overlay hook becomes a cheap no-op.
    pub fn uninstall(&self, engine: &mut Engine) {
        let mut ui_guard = self.ui.lock().expect("boot loader poisoned");
        let Some(ui) = ui_guard.take() else { return };
        despawn(engine, ui.bg);
        despawn(engine, ui.header_bg);
        despawn(engine, ui.title);
        despawn(engine, ui.metrics);
        despawn(engine, ui.footer_bg);
        despawn(engine, ui.progress_bg);
        despawn(engine, ui.progress_fill);
        despawn(engine, ui.pct);
        despawn(engine, ui.latest);
        for e in ui.log {
            despawn(engine, e);
        }
        despawn(engine, ui.preview_bg);
        despawn(engine, ui.preview_placeholder);
        despawn(engine, ui.preview_sprite);
        despawn(engine, ui.preview_label);
        despawn(engine, ui.dots_bg);
        despawn(engine, ui.dots_title);
        for (_, nu) in ui.nodes {
            despawn_node(engine, &nu);
        }
        for (_, e) in ui.chips {
            despawn(engine, e);
        }
    }
}

impl Default for VisualBootSink {
    fn default() -> Self {
        Self::new()
    }
}

impl BootSink for VisualBootSink {
    fn on_event(&self, event: BootEvent) {
        let mut state = self.state.lock().expect("boot loader poisoned");
        let desc = event.describe();
        state.latest = desc.clone();
        state.log.push(desc);
        if state.log.len() > 32 {
            state.log.remove(0);
        }
        // Apply immediately so the DAG appears as events stream in (node
        // phases are lazily tracked before `set_dag` fills in the chip list).
        state.apply(&event);
    }
}

/// The resolver-side key a ROM's sheets are grouped under in the boot plan.
fn rom_key(entry: &LoadedRom) -> String {
    if entry.rom.manifest.entrypoint.is_empty() {
        "root".to_string()
    } else {
        entry.rom.manifest.entrypoint.clone()
    }
}

/// Build the per-ROM chip list (one chip per unique texture sheet / basis job,
/// in plan order) by walking a throwaway [`crate::boot::BootPlan`], so the chip
/// names exactly match the keys the decode/upload events emit.
fn collect_chips(loaded: &LoadedRoms) -> HashMap<String, Vec<Chip>> {
    let engine = crate::Engine::new();
    let sink = classic_rom::NullBootSink;
    let plan = engine.begin_boot(loaded, &sink);

    let mut out: HashMap<String, Vec<Chip>> = HashMap::new();
    for step in &plan.steps {
        if let BootStep::Decode { rom, key, .. } = step {
            out.entry(rom.clone())
                .or_default()
                .push(Chip { name: key.clone(), state: ChipState::Queued });
        }
    }
    for job in &plan.basis_jobs {
        out.entry(job.rom.clone())
            .or_default()
            .push(Chip { name: job.keys[0].clone(), state: ChipState::Queued });
    }
    out
}

// ---- entity spawn/update helpers ------------------------------------------

fn spawn_rect(world: &mut hecs::World, color: [f32; 4], z: f32) -> hecs::Entity {
    world.spawn((
        Transform::new(Vec3::new(0.0, 0.0, z), Vec3::ONE),
        RectRender { color, ignore_cam: true },
    ))
}

fn spawn_text(
    world: &mut hecs::World,
    text: &str,
    scale: f32,
    color: [f32; 4],
    justify: TextJustify,
    z: f32,
) -> hecs::Entity {
    world.spawn((
        Transform::new(Vec3::new(0.0, 0.0, z), Vec3::new(scale, scale, 1.0)),
        SdfTextRender {
            atlas_name: DEFAULT_SDF_FONT.into(),
            color,
            outline_color: [0.0, 0.0, 0.0, 0.0],
            outline_width: 0.0,
            ignore_cam: true,
            text: text.to_string(),
            justify,
            weight: 0.0,
            gamma: 1.0,
        },
    ))
}

fn spawn_sprite(world: &mut hecs::World, z: f32) -> hecs::Entity {
    world.spawn((
        Transform::new(Vec3::new(0.0, 0.0, z), Vec3::ONE),
        SpriteRender {
            position: Vec3::ZERO,
            scale: Vec3::ONE,
            texture: String::new(),
            ignore_cam: true,
            frame: 0.0,
            frame_name: None,
            tile_set_size: Vec2::ONE,
            anchor: Vec2::ZERO,
        },
        UiNode { kind: UiKind::Sprite, ..UiNode::default() },
    ))
}

fn spawn_node(engine: &mut Engine) -> NodeUi {
    let world = &mut engine.world;
    NodeUi {
        rect: spawn_rect(world, NODE_BG, Z_BODY),
        phase_bar: spawn_rect(world, [0.0, 0.0, 0.0, 1.0], Z_BODY_DETAIL),
        name: spawn_text(world, "", 0.7, TEXT_COLOR, TextJustify::Left, Z_BODY_TEXT),
        phase: spawn_text(world, "", 0.55, [0.0, 0.0, 0.0, 1.0], TextJustify::Left, Z_BODY_TEXT),
        legend_swatch: spawn_rect(world, [0.0, 0.0, 0.0, 1.0], Z_BODY_DETAIL),
        legend_name: spawn_text(world, "", 0.5, DIM_TEXT, TextJustify::Left, Z_BODY_TEXT),
    }
}

fn despawn_node(engine: &mut Engine, nu: &NodeUi) {
    despawn(engine, nu.rect);
    despawn(engine, nu.phase_bar);
    despawn(engine, nu.name);
    despawn(engine, nu.phase);
    despawn(engine, nu.legend_swatch);
    despawn(engine, nu.legend_name);
}

fn despawn(engine: &mut Engine, e: hecs::Entity) {
    let _ = engine.world.despawn(e);
}

fn set_rect(engine: &mut Engine, e: hecs::Entity, x: f32, y: f32, w: f32, h: f32) {
    if let Ok(mut tf) = engine.world.get::<&mut Transform>(e) {
        tf.position = Vec3::new(x, y, tf.position.z);
        tf.scale = Vec3::new(w, h, 1.0);
    }
}

fn set_rect_color(engine: &mut Engine, e: hecs::Entity, color: [f32; 4]) {
    if let Ok(mut r) = engine.world.get::<&mut RectRender>(e) {
        r.color = color;
    }
}

fn set_text(
    engine: &mut Engine,
    e: hecs::Entity,
    x: f32,
    y: f32,
    scale: f32,
    text: &str,
    color: [f32; 4],
) {
    if let Ok(mut tf) = engine.world.get::<&mut Transform>(e) {
        tf.position = Vec3::new(x, y, tf.position.z);
        tf.scale = Vec3::new(scale, scale, 1.0);
    }
    if let Ok(mut s) = engine.world.get::<&mut SdfTextRender>(e) {
        s.text = text.to_string();
        s.color = color;
    }
}

fn set_sprite(engine: &mut Engine, e: hecs::Entity, x: f32, y: f32, w: f32, h: f32, texture: &str) {
    if let Ok(mut tf) = engine.world.get::<&mut Transform>(e) {
        tf.position = Vec3::new(x, y, tf.position.z);
    }
    if let Ok(mut n) = engine.world.get::<&mut UiNode>(e) {
        n.size = Vec2::new(w, h);
    }
    if let Ok(mut s) = engine.world.get::<&mut SpriteRender>(e) {
        s.texture = texture.to_string();
    }
}

fn set_enabled(engine: &mut Engine, e: hecs::Entity, enabled: bool) {
    if enabled {
        let _ = engine.world.remove::<(Disabled,)>(e);
    } else {
        let _ = engine.world.insert(e, (Disabled,));
    }
}

/// Measured width of a legend/chip-row label, used to advance the cursor after
/// a ROM name.  Falls back to a glyph-count estimate if the font isn't loaded.
fn measure_legend(engine: &Engine, text: &str, scale: f32) -> f32 {
    engine
        .sdf_fonts
        .get(DEFAULT_SDF_FONT)
        .map(|font| {
            classic_core::sdf_builder::build_sdf_glyph_buffer(
                font,
                text,
                scale,
                TextJustify::Left,
                0.0,
            )
            .text_width
        })
        .unwrap_or(text.len() as f32 * 18.0 * scale)
}

/// Draw a screen-space line strip of two points.
fn line(gfx: &Gfx, x0: f32, y0: f32, x1: f32, y1: f32, color: [f32; 4]) {
    let verts = [x0, y0, 0.0, x1, y1, 0.0];
    let buf = GlBuffer::from_slice(&gfx.gl, glow::ARRAY_BUFFER, &verts, glow::STREAM_DRAW);
    gfx.draw_line_strip(&buf, 0, 2, &Mat4::IDENTITY, &Mat4::IDENTITY, &color);
}

/// Draw an elbow tree connector (down, across, down) from `(x0, y0)` to
/// `(x1, y1)`.
fn elbow(gfx: &Gfx, x0: f32, y0: f32, x1: f32, y1: f32, color: [f32; 4]) {
    let mid_y = (y0 + y1) / 2.0;
    let verts = [x0, y0, 0.0, x0, mid_y, 0.0, x1, mid_y, 0.0, x1, y1, 0.0];
    let buf = GlBuffer::from_slice(&gfx.gl, glow::ARRAY_BUFFER, &verts, glow::STREAM_DRAW);
    gfx.draw_line_strip(&buf, 0, 4, &Mat4::IDENTITY, &Mat4::IDENTITY, &color);
}

/// A simple vertical stack of the DAG nodes (centered), used before the
/// resolved topology is known (`set_dag` hasn't run yet), when the tree layout
/// can't place dependents under their parents.
#[allow(clippy::too_many_arguments)]
fn stacked_layout(
    state: &LoaderState,
    x0: f32,
    y0: f32,
    w: f32,
    h: f32,
    node_w: f32,
    node_h: f32,
    s: f32,
) -> Vec<(f32, f32)> {
    let n = state.nodes.len();
    if n == 0 {
        return Vec::new();
    }
    let center_x = x0 + w / 2.0 - node_w / 2.0;
    let gap =
        ((h - n as f32 * node_h) / (n.saturating_sub(1) as f32).max(1.0)).clamp(6.0 * s, 20.0 * s);
    (0..n).map(|i| (center_x, y0 + i as f32 * (node_h + gap))).collect()
}

/// The narrow stacked layout: centered, with a gap clamped so the DAG stays
/// readable on short windows.
#[allow(clippy::too_many_arguments)]
fn narrow_stacked(
    state: &LoaderState,
    body_top: f32,
    body_h: f32,
    node_w: f32,
    node_h: f32,
    vw: f32,
    s: f32,
) -> Vec<(f32, f32)> {
    let n = state.nodes.len();
    if n == 0 {
        return Vec::new();
    }
    let center_x = vw / 2.0 - node_w / 2.0;
    let gap = ((body_h - n as f32 * node_h) / (n.saturating_sub(1) as f32).max(1.0))
        .clamp(6.0 * s, 20.0 * s);
    (0..n).map(|i| (center_x, body_top + i as f32 * (node_h + gap))).collect()
}

/// Compute a layered tree layout for the DAG: leaf nodes (no deps) sit at the
/// top, distributed evenly; each dependent is centered under its parents.
/// Returns the top-left position of each node (indexed like `state.nodes`).
fn tree_layout(
    state: &LoaderState,
    x0: f32,
    y0: f32,
    w: f32,
    h: f32,
    node_w: f32,
    node_h: f32,
) -> Vec<(f32, f32)> {
    let n = state.nodes.len();
    if n == 0 {
        return Vec::new();
    }

    // Depth (level) per node: leaves are 0, a dependent is max(dep depth) + 1.
    // `state.nodes` is topological (deps first), so one pass suffices.
    let mut levels = vec![0usize; n];
    let mut max_level = 0usize;
    for i in 0..n {
        let mut lvl = 0usize;
        for dep in &state.nodes[i].deps {
            if let Some(&di) = state.node_index.get(dep) {
                lvl = lvl.max(levels[di] + 1);
            }
        }
        levels[i] = lvl;
        max_level = max_level.max(lvl);
    }
    let mut by_level: Vec<Vec<usize>> = vec![Vec::new(); max_level + 1];
    for i in 0..n {
        by_level[levels[i]].push(i);
    }

    let level_h = h / (max_level as f32 + 1.0);
    let mut positions = vec![(0.0f32, 0.0f32); n];
    for (level, in_level) in by_level.iter().enumerate() {
        let y = y0 + level as f32 * level_h + (level_h - node_h) / 2.0;
        if level == 0 {
            let count = in_level.len().max(1) as f32;
            for (j, &ni) in in_level.iter().enumerate() {
                let center = x0 + w * (j as f32 + 0.5) / count;
                positions[ni] = (center - node_w / 2.0, y);
            }
        } else {
            for &ni in in_level {
                let parents: Vec<usize> = state.nodes[ni]
                    .deps
                    .iter()
                    .filter_map(|d| state.node_index.get(d).copied())
                    .collect();
                let center = if parents.is_empty() {
                    x0 + w / 2.0
                } else {
                    parents.iter().map(|&pi| positions[pi].0 + node_w / 2.0).sum::<f32>()
                        / parents.len() as f32
                };
                positions[ni] = (center - node_w / 2.0, y);
            }
        }
    }
    positions
}

/// A stable, per-ROM colour picked from a small palette (used to colour-code
/// the resource dots by their owning ROM).
fn rom_color(name: &str) -> [f32; 4] {
    let mut hash: u32 = 0;
    for b in name.bytes() {
        hash = hash.wrapping_mul(31).wrapping_add(b as u32);
    }
    const PALETTE: [[f32; 3]; 6] = [
        [0.36, 0.72, 0.95],
        [0.45, 0.85, 0.50],
        [0.85, 0.55, 0.95],
        [0.95, 0.72, 0.35],
        [0.95, 0.42, 0.52],
        [0.42, 0.80, 0.80],
    ];
    let c = PALETTE[(hash % PALETTE.len() as u32) as usize];
    [c[0], c[1], c[2], 1.0]
}

/// Shade a ROM's colour by the chip's decode/upload state (queued = dim,
/// decoded = mid, uploaded = full).
fn chip_color(rom: [f32; 4], state: ChipState) -> [f32; 4] {
    let k = match state {
        ChipState::Queued => 0.25,
        ChipState::Decoded => 0.6,
        ChipState::Uploaded => 1.0,
    };
    [rom[0] * k, rom[1] * k, rom[2] * k, 1.0]
}
