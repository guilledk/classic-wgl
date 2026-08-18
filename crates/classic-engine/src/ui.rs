//! # Skill: `classic-ui`
//!
//! **Read `.claude/skills/classic-ui/SKILL.md` before working on this module.**
//!
//! Retained-mode UI layout manager.
//!
//! Port of `UIManager` + `UIContainer` from `src/classic/ui.ts`.
//! UI elements are regular ECS entities with `Transform` + a render component
//! (`RectRender` / `SdfTextRender`) + a `UiNode` layout component.

use std::collections::HashMap;

use glam::{Vec2, Vec4};
use hecs::World;

use classic_core::collision::{HandlerKind, PhysicsProvider};
use classic_core::components::TextJustify;
use classic_core::components::{
    Disabled, RectRender, SdfTextRender, SpriteRender, Transform, UiAlign, UiAnchor, UiChild,
    UiKind, UiNode,
};

struct UiColliderEntry {
    elem: hecs::Entity,
    collider_pid: u32,
    base_color: [f32; 4],
    click_frames: u32,
}

/// Options for `spawn_button`.
pub struct ButtonOptions {
    pub text: Option<String>,
    pub text_scale: f32,
    pub text_color: [f32; 4],
    pub sdf_text: bool,
    pub sprite: Option<String>,
    pub sprite_frame: f32,
    pub sprite_tile_set: [f32; 2],
    pub click_priority: i32,
    pub hover: bool,
    pub click_feedback: Option<u32>,
    pub click_action: Option<Box<dyn FnMut() -> bool>>,
}

impl Default for ButtonOptions {
    fn default() -> Self {
        Self {
            text: None,
            text_scale: 0.5,
            text_color: [1.0, 1.0, 1.0, 1.0],
            sdf_text: false,
            sprite: None,
            sprite_frame: 0.0,
            sprite_tile_set: [1.0, 1.0],
            click_priority: 0,
            hover: false,
            click_feedback: None,
            click_action: None,
        }
    }
}

pub struct UIManager {
    pub root: hecs::Entity,
    pub dirty: bool,
    pub viewport_w: f32,
    pub viewport_h: f32,
    elements: HashMap<String, hecs::Entity>,
    index_counter: u32,
    zlayer: i32,
    element_colliders: Vec<UiColliderEntry>,
}

impl UIManager {
    /// Create a new UIManager with a root container sized to the viewport.
    pub fn new(vp_w: f32, vp_h: f32, world: &mut World) -> Self {
        let root =
            Self::spawn_container_internal(world, vp_w, vp_h, [0.0, 0.0, 0.0, 0.0], 0, &mut 0);

        let mut ui = Self {
            root,
            dirty: true,
            viewport_w: vp_w,
            viewport_h: vp_h,
            elements: HashMap::new(),
            index_counter: 0,
            zlayer: -1000,
            element_colliders: Vec::new(),
        };
        ui.mark_dirty();
        ui
    }

    fn gen_name(&mut self, kind: &str) -> String {
        let n = self.index_counter;
        self.index_counter += 1;
        format!("ui-{n}-{kind}")
    }

    // ---- factory methods --------------------------------------------------

    /// Create a solid-color rectangle container.
    pub fn spawn_container(
        &mut self,
        world: &mut World,
        w: f32,
        h: f32,
        color: [f32; 4],
    ) -> hecs::Entity {
        let e = Self::spawn_container_internal(
            world,
            w,
            h,
            color,
            self.zlayer,
            &mut self.index_counter,
        );
        let name = self.gen_name("container");
        self.elements.insert(name, e);
        self.mark_dirty();
        e
    }

    fn spawn_container_internal(
        world: &mut World,
        w: f32,
        h: f32,
        color: [f32; 4],
        zlayer: i32,
        counter: &mut u32,
    ) -> hecs::Entity {
        let n = *counter;
        *counter += 1;
        let _name = format!("ui-{n}-container");
        world.spawn((
            Transform::new(glam::Vec3::new(0.0, 0.0, zlayer as f32), glam::Vec3::ONE),
            RectRender { color, ignore_cam: true },
            UiNode {
                parent: None,
                children: Vec::new(),
                size: Vec2::new(w, h),
                anchor: UiAnchor::MidCenter,
                fixed: false,
                clip_children: false,
                scroll_y: 0.0,
                clip_rect: Vec4::ZERO,
                kind: UiKind::Container,
            },
        ))
    }

    /// Create an SDF text element.
    pub fn spawn_sdf_text(
        &mut self,
        world: &mut World,
        text: &str,
        scale: f32,
        max_width: f32,
        color: [f32; 4],
        justify: TextJustify,
    ) -> hecs::Entity {
        let name = self.gen_name("sdf-text");
        let e = world.spawn((
            Transform::new(
                glam::Vec3::new(0.0, 0.0, self.zlayer as f32),
                glam::Vec3::new(scale, scale, 1.0),
            ),
            SdfTextRender {
                atlas_name: classic_core::components::DEFAULT_SDF_FONT.into(),
                color,
                bgcolor: [0.0, 0.0, 0.0, 0.0],
                outline_color: [0.0, 0.0, 0.0, 0.0],
                outline_width: 0.0,
                shadow_offset: [1.0, 1.0],
                shadow_color: [0.0, 0.0, 0.0, 0.5],
                shadow_blur: 0.0,
                ignore_cam: true,
                text: text.to_string(),
                justify,
                weight: 0.0,
                gamma: 1.0,
            },
            UiNode {
                parent: None,
                children: Vec::new(),
                size: Vec2::new(max_width, 0.0),
                anchor: UiAnchor::TopLeft,
                fixed: false,
                clip_children: false,
                scroll_y: 0.0,
                clip_rect: Vec4::ZERO,
                kind: UiKind::SdfText,
            },
        ));
        self.elements.insert(name, e);
        self.mark_dirty();
        e
    }

    /// Create a flex-like array container (vertical or horizontal).
    pub fn spawn_array(
        &mut self,
        world: &mut World,
        vertical: bool,
        align: UiAlign,
        spacing: f32,
        color: [f32; 4],
    ) -> hecs::Entity {
        let name = self.gen_name("array");
        let e = world.spawn((
            Transform::new(glam::Vec3::new(0.0, 0.0, self.zlayer as f32), glam::Vec3::ONE),
            RectRender { color, ignore_cam: true },
            UiNode {
                parent: None,
                children: Vec::new(),
                size: Vec2::new(10.0, 10.0),
                anchor: UiAnchor::TopLeft,
                fixed: false,
                clip_children: false,
                scroll_y: 0.0,
                clip_rect: Vec4::ZERO,
                kind: UiKind::Array { vertical, align, spacing },
            },
        ));
        self.elements.insert(name, e);
        self.mark_dirty();
        e
    }

    /// Create a single-child padding wrapper.
    pub fn spawn_padding(
        &mut self,
        world: &mut World,
        top: f32,
        right: f32,
        bottom: f32,
        left: f32,
        color: [f32; 4],
    ) -> hecs::Entity {
        let name = self.gen_name("padding");
        let e = world.spawn((
            Transform::new(glam::Vec3::new(0.0, 0.0, self.zlayer as f32), glam::Vec3::ONE),
            RectRender { color, ignore_cam: true },
            UiNode {
                parent: None,
                children: Vec::new(),
                size: Vec2::new(10.0, 10.0),
                anchor: UiAnchor::TopLeft,
                fixed: false,
                clip_children: false,
                scroll_y: 0.0,
                clip_rect: Vec4::ZERO,
                kind: UiKind::Padding { top, right, bottom, left },
            },
        ));
        self.elements.insert(name, e);
        self.mark_dirty();
        e
    }

    /// Create a texture-based sprite UI element.
    pub fn spawn_sprite(
        &mut self,
        world: &mut World,
        texture: &str,
        width: f32,
        height: f32,
        frame: f32,
        tile_set_size: [f32; 2],
    ) -> hecs::Entity {
        let name = self.gen_name("sprite");
        let e = world.spawn((
            Transform::new(glam::Vec3::new(0.0, 0.0, self.zlayer as f32), glam::Vec3::ONE),
            SpriteRender {
                position: glam::Vec3::ZERO,
                scale: glam::Vec3::ONE,
                texture: texture.to_string(),
                ignore_cam: true,
                frame,
                tile_set_size: glam::Vec2::new(tile_set_size[0], tile_set_size[1]),
                anchor: glam::Vec2::ZERO,
            },
            UiNode {
                parent: None,
                children: Vec::new(),
                size: Vec2::new(width, height),
                anchor: UiAnchor::TopLeft,
                fixed: false,
                clip_children: false,
                scroll_y: 0.0,
                clip_rect: Vec4::ZERO,
                kind: UiKind::Sprite,
            },
        ));
        self.elements.insert(name, e);
        self.mark_dirty();
        e
    }

    /// Create an interactive button: container + optional text/sprite child +
    /// collider with click handler + optional hover highlighting.
    pub fn spawn_button(
        &mut self,
        world: &mut World,
        physics: &mut PhysicsProvider,
        width: f32,
        height: f32,
        color: [f32; 4],
        mut opts: ButtonOptions,
    ) -> hecs::Entity {
        let container = self.spawn_container(world, width, height, color);

        if let Some(sprite_name) = opts.sprite.take() {
            let sprite = self.spawn_sprite(
                world,
                &sprite_name,
                width,
                height,
                opts.sprite_frame,
                opts.sprite_tile_set,
            );
            self.container_add_child(
                world,
                container,
                sprite,
                UiAnchor::MidCenter,
                UiAnchor::MidCenter,
            );
        } else if let Some(text) = opts.text.take() {
            if opts.sdf_text {
                let sdf_scale = opts.text_scale * 2.5;
                let child = self.spawn_sdf_text(
                    world,
                    &text,
                    sdf_scale,
                    300.0,
                    opts.text_color,
                    TextJustify::Center,
                );
                self.container_add_child(
                    world,
                    container,
                    child,
                    UiAnchor::MidCenter,
                    UiAnchor::MidCenter,
                );
            }
        }

        let pid = self.add_collider_to_elem(world, container, physics);
        physics.set_collider_consumes_click(pid, true);
        if opts.click_priority != 0 {
            physics.set_collider_click_priority(pid, opts.click_priority);
        }

        if let Some(mut action) = opts.click_action {
            let feedback_frames = opts.click_feedback.unwrap_or(0);
            let container_e = container;
            physics.add_collider_handler(pid, HandlerKind::Click, move || {
                let _ = (feedback_frames, container_e);
                action()
            });
        }

        container
    }

    /// Register a collider for a UI element (for hover + click detection).
    /// Returns the PhysicsProvider collider pid.
    pub fn add_collider_to_elem(
        &mut self,
        world: &mut World,
        elem: hecs::Entity,
        physics: &mut PhysicsProvider,
    ) -> u32 {
        let size = world.get::<&UiNode>(elem).map(|n| n.size).unwrap_or(Vec2::ZERO);
        let pos = world
            .get::<&Transform>(elem)
            .map(|t| (t.position.x, t.position.y))
            .unwrap_or((0.0, 0.0));

        let verts = vec![
            glam::Vec3::new(0.0, 0.0, 0.0),
            glam::Vec3::new(size.x, 0.0, 0.0),
            glam::Vec3::new(size.x, size.y, 0.0),
            glam::Vec3::new(0.0, size.y, 0.0),
        ];
        let shape = classic_core::collision::polygon_from_verts(verts);
        let mut collider = classic_core::components::ColliderData::new(shape);
        collider.position = glam::Vec3::new(pos.0, pos.1, 0.0);
        collider.scale = glam::Vec3::ONE;

        let pid = physics.register_collider(collider);

        let base_color =
            world.get::<&RectRender>(elem).map(|r| r.color).unwrap_or([0.0, 0.0, 0.0, 0.0]);

        self.element_colliders.push(UiColliderEntry {
            elem,
            collider_pid: pid,
            base_color,
            click_frames: 0,
        });

        pid
    }

    // ---- child management -------------------------------------------------

    /// Add a child to a container with anchor-based positioning.
    pub fn container_add_child(
        &mut self,
        world: &mut World,
        container: hecs::Entity,
        child: hecs::Entity,
        self_anchor: UiAnchor,
        child_anchor: UiAnchor,
    ) {
        if let Ok(mut node) = world.get::<&mut UiNode>(container) {
            node.children.push(UiChild { entity: child, self_anchor, child_anchor });
        }
        if let Ok(mut child_node) = world.get::<&mut UiNode>(child) {
            child_node.parent = Some(container);
        }
        self.mark_dirty();
    }

    /// Attach multiple children to a container, all with the same anchor pair.
    pub fn add_children(
        &mut self,
        world: &mut World,
        container: hecs::Entity,
        children: &[hecs::Entity],
        self_anchor: UiAnchor,
        child_anchor: UiAnchor,
    ) {
        for &child in children {
            self.container_add_child(world, container, child, self_anchor, child_anchor);
        }
    }

    /// Convenience: add child to root container.
    pub fn root_add_child(
        &mut self,
        world: &mut World,
        child: hecs::Entity,
        self_anchor: UiAnchor,
        child_anchor: UiAnchor,
    ) {
        let root = self.root;
        self.container_add_child(world, root, child, self_anchor, child_anchor);
    }

    // ---- layout -----------------------------------------------------------

    /// Mark the layout as needing refresh.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Resize the root container for a new viewport.
    pub fn resize(&mut self, world: &mut World, vp_w: f32, vp_h: f32) {
        self.viewport_w = vp_w;
        self.viewport_h = vp_h;
        if let Ok(mut tf) = world.get::<&mut Transform>(self.root) {
            tf.scale.x = vp_w;
            tf.scale.y = vp_h;
        }
        if let Ok(mut node) = world.get::<&mut UiNode>(self.root) {
            node.size = Vec2::new(vp_w, vp_h);
        }

        // Resize full-width child containers (top bar) to match new viewport.
        let children: Vec<hecs::Entity> = world
            .get::<&UiNode>(self.root)
            .map(|n| n.children.iter().map(|c| c.entity).collect())
            .unwrap_or_default();
        for child_e in &children {
            if let Ok(mut child_node) = world.get::<&mut UiNode>(*child_e) {
                if child_node.kind == UiKind::Container && child_node.parent == Some(self.root) {
                    child_node.size.x = vp_w;
                }
            }
        }

        self.mark_dirty();
    }

    /// Refresh the full layout tree.
    pub fn refresh_layout(&mut self, world: &mut World) {
        self.measure_and_position(self.root, world);
        self.dirty = false;
    }

    /// Layout a standalone entity and its children.
    /// For entities NOT in the root tree — call after manually setting position
    /// to run UiKind-aware layout (array stacking, padding, anchor-based children).
    pub fn layout_standalone(&self, entity: hecs::Entity, world: &mut World) {
        self.measure_and_position(entity, world);
    }

    fn measure_and_position(&self, entity: hecs::Entity, world: &mut World) {
        let kind =
            world.get::<&UiNode>(entity).map(|n| n.kind.clone()).unwrap_or(UiKind::Container);

        match kind {
            UiKind::Array { .. } => {
                self.layout_array(entity, world);
            }
            UiKind::Padding { .. } => {
                self.layout_padding(entity, world);
            }
            _ => {
                let children: Vec<(hecs::Entity, UiAnchor, UiAnchor)> = world
                    .get::<&UiNode>(entity)
                    .map(|n| {
                        n.children
                            .iter()
                            .map(|c| (c.entity, c.self_anchor, c.child_anchor))
                            .collect()
                    })
                    .unwrap_or_default();
                if children.is_empty() {
                    return;
                }
                for (child_entity, _sa, _ca) in &children {
                    self.measure_and_position(*child_entity, world);
                }
                for (child_entity, self_anchor, child_anchor) in &children {
                    Self::set_child_position(
                        entity,
                        *child_entity,
                        *self_anchor,
                        *child_anchor,
                        world,
                    );
                }
            }
        }
    }

    /// Layout for UIArray: measure children, resize self, stack along main axis.
    fn layout_array(&self, entity: hecs::Entity, world: &mut World) {
        let (vertical, align, spacing) = match world.get::<&UiNode>(entity).unwrap().kind.clone() {
            UiKind::Array { vertical, align, spacing } => (vertical, align, spacing),
            _ => return,
        };

        let children: Vec<hecs::Entity> = world
            .get::<&UiNode>(entity)
            .map(|n| n.children.iter().map(|c| c.entity).collect())
            .unwrap_or_default();

        for &child in &children {
            self.measure_and_position(child, world);
        }

        let mut total_main = 0.0_f32;
        let mut max_cross = 0.0_f32;
        let enabled: Vec<(hecs::Entity, f32, f32)> = children
            .iter()
            .filter_map(|&e| {
                if world.get::<&Disabled>(e).is_ok() {
                    return None;
                }
                world.get::<&UiNode>(e).ok().map(|n| (e, n.size.x, n.size.y))
            })
            .collect();

        for &(_e, w, h) in &enabled {
            let main = if vertical { h } else { w };
            let cross = if vertical { w } else { h };
            total_main += main + spacing;
            max_cross = max_cross.max(cross);
        }
        total_main = (total_main - spacing).max(0.0);

        let new_w = if vertical { max_cross } else { total_main };
        let new_h = if vertical { total_main } else { max_cross };

        {
            let mut node = world.get::<&mut UiNode>(entity).unwrap();
            node.size = Vec2::new(new_w, new_h);
        }
        {
            let mut tf = world.get::<&mut Transform>(entity).unwrap();
            tf.scale.x = new_w;
            tf.scale.y = new_h;
        }

        let (px, py) = {
            let tf = world.get::<&Transform>(entity).unwrap();
            (tf.position.x, tf.position.y)
        };

        let mut offset = 0.0_f32;
        for &(_e, w, h) in &enabled {
            let main = if vertical { h } else { w };
            let cross = if vertical { w } else { h };

            let cross_offset = match align {
                UiAlign::Left => 0.0_f32,
                UiAlign::Center => max_cross / 2.0 - cross / 2.0,
                UiAlign::Right => max_cross - cross,
            };

            let x = if vertical { px + cross_offset } else { px + offset };
            let y = if vertical { py + offset } else { py + cross_offset };

            if let Ok(mut child_tf) = world.get::<&mut Transform>(_e) {
                child_tf.position.x = x;
                child_tf.position.y = y;
            }

            offset += main + spacing;
        }
    }

    /// Layout for UIPadding: resize self around the single child, position child at offset.
    fn layout_padding(&self, entity: hecs::Entity, world: &mut World) {
        let (top, right, bottom, left) = match world.get::<&UiNode>(entity).unwrap().kind.clone() {
            UiKind::Padding { top, right, bottom, left } => (top, right, bottom, left),
            _ => return,
        };

        let children: Vec<hecs::Entity> = world
            .get::<&UiNode>(entity)
            .map(|n| n.children.iter().map(|c| c.entity).collect())
            .unwrap_or_default();

        for &child in &children {
            self.measure_and_position(child, world);
        }

        let Some(&child_entity) = children.iter().find(|&&e| world.get::<&Disabled>(e).is_err())
        else {
            return;
        };

        let child_size = match world.get::<&UiNode>(child_entity) {
            Ok(node) => (node.size.x, node.size.y),
            Err(_) => return,
        };

        let new_w = child_size.0 + left + right;
        let new_h = child_size.1 + top + bottom;

        {
            let mut node = world.get::<&mut UiNode>(entity).unwrap();
            node.size = Vec2::new(new_w, new_h);
        }
        {
            let mut tf = world.get::<&mut Transform>(entity).unwrap();
            tf.scale.x = new_w;
            tf.scale.y = new_h;
        }

        let (px, py) = {
            let tf = world.get::<&Transform>(entity).unwrap();
            (tf.position.x, tf.position.y)
        };

        if let Ok(mut child_tf) = world.get::<&mut Transform>(child_entity) {
            child_tf.position.x = px + left;
            child_tf.position.y = py + top;
        }
    }

    // ---- collider integration ---------------------------------------------

    /// Sync all UI element collider shapes/positions from current UiNode state.
    pub fn sync_colliders(&mut self, world: &World, physics: &mut PhysicsProvider) {
        for entry in &self.element_colliders {
            let Ok(node) = world.get::<&UiNode>(entry.elem) else {
                continue;
            };
            let Ok(tf) = world.get::<&Transform>(entry.elem) else {
                continue;
            };
            physics.sync_collider_rect(
                entry.collider_pid,
                tf.position.x,
                tf.position.y,
                node.size.x,
                node.size.y,
            );
        }
    }

    /// Per-frame hover highlighting update. Call after physics.begin_frame + perform_calls.
    pub fn update_hover(&mut self, world: &mut World, physics: &PhysicsProvider) {
        let mouse_pid = 0u32;

        for entry in &mut self.element_colliders {
            if entry.click_frames > 0 {
                entry.click_frames -= 1;
                if entry.click_frames > 0 {
                    if let Ok(mut rect) = world.get::<&mut RectRender>(entry.elem) {
                        let b = entry.base_color;
                        rect.color = [1.0, 1.0, 1.0, b[3]];
                    }
                    continue;
                }
            }

            let hovered = physics.gjk_test(entry.collider_pid, mouse_pid);
            if let Ok(mut rect) = world.get::<&mut RectRender>(entry.elem) {
                let b = entry.base_color;
                if hovered {
                    rect.color = [
                        (b[0] + (1.0 - b[0]) * 0.25).min(1.0),
                        (b[1] + (1.0 - b[1]) * 0.25).min(1.0),
                        (b[2] + (1.0 - b[2]) * 0.25).min(1.0),
                        b[3],
                    ];
                } else {
                    rect.color = b;
                }
            }
        }
    }

    /// Collect all collider PIDs for an entity and its recursive children.
    pub fn collect_collider_pids(&self, world: &World, entity: hecs::Entity) -> Vec<u32> {
        let mut pids: Vec<u32> = Vec::new();
        let mut stack = vec![entity];
        while let Some(e) = stack.pop() {
            for entry in &self.element_colliders {
                if entry.elem == e {
                    pids.push(entry.collider_pid);
                }
            }
            if let Ok(node) = world.get::<&UiNode>(e) {
                for child in &node.children {
                    stack.push(child.entity);
                }
            }
        }
        pids
    }

    /// The collider pid registered for a UI element, if any.
    pub fn collider_pid_for(&self, elem: hecs::Entity) -> Option<u32> {
        self.element_colliders.iter().find(|e| e.elem == elem).map(|e| e.collider_pid)
    }

    pub fn set_button_base_color(&mut self, elem: hecs::Entity, color: [f32; 4]) {
        for entry in &mut self.element_colliders {
            if entry.elem == elem {
                entry.base_color = color;
                return;
            }
        }
    }

    /// After a container's position has been manually set, call this to
    /// reposition all its children according to their anchors.
    /// Also applies parent container's scroll_y offset and propagates
    /// clip_rect when clip_children is true.
    pub fn position_children_of(container: hecs::Entity, world: &mut World) {
        let (jobs, clip_rect, _sc_y) = {
            let Ok(tf) = world.get::<&Transform>(container) else {
                return;
            };
            let Ok(node) = world.get::<&UiNode>(container) else {
                return;
            };
            let clip = if node.clip_children {
                Vec4::new(tf.position.x, tf.position.y, node.size.x, node.size.y)
            } else {
                Vec4::ZERO
            };
            let sy = node.scroll_y;
            let jobs: Vec<(hecs::Entity, f32, f32)> = node
                .children
                .iter()
                .filter_map(|c| {
                    let cn = world.get::<&UiNode>(c.entity).ok()?;
                    let po = c.self_anchor.offset(node.size.x, node.size.y);
                    let co = c.child_anchor.offset(cn.size.x, cn.size.y);
                    Some((c.entity, tf.position.x + po.x - co.x, tf.position.y + po.y - co.y - sy))
                })
                .collect();
            (jobs, clip, sy)
        };
        for (child_e, x, y) in &jobs {
            if let Ok(mut ctf) = world.get::<&mut Transform>(*child_e) {
                ctf.position.x = *x;
                ctf.position.y = *y;
            }
            if clip_rect != Vec4::ZERO {
                if let Ok(mut cn) = world.get::<&mut UiNode>(*child_e) {
                    cn.clip_rect = clip_rect;
                }
            }
        }
    }

    /// Compute the position for one child based on anchors, and update its Transform.
    fn set_child_position(
        container: hecs::Entity,
        child: hecs::Entity,
        self_anchor: UiAnchor,
        child_anchor: UiAnchor,
        world: &mut World,
    ) {
        let (px, py, pw, ph) = {
            let tf = world.get::<&Transform>(container).unwrap();
            let node = world.get::<&UiNode>(container).unwrap();
            (tf.position.x, tf.position.y, node.size.x, node.size.y)
        };
        let (cw, ch) = {
            let node = world.get::<&UiNode>(child).unwrap();
            (node.size.x, node.size.y)
        };
        let parent_off = self_anchor.offset(pw, ph);
        let child_off = child_anchor.offset(cw, ch);
        if let Ok(mut child_tf) = world.get::<&mut Transform>(child) {
            child_tf.position.x = px + parent_off.x - child_off.x;
            child_tf.position.y = py + parent_off.y - child_off.y;
        }
    }
}
