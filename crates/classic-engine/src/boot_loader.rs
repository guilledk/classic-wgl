//! Boot loading screen: a GL-only overlay driven by the [`BootSink`] stream.
//!
//! [`VisualBootSink`] records the boot event stream into a small mutable state
//! (DAG node phases, per-sheet resource chips, a log scroller, live CPU/RSS)
//! and renders it with the low-level [`classic_gfx::Gfx`] draw calls
//! (`draw_rect` / `draw_line_strip` / `draw_sdf`).  It draws *before* the
//! engine is booted (in the desktop/web boot loops), so it cannot use the
//! retained-mode [`crate::ui::UIManager`] (which is only installed at the end
//! of boot) — but it reuses the same SDF text primitives
//! (`build_sdf_glyph_buffer` + `draw_sdf` + the embedded DejaVu Sans atlas).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use classic_core::components::{TextJustify, DEFAULT_SDF_FONT};
use classic_core::sdf_builder::build_sdf_glyph_buffer;
use classic_core::types::SdfFontMetrics;
use classic_gfx::{Gfx, GlBuffer, RenderSettings, SpriteRegion};
use classic_rom::{BootEvent, BootSink, LoadedRom, LoadedRoms};
use glam::{Mat4, Vec3};

use crate::boot::BootStep;

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
    started: Instant,
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
            started: Instant::now(),
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

/// A [`BootSink`] that renders the boot loading screen.  Draw it each boot-loop
/// frame with [`VisualBootSink::draw`]; populate the DAG topology with
/// [`VisualBootSink::set_dag`] once the resolved [`LoadedRoms`] is available.
pub struct VisualBootSink {
    state: Mutex<LoaderState>,
}

impl VisualBootSink {
    pub fn new() -> Self {
        Self { state: Mutex::new(LoaderState::new()) }
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

    /// Render the loading screen into the engine's current GL context, sizing
    /// the GL viewport to `(vw, vh)` first (the boot loop runs before
    /// [`crate::Engine::frame`], so the gfx viewport is not yet resized).
    pub fn draw(&self, engine: &mut crate::Engine, vw: f32, vh: f32) {
        let state = self.state.lock().expect("boot loader poisoned");
        let Some(gfx) = engine.gfx.as_mut() else { return };
        let Some(font) = engine.sdf_fonts.get(DEFAULT_SDF_FONT).cloned() else { return };

        gfx.resize(vw, vh);
        gfx.begin_frame();

        // Responsive scale: 1.0 at a 1280px-wide viewport, clamped so text
        // stays legible on small windows and doesn't blow up on large ones.
        let s = (vw / 1280.0).clamp(0.6, 2.0);
        let pad = PAD * s;
        let header_h = HEADER_H * s;
        let node_w = NODE_W * s;
        let node_h = NODE_H * s;
        let chip_w = CHIP_W * s;
        let chip_gap = CHIP_GAP * s;
        let chip_row_h = CHIP_ROW_H * s;
        let footer_h = FOOTER_H * s;

        // Background.
        rect(gfx, 0.0, 0.0, vw, vh, BG);

        // Header: title (left) + live metrics (right).
        rect(gfx, 0.0, 0.0, vw, header_h, HEADER_BG);
        let title = if state.spec.is_empty() {
            "CLASSIC".to_string()
        } else {
            format!("CLASSIC / {}", state.spec.to_uppercase())
        };
        draw_text(gfx, &font, pad, 8.0 * s, &title, 0.8 * s, TEXT_COLOR);

        let metrics = format!(
            "cpu {}%   rss {:.1} MiB   {:.1}s",
            state.cpu_percent,
            state.rss_bytes as f64 / (1024.0 * 1024.0),
            state.started.elapsed().as_secs_f32(),
        );
        let mw = measure_text(&font, &metrics, 0.6 * s);
        draw_text(gfx, &font, (vw - pad - mw).max(pad), 8.0 * s, &metrics, 0.6 * s, DIM_TEXT);

        // Body: (wide) a DAG tree on the left with an asset preview + per-ROM
        // resource dots on the right; (narrow) a simple stacked DAG.
        let body_top = header_h + pad;
        let body_bottom = vh - footer_h;
        let body_h = (body_bottom - body_top).max(60.0);

        if vw >= WIDE_BREAKPOINT {
            // ---- Wide: two columns ----
            let dag_right = vw * DAG_FRACTION;
            let dag_w = dag_right - pad;
            let right_x = dag_right + pad;
            let right_w = vw - pad - right_x;
            let mid_y = body_top + body_h * 0.5;

            // DAG layout: a layered tree (deps fan into the root) once the
            // topology is known — from the parsed manifests (`set_dag`) or the
            // deps surfaced in `RomParsed` events — or a simple vertical stack
            // before any deps have been discovered.
            let tree_ready = state.dag_set || state.nodes.iter().any(|n| !n.deps.is_empty());
            let positions = if tree_ready {
                tree_layout(&state, pad, body_top, dag_w, body_h, node_w, node_h)
            } else {
                stacked_layout(&state, pad, body_top, dag_w, body_h, node_w, node_h, s)
            };
            if tree_ready {
                for (i, node) in state.nodes.iter().enumerate() {
                    for dep_name in &node.deps {
                        let Some(&di) = state.node_index.get(dep_name) else { continue };
                        let (dx, dy) = positions[di];
                        let (sx, sy) = positions[i];
                        elbow(
                            gfx,
                            dx + node_w / 2.0,
                            dy + node_h,
                            sx + node_w / 2.0,
                            sy,
                            EDGE_COLOR,
                        );
                    }
                }
            }
            for (i, node) in state.nodes.iter().enumerate() {
                let (x, y) = positions[i];
                draw_node(gfx, &font, x, y, node_w, node_h, node, s);
            }

            // Right column: preview (top) + resource dots (bottom).
            let preview_h = (mid_y - body_top - pad).max(40.0);
            draw_preview(
                gfx,
                &font,
                right_x,
                body_top,
                right_w,
                preview_h,
                state.last_texture.as_deref(),
                s,
            );
            let dots_top = mid_y + pad;
            let dots_h = (body_bottom - dots_top).max(40.0);
            draw_dots(
                gfx,
                &font,
                right_x,
                dots_top,
                right_w,
                dots_h,
                &state.nodes,
                chip_w,
                chip_gap,
                s,
            );
        } else {
            // ---- Narrow: stacked DAG + chip rows ----
            let center_x = vw / 2.0 - node_w / 2.0;
            let node_gap = (body_h - (state.nodes.len() as f32) * node_h)
                / (state.nodes.len().saturating_sub(1) as f32).max(1.0);
            let node_gap = node_gap.clamp(6.0 * s, (pad + 8.0) * s);

            let mut node_positions: Vec<(f32, f32)> = Vec::with_capacity(state.nodes.len());
            for (i, node) in state.nodes.iter().enumerate() {
                let y = body_top + i as f32 * (node_h + node_gap);
                node_positions.push((center_x, y));
                draw_node(gfx, &font, center_x, y, node_w, node_h, node, s);
            }
            for (i, node) in state.nodes.iter().enumerate() {
                for dep_name in &node.deps {
                    let Some(&di) = state.node_index.get(dep_name) else { continue };
                    let Some(&(dx, dy)) = node_positions.get(di) else { continue };
                    let (sx, sy) = node_positions[i];
                    line(gfx, dx + node_w / 2.0, dy + node_h, sx + node_w / 2.0, sy, EDGE_COLOR);
                }
            }

            // Chip rows (one per ROM), colour-coded by ROM.
            let chip_y0 = body_top + (node_h + node_gap) * state.nodes.len() as f32 + pad;
            let mut cy = chip_y0;
            for node in &state.nodes {
                if node.chips.is_empty() {
                    continue;
                }
                let rc = rom_color(&node.name);
                draw_text(gfx, &font, pad, cy + 1.0, &node.name, 0.6 * s, TEXT_COLOR);
                let mut cx = pad + measure_text(&font, &node.name, 0.6 * s) + pad;
                for chip in &node.chips {
                    rect(gfx, cx, cy + 3.0 * s, chip_w, chip_w, chip_color(rc, chip.state));
                    cx += chip_w + chip_gap;
                }
                cy += chip_row_h + 6.0 * s;
            }
        }

        // Footer: global progress bar + latest event + log scroller.
        let footer_top = vh - footer_h;
        rect(gfx, 0.0, footer_top, vw, footer_h, HEADER_BG);

        let bar_x = pad;
        let bar_w = vw - pad * 2.0 - 56.0 * s;
        let bar_y = footer_top + pad;
        let bar_h = 8.0 * s;
        rect(gfx, bar_x, bar_y, bar_w, bar_h, BAR_BG);
        let frac = state.progress().clamp(0.0, 1.0);
        rect(gfx, bar_x, bar_y, bar_w * frac, bar_h, [0.40, 0.85, 0.45, 1.0]);
        let pct = format!("{:.0}%", frac * 100.0);
        draw_text(gfx, &font, bar_x + bar_w + 8.0 * s, bar_y - 3.0 * s, &pct, 0.7 * s, TEXT_COLOR);

        let mut ly = footer_top + pad + bar_h + 8.0 * s;
        draw_text(gfx, &font, pad, ly, &state.latest, 0.7 * s, TEXT_COLOR);
        ly += 20.0 * s;
        for line in state.log.iter().rev().take(LOG_LINES - 1).rev() {
            draw_text(gfx, &font, pad, ly, line, 0.55 * s, DIM_TEXT);
            ly += 15.0 * s;
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

/// Draw a filled screen-space rectangle.
fn rect(gfx: &Gfx, x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) {
    let model =
        Mat4::from_translation(Vec3::new(x, y, 0.0)) * Mat4::from_scale(Vec3::new(w, h, 1.0));
    gfx.draw_rect(&model, &Mat4::IDENTITY, &color, true);
}

/// Draw a screen-space line strip of two points.
fn line(gfx: &Gfx, x0: f32, y0: f32, x1: f32, y1: f32, color: [f32; 4]) {
    let verts = [x0, y0, 0.0, x1, y1, 0.0];
    let buf = GlBuffer::from_slice(&gfx.gl, glow::ARRAY_BUFFER, &verts, glow::STREAM_DRAW);
    gfx.draw_line_strip(&buf, 0, 2, &Mat4::IDENTITY, &Mat4::IDENTITY, &color);
}

/// Draw one DAG node box (name + phase label + a mini progress bar).
#[allow(clippy::too_many_arguments)]
fn draw_node(
    gfx: &Gfx,
    font: &SdfFontMetrics,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    node: &RomNode,
    s: f32,
) {
    rect(gfx, x, y, w, h, NODE_BG);
    let phase_color = node.phase.color();
    rect(gfx, x, y, w, 3.0 * s, phase_color);

    let fetch = if node.phase == RomPhase::Fetching {
        if node.total > 0 {
            format!("  {} / {} B", node.received, node.total)
        } else {
            format!("  {} B", node.received)
        }
    } else {
        String::new()
    };
    draw_text(gfx, font, x + PAD * s, y + 8.0 * s, &node.name, 0.7 * s, TEXT_COLOR);
    draw_text(
        gfx,
        font,
        x + PAD * s,
        y + 26.0 * s,
        &format!("{}{}", node.phase.label(), fetch),
        0.55 * s,
        phase_color,
    );
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

/// Draw an elbow tree connector (down, across, down) from `(x0, y0)` to
/// `(x1, y1)`.
fn elbow(gfx: &Gfx, x0: f32, y0: f32, x1: f32, y1: f32, color: [f32; 4]) {
    let mid_y = (y0 + y1) / 2.0;
    let verts = [x0, y0, 0.0, x0, mid_y, 0.0, x1, mid_y, 0.0, x1, y1, 0.0];
    let buf = GlBuffer::from_slice(&gfx.gl, glow::ARRAY_BUFFER, &verts, glow::STREAM_DRAW);
    gfx.draw_line_strip(&buf, 0, 4, &Mat4::IDENTITY, &Mat4::IDENTITY, &color);
}

/// Draw the asset preview pane: the most recently uploaded texture, fit to the
/// pane preserving aspect ratio (or a placeholder before the first upload).
#[allow(clippy::too_many_arguments)]
fn draw_preview(
    gfx: &Gfx,
    font: &SdfFontMetrics,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    texture_name: Option<&str>,
    s: f32,
) {
    rect(gfx, x, y, w, h, NODE_BG);
    let label = texture_name.unwrap_or("preview");
    let dims = texture_name.and_then(|n| gfx.textures.get(n)).map(|t| t.size);
    match dims {
        Some((tw, th)) => {
            let scale = (w / tw.max(1) as f32).min(h / th.max(1) as f32);
            let dw = tw as f32 * scale;
            let dh = th as f32 * scale;
            let px = x + (w - dw) / 2.0;
            let py = y + (h - dh) / 2.0;
            let model = Mat4::from_translation(Vec3::new(px, py, 0.0))
                * Mat4::from_scale(Vec3::new(dw, dh, 1.0));
            let settings = RenderSettings {
                ambient: [1.0, 1.0, 1.0],
                light_dir: [0.0, 0.0, 1.0],
                light_color: [1.0, 1.0, 1.0],
                depth_span: [0.0, 1.0],
                ppm: 64.0,
                shadow: None,
            };
            gfx.draw_sprite(
                &model,
                &Mat4::IDENTITY,
                texture_name.unwrap(),
                SpriteRegion::Grid { frame: 0.0, tile_set_size: [1.0, 1.0] },
                true,
                1.0,
                &settings,
            );
        }
        None => {
            draw_text(gfx, font, x + PAD * s, y + PAD * s, "no asset yet", 0.55 * s, DIM_TEXT);
        }
    }
    draw_text(gfx, font, x + PAD * s, y + h - 20.0 * s, label, 0.5 * s, DIM_TEXT);
}

/// Draw the per-ROM resource dots (one square per sheet, colour-coded by ROM
/// and shaded by decode/upload state) with a compact colour legend.
#[allow(clippy::too_many_arguments)]
fn draw_dots(
    gfx: &Gfx,
    font: &SdfFontMetrics,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    nodes: &[RomNode],
    chip_w: f32,
    chip_gap: f32,
    s: f32,
) {
    rect(gfx, x, y, w, h, NODE_BG);
    draw_text(gfx, font, x + PAD * s, y + PAD * s, "resources", 0.55 * s, DIM_TEXT);

    let legend_y = y + PAD * s + 20.0 * s;
    let mut lx = x + PAD * s;
    for node in nodes {
        let rc = rom_color(&node.name);
        rect(gfx, lx, legend_y + 2.0 * s, chip_w, chip_w, rc);
        lx += chip_w + chip_gap;
        draw_text(gfx, font, lx, legend_y, &node.name, 0.5 * s, DIM_TEXT);
        lx += measure_text(font, &node.name, 0.5 * s) + chip_gap * 4.0;
    }

    let mut cx = x + PAD * s;
    let mut cy = legend_y + 24.0 * s;
    let max_x = x + w - PAD * s;
    for node in nodes {
        let rc = rom_color(&node.name);
        for chip in &node.chips {
            rect(gfx, cx, cy, chip_w, chip_w, chip_color(rc, chip.state));
            cx += chip_w + chip_gap;
            if cx + chip_w > max_x {
                cx = x + PAD * s;
                cy += chip_w + chip_gap;
            }
        }
        // A small gap between one ROM's run and the next.
        cx += chip_gap * 2.0;
    }
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

/// Measure the rendered width of `text` at `scale` (left-justified).
fn measure_text(font: &SdfFontMetrics, text: &str, scale: f32) -> f32 {
    build_sdf_glyph_buffer(font, text, scale, TextJustify::Left, 0.0).text_width
}

/// Draw `text` left-justified with its top-left corner at `(x, y)`.
fn draw_text(
    gfx: &Gfx,
    font: &SdfFontMetrics,
    x: f32,
    y: f32,
    text: &str,
    scale: f32,
    color: [f32; 4],
) {
    let buf = build_sdf_glyph_buffer(font, text, scale, TextJustify::Left, 0.0);
    if buf.vertex_count == 0 {
        return;
    }
    let glyph_buf =
        GlBuffer::from_slice(&gfx.gl, glow::ARRAY_BUFFER, &buf.vertices, glow::DYNAMIC_DRAW);
    let atlas_name = format!("{DEFAULT_SDF_FONT}-sdf");
    let model = Mat4::from_translation(Vec3::new(x, y, 0.0))
        * Mat4::from_scale(Vec3::new(buf.text_width, buf.text_height, 1.0));
    gfx.draw_sdf(
        &model,
        &Mat4::IDENTITY,
        &atlas_name,
        &color,
        &[0.0, 0.0, 0.0, 0.0],
        0.0,
        font.spread,
        &font.atlas_size,
        0.0,
        1.0,
        buf.vertex_count as i32,
        &glyph_buf,
        true,
    );
}
