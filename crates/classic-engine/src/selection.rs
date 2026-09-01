//! # Skill: `classic-physics`
//!
//! **Read `.agents/skills/classic-physics/SKILL.md` before working on this module.**
//!
//! Host-owned RTS selection.  The [`SelectionSet`] lives on the [`Engine`] and
//! is driven by the host's click/drag-box/shift input (see `Engine::frame`);
//! ROM guests read the set through `selected_names()` and can clear it through
//! `selection_clear()`.  Hit-testing walks the physics `point_query`/`box_query`
//! results (collider pid → name → entity) and gates on the [`Selectable`]
//! component.

use std::collections::{BTreeSet, HashSet};

use classic_core::collision::polygon_from_verts;
use classic_core::components::{
    ColliderData, DebugName, IsoSprite, IsoVehicle, Selectable, Tilemap,
};
use classic_core::math::iso_to_cartesian_4;
use classic_core::tilemap::bilinear_height;
use classic_core::{RoleKind, Transform};
use glam::{Mat4, Vec2, Vec3};

use crate::Engine;

/// The current RTS selection: a set of entity handles plus a monotonic
/// `version` bumped on every membership change.
#[derive(Clone, Debug, Default)]
pub struct SelectionSet {
    pub selected: BTreeSet<hecs::Entity>,
    pub version: u64,
}

impl SelectionSet {
    pub fn is_empty(&self) -> bool {
        self.selected.is_empty()
    }

    pub fn len(&self) -> usize {
        self.selected.len()
    }

    pub fn contains(&self, e: hecs::Entity) -> bool {
        self.selected.contains(&e)
    }
}

impl Engine {
    /// The current selection set (read-only).
    pub fn selection(&self) -> &SelectionSet {
        &self.selection
    }

    /// Select the top selectable entity under a screen point.  A plain click
    /// (`additive == false`) uses unit/building group semantics: clicking a
    /// unit replaces the set with it, while clicking a building or empty ground
    /// keeps any selected unit (a command target, not a re-selection) and only
    /// replaces/clears when no unit is selected.  A shift-click (`additive ==
    /// true`) toggles membership.
    pub fn select_at(&mut self, x: f32, y: f32, additive: bool) {
        let hit = self.selectable_at(x, y);
        let before = self.selection.selected.clone();
        if additive {
            if let Some((e, _group)) = hit {
                if !self.selection.selected.remove(&e) {
                    self.selection.selected.insert(e);
                }
            }
        } else {
            let has_unit = self.has_selected_unit();
            match hit {
                Some((e, 0)) => {
                    // Click a unit → select only it.
                    self.selection.selected.clear();
                    self.selection.selected.insert(e);
                }
                Some((e, _group)) => {
                    // Click a building → keep a selected unit (command), else
                    // select the building.
                    if !has_unit {
                        self.selection.selected.clear();
                        self.selection.selected.insert(e);
                    }
                }
                None => {
                    // Click empty ground → keep a selected unit (move command),
                    // else clear.
                    if !has_unit {
                        self.selection.selected.clear();
                    }
                }
            }
        }
        if self.selection.selected != before {
            self.selection.version += 1;
        }
    }

    /// Whether the selection contains a unit (`Selectable.group == 0`), i.e. a
    /// vehicle/character rather than a building/structure.
    pub fn has_selected_unit(&self) -> bool {
        self.selection
            .selected
            .iter()
            .any(|e| self.world.get::<&Selectable>(*e).map(|s| s.group == 0).unwrap_or(false))
    }

    /// Select every selectable entity whose collider intersects the screen
    /// rectangle `begin → end`.  `additive` adds to the current set instead of
    /// replacing it.
    pub fn select_box(&mut self, begin: (f32, f32), end: (f32, f32), additive: bool) {
        let hits = self.selectables_in_box(begin, end);
        let before = self.selection.selected.clone();
        if additive {
            for e in hits {
                self.selection.selected.insert(e);
            }
        } else {
            self.selection.selected = hits.into_iter().collect();
        }
        if self.selection.selected != before {
            self.selection.version += 1;
        }
    }

    /// Clear the selection (bumps `version` only if it was non-empty).
    pub fn selection_clear(&mut self) {
        if !self.selection.selected.is_empty() {
            self.selection.selected.clear();
            self.selection.version += 1;
        }
    }

    /// The debug names of the currently-selected entities.
    pub fn selected_names(&self) -> Vec<String> {
        self.selection
            .selected
            .iter()
            .filter_map(|e| self.world.get::<&DebugName>(*e).ok().map(|n| n.0.clone()))
            .collect()
    }

    /// The top [`Selectable`] entity under a screen point, plus its `group`, if
    /// any.
    fn selectable_at(&self, x: f32, y: f32) -> Option<(hecs::Entity, u32)> {
        self.physics.point_query(x, y).into_iter().find_map(|pid| {
            let name = self.collider_names.get(&pid)?;
            let entity = self.names.get(name)?;
            if self.is_disabled(*entity) {
                return None;
            }
            let sel = self.world.get::<&Selectable>(*entity).ok()?;
            Some((*entity, sel.group))
        })
    }

    /// The [`Selectable`] entities whose colliders intersect a screen rectangle.
    fn selectables_in_box(&self, begin: (f32, f32), end: (f32, f32)) -> Vec<hecs::Entity> {
        self.physics
            .box_query(begin.0, begin.1, end.0, end.1)
            .into_iter()
            .filter_map(|pid| {
                let name = self.collider_names.get(&pid)?;
                let entity = self.names.get(name)?;
                (!self.is_disabled(*entity) && self.world.get::<&Selectable>(*entity).is_ok())
                    .then_some(*entity)
            })
            .collect()
    }

    /// The footprint corners (iso tile space, relative to the sprite position)
    /// used to build a selectable entity's collider: the `IsoSprite.footprint`
    /// when present, else the vehicle's `path_footprint` AABB, else a default
    /// unit diamond.
    fn selectable_footprint(&self, entity: hecs::Entity) -> Vec<Vec2> {
        if let Ok(s) = self.world.get::<&IsoSprite>(entity) {
            if !s.footprint.is_empty() {
                return s.footprint.clone();
            }
        }
        if let Ok(v) = self.world.get::<&IsoVehicle>(entity) {
            if !v.path_footprint.is_empty() {
                let min_x = v.path_footprint.iter().map(|o| o.0).min().unwrap_or(0) as f32 - 0.5;
                let max_x = v.path_footprint.iter().map(|o| o.0).max().unwrap_or(0) as f32 + 0.5;
                let min_y = v.path_footprint.iter().map(|o| o.1).min().unwrap_or(0) as f32 - 0.5;
                let max_y = v.path_footprint.iter().map(|o| o.1).max().unwrap_or(0) as f32 + 0.5;
                return vec![
                    Vec2::new(max_x, min_y),
                    Vec2::new(max_x, max_y),
                    Vec2::new(min_x, max_y),
                    Vec2::new(min_x, min_y),
                ];
            }
        }
        vec![Vec2::new(0.5, -0.5), Vec2::new(0.5, 0.5), Vec2::new(-0.5, 0.5), Vec2::new(-0.5, -0.5)]
    }

    /// Build the world-space footprint polygon for a selectable entity (iso
    /// footprint corners → world cartesian, height-lifted like the sprite model).
    fn selectable_world_polygon(
        &self,
        tm_entity: hecs::Entity,
        iso_to_cart_world: &Mat4,
        tilemap_pos: Vec3,
        pos: Vec2,
        footprint: &[Vec2],
    ) -> Option<classic_core::components::Shape> {
        let tm = self.world.get::<&Tilemap>(tm_entity).ok()?;
        let hd = &tm.height_data;
        let sx = tm.size_x;
        let sy = tm.size_y;
        let hs = tm.height_scale;
        let mut world_verts = Vec::with_capacity(footprint.len());
        for pt in footprint {
            let px = pos.x + pt.x;
            let py = pos.y + pt.y;
            let h = bilinear_height(hd, sx, sy, px, py);
            let mut v = iso_to_cart_world.transform_point3(Vec3::new(px, py, 0.0));
            v += tilemap_pos;
            v.y -= h * hs;
            world_verts.push(v);
        }
        Some(polygon_from_verts(world_verts))
    }

    /// Ensure every selectable entity has an up-to-date world-space collider
    /// (projected to screen by the physics system each frame).  Runs every frame
    /// before `begin_frame`; creates colliders on first sight and updates them in
    /// place afterwards.  Disabled entities are skipped.
    pub fn sync_selectable_colliders(&mut self) {
        let Some(tm_entity) = self.entity_by_role(RoleKind::Tilemap) else { return };

        let (iso_to_cart_world, tilemap_pos) = {
            let Some(tm_tf) = self.world.get::<&Transform>(tm_entity).ok() else { return };
            (iso_to_cartesian_4() * Mat4::from_scale(tm_tf.scale), tm_tf.position)
        };

        // Phase 1: gather (name, world polygon) for every visible selectable.
        let mut updates: Vec<(String, classic_core::components::Shape)> = Vec::new();
        {
            // Position comes from `Transform`, not `IsoSprite.position`: the
            // latter is only written at spawn/clone time and goes stale once an
            // entity moves (the vehicle sim and `set_pos` both write `Transform`
            // exclusively).  Reading `IsoSprite.position` left a moved LRV's
            // collider stuck at its spawn point, so it stopped being clickable
            // after driving away.
            let mut query = self.world.query::<(&Selectable, &IsoSprite, &Transform)>();
            for (entity, (_sel, _sprite, tf)) in query.iter() {
                if self.is_disabled(entity) {
                    continue;
                }
                let pos = Vec2::new(tf.position.x, tf.position.y);
                let footprint = self.selectable_footprint(entity);
                if let Some(shape) = self.selectable_world_polygon(
                    tm_entity,
                    &iso_to_cart_world,
                    tilemap_pos,
                    pos,
                    &footprint,
                ) {
                    updates.push((self.debug_name(entity), shape));
                }
            }
        }

        // Phase 2: register or update colliders in place.
        let mut active: HashSet<String> = HashSet::new();
        for (name, shape) in updates {
            active.insert(name.clone());
            if let Some(&pid) = self.collider_pids.get(&name) {
                self.physics.update_world_shape(pid, shape);
                self.physics.set_collider_enabled(pid, true);
            } else {
                self.register_named_collider(&name, ColliderData::world(shape));
            }
            self.selectable_colliders.insert(name);
        }

        // Phase 3: disable stale selectable colliders (their entity became
        // disabled or lost its Selectable), so point_query/pick_at skip them.
        let stale: Vec<String> =
            self.selectable_colliders.iter().filter(|n| !active.contains(*n)).cloned().collect();
        for name in stale {
            if let Some(&pid) = self.collider_pids.get(&name) {
                self.physics.set_collider_enabled(pid, false);
            }
            self.selectable_colliders.remove(&name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use classic_core::components::Selectable;

    /// Spawn a named selectable entity with an AABB collider at `(x, y, w, h)`.
    fn add_selectable(engine: &mut Engine, name: &str, x: f32, y: f32, w: f32, h: f32, group: u32) {
        assert!(engine.spawn_named(name));
        let entity = *engine.names.get(name).unwrap();
        engine.world.insert_one(entity, Selectable { priority: 0, group }).unwrap();
        engine.spawn_collider(name, x, y, w, h);
    }

    #[test]
    fn click_selects_top_entity_only() {
        let mut e = Engine::new_for_test();
        add_selectable(&mut e, "a", 10.0, 10.0, 10.0, 10.0, 0);
        add_selectable(&mut e, "b", 30.0, 10.0, 10.0, 10.0, 0);
        e.physics.begin_frame();

        e.select_at(15.0, 15.0, false);
        assert_eq!(e.selected_names(), vec!["a".to_string()]);

        // A plain click on empty ground keeps the selected unit (a move command).
        e.select_at(100.0, 100.0, false);
        assert_eq!(e.selected_names(), vec!["a".to_string()]);
    }

    #[test]
    fn click_building_with_unit_selected_keeps_unit() {
        let mut e = Engine::new_for_test();
        add_selectable(&mut e, "a", 10.0, 10.0, 10.0, 10.0, 0);
        add_selectable(&mut e, "bldg", 60.0, 60.0, 10.0, 10.0, 1);
        e.physics.begin_frame();

        e.select_at(15.0, 15.0, false);
        assert_eq!(e.selected_names(), vec!["a".to_string()]);

        // Clicking a building while a unit is selected is a command, not a
        // re-selection: the unit stays selected.
        e.select_at(65.0, 65.0, false);
        assert_eq!(e.selected_names(), vec!["a".to_string()]);
    }

    #[test]
    fn empty_click_clears_when_only_building_selected() {
        let mut e = Engine::new_for_test();
        add_selectable(&mut e, "bldg", 60.0, 60.0, 10.0, 10.0, 1);
        e.physics.begin_frame();

        // With no unit selected, a building click selects the building.
        e.select_at(65.0, 65.0, false);
        assert_eq!(e.selected_names(), vec!["bldg".to_string()]);

        // And an empty click clears it.
        e.select_at(10.0, 10.0, false);
        assert!(e.selection.is_empty());
    }

    #[test]
    fn shift_click_toggles_additively() {
        let mut e = Engine::new_for_test();
        add_selectable(&mut e, "a", 10.0, 10.0, 10.0, 10.0, 0);
        add_selectable(&mut e, "b", 30.0, 10.0, 10.0, 10.0, 0);
        e.physics.begin_frame();

        e.select_at(15.0, 15.0, false);
        e.select_at(35.0, 15.0, true);
        let mut names = e.selected_names();
        names.sort();
        assert_eq!(names, vec!["a".to_string(), "b".to_string()]);

        // Shift-clicking an already-selected entity removes it.
        e.select_at(15.0, 15.0, true);
        assert_eq!(e.selected_names(), vec!["b".to_string()]);
    }

    #[test]
    fn drag_box_selects_multiple_and_respects_additive() {
        let mut e = Engine::new_for_test();
        add_selectable(&mut e, "a", 10.0, 10.0, 10.0, 10.0, 0);
        add_selectable(&mut e, "b", 30.0, 10.0, 10.0, 10.0, 0);
        add_selectable(&mut e, "c", 10.0, 40.0, 10.0, 10.0, 0);
        e.physics.begin_frame();

        e.select_box((5.0, 5.0), (45.0, 25.0), false);
        let mut names = e.selected_names();
        names.sort();
        assert_eq!(names, vec!["a".to_string(), "b".to_string()]);

        // Additive box adds to the existing set.
        e.select_box((5.0, 35.0), (25.0, 55.0), true);
        let mut names = e.selected_names();
        names.sort();
        assert_eq!(names, vec!["a".to_string(), "b".to_string(), "c".to_string()]);

        // Non-additive box replaces the set.
        e.select_box((5.0, 35.0), (25.0, 55.0), false);
        assert_eq!(e.selected_names(), vec!["c".to_string()]);
    }

    #[test]
    fn version_bumps_only_on_change() {
        let mut e = Engine::new_for_test();
        add_selectable(&mut e, "a", 10.0, 10.0, 10.0, 10.0, 0);
        e.physics.begin_frame();

        e.select_at(15.0, 15.0, false);
        let v1 = e.selection.version;
        assert!(v1 > 0);

        // Selecting the same entity again (plain click) does not bump version.
        e.select_at(15.0, 15.0, false);
        assert_eq!(e.selection.version, v1);

        // Clearing a non-empty selection bumps it.
        e.selection_clear();
        assert_eq!(e.selection.version, v1 + 1);

        // Clearing an already-empty selection does not.
        e.selection_clear();
        assert_eq!(e.selection.version, v1 + 1);
    }
}
