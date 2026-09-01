//! # Skill: `classic-ui`
//!
//! **Read `.claude/skills/classic-ui/SKILL.md` before working on this module.**
//!
//! Host-owned container-inventory hover tooltip, rendered through the
//! retained-mode UI toolkit as a single grid of icon + amount children.
//!
//! The guest is a thin *intent* layer: it reports *which* entity is hovered and
//! *when* to show/hide (via [`Engine::inventory_ui_show`]); the host owns the
//! inventory data and the rendering.  The guest populates inventory contents
//! with the generic `inventory_add` import; the tooltip resolves icons and
//! amounts host-side from the entity's [`Inventory`] + [`ItemRegistry`].

use hecs::Entity;

use classic_core::components::{
    SdfTextRender, TextJustify, Transform, UiAlign, UiAnchor, UiNode, DEFAULT_SDF_FONT,
};
use classic_core::inventory::Inventory;
use classic_core::sdf_builder::build_sdf_glyph_buffer;

use crate::Engine;

/// Packed-atlas texture holding the item icons (frame name == item name).
const ICON_TEXTURE: &str = "icons";
/// Target content size (px) of an icon cell.
const ICON_SIZE: f32 = 48.0;
/// SDF scale for the amount labels.
const TEXT_SCALE: f32 = 0.6;
const COL_GAP: f32 = 8.0;
const ROW_GAP: f32 = 4.0;
/// Gap (px) above the hovered container's ground anchor (the container sprite
/// extends well above its contact point, so this clears its top edge; tune via
/// the manual hover check).
const VERT_OFFSET: f32 = 100.0;
/// Z layer of the tooltip panel, below (more negative than) the default HUD
/// (-1000) so the tooltip draws on top of it.  The render list is sorted
/// descending and drawn forward, so the *smallest* (most-negative) sort key is
/// drawn last = on top.
const TOOLTIP_BG_Z: f32 = -1100.0;
/// Z layer of the icon/amount cells, below the panel so they draw on top of it.
const TOOLTIP_CELL_Z: f32 = -1200.0;
const BG_COLOR: [f32; 4] = [0.0, 0.0, 0.0, 0.72];
const TEXT_COLOR: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

/// Host-owned tooltip state: the hover target and the (lazily created) grid,
/// plus the item signature the grid currently reflects.
///
/// A single `UiKind::Grid` is used (no wrapping `Padding`) so the grid owns its
/// children's absolute layout: `layout_grid` positions cells relative to the
/// grid's *current* `Transform.position`, so re-running it after moving the grid
/// drags the cells along with it.  Nested wrappers break this (the wrapper
/// repositions its child after the child has already laid out its own children).
#[derive(Default)]
pub struct InventoryUi {
    target: Option<Entity>,
    grid: Option<Entity>,
    /// The grid's current children (icon + amount per item), despawned on
    /// rebuild.
    cells: Vec<Entity>,
    /// The `(item_id, count)` signature the grid reflects.
    signature: Vec<(u32, u32)>,
    /// The entity the grid was last built for (a target switch rebuilds).
    built_for: Option<Entity>,
}

impl InventoryUi {
    /// Set (or clear) the hover target.  Empty target hides the tooltip.
    pub fn set_target(&mut self, target: Option<Entity>) {
        self.target = target;
    }

    /// Reconcile the tooltip with the current hover target.  Called each frame
    /// from [`Engine::frame`] (via a take-and-restore so `self` can re-borrow
    /// `engine`).  Rebuilds the grid contents only when the hovered entity or
    /// its stacks change, but re-projects the panel to the container's screen
    /// anchor every frame (so it tracks camera pan/zoom).  No-op until the demo
    /// layer has installed a [`UIManager`].
    pub fn sync(&mut self, engine: &mut Engine) {
        let Some(target) = self.target else {
            self.hide(engine);
            return;
        };

        let Some(stacks) =
            engine.world.get::<&Inventory>(target).ok().map(|inv| inv.stacks.clone())
        else {
            self.hide(engine);
            return;
        };
        if stacks.is_empty() {
            self.hide(engine);
            return;
        }

        if self.grid.is_none() {
            if engine.ui.is_none() {
                return;
            }
            self.build(engine);
        }

        let changed = self.built_for != Some(target) || self.signature != stacks;
        if changed {
            self.rebuild(engine, &stacks);
            self.signature = stacks;
            self.built_for = Some(target);
        }

        self.position(engine, target);

        if let Some(grid) = self.grid {
            engine.set_enabled(grid, true);
        }
    }

    fn hide(&mut self, engine: &mut Engine) {
        if let Some(grid) = self.grid {
            engine.set_enabled(grid, false);
        }
    }

    /// Create the tooltip grid (hidden until populated).
    fn build(&mut self, engine: &mut Engine) {
        let Some(ui) = engine.ui.as_mut() else { return };
        let grid = ui.spawn_grid(&mut engine.world, 2, COL_GAP, ROW_GAP, UiAlign::Center, BG_COLOR);
        self.set_z(engine, grid);
        engine.set_enabled(grid, false);
        self.grid = Some(grid);
    }

    /// Rebuild the grid children from `stacks` (icon + amount per item).
    fn rebuild(&mut self, engine: &mut Engine, stacks: &[(u32, u32)]) {
        let Some(grid) = self.grid else { return };

        // Despawn the previous cells and reset the grid's child list.
        for e in self.cells.drain(..) {
            let _ = engine.world.despawn(e);
        }
        if let Ok(mut node) = engine.world.get::<&mut UiNode>(grid) {
            node.children.clear();
        }

        // Spawn fresh icon + amount cells in order (row-major, 2 columns).
        let mut cells = Vec::new();
        {
            let Some(ui) = engine.ui.as_mut() else { return };
            for (item_id, count) in stacks {
                // Only spawn the icon when its frame resolves in the shared
                // `icons` sheet — otherwise the UiSprite render arm would fall
                // back to a blank grid frame.
                let Some(def) = engine.items.def(*item_id) else { continue };
                let icon = def.icon_frame_name().to_string();
                if Engine::resolve_frame(&engine.frame_tables, ICON_TEXTURE, &icon).is_some() {
                    let se = ui.spawn_sprite_frame(
                        &mut engine.world,
                        ICON_TEXTURE,
                        &icon,
                        ICON_SIZE,
                        ICON_SIZE,
                    );
                    ui.container_add_child(
                        &mut engine.world,
                        grid,
                        se,
                        UiAnchor::TopLeft,
                        UiAnchor::TopLeft,
                    );
                    cells.push(se);
                }
                let text = count.to_string();
                let te = ui.spawn_sdf_text(
                    &mut engine.world,
                    &text,
                    TEXT_SCALE,
                    200.0,
                    TEXT_COLOR,
                    TextJustify::Left,
                );
                ui.container_add_child(
                    &mut engine.world,
                    grid,
                    te,
                    UiAnchor::TopLeft,
                    UiAnchor::TopLeft,
                );
                cells.push(te);
            }
        }
        // Measure the freshly spawned amount labels (the render loop only
        // measures them on the following frame) before the next layout.
        for e in &cells {
            self.measure_text(engine, *e);
        }
        self.set_z(engine, grid);
        self.cells = cells;
    }

    /// Re-project the tooltip to the target's screen anchor and re-run the grid
    /// layout so the cells track the grid.  Called every frame while hovering.
    fn position(&mut self, engine: &mut Engine, target: Entity) {
        let Some(grid) = self.grid else { return };

        let (x, y) = engine
            .world
            .get::<&Transform>(target)
            .map(|t| (t.position.x, t.position.y))
            .unwrap_or((0.0, 0.0));
        let Some((sx, sy)) = engine.iso_to_screen_px(x, y) else { return };

        // First layout: size the grid (and place cells at its current spot).
        if let Some(ui) = engine.ui.as_mut() {
            ui.layout_standalone(grid, &mut engine.world);
        }
        let (w, h) =
            engine.world.get::<&UiNode>(grid).map(|n| (n.size.x, n.size.y)).unwrap_or((0.0, 0.0));

        if let Ok(mut tf) = engine.world.get::<&mut Transform>(grid) {
            tf.position.x = sx - w / 2.0;
            tf.position.y = sy - h - VERT_OFFSET;
        }
        // Second layout: re-position the cells under the moved grid.
        if let Some(ui) = engine.ui.as_mut() {
            ui.layout_standalone(grid, &mut engine.world);
        }
    }

    /// Measure an SDF text cell and write its true size into `UiNode.size`.
    fn measure_text(&self, engine: &mut Engine, entity: Entity) {
        let Ok(text) = engine.world.get::<&SdfTextRender>(entity).map(|s| s.text.clone()) else {
            return;
        };
        let scale = engine.world.get::<&Transform>(entity).map(|t| t.scale.x).unwrap_or(1.0);
        let Some(font) = engine.sdf_fonts.get(DEFAULT_SDF_FONT).cloned() else { return };
        let buf = build_sdf_glyph_buffer(&font, &text, scale, TextJustify::Left, 0.0);
        if let Ok(mut node) = engine.world.get::<&mut UiNode>(entity) {
            node.size.x = buf.text_width;
            node.size.y = buf.text_height;
        }
    }

    /// Set the draw Z: the panel itself gets [`TOOLTIP_BG_Z`] and every
    /// descendant (the icon/amount cells) gets [`TOOLTIP_CELL_Z`], so the cells
    /// draw on top of the panel background.
    fn set_z(&self, engine: &mut Engine, grid: Entity) {
        if let Ok(mut tf) = engine.world.get::<&mut Transform>(grid) {
            tf.position.z = TOOLTIP_BG_Z;
        }
        let mut stack = vec![grid];
        while let Some(e) = stack.pop() {
            if let Ok(node) = engine.world.get::<&UiNode>(e) {
                for child in &node.children {
                    if let Ok(mut ctf) = engine.world.get::<&mut Transform>(child.entity) {
                        ctf.position.z = TOOLTIP_CELL_Z;
                    }
                    stack.push(child.entity);
                }
            }
        }
    }
}
