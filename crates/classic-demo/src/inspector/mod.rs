//! Entity inspector: `Backquote` pauses the simulation and opens two panels
//! over the game, a ROM → assets / entities tree ([`tree`]) on the left and
//! the selection's detail ([`detail`]) on the right; a click in the world
//! picks the entity under it ([`pick`]).
//!
//! Installed with the dev HUD (`host_features && !CLASSIC_NO_UI`) and closed
//! by default, so a run that never presses the key draws and traces nothing
//! new.  Everything here runs in host hooks — the toggle, clicks and wheel in
//! a pre-update hook (so a toggle pauses the very frame it is pressed), the
//! panel text in an `on_update` closure, the outline in an overlay — never in
//! `on_sim_update`, so it keeps running while the simulation is held.
//!
//! The panels are virtualised lists: each owns a fixed pool of SDF text rows
//! that are rewritten from a `Vec<Row>` at the current scroll offset, and
//! one screen collider whose click is mapped back to a row by the pointer's
//! y.  The colliders are plain physics colliders (no UI hover tint), enabled
//! only while open.  A preview box under the detail header shows a sprite
//! selection ([`InspectorState::preview_rect`]): [`preview`] draws its frame
//! there, with pose steppers for a sheet with pose axes.

mod detail;
mod pick;
mod preview;
mod tree;

#[cfg(test)]
mod tests;

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use classic_core::collision::{polygon_from_verts, HandlerKind};
use classic_core::components::{
    ColliderData, Disabled, SdfTextRender, TextJustify, Transform, UiAnchor, UiNode,
};
use classic_engine::Engine;
use glam::{Vec2, Vec3, Vec4};

pub use preview::{PoseAxis, PreviewState};
pub use tree::{AssetKind, Group};

use crate::state::DemoStateRef;

/// The key that opens and closes the inspector (winit `KeyCode` debug name
/// natively, `KeyboardEvent.code` on the web).
pub const TOGGLE_KEY: &str = "Backquote";
/// Row pitch of both panels, in pixels.
pub const ROW_H: f32 = 18.0;
/// Height of the sprite preview box in the detail panel.
pub const PREVIEW_H: f32 = 160.0;
const TEXT_SCALE: f32 = 0.42;
const TITLE_SCALE: f32 = 0.5;
const PAD: f32 = 6.0;
const TITLE_H: f32 = 24.0;
const MARGIN: f32 = 8.0;
/// Below the dev HUD's 68 px top bar.
const TOP: f32 = 76.0;
const TREE_W: f32 = 340.0;
const DETAIL_W: f32 = 440.0;
/// Text rows per panel pool (enough for a ~1800 px tall viewport).
const MAX_ROWS: usize = 96;
/// Frames between rebuilds of the detail's live rows (screen position).
const DETAIL_REFRESH_FRAMES: u32 = 10;
/// Above every other UI layer (the HUD sits at -1000 … -1500; lower z draws
/// later, on top).
const Z_PANEL: f32 = -1600.0;
const Z_INNER: f32 = -1605.0;
const Z_TEXT: f32 = -1610.0;
const PANEL_COLOR: [f32; 4] = [0.04, 0.05, 0.08, 0.96];
const PREVIEW_COLOR: [f32; 4] = [0.12, 0.13, 0.17, 1.0];
/// Pose stepper rows in the preview box (direction, pitch, roll).
const STEPPER_ROWS: usize = 3;
const TITLE_COLOR: [f32; 4] = [1.0, 0.6, 0.3, 1.0];
/// Click priorities: the panels over any game UI (the preview's steppers
/// over their panel), the backdrop under all.
const PANEL_PRIORITY: i32 = 100;
const PREVIEW_PRIORITY: i32 = 101;
const BACKDROP_PRIORITY: i32 = -100;

/// What the inspector has selected.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Selection {
    /// A loaded ROM, by index into `Engine::loaded_roms`.
    Rom(usize),
    Asset(AssetRef),
    /// A named entity (its full, namespace-qualified name).
    Entity(String),
}

/// A manifest asset: its ROM, kind and bare (manifest) name.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AssetRef {
    pub rom: usize,
    pub kind: AssetKind,
    pub name: String,
}

/// What clicking a row does.
#[derive(Clone, Debug, PartialEq)]
pub enum RowAction {
    /// Expand / collapse a ROM (and select it).
    ToggleRom(usize),
    /// Expand / collapse a group under a ROM slot.
    ToggleGroup(usize, Group),
    /// Show page `.2` of a slot's group.
    Page(usize, Group, usize),
    Select(Selection),
    /// Rebuild the entity lists and the selection's detail.
    Refresh,
}

/// One display line of a panel.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub text: String,
    pub color: [f32; 4],
    pub action: Option<RowAction>,
}

/// A screen rectangle (top-left origin).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, p: Vec2) -> bool {
        p.x >= self.x && p.x < self.x + self.w && p.y >= self.y && p.y < self.y + self.h
    }

    fn clip(&self) -> Vec4 {
        Vec4::new(self.x, self.y, self.w, self.h)
    }
}

/// One panel: its background, title origin and the rows' area.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PanelLayout {
    pub panel: Rect,
    pub title: Vec2,
    pub list: Rect,
}

impl PanelLayout {
    fn new(panel: Rect) -> Self {
        let title = Vec2::new(panel.x + PAD, panel.y + PAD);
        let list = Rect {
            x: panel.x + PAD,
            y: panel.y + PAD + TITLE_H,
            w: panel.w - 2.0 * PAD,
            h: (panel.h - 2.0 * PAD - TITLE_H).max(0.0),
        };
        Self { panel, title, list }
    }

    /// Rows that fit the list area.
    pub fn visible_rows(&self) -> usize {
        ((self.list.h / ROW_H).floor().max(0.0) as usize).min(MAX_ROWS)
    }

    /// The visible row index under `p`, if any.
    pub fn row_at(&self, p: Vec2) -> Option<usize> {
        if !self.list.contains(p) {
            return None;
        }
        let i = ((p.y - self.list.y) / ROW_H).floor() as usize;
        (i < self.visible_rows()).then_some(i)
    }
}

/// Where both panels (and the preview box) sit for a viewport.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Layout {
    pub tree: PanelLayout,
    pub detail: PanelLayout,
    pub preview: Option<Rect>,
}

/// Lay the panels out for a `vw` × `vh` viewport: the tree down the left
/// edge, the detail down the right, both under the dev top bar; with
/// `preview`, the detail rows start below a [`PREVIEW_H`] box.
pub fn compute_layout(vw: f32, vh: f32, preview: bool) -> Layout {
    let h = (vh - TOP - MARGIN).max(TITLE_H + 4.0 * ROW_H);
    let tree_w = TREE_W.min(vw * 0.3);
    let detail_w = DETAIL_W.min(vw * 0.36);
    let tree = PanelLayout::new(Rect { x: MARGIN, y: TOP, w: tree_w, h });
    let mut detail = PanelLayout::new(Rect { x: vw - MARGIN - detail_w, y: TOP, w: detail_w, h });
    let preview = preview.then(|| {
        let r = Rect { h: PREVIEW_H, ..detail.list };
        detail.list.y += PREVIEW_H + PAD;
        detail.list.h = (detail.list.h - PREVIEW_H - PAD).max(0.0);
        r
    });
    Layout { tree, detail, preview }
}

/// The largest useful scroll offset of `len` rows in a `visible`-row window.
pub fn max_scroll(len: usize, visible: usize) -> usize {
    len.saturating_sub(visible)
}

/// Which collider took this frame's click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    Tree,
    Detail,
    Preview,
    World,
}

/// The inspector's entities and colliders.
#[derive(Debug)]
struct Panels {
    tree_bg: hecs::Entity,
    detail_bg: hecs::Entity,
    preview_bg: hecs::Entity,
    preview_label: hecs::Entity,
    /// Per stepper row: its `[-]`, value and `[+]` texts.
    preview_steps: Vec<[hecs::Entity; 3]>,
    tree_title: hecs::Entity,
    detail_title: hecs::Entity,
    tree_rows: Vec<hecs::Entity>,
    detail_rows: Vec<hecs::Entity>,
    tree_pid: u32,
    detail_pid: u32,
    backdrop_pid: u32,
    preview_pid: u32,
    /// Whether the preview box is shown (tracked to toggle it on change only).
    preview_shown: bool,
}

/// The inspector state, held in `DemoState`.
#[derive(Debug, Default)]
pub struct InspectorState {
    pub open: bool,
    pub selection: Option<Selection>,
    /// Expanded ROMs, by `loaded_roms` index.
    pub expanded_roms: HashSet<usize>,
    /// Expanded groups, by ROM slot (`loaded_roms.len()` = unscoped entities).
    pub expanded: HashSet<(usize, Group)>,
    /// The page shown of each paged group.
    pub pages: HashMap<(usize, Group), usize>,
    pub tree_scroll: usize,
    pub detail_scroll: usize,
    index: tree::EntityIndex,
    tree_rows: Vec<Row>,
    detail_rows: Vec<Row>,
    /// The cached component dump of the selected entity.
    components: Option<(String, Vec<Row>)>,
    tree_dirty: bool,
    detail_age: u32,
    /// An entity to scroll into view once the tree is rebuilt.
    reveal: Option<String>,
    layout: Layout,
    panels: Option<Panels>,
    /// Set by the colliders' click handlers, consumed by the pre-update hook.
    hits: Rc<Cell<Option<Hit>>>,
    /// The sprite preview's pose.
    pub preview: PreviewState,
}

impl InspectorState {
    /// Select `sel` (a no-op when already selected).
    pub fn select(&mut self, sel: Selection) {
        if self.selection.as_ref() == Some(&sel) {
            return;
        }
        self.selection = Some(sel);
        self.detail_scroll = 0;
        self.components = None;
        self.detail_age = u32::MAX;
        self.tree_dirty = true;
    }

    /// Expand the tree down to `name` (its ROM, Entities, page) and scroll it
    /// into view on the next rebuild.
    pub fn reveal(&mut self, engine: &Engine, name: &str) {
        self.index.refresh(engine, false);
        let Some((slot, i)) = self.index.locate(name) else { return };
        if slot < engine.loaded_roms.len() {
            self.expanded_roms.insert(slot);
        }
        self.expanded.insert((slot, Group::Entities));
        self.pages.insert((slot, Group::Entities), tree::page_of(i));
        self.reveal = Some(name.to_string());
        self.tree_dirty = true;
    }

    /// Run a clicked row's action.
    pub fn apply(&mut self, engine: &Engine, action: RowAction) {
        match action {
            RowAction::ToggleRom(i) => {
                if !self.expanded_roms.remove(&i) {
                    self.expanded_roms.insert(i);
                }
                self.select(Selection::Rom(i));
            }
            RowAction::ToggleGroup(slot, g) => {
                if !self.expanded.remove(&(slot, g)) {
                    self.expanded.insert((slot, g));
                }
            }
            RowAction::Page(slot, g, p) => {
                self.pages.insert((slot, g), p);
            }
            RowAction::Select(sel) => {
                if let Selection::Entity(name) = &sel {
                    self.reveal(engine, name);
                }
                self.select(sel);
            }
            RowAction::Refresh => {
                self.index.refresh(engine, true);
                self.components = None;
                self.detail_age = u32::MAX;
            }
        }
        self.tree_dirty = true;
    }

    /// The current tree rows (rebuilt by the per-frame sync).
    pub fn tree_rows(&self) -> &[Row] {
        &self.tree_rows
    }

    /// The current detail rows.
    pub fn detail_rows(&self) -> &[Row] {
        &self.detail_rows
    }

    /// The panels' current layout.
    pub fn layout(&self) -> Layout {
        self.layout
    }

    /// The sprite preview box (screen pixels) while the inspector is open on
    /// a sprite entity: the area the sprite-preview layer draws into.
    pub fn preview_rect(&self) -> Option<Rect> {
        if self.open {
            self.layout.preview
        } else {
            None
        }
    }

    /// The selected entity's name, if an entity is selected.
    pub fn selected_entity(&self) -> Option<&str> {
        match &self.selection {
            Some(Selection::Entity(n)) => Some(n),
            _ => None,
        }
    }
}

/// Install the inspector: its (hidden) panels and colliders, and the hooks
/// that drive it.  Call once, after the rest of the dev HUD.
pub fn install(engine: &mut Engine, state: &DemoStateRef) {
    let hits = state.borrow().inspector.hits.clone();
    let panels = spawn_panels(engine, &hits);
    state.borrow_mut().inspector.panels = Some(panels);
    {
        let s = state.clone();
        engine.on_pre_update(move |e| pre_update(e, &s));
    }
    {
        let s = state.clone();
        engine.on_update(move |e| sync(e, &s));
    }
    {
        let s = state.clone();
        engine.add_overlay(move |e| pick::draw_selection(e, &s));
    }
    {
        let s = state.clone();
        engine.add_overlay(move |e| preview::draw(e, &s));
    }
}

fn spawn_text(
    engine: &mut Engine,
    parent: hecs::Entity,
    scale: f32,
    color: [f32; 4],
) -> hecs::Entity {
    let ui = engine.ui.as_mut().expect("spawn_box created the manager");
    let e = ui.spawn_sdf_text(&mut engine.world, "", scale, 0.0, color, TextJustify::Left);
    ui.container_add_child(&mut engine.world, parent, e, UiAnchor::TopLeft, UiAnchor::TopLeft);
    if let Ok(mut tf) = engine.world.get::<&mut Transform>(e) {
        tf.position.z = Z_TEXT;
    }
    e
}

fn spawn_box(engine: &mut Engine, color: [f32; 4], z: f32, clip: bool) -> hecs::Entity {
    engine.game_ui();
    let ui = engine.ui.as_mut().expect("game_ui created the manager");
    let e = ui.spawn_container(&mut engine.world, 10.0, 10.0, color);
    if let Ok(mut tf) = engine.world.get::<&mut Transform>(e) {
        tf.position.z = z;
    }
    if let Ok(mut node) = engine.world.get::<&mut UiNode>(e) {
        node.anchor = UiAnchor::TopLeft;
        node.clip_children = clip;
    }
    e
}

/// A screen collider that records `hit` on click (consuming it), disabled.
fn spawn_collider(
    engine: &mut Engine,
    hits: &Rc<Cell<Option<Hit>>>,
    hit: Hit,
    priority: i32,
) -> u32 {
    let shape = polygon_from_verts(vec![
        Vec3::ZERO,
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(1.0, 1.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
    ]);
    let pid = engine.physics.register_collider(ColliderData::new(shape));
    engine.physics.set_collider_consumes_click(pid, true);
    engine.physics.set_collider_click_priority(pid, priority);
    let h = hits.clone();
    engine.physics.add_collider_handler(pid, HandlerKind::Click, move || {
        h.set(Some(hit));
        true
    });
    engine.physics.set_collider_enabled(pid, false);
    pid
}

/// Spawn both panels hidden.  Built on the game UI's manager but outside its
/// root tree (the sync places every element itself), and hidden as each is
/// made: a just-spawned entity is its archetype's last, so marking it
/// `Disabled` reorders nobody else's draw order (the goldens stay put).
fn spawn_panels(engine: &mut Engine, hits: &Rc<Cell<Option<Hit>>>) -> Panels {
    let tree_bg = spawn_box(engine, PANEL_COLOR, Z_PANEL, true);
    let detail_bg = spawn_box(engine, PANEL_COLOR, Z_PANEL, true);
    let preview_bg = spawn_box(engine, PREVIEW_COLOR, Z_INNER, false);
    {
        let ui = engine.ui.as_mut().expect("spawn_box created the manager");
        ui.container_add_child(
            &mut engine.world,
            detail_bg,
            preview_bg,
            UiAnchor::TopLeft,
            UiAnchor::TopLeft,
        );
    }
    let preview_label = spawn_text(engine, preview_bg, TEXT_SCALE, tree::DIM);
    let preview_steps: Vec<[hecs::Entity; 3]> = (0..STEPPER_ROWS)
        .map(|_| {
            [tree::LINK, tree::WHITE, tree::LINK]
                .map(|color| spawn_text(engine, preview_bg, TEXT_SCALE, color))
        })
        .collect();
    let tree_title = spawn_text(engine, tree_bg, TITLE_SCALE, TITLE_COLOR);
    let detail_title = spawn_text(engine, detail_bg, TITLE_SCALE, TITLE_COLOR);
    let tree_rows =
        (0..MAX_ROWS).map(|_| spawn_text(engine, tree_bg, TEXT_SCALE, tree::WHITE)).collect();
    let detail_rows =
        (0..MAX_ROWS).map(|_| spawn_text(engine, detail_bg, TEXT_SCALE, tree::WHITE)).collect();
    let panels = Panels {
        tree_bg,
        detail_bg,
        preview_bg,
        preview_label,
        preview_steps,
        tree_title,
        detail_title,
        tree_rows,
        detail_rows,
        tree_pid: spawn_collider(engine, hits, Hit::Tree, PANEL_PRIORITY),
        detail_pid: spawn_collider(engine, hits, Hit::Detail, PANEL_PRIORITY),
        backdrop_pid: spawn_collider(engine, hits, Hit::World, BACKDROP_PRIORITY),
        preview_pid: spawn_collider(engine, hits, Hit::Preview, PREVIEW_PRIORITY),
        preview_shown: false,
    };
    // Last-spawned first, so each hide is a pop off its archetype's end.
    let mut all = vec![tree_bg, detail_bg, preview_bg, preview_label];
    all.extend(panels.preview_steps.iter().flatten());
    all.extend([tree_title, detail_title]);
    all.extend(panels.tree_rows.iter().chain(&panels.detail_rows));
    for &e in all.iter().rev() {
        let _ = engine.world.insert_one(e, Disabled);
    }
    panels
}

/// Open or close: show / hide the panels, enable their colliders and hold
/// or release the simulation.
fn set_open(engine: &mut Engine, state: &DemoStateRef, open: bool) {
    let mut s = state.borrow_mut();
    let st = &mut s.inspector;
    st.open = open;
    st.hits.set(None);
    engine.set_sim_paused(open);
    let Some(p) = st.panels.as_mut() else { return };
    engine.set_enabled(p.tree_bg, open);
    engine.set_enabled(p.detail_bg, open);
    // `set_enabled` showed the preview box with its panel; the sync re-hides it.
    p.preview_shown = open;
    // Likewise the steppers' collider.
    for pid in [p.tree_pid, p.detail_pid, p.backdrop_pid, p.preview_pid] {
        engine.physics.set_collider_enabled(pid, open);
    }
    if open {
        st.tree_dirty = true;
        st.detail_age = u32::MAX;
        st.layout = Layout::default();
    }
}

/// Pre-update hook: the toggle key, then (while open) this frame's click and
/// the wheel over a panel.
fn pre_update(engine: &mut Engine, state: &DemoStateRef) {
    if engine.input.was_key_pressed(TOGGLE_KEY) {
        let open = !state.borrow().inspector.open;
        set_open(engine, state, open);
    }
    if !state.borrow().inspector.open {
        return;
    }
    let mouse = engine.input.mouse_pos;
    let mut s = state.borrow_mut();
    let st = &mut s.inspector;
    match st.hits.take() {
        Some(Hit::Tree) => {
            if let Some(i) = st.layout.tree.row_at(mouse) {
                if let Some(action) =
                    st.tree_rows.get(st.tree_scroll + i).and_then(|r| r.action.clone())
                {
                    st.apply(engine, action);
                }
            }
        }
        Some(Hit::Detail) => {
            if let Some(i) = st.layout.detail.row_at(mouse) {
                if let Some(action) =
                    st.detail_rows.get(st.detail_scroll + i).and_then(|r| r.action.clone())
                {
                    st.apply(engine, action);
                }
            }
        }
        Some(Hit::Preview) => {
            if let Some(r) = st.preview_rect() {
                let axes = st.preview.axes().map(|a| preview::stepper_axes(&a)).unwrap_or_default();
                let layout = preview::box_layout(r, &axes);
                if let Some((axis, delta)) = preview::hit(&layout, mouse) {
                    if st.preview.step(axis, delta) {
                        st.detail_age = u32::MAX;
                    }
                }
            }
        }
        Some(Hit::World) => {
            pick::pick(engine, st, mouse.x, mouse.y);
        }
        None => {}
    }

    let wheel = engine.input.mouse_wheel;
    if wheel.abs() > 0.01 {
        // One notch is ~0.3 (`2 · deltaY / height`): three rows.
        let rows = ((wheel.abs() * 10.0).round() as usize).max(3);
        let up = wheel > 0.0;
        let scroll = |v: &mut usize, len: usize, visible: usize| {
            *v =
                if up { v.saturating_sub(rows) } else { (*v + rows).min(max_scroll(len, visible)) };
        };
        if st.layout.tree.panel.contains(mouse) {
            scroll(&mut st.tree_scroll, st.tree_rows.len(), st.layout.tree.visible_rows());
            engine.input.mouse_wheel = 0.0;
        } else if st.layout.detail.panel.contains(mouse) {
            scroll(&mut st.detail_scroll, st.detail_rows.len(), st.layout.detail.visible_rows());
            engine.input.mouse_wheel = 0.0;
        }
    }
}

/// Write a panel's rows into its text pool at `scroll`.
fn write_rows(engine: &mut Engine, pool: &[hecs::Entity], rows: &[Row], scroll: usize, list: Rect) {
    let visible = ((list.h / ROW_H).floor().max(0.0) as usize).min(pool.len());
    for (i, &e) in pool.iter().enumerate() {
        let row = if i < visible { rows.get(scroll + i) } else { None };
        if let Ok(mut sdf) = engine.world.get::<&mut SdfTextRender>(e) {
            match row {
                Some(r) => {
                    if sdf.text != r.text {
                        sdf.text.clone_from(&r.text);
                    }
                    sdf.color = r.color;
                }
                None => {
                    if !sdf.text.is_empty() {
                        sdf.text.clear();
                    }
                }
            }
        }
        if row.is_some() {
            if let Ok(mut tf) = engine.world.get::<&mut Transform>(e) {
                tf.position = Vec3::new(list.x, list.y + i as f32 * ROW_H, Z_TEXT);
            }
            if let Ok(mut node) = engine.world.get::<&mut UiNode>(e) {
                node.clip_rect = list.clip();
            }
        }
    }
}

fn place_box(engine: &mut Engine, e: hecs::Entity, r: Rect) {
    if let Ok(mut tf) = engine.world.get::<&mut Transform>(e) {
        tf.position.x = r.x;
        tf.position.y = r.y;
    }
    if let Ok(mut node) = engine.world.get::<&mut UiNode>(e) {
        node.size = Vec2::new(r.w, r.h);
    }
}

fn place_title(engine: &mut Engine, e: hecs::Entity, text: &str, at: Vec2, clip: Rect) {
    if let Ok(mut sdf) = engine.world.get::<&mut SdfTextRender>(e) {
        if sdf.text != text {
            sdf.text = text.to_string();
        }
    }
    if let Ok(mut tf) = engine.world.get::<&mut Transform>(e) {
        tf.position = Vec3::new(at.x, at.y, Z_TEXT);
    }
    if let Ok(mut node) = engine.world.get::<&mut UiNode>(e) {
        node.clip_rect = clip.clip();
    }
}

/// Write the preview box's frame line and stepper rows.
fn sync_preview(engine: &mut Engine, p: &Panels, st: &PreviewState, name: &str, r: Rect) {
    let steps = st.stepper_text();
    let layout = preview::box_layout(r, &steps.iter().map(|(a, _)| *a).collect::<Vec<_>>());
    let label = preview::label(engine, name, st);
    place_title(engine, p.preview_label, &label, layout.label, r);
    for (i, texts) in p.preview_steps.iter().enumerate() {
        let row = steps.get(i).zip(layout.steppers.get(i));
        let [minus, value, plus] = *texts;
        match row {
            Some(((_, text), &(_, m, at, pl))) => {
                place_title(engine, minus, "[-]", Vec2::new(m.x + 2.0, m.y), r);
                place_title(engine, value, text, at, r);
                place_title(engine, plus, "[+]", Vec2::new(pl.x + 2.0, pl.y), r);
            }
            None => {
                for e in [minus, value, plus] {
                    place_title(engine, e, "", Vec2::new(r.x, r.y), r);
                }
            }
        }
    }
}

/// Host update: rebuild what changed and place the panels.
fn sync(engine: &mut Engine, state: &DemoStateRef) {
    let mut s = state.borrow_mut();
    let st = &mut s.inspector;
    if !st.open {
        return;
    }

    // Model: entity lists, tree rows, detail rows.
    if st.index.refresh(engine, false) {
        st.tree_dirty = true;
    }
    if st.tree_dirty {
        st.tree_rows = tree::build_rows(engine, st);
        st.tree_dirty = false;
        if let Some(name) = st.reveal.take() {
            let want = Some(RowAction::Select(Selection::Entity(name)));
            if let Some(i) = st.tree_rows.iter().position(|r| r.action == want) {
                let visible = st.layout.tree.visible_rows().max(1);
                if i < st.tree_scroll || i >= st.tree_scroll + visible {
                    st.tree_scroll = i.saturating_sub(visible / 3);
                }
            }
        }
    }
    let selected = st.selected_entity().map(str::to_string);
    if st.preview.follow(engine, selected.as_deref()) {
        st.detail_age = u32::MAX;
    }
    st.detail_age = st.detail_age.saturating_add(1);
    if st.detail_age > DETAIL_REFRESH_FRAMES {
        st.detail_age = 0;
        st.detail_rows = match st.selection.clone() {
            None => vec![Row {
                text: "Click a tree row, or an entity in the world.".into(),
                color: tree::DIM,
                action: None,
            }],
            Some(sel) => {
                let count = match &sel {
                    Selection::Rom(i) => st.index.slot(*i).len(),
                    _ => 0,
                };
                let mut rows = detail::build(engine, &sel, count);
                if let Selection::Entity(name) = &sel {
                    rows.extend(preview::rows(engine, name, &st.preview));
                    if st.components.as_ref().is_none_or(|(n, _)| n != name) {
                        st.components = Some((name.clone(), detail::component_rows(engine, name)));
                    }
                    if let Some((_, comps)) = &st.components {
                        rows.extend(comps.iter().cloned());
                    }
                }
                rows
            }
        };
    }

    // View: layout, then the pools.
    let (vw, vh) = engine.viewport_size();
    let (vw, vh) = if vw > 0.5 && vh > 0.5 { (vw, vh) } else { (1280.0, 720.0) };
    let preview = detail::has_preview(engine, st.selection.as_ref());
    let layout = compute_layout(vw, vh, preview);
    st.tree_scroll = st.tree_scroll.min(max_scroll(st.tree_rows.len(), layout.tree.visible_rows()));
    st.detail_scroll =
        st.detail_scroll.min(max_scroll(st.detail_rows.len(), layout.detail.visible_rows()));
    let Some(p) = st.panels.as_mut() else { return };
    if layout != st.layout {
        st.layout = layout;
        place_box(engine, p.tree_bg, layout.tree.panel);
        place_box(engine, p.detail_bg, layout.detail.panel);
        for (pid, r) in [(p.tree_pid, layout.tree.panel), (p.detail_pid, layout.detail.panel)] {
            engine.physics.sync_collider_rect(pid, r.x, r.y, r.w, r.h);
        }
        engine.physics.sync_collider_rect(p.backdrop_pid, 0.0, 0.0, vw, vh);
        if let Some(r) = layout.preview {
            place_box(engine, p.preview_bg, r);
            engine.physics.sync_collider_rect(p.preview_pid, r.x, r.y, r.w, r.h);
        }
    }
    if preview != p.preview_shown {
        p.preview_shown = preview;
        engine.set_enabled(p.preview_bg, preview);
        engine.physics.set_collider_enabled(p.preview_pid, preview);
    }
    if let (Some(r), Some(name)) = (layout.preview, selected.as_deref()) {
        sync_preview(engine, p, &st.preview, name, r);
    }

    let paused = if engine.sim_paused() { "sim paused" } else { "sim running" };
    let tree_title = format!("INSPECTOR  ({paused}; ` closes)");
    place_title(engine, p.tree_title, &tree_title, layout.tree.title, layout.tree.panel);
    let detail_title = detail::title(engine, st.selection.as_ref());
    place_title(engine, p.detail_title, &detail_title, layout.detail.title, layout.detail.panel);
    write_rows(engine, &p.tree_rows, &st.tree_rows, st.tree_scroll, layout.tree.list);
    write_rows(engine, &p.detail_rows, &st.detail_rows, st.detail_scroll, layout.detail.list);
}
