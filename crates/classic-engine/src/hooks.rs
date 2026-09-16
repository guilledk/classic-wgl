//! The engine hook surface: per-frame callback registration and the host API
//! guests and prefabs drive (entities, terrain, animation, paths, workers,
//! colliders, events, lights, UI, saves).

use std::sync::Arc;

use classic_core::components::{
    Animator, ColliderData, DebugName, IsoAgent, IsoSprite, IsoVehicle, Light, NavMesh, RectRender,
    Role, SdfTextRender, Selectable, TextJustify, Tilemap, UiAlign, UiAnchor, UiNode,
};
use classic_core::math::{iso_camera_matrix, iso_world_pos};
use classic_core::pathfinder;
use classic_core::tilemap::{bilinear_height, sample_height_mesh, PPM_TARGET, TILE_M};
use classic_core::{RoleKind, SpriteRender, Transform};
use glam::{Mat4, Vec3};

use crate::{ui, Engine, GuestEvent};

impl Engine {
    pub fn on_update(&mut self, f: impl FnMut(&mut Engine) + 'static) {
        self.update_fns.push(Box::new(f));
    }

    /// Register a pre-update callback, run every frame *before* the
    /// `on_update` closures (used by the demo to route the mouse wheel to the
    /// text panel before the camera zoom handler sees it).
    pub fn on_pre_update(&mut self, f: impl FnMut(&mut Engine) + 'static) {
        self.pre_update_hooks.push(Box::new(f));
    }

    /// Register a callback run when a selection drag just ended (replaces the
    /// hardcoded editor-paint that used to live in `frame()`).
    pub fn on_selection_end(&mut self, f: impl FnMut(&mut Engine) + 'static) {
        self.selection_end_hooks.push(Box::new(f));
    }

    /// Register a debug overlay callback, run in the GL draw phase after the
    /// main render list (footprint polygons, agent ring, compass rose, ...).
    pub fn add_overlay(&mut self, f: impl FnMut(&mut Engine) + 'static) {
        self.overlay_hooks.push(Box::new(f));
    }

    /// Install the CLASSIC_TEST per-frame runner (a single callback invoked
    /// once per frame when `CLASSIC_TEST` is active).
    pub fn set_test_runner(&mut self, f: impl FnMut(&mut Engine) + 'static) {
        self.test_runner = Some(Box::new(f));
    }

    /// Current frame counter (used by the test runner for frame scheduling).
    pub fn frame_number(&self) -> u64 {
        self.debug_frame
    }

    /// Last-frame viewport size (used by the test runner to synthesise
    /// normalised mouse coordinates).
    pub fn viewport_size(&self) -> (f32, f32) {
        (self.last_vw, self.last_vh)
    }

    pub fn load_state(&mut self, json: &str) -> Result<(), anyhow::Error> {
        let ns = self.namespace.clone();
        self.load_state_in(&ns, json).map(|_| ())
    }

    /// Load the entity graph into the world under an explicit namespace,
    /// returning the (qualified) keys of the entities added.  The multi-ROM
    /// hydration path uses this so cross-entity references can be rewritten
    /// with the *referring* ROM's namespace rather than the last one loaded.
    pub(crate) fn load_state_in(
        &mut self,
        ns: &str,
        json: &str,
    ) -> Result<Vec<String>, anyhow::Error> {
        // Parse via raw Value to preserve JSON key order (serde_json Map is ordered
        // under `preserve_order`). The typed HashMap on StateData drops ordering.
        let root: serde_json::Value = serde_json::from_str(json)?;
        let entities_obj = root
            .get("entities")
            .and_then(|v| v.as_object())
            .ok_or_else(|| anyhow::anyhow!("state.json missing 'entities' key"))?;

        let mut inserted = Vec::new();
        for (name, val) in entities_obj {
            let ed: classic_core::types::EntityData = serde_json::from_value(val.clone())?;
            let mut builder = hecs::EntityBuilder::new();
            for comp in &ed.components {
                let spawner = classic_core::registry::lookup(&comp.comp_type)
                    .ok_or_else(|| anyhow::anyhow!("unknown component type: {}", comp.comp_type))?;
                spawner(&mut builder, comp.fields.clone())?;
            }
            if ed.components.is_empty() {
                builder.add(());
            }
            let entity = self.world.spawn(builder.build());
            let key = self.entity_key_ns(ns, name);
            self.world.insert_one(entity, DebugName(key.clone())).ok();
            self.names.insert(key.clone(), entity);
            self.name_order.push(key.clone());
            inserted.push(key);
        }
        Ok(inserted)
    }

    pub fn debug_name(&self, entity: hecs::Entity) -> String {
        self.world
            .get::<&DebugName>(entity)
            .map(|n| n.0.clone())
            .unwrap_or_else(|_| format!("e#{:?}", entity.id()))
    }

    /// Find the entity tagged with the given [`RoleKind`].
    pub fn entity_by_role(&self, kind: RoleKind) -> Option<hecs::Entity> {
        self.world
            .query::<&Role>()
            .iter()
            .find(|(_, role)| role.value == kind)
            .map(|(entity, _)| entity)
    }

    /// Find the name of the entity tagged with the given [`RoleKind`].
    pub fn name_by_role(&self, kind: RoleKind) -> Option<String> {
        self.entity_by_role(kind).map(|e| self.debug_name(e))
    }

    /// Serialise all named entities to a state JSON string.
    pub fn dump_state(&self) -> String {
        let entities = self.dump_state_value();
        let root = serde_json::json!({ "entities": entities });
        serde_json::to_string_pretty(&root).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
    }

    fn dump_state_value(&self) -> serde_json::Value {
        let mut entities = serde_json::Map::new();

        for name in &self.name_order {
            let Some(&entity) = self.names.get(name) else { continue };
            let components = self.dump_entity_components(entity);
            if !components.is_empty() {
                entities.insert(name.clone(), serde_json::json!({ "components": components }));
            }
        }

        serde_json::Value::Object(entities)
    }

    /// Serialize a single named entity's component list (the `components`
    /// array of a `state.json` entry), using the registry dumpers.
    pub fn dump_entity_components(&self, entity: hecs::Entity) -> Vec<serde_json::Value> {
        let regs = classic_core::registry::ordered_regs();
        let mut components: Vec<serde_json::Value> = Vec::new();
        let mut dumped = std::collections::HashSet::new();

        for reg in &regs {
            if dumped.contains(reg.name) {
                continue;
            }
            if let Some(val) = reg.dump_value(&self.world, entity) {
                components.push(val);
                dumped.insert(reg.name);
                for sub in reg.subsumes {
                    dumped.insert(sub);
                }
            }
            // Try subsumed components first (they may match)
            for sub in reg.subsumes {
                if dumped.contains(sub) {
                    continue;
                }
                // Check if there's a subsumed reg with a dumper
                if let Some(val) = regs
                    .iter()
                    .find(|r| r.name == *sub)
                    .and_then(|r| r.dump_value(&self.world, entity))
                {
                    components.push(val);
                    dumped.insert(sub);
                }
            }
        }

        components
    }

    /// Named-entity query/access helpers for the guest-code layer.  They mirror
    /// the `load_state`/`dump_state` bookkeeping (names + name_order) so guest
    /// code can spawn/despawn/lookup entities and round-trip components through
    /// the registry without per-field glue.
    pub fn has_name(&self, name: &str) -> bool {
        self.names.contains_key(name)
    }

    pub fn entity_names(&self) -> Vec<String> {
        self.name_order.clone()
    }

    /// Spawn an empty named entity (registers it in `names`/`name_order`).
    /// Returns false if the name is already taken.
    pub fn spawn_named(&mut self, name: &str) -> bool {
        if self.names.contains_key(name) {
            return false;
        }
        let entity = self.world.spawn(());
        self.world.insert_one(entity, DebugName(name.to_string())).ok();
        self.names.insert(name.to_string(), entity);
        self.name_order.push(name.to_string());
        true
    }

    /// Despawn a named entity and drop its name registration.
    pub fn despawn_named(&mut self, name: &str) -> bool {
        let Some(entity) = self.names.remove(name) else { return false };
        self.name_order.retain(|n| n != name);
        let _ = self.world.despawn(entity);
        true
    }

    /// Read a named entity's position (from its `Transform`).
    pub fn get_pos(&self, name: &str) -> Option<(f32, f32, f32)> {
        let entity = *self.names.get(name)?;
        self.world
            .get::<&Transform>(entity)
            .ok()
            .map(|tf| (tf.position.x, tf.position.y, tf.position.z))
    }

    /// Write a named entity's position (into its `Transform`, creating a
    /// default one if the entity has none yet).
    pub fn set_pos(&mut self, name: &str, x: f32, y: f32, z: f32) -> bool {
        let Some(&entity) = self.names.get(name) else { return false };
        if self.world.get::<&Transform>(entity).is_err() {
            let _ = self.world.insert_one(
                entity,
                Transform::new(glam::Vec3::new(x, y, z), glam::Vec3::new(1.0, 1.0, 1.0)),
            );
            return true;
        }
        if let Ok(mut tf) = self.world.get::<&mut Transform>(entity) {
            tf.position.x = x;
            tf.position.y = y;
            tf.position.z = z;
            true
        } else {
            false
        }
    }

    /// Set a named entity's `IsoSprite` frame index.  When the sprite's texture
    /// has a packed-atlas frame table, the matching `frame_name` is resolved so
    /// the packed path is used; otherwise the uniform-grid path takes over.
    pub fn set_sprite_frame(&mut self, name: &str, frame: f32) -> bool {
        let Some(&entity) = self.names.get(name) else { return false };
        let Ok(mut sprite) = self.world.get::<&mut IsoSprite>(entity) else { return false };
        sprite.frame = frame;
        sprite.frame_name = if self.frame_tables.contains_key(&sprite.texture) {
            Some(format!("{}_{}", sprite.texture, frame as u32))
        } else {
            None
        };
        true
    }

    /// Read a named entity's `IsoSprite` frame index.
    pub fn get_sprite_frame(&self, name: &str) -> Option<f32> {
        let entity = *self.names.get(name)?;
        self.world.get::<&IsoSprite>(entity).ok().map(|s| s.frame)
    }

    /// Set a named entity's `IsoSprite` tint colour (RGBA).
    pub fn set_sprite_color(&mut self, name: &str, color: [f32; 4]) -> bool {
        let Some(&entity) = self.names.get(name) else { return false };
        let Ok(mut sprite) = self.world.get::<&mut IsoSprite>(entity) else { return false };
        sprite.color = color;
        true
    }

    /// Set a named entity's `IsoSprite` visual offset (`frame_offset`, in
    /// Blender-world metres: drift in x/y, altitude in z).  Lets guests elevate
    /// a runtime sprite (e.g. a container sliding out of a rocket).  Only valid
    /// for sprites without an animator or vehicle sim that overwrites
    /// `frame_offset` each frame.
    pub fn set_sprite_offset(&mut self, name: &str, dx: f32, dy: f32, dz: f32) -> bool {
        let Some(&entity) = self.names.get(name) else { return false };
        let Ok(mut sprite) = self.world.get::<&mut IsoSprite>(entity) else { return false };
        sprite.frame_offset = glam::Vec3::new(dx, dy, dz);
        true
    }

    /// Spawn a new `IsoSprite` entity cloned from a template entity (e.g. a
    /// mouse-follow placement ghost), so a guest can drop copies at runtime.
    /// Copies the template's `IsoSprite` and `Transform` (the latter carries the
    /// live position written by `set_pos`), plus any gameplay markers
    /// (`Selectable`, `Inventory`) the template carries; the caller then adjusts
    /// the clone with `set_pos`/`set_sprite_frame`/`set_sprite_color` as usual.
    /// Returns `false` when the name is taken, the template is unknown, or the
    /// template has no `IsoSprite`.
    pub fn spawn_sprite_clone(&mut self, template: &str, name: &str) -> bool {
        if self.names.contains_key(name) {
            return false;
        }
        let Some(&template_entity) = self.names.get(template) else { return false };
        let sprite = match self.world.get::<&IsoSprite>(template_entity) {
            Ok(s) => (*s).clone(),
            Err(_) => return false,
        };
        let transform = self
            .world
            .get::<&Transform>(template_entity)
            .ok()
            .map(|t| (*t).clone())
            .unwrap_or_else(|| Transform::new(sprite.position, sprite.scale));
        let selectable = self.world.get::<&Selectable>(template_entity).ok().map(|s| *s);
        let inventory = self
            .world
            .get::<&classic_core::inventory::Inventory>(template_entity)
            .ok()
            .map(|i| (*i).clone());

        let mut builder = hecs::EntityBuilder::new();
        builder.add(sprite);
        builder.add(transform);
        if let Some(s) = selectable {
            builder.add(s);
        }
        if let Some(inv) = inventory {
            builder.add(inv);
        }
        let entity = self.world.spawn(builder.build());
        self.register_named_entity(name, entity);
        true
    }

    /// The iso tile coordinates under the mouse cursor (from the tilemap).
    pub fn mouse_iso(&self) -> Option<(f32, f32)> {
        let tm_entity = self.entity_by_role(RoleKind::Tilemap)?;
        let tm = self.world.get::<&Tilemap>(tm_entity).ok()?;
        Some((tm.mouse_iso_pos.x, tm.mouse_iso_pos.y))
    }

    /// Show the container-inventory hover tooltip for a named entity, or hide
    /// it when `name` is empty.  The host resolves the entity and renders the
    /// tooltip from its `Inventory`; the guest only supplies *which* entity is
    /// hovered and *when* to show/hide.
    pub fn inventory_ui_show(&mut self, name: &str) {
        let target = if name.is_empty() { None } else { self.names.get(name).copied() };
        self.inventory_ui.set_target(target);
    }

    /// Project an iso tile coordinate (at ground height) to camera-view screen
    /// pixels (before pan/zoom), the same space `camera.position` lives in.
    /// Returns `None` when no Tilemap-role entity exists.
    pub fn iso_to_screen(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        self.entity_by_role(RoleKind::Tilemap)?;
        let world = iso_world_pos(x, y, 0.0);
        let view = iso_camera_matrix().transform_point3(world);
        Some((view.x * PPM_TARGET, -view.y * PPM_TARGET))
    }

    /// The world → screen transform for the current frame, derived from the
    /// camera (`T(-fix) · S(scale)`), matching the sprite/terrain projection.
    pub(crate) fn world_to_screen_matrix(&self, vw: f32, vh: f32) -> Mat4 {
        let size = Vec3::new(vw, vh, 0.0);
        let fix = self.camera.position * self.camera.scale - size / Vec3::new(2.0, 2.0, 1.0);
        Mat4::from_translation(-fix) * Mat4::from_scale(self.camera.scale)
    }

    /// Project an iso tile coordinate (at terrain height) to screen pixels
    /// (top-left origin), matching the engine's sprite model + camera math.
    pub fn iso_to_screen_px(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        let tm_entity = self.entity_by_role(RoleKind::Tilemap)?;
        let tm = self.world.get::<&Tilemap>(tm_entity).ok()?;
        let tm_tf = self.world.get::<&Transform>(tm_entity).ok()?;

        let h = bilinear_height(&tm.height_data, tm.size_x, tm.size_y, x, y);
        let world = iso_world_pos(x, y, h) + tm_tf.position;
        let view = iso_camera_matrix().transform_point3(world);
        let screen = Vec3::new(view.x * PPM_TARGET, -view.y * PPM_TARGET, 0.0);

        let (vw, vh) = self.viewport_size();
        let cam = self.world_to_screen_matrix(vw, vh);
        let screen = cam.transform_point3(screen);
        Some((screen.x, screen.y))
    }

    /// Convert an iso tile coordinate to a **world-metre** point (Blender
    /// canonical: `(tx·TILE_M, −ty·TILE_M, h)`), with the tilemap's
    /// `Transform.position` treated as a world-metre offset.
    ///
    /// `elevation` is metres above the sampled terrain surface (same units as
    /// `height_data`).  Height lives in **z alone**; the light-space conversion
    /// (and the isometric screen shear) are the consumer's concern.  Returns
    /// `None` without a Tilemap-role entity.
    pub fn iso_to_world(&self, x: f32, y: f32, elevation: f32) -> Option<Vec3> {
        let tm_entity = self.entity_by_role(RoleKind::Tilemap)?;
        let (tm, tm_tf) = {
            let tm = self.world.get::<&Tilemap>(tm_entity).ok()?;
            let tf = self.world.get::<&Transform>(tm_entity).ok()?;
            (tm, tf)
        };
        let h = sample_height_mesh(&tm.height_data, tm.size_x, tm.size_y, x, y);
        Some(iso_world_pos(x, y, h + elevation) + tm_tf.position)
    }

    /// Terrain height (world metres) at the given iso tile coordinate.
    pub fn height_at(&self, x: f32, y: f32) -> f32 {
        let Some(tm_entity) = self.entity_by_role(RoleKind::Tilemap) else { return 0.0 };
        let Ok(tm) = self.world.get::<&Tilemap>(tm_entity) else { return 0.0 };
        sample_height_mesh(&tm.height_data, tm.size_x, tm.size_y, x, y)
    }

    /// Write one tile index at tile coordinate `(x, y)` (bounds-checked).
    pub fn set_tile(&mut self, x: i32, y: i32, id: u32) -> bool {
        let Some(tm_entity) = self.entity_by_role(RoleKind::Tilemap) else { return false };
        let Ok(mut tm) = self.world.get::<&mut Tilemap>(tm_entity) else { return false };
        if x < 0 || y < 0 || x >= tm.size_x || y >= tm.size_y {
            return false;
        }
        let idx = (y as usize) * tm.size_x as usize + x as usize;
        let Some(t) = tm.data.get_mut(idx) else { return false };
        *t = id;
        true
    }

    /// Write one height vertex at coordinate `(x, y)` (bounds-checked; the
    /// height grid is a `(size_x + 1) × (size_y + 1)` vertex grid).
    pub fn set_height(&mut self, x: i32, y: i32, h: f32) -> bool {
        let Some(tm_entity) = self.entity_by_role(RoleKind::Tilemap) else { return false };
        let Ok(mut tm) = self.world.get::<&mut Tilemap>(tm_entity) else { return false };
        if x < 0 || y < 0 || x > tm.size_x || y > tm.size_y {
            return false;
        }
        let idx = (y as usize) * (tm.size_x as usize + 1) + x as usize;
        let Some(cell) = tm.height_data.get_mut(idx) else { return false };
        *cell = h.max(0.0);
        true
    }

    /// Rebuild the tilemap mesh and re-derive nav walkability after in-place
    /// tile/height edits (the guest-facing terrain-edit tail).
    pub fn rebuild_terrain(&mut self) -> bool {
        if self.entity_by_role(RoleKind::Tilemap).is_none() {
            return false;
        }
        self.rebuild_tilemap_mesh();
        self.sync_nav_heights();
        true
    }

    /// Bulk-write the tilemap tile grid from a guest-provided `u32` array
    /// (row-major, `size_x * size_y`).  Replaces the grid wholesale — the
    /// loaded component may be empty (`state_lunar.json` declares `"data":
    /// null`, generated at runtime).
    pub fn set_tiles_bulk(&mut self, tiles: &[u32]) -> bool {
        let Some(tm_entity) = self.entity_by_role(RoleKind::Tilemap) else { return false };
        let Ok(mut tm) = self.world.get::<&mut Tilemap>(tm_entity) else { return false };
        if tiles.len() != (tm.size_x * tm.size_y) as usize {
            return false;
        }
        tm.data = tiles.to_vec();
        true
    }

    /// Bulk-write the tilemap height vertex grid from a guest-provided `f32`
    /// array (`(size_x + 1) * (size_y + 1)`).  Replaces the grid wholesale.
    pub fn set_heights_bulk(&mut self, heights: &[f32]) -> bool {
        let Some(tm_entity) = self.entity_by_role(RoleKind::Tilemap) else { return false };
        let Ok(mut tm) = self.world.get::<&mut Tilemap>(tm_entity) else { return false };
        if heights.len() != ((tm.size_x + 1) * (tm.size_y + 1)) as usize {
            return false;
        }
        tm.height_data = heights.to_vec();
        true
    }

    /// Bulk-write the nav walkability grid from a guest-provided `u32` array
    /// (`size_x * size_y`, `1` = walkable).  Replaces the grid wholesale.
    pub fn set_nav_bulk(&mut self, nav: &[u32]) -> bool {
        let Some(nav_entity) = self.entity_by_role(RoleKind::NavMesh) else { return false };
        {
            let Ok(mut nm) = self.world.get::<&mut NavMesh>(nav_entity) else { return false };
            if nav.len() != (nm.size_x * nm.size_y) as usize {
                return false;
            }
            nm.data = nav.to_vec();
        }
        self.refresh_nav_snapshot();
        true
    }

    /// Upload a raw RGBA tileset texture for the tilemap (guest-generated
    /// tileset).  Replaces the current tileset under the Tilemap's `tile_set`.
    pub fn set_tileset_bulk(&mut self, rgba: &[u8], w: u32, h: u32) -> bool {
        let Some(tm_entity) = self.entity_by_role(RoleKind::Tilemap) else { return false };
        let Ok(tm) = self.world.get::<&Tilemap>(tm_entity) else { return false };
        let tile_set = tm.tile_set.clone();
        let Some(gfx) = self.gfx.as_mut() else { return false };
        gfx.add_texture_rgba8(&tile_set, rgba, w, h);
        true
    }

    /// Commit the tilemap terrain: install (first call) or rebuild (later
    /// calls) the tilemap mesh + tile data texture and re-upload the nav
    /// overlay.  Used by ROM guests to own their map, whether generated
    /// (bulk-uploaded via the `set_*` imports) or hand-authored (inline
    /// `state.json` data, hydrated here).  Does NOT re-derive walkability (the
    /// guest's nav grid is authoritative).  A tilemap with no height data is
    /// treated as flat (height 1.0 everywhere).
    pub fn commit_terrain(&mut self, height_scale: f32) -> bool {
        let Some(tm_entity) = self.entity_by_role(RoleKind::Tilemap) else {
            return false;
        };
        if let Ok(mut tm) = self.world.get::<&mut Tilemap>(tm_entity) {
            tm.height_scale = height_scale;
        }
        let installed = self.tilemap_gpu.contains_key(&self.debug_name(tm_entity));
        if installed {
            self.rebuild_tilemap_mesh();
        } else {
            let (tiles, mut heights, size_x, size_y) = {
                let tm = self.world.get::<&Tilemap>(tm_entity).unwrap();
                (tm.data.clone(), tm.height_data.clone(), tm.size_x, tm.size_y)
            };
            // A tilemap with no height data (e.g. a hand-authored map with only
            // inline tiles) renders flat at height 1.0.
            if heights.is_empty() {
                heights = vec![1.0f32; (size_x as usize + 1) * (size_y as usize + 1)];
            }
            self.finish_tilemap_init(tm_entity, tiles, heights, Some(height_scale));
        }
        self.rebuild_nav_gpu();
        self.refresh_nav_snapshot();
        true
    }

    /// Set a named entity's `Animator` to play a looping animation.
    pub fn set_anim(&mut self, name: &str, anim: &str) -> bool {
        let Some(&entity) = self.names.get(name) else { return false };
        if let Ok(mut a) = self.world.get::<&mut Animator>(entity) {
            a.animation = Some(anim.to_string());
            a.playing = true;
            a.repeat = true;
            true
        } else {
            false
        }
    }

    /// Restart a named entity's `Animator` from frame zero: reset the transient
    /// `counter`/`frame`/`offset`, then play `anim` (looping if `repeat`).
    pub fn start_anim(&mut self, name: &str, anim: &str, repeat: bool) -> bool {
        let Some(&entity) = self.names.get(name) else { return false };
        if let Ok(mut a) = self.world.get::<&mut Animator>(entity) {
            a.animation = Some(anim.to_string());
            a.repeat = repeat;
            a.playing = true;
            a.counter = 0.0;
            a.frame = 0.0;
            a.offset = Vec3::ZERO;
            true
        } else {
            false
        }
    }

    /// Read a named entity's current animation name and frame.
    pub fn get_anim(&self, name: &str) -> Option<(String, f32)> {
        let entity = *self.names.get(name)?;
        let a = self.world.get::<&Animator>(entity).ok()?;
        Some((a.animation.clone().unwrap_or_default(), a.frame))
    }

    /// Whether a named texture is available (registered from the ROM's
    /// resources or already uploaded to GL).  The name must already be
    /// namespace-resolved (see [`Engine::resolve_resource`]).
    pub fn has_texture(&self, name: &str) -> bool {
        let in_gfx = self.gfx.as_ref().map(|g| g.textures.contains_key(name)).unwrap_or(false);
        in_gfx || self.texture_names.contains(name)
    }

    /// Whether a named SDF font is available (loaded into `sdf_fonts`).  The
    /// name must already be namespace-resolved (see [`Engine::resolve_resource`]).
    pub fn has_font(&self, name: &str) -> bool {
        self.sdf_fonts.contains_key(name)
    }

    /// Whether a named animation is registered.
    pub fn has_animation(&self, name: &str) -> bool {
        self.animations.contains_key(name)
    }

    /// The pixel dimensions of a loaded texture, if any.
    pub fn texture_size(&self, name: &str) -> Option<(u32, u32)> {
        self.gfx.as_ref().and_then(|g| g.textures.get(name)).map(|t| t.size)
    }

    /// A* path over the nav mesh between two integer tile coordinates.
    /// Returns the full path (inclusive of both endpoints) or `None`.
    pub fn find_path(&self, from: (i32, i32), to: (i32, i32)) -> Option<Vec<(i32, i32)>> {
        let nav_entity = self.entity_by_role(RoleKind::NavMesh)?;
        let nav = self.world.get::<&NavMesh>(nav_entity).ok()?;
        let nav_i32: Vec<i32> = nav.data.iter().map(|&v| v as i32).collect();
        pathfinder::find_path(&nav_i32, nav.size_x, nav.size_y, from, to)
    }

    /// Footprint-aware A* over the nav mesh: treat the moving agent as a
    /// multi-tile object by eroding the walkability grid by `footprint` (a set
    /// of integer tile offsets from the anchor cell) before searching.  Used by
    /// `vehicle_goto`; the humanoid `IsoAgent` keeps the plain [`find_path`].
    pub fn find_path_for_footprint(
        &self,
        from: (i32, i32),
        to: (i32, i32),
        footprint: &[(i32, i32)],
    ) -> Option<Vec<(i32, i32)>> {
        let nav_entity = self.entity_by_role(RoleKind::NavMesh)?;
        let nav = self.world.get::<&NavMesh>(nav_entity).ok()?;
        let nav_i32: Vec<i32> = nav.data.iter().map(|&v| v as i32).collect();
        pathfinder::find_path_for_footprint(&nav_i32, nav.size_x, nav.size_y, from, to, footprint)
    }

    /// Footprint-, slope- and jump-aware A* for a wheeled vehicle (the
    /// synchronous fallback, used under `synchronous_workers`).  Builds the
    /// vehicle nav snapshot and delegates to [`pathfinder::find_vehicle_path_snapshot`],
    /// the same single code path the worker runs.
    #[allow(clippy::too_many_arguments)]
    pub fn find_vehicle_path(
        &self,
        from: (i32, i32),
        to: (i32, i32),
        footprint: &[(i32, i32)],
        pitch_max: f32,
        roll_max: f32,
        wheelbase_m: f32,
        track_m: f32,
        safe_fall_m: f32,
        jump_cost: f32,
        turn_cost: f32,
    ) -> Option<Vec<(i32, i32)>> {
        let obstacles = self.compute_nav_obstacles();
        let snapshot = self.build_vehicle_nav_snapshot(&obstacles)?;
        let result = pathfinder::find_vehicle_path_snapshot(
            &snapshot,
            from,
            to,
            footprint,
            pitch_max,
            roll_max,
            wheelbase_m,
            track_m,
            safe_fall_m,
            jump_cost,
            turn_cost,
        );
        classic_core::cl_info!(
            classic_core::instrument::Chan::Path,
            "find_vehicle_path {} -> {}: footprint={} tiles, pitch={:.3}rad fall={}m, found={}",
            format!("{from:?}"),
            format!("{to:?}"),
            footprint.len(),
            pitch_max.min(roll_max),
            safe_fall_m,
            result.is_some(),
        );
        result
    }

    /// Rebuild the shared nav snapshot from the live `NavMesh` component and
    /// re-share it with the pathfinding worker.  Bumps `nav_version`.
    pub(crate) fn refresh_nav_snapshot(&mut self) {
        let Some(nav_entity) = self.entity_by_role(RoleKind::NavMesh) else {
            return;
        };
        let (size_x, size_y, data) = {
            let Ok(nav) = self.world.get::<&NavMesh>(nav_entity) else {
                return;
            };
            (nav.size_x, nav.size_y, nav.data.iter().map(|&v| v as i32).collect::<Vec<_>>())
        };
        // Unified obstacles: footprints of `blocks_nav` colliders block both
        // humanoid and vehicle pathfinding.
        let obstacles = self.compute_nav_obstacles();
        let combined: Vec<i32> = data.iter().zip(&obstacles).map(|(&d, &o)| d & o).collect();
        let snapshot = Arc::new(pathfinder::NavSnapshot::new(size_x, size_y, combined));
        if let Some(worker) = self.pathfinder.as_mut() {
            worker.set_snapshot(Arc::clone(&snapshot));
        }
        self.nav_snapshot = snapshot;

        // Rebuild + push the vehicle nav snapshot (structural nav + heights +
        // tile metre length) so the worker can run vehicle A* off-thread too.
        if let Some(vehicle_snapshot) = self.build_vehicle_nav_snapshot(&obstacles) {
            if let Some(worker) = self.pathfinder.as_mut() {
                worker.set_vehicle_snapshot(Arc::clone(&vehicle_snapshot));
            }
            self.vehicle_nav_snapshot = vehicle_snapshot;
        }

        self.refresh_guest_worker_nav();
        self.nav_version = self.nav_version.wrapping_add(1);
    }

    /// Build the unified obstacle grid (0 = blocked, 1 = open) from the
    /// footprints of every non-disabled entity whose collider has `blocks_nav`
    /// set.  Used for both humanoid and vehicle pathfinding.
    pub(crate) fn compute_nav_obstacles(&self) -> Vec<i32> {
        let Some(nav_entity) = self.entity_by_role(RoleKind::NavMesh) else { return Vec::new() };
        let Ok(nav) = self.world.get::<&NavMesh>(nav_entity) else { return Vec::new() };
        let (size_x, size_y) = (nav.size_x, nav.size_y);
        let mut grid = vec![1i32; (size_x * size_y) as usize];

        for (entity, (iso, tf)) in self.world.query::<(&IsoSprite, &Transform)>().iter() {
            if self.is_disabled(entity) || iso.footprint.is_empty() {
                continue;
            }
            let name = self.debug_name(entity);
            let Some(&pid) = self.collider_pids.get(&name) else { continue };
            if !self.physics.collider_blocks_nav(pid) {
                continue;
            }
            // Rasterize the footprint AABB (iso tile coords at the entity position).
            let mut min_x = f32::INFINITY;
            let mut max_x = f32::NEG_INFINITY;
            let mut min_y = f32::INFINITY;
            let mut max_y = f32::NEG_INFINITY;
            for pt in &iso.footprint {
                min_x = min_x.min(tf.position.x + pt.x);
                max_x = max_x.max(tf.position.x + pt.x);
                min_y = min_y.min(tf.position.y + pt.y);
                max_y = max_y.max(tf.position.y + pt.y);
            }
            let x0 = (min_x.floor() as i32).clamp(0, size_x - 1);
            let x1 = (max_x.floor() as i32).clamp(0, size_x - 1);
            let y0 = (min_y.floor() as i32).clamp(0, size_y - 1);
            let y1 = (max_y.floor() as i32).clamp(0, size_y - 1);
            for ty in y0..=y1 {
                for tx in x0..=x1 {
                    grid[(ty * size_x + tx) as usize] = 0;
                }
            }
        }
        grid
    }

    /// Build the [`pathfinder::VehicleNavSnapshot`] the worker uses for vehicle
    /// A*, from the live `NavMesh` (structural nav) and `Tilemap` (heights +
    /// the fixed `TILE_M` tile metre length).  Returns `None` when either
    /// component is missing.
    pub(crate) fn build_vehicle_nav_snapshot(
        &self,
        obstacles: &[i32],
    ) -> Option<Arc<pathfinder::VehicleNavSnapshot>> {
        let nav_entity = self.entity_by_role(RoleKind::NavMesh)?;
        let nav = self.world.get::<&NavMesh>(nav_entity).ok()?;
        let size_x = nav.size_x;
        let size_y = nav.size_y;
        // The vehicle `structural` grid gates hard obstacles (blocking
        // colliders); slope climbability is derived per-request from pitch/roll.
        let structural: Vec<i32> = obstacles.to_vec();

        let tm_entity = self.entity_by_role(RoleKind::Tilemap)?;
        let tm = self.world.get::<&Tilemap>(tm_entity).ok()?;
        Some(Arc::new(pathfinder::VehicleNavSnapshot::new(
            size_x,
            size_y,
            structural,
            tm.height_data.clone(),
            TILE_M,
        )))
    }

    /// Run background work (pathfinding and the Tier-3 guest worker) inline on
    /// the render thread — the deterministic test/golden harness — instead of
    /// offloading it.  The single determinism switch: it decides how the
    /// workers' job queues are built.  Set it before installing the guest worker;
    /// a pathfinder already spawned in the other mode is rebuilt on next use.
    pub fn set_synchronous_workers(&mut self, synchronous: bool) {
        self.synchronous_workers = synchronous;
        if self.pathfinder.as_ref().is_some_and(|w| w.is_synchronous() != synchronous) {
            self.pathfinder = None;
        }
    }

    /// The nav-snapshot version, bumped on every rebuild.
    pub fn nav_version(&self) -> u64 {
        self.nav_version
    }

    /// Submit an A* path request over the nav mesh and return its request id.
    ///
    /// When `synchronous_workers` is off, the search runs on a background
    /// worker (native thread or web `Worker`); the result is collected via
    /// [`Engine::poll_path`].  In synchronous mode the search runs inline and
    /// the result is immediately available to `poll_path`.
    pub fn request_path(&mut self, from: (i32, i32), to: (i32, i32)) -> u64 {
        let id = self.next_path_id;
        self.next_path_id = self.next_path_id.wrapping_add(1);
        self.ensure_pathfinder();
        if let Some(worker) = self.pathfinder.as_mut() {
            worker.request_path(id, from, to);
        }
        id
    }

    /// Poll a previously submitted path request (non-blocking).
    ///
    /// Returns [`pathfinder::PathPoll::Pending`] while the search is still
    /// running, [`pathfinder::PathPoll::Path`] with the route, or
    /// [`pathfinder::PathPoll::NoPath`] if no route exists.
    pub fn poll_path(&mut self, id: u64) -> pathfinder::PathPoll {
        if let Some(worker) = self.pathfinder.as_mut() {
            return worker.poll_path(id);
        }
        pathfinder::PathPoll::Pending
    }

    /// Block until all in-flight worker jobs have completed.  Determinism
    /// barrier, called at frame boundaries when `CLASSIC_TEST` is active
    /// (no-op on web, where determinism is handled by the sync fallback).
    pub fn join_workers(&mut self) {
        if let Some(worker) = self.pathfinder.as_ref() {
            worker.join();
        }
        if let Some(worker) = self.guest_worker.as_ref() {
            worker.join();
        }
    }

    /// Spawn the pathfinding worker on first use (synchronous under
    /// `synchronous_workers`), sharing the current nav snapshot and vehicle nav
    /// snapshot.
    pub(crate) fn ensure_pathfinder(&mut self) {
        if self.pathfinder.is_none() {
            let snapshot = Arc::clone(&self.nav_snapshot);
            let mut worker = if self.synchronous_workers {
                classic_worker::PathfinderWorker::new_synchronous(snapshot)
            } else {
                classic_worker::PathfinderWorker::new(snapshot)
            };
            worker.set_vehicle_snapshot(Arc::clone(&self.vehicle_nav_snapshot));
            self.pathfinder = Some(worker);
        }
    }

    /// Install the background guest worker (Tier 3), sharing the current nav
    /// snapshot.  The worker runs a second `.wasm` instance against the reduced
    /// pure-import surface (see `classic-worker::guest_worker`).  Under
    /// `synchronous_workers` entries run inline on the render thread (the
    /// deterministic test/golden harness).
    pub fn install_guest_worker(&mut self, wasm: &[u8]) -> Result<(), String> {
        let worker = classic_worker::GuestWorker::new(
            wasm,
            Arc::clone(&self.nav_snapshot),
            self.synchronous_workers,
        )?;
        self.guest_worker = Some(worker);
        Ok(())
    }

    /// Install the background guest worker (Tier 3) from a module already
    /// compiled off-thread (the async native path).  Shares the current nav
    /// snapshot; instantiate uses the same engine that compiled the module.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn install_guest_worker_compiled(
        &mut self,
        compiled: &classic_worker::CompiledWorker,
    ) -> Result<(), String> {
        let worker = classic_worker::GuestWorker::new_compiled(
            compiled,
            Arc::clone(&self.nav_snapshot),
            self.synchronous_workers,
        )?;
        self.guest_worker = Some(worker);
        Ok(())
    }

    /// Submit a background guest task: run the named export of the worker guest
    /// with `arg` as its input bytes.  Returns a task id to poll with
    /// [`Engine::poll_task`].
    pub fn spawn_task(&mut self, entry: &str, arg: Vec<u8>) -> u64 {
        let id = self.next_task_id;
        self.next_task_id = self.next_task_id.wrapping_add(1);
        if let Some(worker) = self.guest_worker.as_mut() {
            worker.spawn_task(id, entry, arg);
        }
        id
    }

    /// Poll a previously submitted background task.  `None` while pending,
    /// `Some(Ok(bytes))` with the result, or `Some(Err(msg))` if it trapped.
    pub fn poll_task(&mut self, id: u64) -> Option<Result<Vec<u8>, String>> {
        self.guest_worker.as_mut().and_then(|worker| worker.poll_task(id))
    }

    /// Re-share the current nav snapshot with the background guest worker (and
    /// the pathfinding worker), e.g. after a terrain rebuild.
    fn refresh_guest_worker_nav(&mut self) {
        if let Some(worker) = self.guest_worker.as_mut() {
            worker.set_nav(Arc::clone(&self.nav_snapshot));
        }
    }

    /// Read the camera position (x, y) and uniform scale.
    pub fn get_camera(&self) -> (f32, f32, f32) {
        (self.camera.position.x, self.camera.position.y, self.camera.scale.x)
    }

    /// Set the camera position (x, y) and uniform scale.
    pub fn set_camera(&mut self, x: f32, y: f32, scale: f32) {
        self.camera.position.x = x;
        self.camera.position.y = y;
        self.camera.scale.x = scale;
        self.camera.scale.y = scale;
    }

    /// Show or hide the tilemap editor grid overlay.
    pub fn set_grid(&mut self, show: bool) {
        self.show_grid = show;
    }

    /// Register a collider and remember its owning entity's name, so
    /// [`Engine::pick_at`] can resolve it.
    pub fn register_named_collider(&mut self, name: &str, collider: ColliderData) -> u32 {
        let pid = self.physics.register_collider(collider);
        self.collider_names.insert(pid, name.to_string());
        self.collider_pids.insert(name.to_string(), pid);
        pid
    }

    /// Attach an axis-aligned rectangle collider to a named entity, at a screen
    /// position and size.  Combined with `subscribe`, this makes arbitrary
    /// (screen-space) entities clickable/hoverable from a guest.
    pub fn spawn_collider(&mut self, name: &str, x: f32, y: f32, w: f32, h: f32) -> bool {
        if !self.names.contains_key(name) {
            return false;
        }
        let verts = vec![
            glam::Vec3::new(0.0, 0.0, 0.0),
            glam::Vec3::new(w, 0.0, 0.0),
            glam::Vec3::new(w, h, 0.0),
            glam::Vec3::new(0.0, h, 0.0),
        ];
        let mut collider = ColliderData::new(classic_core::collision::polygon_from_verts(verts));
        collider.position = glam::Vec3::new(x, y, 0.0);
        collider.scale = glam::Vec3::ONE;
        self.register_named_collider(name, collider);
        true
    }

    /// The name of the top gameplay entity under a screen point, optionally
    /// filtered to entities carrying `filter`'s component (empty = any).  The
    /// filter is a component type name (e.g. `"Inventory"`, `"Selectable"`);
    /// an unknown name matches nothing.
    pub fn pick_at(&self, x: f32, y: f32, filter: &str) -> Option<String> {
        self.physics.point_query(x, y).into_iter().find_map(|pid| {
            let name = self.collider_names.get(&pid)?;
            if filter.is_empty() {
                return Some(name.clone());
            }
            let entity = self.names.get(name)?;
            if self.has_component(*entity, filter) {
                Some(name.clone())
            } else {
                None
            }
        })
    }

    /// Mark (or clear) a named entity's collider as a navigation obstacle.
    /// Rebuilds the nav snapshots so the change is reflected immediately.
    /// Returns `false` when the entity has no registered collider.
    pub fn set_collider_blocks_nav(&mut self, name: &str, blocks: bool) -> bool {
        if let Some(&pid) = self.collider_pids.get(name) {
            self.physics.set_collider_blocks_nav(pid, blocks);
            self.refresh_nav_snapshot();
            true
        } else {
            false
        }
    }

    /// Whether `entity` carries the named component.  The empty name matches
    /// every entity; recognized component type names map to a runtime check.
    fn has_component(&self, entity: hecs::Entity, name: &str) -> bool {
        match name {
            "" => true,
            "Inventory" => self.world.get::<&classic_core::inventory::Inventory>(entity).is_ok(),
            "Selectable" => self.world.get::<&Selectable>(entity).is_ok(),
            "IsoSprite" => self.world.get::<&IsoSprite>(entity).is_ok(),
            "IsoVehicle" => self.world.get::<&IsoVehicle>(entity).is_ok(),
            "Sprite" => self.world.get::<&SpriteRender>(entity).is_ok(),
            "SdfTextRender" => self.world.get::<&SdfTextRender>(entity).is_ok(),
            "Tilemap" => self.world.get::<&Tilemap>(entity).is_ok(),
            _ => false,
        }
    }

    /// The name of the top *subscribed* entity under a screen point, if any.
    pub(crate) fn pick_subscribed(&self, x: f32, y: f32) -> Option<String> {
        self.physics.point_query(x, y).into_iter().find_map(|pid| {
            self.collider_names.get(&pid).cloned().filter(|n| self.subscribed.contains(n))
        })
    }

    /// Subscribe a named entity to interaction events (click/enter/exit).
    pub fn subscribe(&mut self, name: &str) -> bool {
        if !self.names.contains_key(name) {
            return false;
        }
        self.subscribed.insert(name.to_string());
        true
    }

    /// Pop the next queued guest event, if any.
    pub fn poll_event(&mut self) -> Option<GuestEvent> {
        self.guest_events.pop_front()
    }

    /// Set a host-provided boolean flag visible to ROM guests.
    pub fn set_guest_flag(&mut self, name: &str, value: bool) {
        self.guest_flags.insert(name.to_string(), value);
    }

    /// Read a host-provided boolean flag (false when unset).
    pub fn guest_flag(&self, name: &str) -> bool {
        self.guest_flags.get(name).copied().unwrap_or(false)
    }

    /// Read the light uniforms (ambient, direction, color).
    pub fn get_light(&self) -> ([f32; 3], [f32; 3], [f32; 3]) {
        (self.light_ambient, self.light_dir, self.light_color)
    }

    /// Set the light uniforms (ambient, direction, color).
    pub fn set_light(&mut self, ambient: [f32; 3], dir: [f32; 3], color: [f32; 3]) {
        self.light_ambient = ambient;
        self.light_dir = dir;
        self.light_color = color;
    }

    /// Spawn a dynamic light entity, returning its handle (or `None` when the
    /// light table is full).  A `ttl` of `None` makes the light persistent; a
    /// finite `ttl` (seconds) auto-releases it after decaying.
    pub fn spawn_light(&mut self, light: Light, ttl: Option<f32>) -> Option<u32> {
        self.light_handles.spawn(&mut self.world, light, ttl)
    }

    /// Overwrite an active light entity's parameters by handle.
    pub fn update_light(&mut self, handle: u32, light: Light) -> bool {
        self.light_handles.set(&mut self.world, handle, light)
    }

    /// Read an active light's current parameters by handle (`None` if the
    /// handle is inactive).
    pub fn light_by_handle(&self, handle: u32) -> Option<Light> {
        self.light_handles.get(&self.world, handle)
    }

    /// Despawn a light entity and release its handle.
    pub fn release_light(&mut self, handle: u32) -> bool {
        self.light_handles.release(&mut self.world, handle)
    }

    /// Gather the active lights from the world, resolving parent attachments to
    /// **world-metre** positions (the same space the lit shaders evaluate them
    /// in).  A parented light treats `Light.position` as a world-metre offset
    /// from the parent's ground point (`iso_to_world`); an unparented light's
    /// position is already world metres.
    pub fn gather_lights(&self) -> Vec<Light> {
        let mut lights = Vec::new();
        for (_e, light) in self.world.query::<&Light>().iter() {
            let mut l = light.clone();
            if let Some(parent_name) = l.parent.as_deref() {
                // Parent resolution must not fail *open*: a dangling name, a
                // missing `Transform`, or a missing tilemap used to silently
                // reinterpret the relative offset as an absolute position, so
                // the light teleported to a random spot with no warning.
                match self.names.get(parent_name) {
                    Some(&pe) => match self.world.get::<&Transform>(pe) {
                        Ok(tf) => match self.iso_to_world(tf.position.x, tf.position.y, 0.0) {
                            Some(base) => {
                                // The parent sprite renders at
                                // `tile + frame_offset` (its animated descent
                                // carries altitude + drift); `iso_to_world`
                                // only sees the tile.  Fold the frame offset
                                // into the base so an attached light follows
                                // the parent's actual position — a descending
                                // rocket's burn light must track the nozzle,
                                // not the landing pad.  Mirror
                                // `compute_iso_sprite_model`: altitude
                                // (`fo.z`) lands in z, horizontal drift in x/y.
                                let fo = self.parent_frame_offset(pe);
                                l.position = base + fo + l.position;
                            }
                            None => classic_core::cl_warn!(
                                classic_core::instrument::Chan::Render,
                                "light parent {parent_name:?} has no tilemap to resolve against; \
                                 skipping the light"
                            ),
                        },
                        Err(_) => {
                            classic_core::cl_warn!(
                                classic_core::instrument::Chan::Render,
                                "light parent {parent_name:?} has no Transform; skipping the light"
                            );
                            continue;
                        }
                    },
                    None => {
                        classic_core::cl_warn!(
                            classic_core::instrument::Chan::Render,
                            "light parent {parent_name:?} not found; skipping the light"
                        );
                        continue;
                    }
                }
            }
            lights.push(l);
        }
        // `Light.radius` is authored in legacy light-space px (64 px/metre, the
        // pre-unification unit the shader used); convert it to world metres so
        // `dist / radius` in the shader compares like-for-like with `position`.
        for l in &mut lights {
            l.radius /= PPM_TARGET;
        }
        // `MAX_LIGHTS` bounds the UBO block, but `gather_lights` reads every
        // `Light` entity — including `state.json`-declared and directly-spawned
        // ones that never went through `LightHandles`.  Enforce the budget here
        // so the shader's `count` never disagrees with the packed array (which
        // would otherwise drop the tail in arbitrary hecs order).
        lights.truncate(classic_gfx::MAX_LIGHTS);
        for l in &lights {
            classic_core::cl_debug!(
                classic_core::instrument::Chan::Render,
                "light: parent={:?} pos=({:.1},{:.1},{:.1}) radius={:.1} intensity={:.2}",
                l.parent,
                l.position.x,
                l.position.y,
                l.position.z,
                l.radius,
                l.intensity
            );
        }
        lights
    }

    /// The visual `frame_offset` (Blender-world metres: drift in x/y, altitude
    /// in z) of a parent entity, or `Vec3::ZERO` when the parent has none (a
    /// static sprite).  Animated `IsoSprite` / `IsoAgent` entities carry the
    /// descent/run offset here; `gather_lights` folds it into an attached
    /// light's position so the light tracks the parent's animated motion.
    fn parent_frame_offset(&self, parent: hecs::Entity) -> Vec3 {
        if let Ok(s) = self.world.get::<&IsoSprite>(parent) {
            return s.frame_offset;
        }
        if let Ok(a) = self.world.get::<&IsoAgent>(parent) {
            return a.frame_offset;
        }
        Vec3::ZERO
    }

    /// Spawn a named screen-space solid-color rectangle (a HUD element).
    pub fn spawn_rect(
        &mut self,
        name: &str,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: [f32; 4],
    ) -> bool {
        if self.names.contains_key(name) {
            return false;
        }
        let entity = self.world.spawn((
            Transform::new(glam::Vec3::new(x, y, 0.0), glam::Vec3::new(w, h, 1.0)),
            RectRender { color, ignore_cam: true },
        ));
        self.world.insert_one(entity, DebugName(name.to_string())).ok();
        self.names.insert(name.to_string(), entity);
        self.name_order.push(name.to_string());
        true
    }

    /// Spawn a named screen-space SDF text label.
    pub fn spawn_text(
        &mut self,
        name: &str,
        x: f32,
        y: f32,
        text: &str,
        scale: f32,
        color: [f32; 4],
    ) -> bool {
        if self.names.contains_key(name) {
            return false;
        }
        let entity = self.world.spawn((
            Transform::new(glam::Vec3::new(x, y, 0.0), glam::Vec3::new(scale, scale, 1.0)),
            SdfTextRender {
                atlas_name: classic_core::components::DEFAULT_SDF_FONT.into(),
                color,
                outline_color: [0.0, 0.0, 0.0, 0.0],
                outline_width: 0.0,
                ignore_cam: true,
                text: text.to_string(),
                justify: classic_core::components::TextJustify::Left,
                weight: 0.0,
                gamma: 1.0,
            },
        ));
        self.world.insert_one(entity, DebugName(name.to_string())).ok();
        self.names.insert(name.to_string(), entity);
        self.name_order.push(name.to_string());
        true
    }

    /// Update a named SDF text label's string.
    pub fn set_text(&mut self, name: &str, text: &str) -> bool {
        let Some(&entity) = self.names.get(name) else { return false };
        if let Ok(mut sdf) = self.world.get::<&mut SdfTextRender>(entity) {
            sdf.text = text.to_string();
            true
        } else {
            false
        }
    }

    /// Register an already-spawned entity under a guest-visible name.
    fn register_named_entity(&mut self, name: &str, entity: hecs::Entity) {
        self.world.insert_one(entity, DebugName(name.to_string())).ok();
        self.names.insert(name.to_string(), entity);
        self.name_order.push(name.to_string());
    }

    // ---- UIManager registration (guest-managed responsive UI) -------------
    //
    // These wrap the `UIManager` factories so a guest can create UI elements
    // that participate in layout (anchoring/array/padding/resize) under a name
    // it controls, without reimplementing any responsiveness.

    /// Spawn a named UI container (solid-color rectangle managed by layout).
    pub fn ui_container(&mut self, name: &str, w: f32, h: f32, color: [f32; 4]) -> bool {
        if self.names.contains_key(name) {
            return false;
        }
        let entity = {
            let Some(ui) = self.ui.as_mut() else { return false };
            ui.spawn_container(&mut self.world, w, h, color)
        };
        self.register_named_entity(name, entity);
        true
    }

    /// Spawn a named UI SDF text label managed by layout.
    pub fn ui_text(
        &mut self,
        name: &str,
        text: &str,
        scale: f32,
        max_width: f32,
        color: [f32; 4],
        justify: TextJustify,
    ) -> bool {
        if self.names.contains_key(name) {
            return false;
        }
        let entity = {
            let Some(ui) = self.ui.as_mut() else { return false };
            ui.spawn_sdf_text(&mut self.world, text, scale, max_width, color, justify)
        };
        self.register_named_entity(name, entity);
        true
    }

    /// Spawn a named UI button (container + centered text + click collider).
    /// The button is registered in the collider-name map and auto-subscribed,
    /// so its clicks surface through the guest event queue.
    pub fn ui_button(&mut self, name: &str, text: &str, w: f32, h: f32, color: [f32; 4]) -> bool {
        if self.names.contains_key(name) {
            return false;
        }
        let (entity, pid) = {
            let Some(ui) = self.ui.as_mut() else { return false };
            let entity = ui.spawn_button(
                &mut self.world,
                &mut self.physics,
                w,
                h,
                color,
                ui::ButtonOptions {
                    text: Some(text.to_string()),
                    text_scale: 0.4,
                    text_color: [1.0, 1.0, 1.0, 1.0],
                    sdf_text: true,
                    hover: true,
                    click_priority: 1,
                    ..Default::default()
                },
            );
            let pid = ui.collider_pid_for(entity);
            (entity, pid)
        };
        self.register_named_entity(name, entity);
        if let Some(pid) = pid {
            self.collider_names.insert(pid, name.to_string());
        }
        self.subscribed.insert(name.to_string());
        true
    }

    /// Spawn a named UI array container (vertical or horizontal stacking).
    pub fn ui_array(
        &mut self,
        name: &str,
        vertical: bool,
        align: UiAlign,
        spacing: f32,
        color: [f32; 4],
    ) -> bool {
        if self.names.contains_key(name) {
            return false;
        }
        let entity = {
            let Some(ui) = self.ui.as_mut() else { return false };
            ui.spawn_array(&mut self.world, vertical, align, spacing, color)
        };
        self.register_named_entity(name, entity);
        true
    }

    /// Spawn a named UI padding wrapper.
    pub fn ui_padding(
        &mut self,
        name: &str,
        top: f32,
        right: f32,
        bottom: f32,
        left: f32,
        color: [f32; 4],
    ) -> bool {
        if self.names.contains_key(name) {
            return false;
        }
        let entity = {
            let Some(ui) = self.ui.as_mut() else { return false };
            ui.spawn_padding(&mut self.world, top, right, bottom, left, color)
        };
        self.register_named_entity(name, entity);
        true
    }

    /// Spawn a named texture-sprite UI element.
    pub fn ui_sprite(
        &mut self,
        name: &str,
        texture: &str,
        w: f32,
        h: f32,
        frame: f32,
        tile_set_size: [f32; 2],
    ) -> bool {
        if self.names.contains_key(name) {
            return false;
        }
        let entity = {
            let Some(ui) = self.ui.as_mut() else { return false };
            ui.spawn_sprite(&mut self.world, texture, w, h, frame, tile_set_size)
        };
        self.register_named_entity(name, entity);
        true
    }

    /// Attach a named UI element as a child of another (anchor-based layout).
    pub fn ui_add_child(
        &mut self,
        parent: &str,
        child: &str,
        self_anchor: UiAnchor,
        child_anchor: UiAnchor,
    ) -> bool {
        let Some(&p) = self.names.get(parent) else { return false };
        let Some(&c) = self.names.get(child) else { return false };
        let Some(ui) = self.ui.as_mut() else { return false };
        ui.container_add_child(&mut self.world, p, c, self_anchor, child_anchor);
        true
    }

    /// Attach a named UI element to the root container (viewport-anchored).
    pub fn ui_add_to_root(
        &mut self,
        name: &str,
        self_anchor: UiAnchor,
        child_anchor: UiAnchor,
    ) -> bool {
        let Some(&c) = self.names.get(name) else { return false };
        let Some(ui) = self.ui.as_mut() else { return false };
        ui.root_add_child(&mut self.world, c, self_anchor, child_anchor);
        true
    }

    /// Set a named UI element's size.
    pub fn ui_set_size(&mut self, name: &str, w: f32, h: f32) -> bool {
        let Some(&e) = self.names.get(name) else { return false };
        {
            let Ok(mut n) = self.world.get::<&mut UiNode>(e) else { return false };
            n.size = glam::Vec2::new(w, h);
        }
        if let Some(ui) = self.ui.as_mut() {
            ui.mark_dirty();
        }
        true
    }

    /// Set a named UI element's anchor.
    pub fn ui_set_anchor(&mut self, name: &str, anchor: UiAnchor) -> bool {
        let Some(&e) = self.names.get(name) else { return false };
        {
            let Ok(mut n) = self.world.get::<&mut UiNode>(e) else { return false };
            n.anchor = anchor;
        }
        if let Some(ui) = self.ui.as_mut() {
            ui.mark_dirty();
        }
        true
    }

    /// Set a named UI rectangle's color.
    pub fn ui_set_color(&mut self, name: &str, color: [f32; 4]) -> bool {
        let Some(&e) = self.names.get(name) else { return false };
        if let Ok(mut r) = self.world.get::<&mut RectRender>(e) {
            r.color = color;
            true
        } else {
            false
        }
    }

    /// Set whether a named UI element is fixed (skips responsive layout).
    pub fn ui_set_fixed(&mut self, name: &str, fixed: bool) -> bool {
        let Some(&e) = self.names.get(name) else { return false };
        {
            let Ok(mut n) = self.world.get::<&mut UiNode>(e) else { return false };
            n.fixed = fixed;
        }
        if let Some(ui) = self.ui.as_mut() {
            ui.mark_dirty();
        }
        true
    }

    /// Save raw bytes to a file, handling both native (filesystem) and web
    /// (Blob download).
    pub fn save_bytes(&self, name: &str, bytes: &[u8]) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let dir = &crate::env_config::EnvConfig::get().dump_dir;
            let _ = std::fs::create_dir_all(dir);
            let path = format!("{dir}/{name}");
            if let Err(e) = std::fs::write(&path, bytes) {
                classic_core::cl_warn!(
                    classic_core::instrument::Chan::Dump,
                    "save_bytes: failed to write {path}: {e}"
                );
            } else {
                classic_core::cl_warn!(
                    classic_core::instrument::Chan::Dump,
                    "save_bytes: wrote {path} ({} bytes)",
                    bytes.len()
                );
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            use wasm_bindgen::JsCast;
            if let Some(window) = web_sys::window() {
                let doc = window.document().unwrap();
                let blob_parts = js_sys::Array::new();
                blob_parts.push(&js_sys::Uint8Array::from(bytes).into());
                let blob = web_sys::Blob::new_with_str_sequence(&blob_parts).unwrap();
                let url = web_sys::Url::create_object_url_with_blob(&blob).unwrap();
                let a = doc.create_element("a").unwrap();
                a.set_attribute("download", name).unwrap();
                a.set_attribute("href", &url).unwrap();
                a.dyn_ref::<web_sys::HtmlElement>().unwrap().click();
                web_sys::Url::revoke_object_url(&url).unwrap();
            }
        }
    }

    /// Save a UTF-8 text file (delegates to [`Engine::save_bytes`]).
    pub fn save_file(&self, name: &str, data: &str) {
        self.save_bytes(name, data.as_bytes());
    }

    /// Serialize the current world as a ROM archive and save it to
    /// `<entrypoint>.rom` — the canonical editor save (F10).
    pub fn save_rom(&self) -> bool {
        let Some(rom) = self.dump_rom() else {
            classic_core::cl_warn!(
                classic_core::instrument::Chan::Dump,
                "save_rom: no ROM loaded to save"
            );
            return false;
        };
        let name = format!("{}.rom", rom.manifest.entrypoint);
        match rom.pack() {
            Ok(bytes) => {
                self.save_bytes(&name, &bytes);
                true
            }
            Err(e) => {
                classic_core::cl_warn!(
                    classic_core::instrument::Chan::Dump,
                    "save_rom: failed to pack ROM: {e}"
                );
                false
            }
        }
    }
}
