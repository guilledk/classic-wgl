//! # Skill: `classic-physics`
//!
//! **Read `.claude/skills/classic-physics/SKILL.md` before working on this module.**
//!
//! Collision detection and interaction dispatch.
//!
//! Port of `src/classic/collision.ts`.

use std::collections::HashMap;

use glam::{Mat4, Vec3};

use crate::components::{ColliderData, Shape};
use crate::gjk::{GjkContext, GjkShape};
use crate::quadtree::{Quadtree, RectBounds};
use crate::types::Rect;

// ---------------------------------------------------------------------------
// Shape geometry
// ---------------------------------------------------------------------------

impl Shape {
    pub fn model_matrix(&self, position: Vec3, scale: Vec3) -> Mat4 {
        Mat4::from_translation(Vec3::new(position.x, position.y, 0.0)) * Mat4::from_scale(scale)
    }

    pub fn rect(&self, position: Vec3, scale: Vec3) -> Rect {
        match self {
            Shape::Circle { diameter } => Rect {
                x: position.x - diameter * scale.x / 2.0,
                y: position.y - diameter * scale.y / 2.0,
                width: diameter * scale.x,
                height: diameter * scale.y,
            },
            Shape::Polygon { verts: _, min, max, .. } => {
                let m = self.model_matrix(position, scale);
                let vmin = m.transform_point3(*min);
                let vmax = m.transform_point3(*max);
                Rect {
                    x: vmin.x,
                    y: vmin.y,
                    width: (vmax.x - vmin.x).abs(),
                    height: (vmax.y - vmin.y).abs(),
                }
            }
        }
    }

    pub fn center(&self, position: Vec3, scale: Vec3) -> Vec3 {
        match self {
            Shape::Circle { .. } => position,
            Shape::Polygon { center, .. } => {
                let m = self.model_matrix(position, scale);
                m.transform_point3(*center)
            }
        }
    }

    pub fn support(&self, position: Vec3, scale: Vec3, dir: Vec3) -> Option<Vec3> {
        match self {
            Shape::Circle { diameter } => {
                let r = diameter / 2.0;
                let d = dir.normalize() * r * scale;
                let c = position;
                Some(Vec3::new(c.x + d.x, c.y + d.y, 0.0))
            }
            Shape::Polygon { verts, .. } => {
                let m = self.model_matrix(position, scale);
                let mut best: Option<Vec3> = None;
                let mut best_dot = f32::NEG_INFINITY;
                for v in verts {
                    let wv = m.transform_point3(*v);
                    let dot = dir.dot(wv);
                    if dot > best_dot {
                        best_dot = dot;
                        best = Some(wv);
                    }
                }
                best
            }
        }
    }
}

/// Build a `Shape::Polygon` from world-space vertices, computing `center`,
/// `min`, and `max` automatically.
pub fn polygon_from_verts(verts: Vec<Vec3>) -> Shape {
    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);
    let mut center = Vec3::ZERO;
    for v in &verts {
        min = min.min(*v);
        max = max.max(*v);
        center += *v;
    }
    center /= verts.len().max(1) as f32;
    Shape::Polygon { verts, center, min, max }
}

/// A reference wrapper for GJK queries that bundles shape + position + scale.
pub struct ShapeRef<'a> {
    pub shape: &'a Shape,
    pub position: Vec3,
    pub scale: Vec3,
}

impl GjkShape for ShapeRef<'_> {
    fn center(&self) -> Vec3 {
        self.shape.center(self.position, self.scale)
    }

    fn support(&self, dir: Vec3) -> Option<Vec3> {
        self.shape.support(self.position, self.scale, dir)
    }
}

// ---------------------------------------------------------------------------
// RectBounds impl for collider handles
// ---------------------------------------------------------------------------

/// Lightweight handle for quadtree storage — just enough to reference a
/// collider and provide its bounding rect.
#[derive(Clone)]
pub struct ColliderHandle {
    pub pid: u32,
    pub rect: Rect,
}

impl RectBounds for ColliderHandle {
    fn rect(&self) -> Rect {
        self.rect
    }
}

// ---------------------------------------------------------------------------
// Collider methods
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HandlerKind {
    Enter,
    Exit,
    Click,
    Selection,
    SelectionTemp,
}

// ---------------------------------------------------------------------------
// VirtualCollider
// ---------------------------------------------------------------------------

pub struct VirtualCollider {
    pub pid: u32,
    pub shape: Shape,
    pub position: Vec3,
    pub scale: Vec3,
    pub rect: Rect,
}

impl VirtualCollider {
    pub fn new(pid: u32, shape: Shape) -> Self {
        let pos = Vec3::ZERO;
        let scl = Vec3::ONE;
        let r = shape.rect(pos, scl);
        Self { pid, shape, position: pos, scale: scl, rect: r }
    }

    pub fn update_rect(&mut self) {
        self.rect = self.shape.rect(self.position, self.scale);
    }

    pub fn intersects(&self, other: &Rect) -> bool {
        self.rect.intersects(other)
    }
}

// ---------------------------------------------------------------------------
// PhysicsProvider
// ---------------------------------------------------------------------------

struct ColliderEntry {
    collider: ColliderData,
    /// Interaction handlers, keyed by kind.  Owned by the provider (not the
    /// serializable component) so they never round-trip through `state.json`.
    handlers: HashMap<HandlerKind, Vec<Box<dyn FnMut() -> bool>>>,
    enabled: bool,
}

impl ColliderEntry {
    fn has_handlers(&self, kind: HandlerKind) -> bool {
        self.handlers.get(&kind).is_some_and(|v| !v.is_empty())
    }

    fn add_handler(&mut self, kind: HandlerKind, f: impl FnMut() -> bool + 'static) {
        self.handlers.entry(kind).or_default().push(Box::new(f));
    }
}

pub struct PhysicsProvider {
    next_id: u32,
    entries: HashMap<u32, ColliderEntry>,
    pub mouse: VirtualCollider,
    pub selection: VirtualCollider,
    screen_collider: Rect,
    screen: Quadtree<ColliderHandle>,
    collided: HashMap<u32, HashMap<u32, bool>>,
    colliding: HashMap<u32, HashMap<u32, bool>>,
    /// Set to true when a click handler fires on a collider with `consumes_click`.
    pub consumed_click: bool,
    /// Set by the engine before `perform_calls` — true only on actual mouse presses.
    pub mouse_clicked: bool,
}

impl PhysicsProvider {
    pub fn new() -> Self {
        let mouse = VirtualCollider::new(0, Shape::Circle { diameter: 1.0 });
        let selection = VirtualCollider::new(
            1,
            Shape::Polygon {
                verts: vec![
                    Vec3::new(0.0, 0.0, 0.0),
                    Vec3::new(1.0, 0.0, 0.0),
                    Vec3::new(1.0, 1.0, 0.0),
                    Vec3::new(0.0, 1.0, 0.0),
                ],
                center: Vec3::new(0.5, 0.5, 0.0),
                min: Vec3::ZERO,
                max: Vec3::new(1.0, 1.0, 0.0),
            },
        );
        let screen_collider = Rect::new(0.0, 0.0, 1920.0, 1080.0);
        Self {
            next_id: 2,
            entries: HashMap::new(),
            mouse,
            selection,
            screen_collider,
            screen: Quadtree::new(screen_collider, 10, 4),
            collided: HashMap::new(),
            colliding: HashMap::new(),
            consumed_click: false,
            mouse_clicked: false,
        }
    }

    pub fn resize_screen(&mut self, w: f32, h: f32) {
        self.screen_collider = Rect::new(0.0, 0.0, w, h);
        self.screen.clear();
    }

    pub fn register_collider(&mut self, collider: ColliderData) -> u32 {
        let pid = self.next_id;
        self.next_id += 1;
        self.entries
            .insert(pid, ColliderEntry { collider, handlers: HashMap::new(), enabled: true });
        pid
    }

    pub fn unregister_collider(&mut self, pid: u32) {
        self.entries.remove(&pid);
    }

    /// Rebuild a collider's polygon shape from a (0,0)→(w,h) rect at a new position.
    pub fn sync_collider_rect(&mut self, pid: u32, x: f32, y: f32, w: f32, h: f32) {
        if let Some(entry) = self.entries.get_mut(&pid) {
            entry.collider.position = Vec3::new(x, y, 0.0);
            let verts = vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(w, 0.0, 0.0),
                Vec3::new(w, h, 0.0),
                Vec3::new(0.0, h, 0.0),
            ];
            entry.collider.shape = polygon_from_verts(verts);
        }
    }

    pub fn add_collider_handler(
        &mut self,
        pid: u32,
        kind: HandlerKind,
        f: impl FnMut() -> bool + 'static,
    ) {
        if let Some(entry) = self.entries.get_mut(&pid) {
            entry.add_handler(kind, f);
        }
    }

    pub fn set_collider_consumes_click(&mut self, pid: u32, value: bool) {
        if let Some(entry) = self.entries.get_mut(&pid) {
            entry.collider.consumes_click = value;
        }
    }

    pub fn set_collider_click_priority(&mut self, pid: u32, value: i32) {
        if let Some(entry) = self.entries.get_mut(&pid) {
            entry.collider.click_priority = value;
        }
    }

    pub fn set_collider_enabled(&mut self, pid: u32, enabled: bool) {
        if let Some(entry) = self.entries.get_mut(&pid) {
            entry.enabled = enabled;
        }
    }

    /// Run GJK test between two collider/virtual entries.
    pub fn gjk_test(&self, a_pid: u32, b_pid: u32) -> bool {
        let (shape_a, pos_a, scl_a) = self.shape_of(a_pid);
        let (shape_b, pos_b, scl_b) = self.shape_of(b_pid);
        let a = ShapeRef { shape: shape_a, position: pos_a, scale: scl_a };
        let b = ShapeRef { shape: shape_b, position: pos_b, scale: scl_b };
        GjkContext::new(&a, &b).perform_test()
    }

    fn shape_of(&self, pid: u32) -> (&Shape, Vec3, Vec3) {
        if pid == 0 {
            (&self.mouse.shape, self.mouse.position, self.mouse.scale)
        } else if pid == 1 {
            (&self.selection.shape, self.selection.position, self.selection.scale)
        } else if let Some(e) = self.entries.get(&pid) {
            (&e.collider.shape, e.collider.position, e.collider.scale)
        } else {
            panic!("unknown collider pid {pid}");
        }
    }

    pub fn begin_frame(&mut self) {
        self.screen.clear();
        let sc = self.screen_collider;
        self.screen = Quadtree::new(sc, 10, 4);

        for (&pid, entry) in &self.entries {
            if !entry.enabled {
                continue;
            }
            let r = entry.collider.shape.rect(entry.collider.position, entry.collider.scale);
            if r.intersects(&sc) {
                self.screen.insert(ColliderHandle { pid, rect: r });
            }
        }

        self.mouse.rect = self.mouse.shape.rect(self.mouse.position, self.mouse.scale);
        self.screen.insert(ColliderHandle { pid: 0, rect: self.mouse.rect });
    }

    pub fn begin_selection(&mut self, mouse_pos: Vec3) {
        self.selection.position = Vec3::new(mouse_pos.x, mouse_pos.y, 0.0);
        self.selection.update_rect();
    }

    pub fn update_selection(&mut self, begin: Vec3, end: Vec3) {
        let min = begin.min(end);
        let max = begin.max(end);
        let delta = max - min;
        self.selection.position = Vec3::new(min.x, min.y, 0.0);
        self.selection.scale = Vec3::new(delta.x, delta.y, 1.0);
        self.selection.update_rect();
    }

    pub fn end_selection(&mut self) {
        let candidates = self.screen.retrieve(&self.selection.rect);
        let candidates: Vec<_> = candidates.iter().copied().cloned().collect();
        for ch in &candidates {
            if ch.pid <= 1 {
                continue;
            }
            if !self.gjk_test(1, ch.pid) {
                continue;
            }
            if let Some(entry) = self.entries.get_mut(&ch.pid) {
                if let Some(handlers) = entry.handlers.get_mut(&HandlerKind::Selection) {
                    for h in handlers.iter_mut() {
                        h.as_mut()();
                    }
                }
            }
        }
        self.selection.position = Vec3::new(-1.0, -1.0, 0.0);
        self.selection.scale = Vec3::ONE;
        self.selection.update_rect();
    }

    pub fn perform_calls(&mut self) {
        // Build this-frame collision table.
        self.collided.clone_from(&self.colliding);
        self.colliding.clear();

        let all_pids: Vec<u32> = {
            let mut pids: Vec<u32> = vec![0, 1];
            pids.extend(self.entries.keys().copied());
            pids
        };

        for &a_pid in &all_pids {
            let handle = self.handle_for(a_pid);
            let candidates: Vec<_> =
                self.screen.retrieve(&handle.rect).iter().copied().cloned().collect();
            for ch in &candidates {
                if ch.pid == a_pid {
                    continue;
                }
                if self.gjk_test(a_pid, ch.pid) {
                    self.colliding.entry(a_pid).or_default().insert(ch.pid, true);
                }
            }
        }

        // Click dispatch.
        // Pre-scan for click consumers.
        {
            let mouse_cands: Vec<_> =
                self.screen.retrieve(&self.mouse.rect).iter().copied().cloned().collect();
            for ch in &mouse_cands {
                if ch.pid <= 1 {
                    continue;
                }
                if let Some(entry) = self.entries.get(&ch.pid) {
                    if entry.collider.consumes_click && self.gjk_test(0, ch.pid) {
                        // uiConsumedClick = true (set by caller in engine)
                        break;
                    }
                }
            }
        }

        // Dispatch click handlers sorted by clickPriority desc, pid asc.
        if !self.mouse.rect.intersects(&self.screen_collider) {
            // mouse outside screen, don't dispatch clicks
        } else if self.mouse_clicked {
            let mouse_cands: Vec<_> =
                self.screen.retrieve(&self.mouse.rect).iter().copied().cloned().collect();
            let mut click_targets: Vec<(i32, u32)> = Vec::new();
            for ch in &mouse_cands {
                if ch.pid <= 1 {
                    continue;
                }
                if let Some(entry) = self.entries.get(&ch.pid) {
                    if entry.has_handlers(HandlerKind::Click) && self.gjk_test(0, ch.pid) {
                        click_targets.push((entry.collider.click_priority, ch.pid));
                    }
                }
            }
            click_targets.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
            for (_pri, pid) in click_targets {
                if let Some(entry) = self.entries.get_mut(&pid) {
                    let consumes = entry.collider.consumes_click;
                    let mut stop = false;
                    if let Some(handlers) = entry.handlers.get_mut(&HandlerKind::Click) {
                        for h in handlers.iter_mut() {
                            if h.as_mut()() {
                                stop = true;
                                if consumes {
                                    self.consumed_click = true;
                                }
                                break;
                            }
                        }
                    }
                    if stop {
                        break;
                    }
                }
            }
        }

        // Enter/exit dispatch.
        for &a_pid in self.colliding.keys() {
            if a_pid <= 1 {
                continue;
            }
            if let Some(pairs) = self.colliding.get(&a_pid) {
                for &b_pid in pairs.keys() {
                    let was_colliding =
                        self.collided.get(&a_pid).map(|m| m.contains_key(&b_pid)).unwrap_or(false);
                    if !was_colliding {
                        if let Some(entry) = self.entries.get_mut(&a_pid) {
                            if let Some(handlers) = entry.handlers.get_mut(&HandlerKind::Enter) {
                                for h in handlers.iter_mut() {
                                    h.as_mut()();
                                }
                            }
                        }
                    }
                }
            }
        }

        for (&a_pid, prev_pairs) in &self.collided {
            if a_pid <= 1 {
                continue;
            }
            for &b_pid in prev_pairs.keys() {
                let still_colliding =
                    self.colliding.get(&a_pid).map(|m| m.contains_key(&b_pid)).unwrap_or(false);
                if !still_colliding {
                    if let Some(entry) = self.entries.get_mut(&a_pid) {
                        if let Some(handlers) = entry.handlers.get_mut(&HandlerKind::Exit) {
                            for h in handlers.iter_mut() {
                                h.as_mut()();
                            }
                        }
                    }
                }
            }
        }
    }

    fn handle_for(&self, pid: u32) -> ColliderHandle {
        if pid == 0 {
            ColliderHandle { pid: 0, rect: self.mouse.rect }
        } else if pid == 1 {
            ColliderHandle { pid: 1, rect: self.selection.rect }
        } else if let Some(e) = self.entries.get(&pid) {
            let r = e.collider.shape.rect(e.collider.position, e.collider.scale);
            ColliderHandle { pid, rect: r }
        } else {
            ColliderHandle { pid, rect: Rect::default() }
        }
    }
}

impl Default for PhysicsProvider {
    fn default() -> Self {
        Self::new()
    }
}
