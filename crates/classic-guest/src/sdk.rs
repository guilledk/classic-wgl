//! The host-side SDK: bridges guest host-imports to the engine.
//!
//! [`GuestHost`] is a thin raw-pointer bridge over [`classic_engine::Engine`],
//! shared by every runtime backend (wasmi, wasmtime).  Each runtime wraps it in
//! its own store data and owns its own resource limiter (memory cap).  The
//! heavy lifting lives in safe `Engine` methods; only the pointer deref is
//! `unsafe`.

use classic_core::components::{Light, LightKind, TextJustify, UiAlign, UiAnchor};
use classic_core::fields::FieldDtype;
use classic_core::instrument::Chan;
use classic_core::pathfinder::PathPoll;
use classic_core::terrain::kernels::{FieldOp, Reduce};
use classic_engine::vehicle::{VehicleGotoPoll, VehicleGotoSubmit};
use classic_engine::Engine;

/// Map an integer to a [`UiAnchor`] (0..=8, TopLeft → BotRight).
fn anchor(i: i32) -> UiAnchor {
    match i {
        0 => UiAnchor::TopLeft,
        1 => UiAnchor::TopCenter,
        2 => UiAnchor::TopRight,
        3 => UiAnchor::MidLeft,
        4 => UiAnchor::MidCenter,
        5 => UiAnchor::MidRight,
        6 => UiAnchor::BotLeft,
        7 => UiAnchor::BotCenter,
        _ => UiAnchor::BotRight,
    }
}

/// Map an integer to a [`UiAlign`] (0 = Left, 1 = Center, 2 = Right).
fn align(i: i32) -> UiAlign {
    match i {
        0 => UiAlign::Left,
        2 => UiAlign::Right,
        _ => UiAlign::Center,
    }
}

/// Map an integer to a [`TextJustify`] (0 = Left, 1 = Center, 2 = Right).
fn justify(i: i32) -> TextJustify {
    match i {
        0 => TextJustify::Left,
        2 => TextJustify::Right,
        _ => TextJustify::Center,
    }
}

/// Map an integer to a [`FieldOp`] (0 add, 1 sub, 2 mul, 3 min, 4 max).
fn field_op(i: i32) -> FieldOp {
    match i {
        1 => FieldOp::Sub,
        2 => FieldOp::Mul,
        3 => FieldOp::Min,
        4 => FieldOp::Max,
        _ => FieldOp::Add,
    }
}

/// Map an integer to a [`Reduce`] (0 min, 1 max, 2 mean, 3 variance).
fn reduce_op(i: i32) -> Reduce {
    match i {
        1 => Reduce::Max,
        2 => Reduce::Mean,
        3 => Reduce::Variance,
        _ => Reduce::Min,
    }
}

/// Host state shared with every guest runtime store: a raw pointer to the
/// engine, re-pointed for each guest entry point.
pub struct GuestHost {
    engine: *mut Engine,
    /// The owning ROM's namespace (empty = global).  Guest-supplied names are
    /// qualified/resolved against this so several ROMs' guests can coexist.
    namespace: String,
}

impl GuestHost {
    pub(crate) fn new() -> Self {
        Self { engine: std::ptr::null_mut(), namespace: String::new() }
    }

    /// Re-point the host at the engine for the current call.
    pub(crate) fn set_engine(&mut self, engine: &mut Engine) {
        self.engine = engine as *mut Engine;
    }

    /// Set the owning ROM's namespace (empty = global).  Called once by the
    /// demo layer after `create_runtime`; guest names are then scoped to it.
    pub(crate) fn set_namespace(&mut self, ns: &str) {
        self.namespace = ns.to_string();
    }

    #[inline]
    fn engine(&self) -> &Engine {
        // SAFETY: `GuestHost` is only dereferenced within a single `update`
        // call, on one thread, while `engine` is borrowed for that call.
        unsafe { &*self.engine }
    }

    #[inline]
    fn engine_mut(&mut self) -> &mut Engine {
        // SAFETY: see `engine()`.
        unsafe { &mut *self.engine }
    }

    /// Qualify a guest-supplied name for *spawning* a new entity: prefix the
    /// guest's namespace (a no-op for the global namespace or a `ns::name`).
    pub fn qualify(&self, name: &str) -> String {
        self.engine().entity_key_ns(&self.namespace, name)
    }

    /// Resolve a guest-supplied entity name for *lookup*: a bare name resolves
    /// in the referring (own) namespace first, then the global namespace, then
    /// falls back to the qualified key; an already-qualified `ns::name` passes
    /// through verbatim.  The single indirection point for multi-ROM scoping.
    pub fn resolve(&self, name: &str) -> String {
        if name.contains("::") {
            return name.to_string();
        }
        self.engine()
            .resolve_entity_name(&self.namespace, name)
            .unwrap_or_else(|| self.qualify(name))
    }

    /// Log a message through the `guest` CLASSIC_LOG channel.
    pub fn log(&mut self, msg: &str) {
        classic_core::cl_info!(Chan::Guest, "{}", msg);
    }

    pub fn spawn(&mut self, name: &str) -> i32 {
        let name = self.qualify(name);
        self.engine_mut().spawn_named(&name) as i32
    }

    pub fn despawn(&mut self, name: &str) -> i32 {
        let name = self.resolve(name);
        self.engine_mut().despawn_named(&name) as i32
    }

    pub fn has(&mut self, name: &str) -> i32 {
        let name = self.resolve(name);
        self.engine().has_name(&name) as i32
    }

    /// The ordered list of entity names, as a JSON array.
    pub fn names(&mut self) -> String {
        serde_json::to_string(&self.engine().entity_names()).unwrap_or_default()
    }

    pub fn set_pos(&mut self, name: &str, x: f64, y: f64, z: f64) -> i32 {
        let name = self.resolve(name);
        self.engine_mut().set_pos(&name, x as f32, y as f32, z as f32) as i32
    }

    pub fn get_pos(&mut self, name: &str) -> Option<(f64, f64, f64)> {
        let name = self.resolve(name);
        self.engine().get_pos(&name).map(|(x, y, z)| (x as f64, y as f64, z as f64))
    }

    /// Set a named entity's `IsoSprite` frame index (packed-atlas aware).
    pub fn set_sprite_frame(&mut self, name: &str, frame: f64) -> i32 {
        let name = self.resolve(name);
        self.engine_mut().set_sprite_frame(&name, frame as f32) as i32
    }

    /// Read a named entity's `IsoSprite` frame index (`-1.0` when it has none).
    pub fn get_sprite_frame(&mut self, name: &str) -> f64 {
        let name = self.resolve(name);
        self.engine().get_sprite_frame(&name).map(|f| f as f64).unwrap_or(-1.0)
    }

    /// Set a named entity's `IsoSprite` tint colour (RGBA, `0..=1`).
    pub fn set_sprite_color(&mut self, name: &str, r: f64, g: f64, b: f64, a: f64) -> i32 {
        let name = self.resolve(name);
        self.engine_mut().set_sprite_color(&name, [r as f32, g as f32, b as f32, a as f32]) as i32
    }

    /// Set a named entity's `IsoSprite` visual offset (`frame_offset`; negative
    /// Y lifts the sprite).  Used to elevate runtime sprites (e.g. the
    /// unloading container).
    pub fn set_sprite_offset(&mut self, name: &str, dx: f64, dy: f64, dz: f64) -> i32 {
        let name = self.resolve(name);
        self.engine_mut().set_sprite_offset(&name, dx as f32, dy as f32, dz as f32) as i32
    }

    /// Spawn a new `IsoSprite` entity cloned from a template entity's
    /// `IsoSprite` + `Transform` (e.g. a placement ghost), under a new name.
    pub fn spawn_sprite_clone(&mut self, template: &str, name: &str) -> i32 {
        let template = self.resolve(template);
        let name = self.qualify(name);
        self.engine_mut().spawn_sprite_clone(&template, &name) as i32
    }

    pub fn mouse(&mut self) -> (f64, f64) {
        let p = self.engine().input.mouse_pos;
        (p.x as f64, p.y as f64)
    }

    /// The iso tile coordinates under the mouse cursor.
    pub fn mouse_iso(&mut self) -> Option<(f64, f64)> {
        self.engine().mouse_iso().map(|(x, y)| (x as f64, y as f64))
    }

    /// Project an iso tile coordinate to screen space (none if no Tilemap).
    pub fn iso_to_screen(&mut self, x: f64, y: f64) -> Option<(f64, f64)> {
        self.engine().iso_to_screen(x as f32, y as f32).map(|(sx, sy)| (sx as f64, sy as f64))
    }

    /// Terrain height (world z) at an iso tile coordinate.
    pub fn height_at(&mut self, x: f64, y: f64) -> f64 {
        self.engine().height_at(x as f32, y as f32) as f64
    }

    /// Set a named entity's animator to play a looping animation.
    pub fn set_anim(&mut self, name: &str, anim: &str) -> i32 {
        let name = self.resolve(name);
        self.engine_mut().set_anim(&name, anim) as i32
    }

    /// Restart a named entity's animator from frame zero (optionally one-shot).
    pub fn start_anim(&mut self, name: &str, anim: &str, repeat: i32) -> i32 {
        let name = self.resolve(name);
        self.engine_mut().start_anim(&name, anim, repeat != 0) as i32
    }

    /// Show/hide a named entity (add/remove the `Disabled` marker).
    pub fn set_enabled(&mut self, name: &str, enabled: i32) -> i32 {
        let name = self.resolve(name);
        self.engine_mut().set_enabled_named(&name, enabled != 0) as i32
    }

    /// Whether the editor's agent tool is active.
    pub fn agent_selected(&mut self) -> i32 {
        self.engine().guest_flag("agent_selected") as i32
    }

    /// Whether a UI element consumed this frame's click.
    pub fn ui_consumed_click(&mut self) -> i32 {
        self.engine().guest_flag("ui_consumed_click") as i32
    }

    pub fn delta(&mut self) -> f64 {
        self.engine().time.delta as f64
    }

    pub fn elapsed(&mut self) -> f64 {
        self.engine().time.elapsed
    }

    pub fn was_pressed(&mut self, button: i32) -> i32 {
        if button < 0 {
            return 0;
        }
        self.engine().input.was_mouse_pressed(button as usize) as i32
    }

    pub fn key_down(&mut self, key: &str) -> i32 {
        self.engine().input.is_key_down(key) as i32
    }

    /// Whether a key was pressed this frame (edge-triggered).
    pub fn was_key_pressed(&mut self, key: &str) -> i32 {
        self.engine().input.was_key_pressed(key) as i32
    }

    /// Write one tile index at tile coordinate `(x, y)`.
    pub fn set_tile(&mut self, x: i32, y: i32, id: i32) -> i32 {
        self.engine_mut().set_tile(x, y, id.max(0) as u32) as i32
    }

    /// Write one height vertex at coordinate `(x, y)`.
    pub fn set_height(&mut self, x: i32, y: i32, h: f64) -> i32 {
        self.engine_mut().set_height(x, y, h as f32) as i32
    }

    /// Rebuild the tilemap mesh and nav walkability after terrain edits.
    pub fn rebuild_terrain(&mut self) -> i32 {
        self.engine_mut().rebuild_terrain() as i32
    }

    /// Submit an A* path request over the nav mesh from `(sx, sy)` to
    /// `(ex, ey)`, returning a request id to poll with [`Self::poll_path`].
    pub fn request_path(&mut self, sx: i32, sy: i32, ex: i32, ey: i32) -> i32 {
        self.engine_mut().request_path((sx, sy), (ex, ey)) as i32
    }

    /// Poll a previously submitted path request by id.  The returned
    /// [`PathPoll`] maps onto the ABI's `poll_path` result codes (`0` pending,
    /// `-1` no-path, `>0` waypoint count).
    pub fn poll_path(&mut self, id: i32) -> PathPoll {
        self.engine_mut().poll_path(id as u64)
    }

    /// Submit a background guest task: run the worker guest's named `entry`
    /// export with `arg` as input bytes.  Returns a task id to poll with
    /// [`Self::poll_task`].
    pub fn spawn_task(&mut self, entry: &str, arg: &[u8]) -> i32 {
        self.engine_mut().spawn_task(entry, arg.to_vec()) as i32
    }

    /// Poll a previously submitted background task by id.  `None` while
    /// pending, `Some(Ok(bytes))` with the result, `Some(Err(msg))` on a trap.
    pub fn poll_task(&mut self, id: i32) -> Option<Result<Vec<u8>, String>> {
        self.engine_mut().poll_task(id as u64)
    }

    /// Reposition a wheeled vehicle (body + 4 wheels) and reset its physics.
    pub fn vehicle_teleport(&mut self, name: &str, x: f64, y: f64) -> i32 {
        let name = self.resolve(name);
        self.engine_mut().vehicle_teleport(&name, x as f32, y as f32) as i32
    }

    /// Spawn a wheeled vehicle of a declared type at `(x, y)`.
    pub fn vehicle_spawn(&mut self, def: &str, name: &str, x: f64, y: f64) -> i32 {
        let name = self.qualify(name);
        self.engine_mut().spawn_vehicle(def, &name, x as f32, y as f32) as i32
    }

    /// Set a wheeled vehicle's destination (integer tile coordinates).  Returns
    /// `> 0` (a request id to poll with [`Self::vehicle_goto_poll`]), `0` when
    /// the vehicle is airborne (re-issue next frame), or `-1` for an unknown
    /// vehicle.
    pub fn vehicle_goto(&mut self, name: &str, tx: i32, ty: i32) -> i32 {
        let name = self.resolve(name);
        match self.engine_mut().vehicle_goto(&name, tx, ty) {
            VehicleGotoSubmit::Airborne => 0,
            VehicleGotoSubmit::NoVehicle => -1,
            VehicleGotoSubmit::Submitted(id) => id as i32,
        }
    }

    /// Poll a vehicle path request by id: `0` pending, `1` accepted (path
    /// installed), `-1` no path.
    pub fn vehicle_goto_poll(&mut self, id: i32) -> i32 {
        match self.engine_mut().vehicle_goto_poll(id as u64) {
            VehicleGotoPoll::Pending => 0,
            VehicleGotoPoll::Accepted(_) => 1,
            VehicleGotoPoll::NoPath => -1,
        }
    }

    /// Stop a wheeled vehicle, clearing its movement path.
    pub fn vehicle_stop(&mut self, name: &str) -> i32 {
        let name = self.resolve(name);
        self.engine_mut().vehicle_stop(&name) as i32
    }

    /// Set a wheeled vehicle's speed (tiles per second), mutating its
    /// `IsoVehicle.speed` (e.g. slow a loaded LRV).
    pub fn vehicle_set_speed(&mut self, name: &str, speed: f64) -> i32 {
        let name = self.resolve(name);
        self.engine_mut().vehicle_set_speed(&name, speed as f32) as i32
    }

    /// Non-mutating vehicle reachability probe: run the vehicle's A* to
    /// `(tx, ty)` without driving it.  Returns `1` reachable, `-1` no path,
    /// `0` pending (call again), `-2` unknown vehicle.  On success the waypoints
    /// are stored in `Engine::preview_paths` for the demo overlay.
    pub fn vehicle_probe(&mut self, name: &str, tx: i32, ty: i32) -> i32 {
        let name = self.resolve(name);
        self.engine_mut().vehicle_probe(&name, tx, ty)
    }

    /// Clear a vehicle's drop-preview state (candidate path + probe).  Returns
    /// `1` when anything was cleared, else `0`.
    pub fn vehicle_probe_clear(&mut self, name: &str) -> i32 {
        let name = self.resolve(name);
        self.engine_mut().vehicle_probe_clear(&name) as i32
    }

    /// The max tile radius of a vehicle's collision footprint (`max(|dx|,|dy|)`
    /// over its `path_footprint` cells), `-1.0` when unknown.  Lets a guest
    /// derive pick-up/drop clearance from the real footprint instead of a magic
    /// constant.
    pub fn vehicle_footprint_radius(&mut self, name: &str) -> f64 {
        self.engine().vehicle_footprint_radius(name)
    }

    /// The JSON array of currently RTS-selected entity names.
    pub fn selected_names(&mut self) -> String {
        serde_json::to_string(&self.engine().selected_names()).unwrap_or_else(|_| "[]".into())
    }

    /// Clear the RTS selection set.
    pub fn selection_clear(&mut self) -> i32 {
        self.engine_mut().selection_clear();
        1
    }

    /// Serialize a named entity's inventory to JSON (`"null"` when it has none).
    pub fn inventory_dump(&mut self, name: &str) -> String {
        let name = self.resolve(name);
        self.engine().inventory_dump(&name).unwrap_or_else(|| "null".into())
    }

    /// Read a named entity's raw `Inventory.capacity` (`-1` when it has none).
    pub fn inventory_capacity(&mut self, name: &str) -> i32 {
        let name = self.resolve(name);
        self.engine().inventory_capacity(&name).map(|c| c as i32).unwrap_or(-1)
    }

    /// Add `n` units of an item (by name) to a named entity's inventory,
    /// returning the amount actually added.
    pub fn inventory_add(&mut self, name: &str, item: &str, n: i32) -> i32 {
        if n < 0 {
            return 0;
        }
        let name = self.resolve(name);
        self.engine_mut().inventory_add(&name, item, n as u32) as i32
    }

    /// Remove `n` units of an item (by name) from a named entity's inventory,
    /// returning the amount actually removed.
    pub fn inventory_remove(&mut self, name: &str, item: &str, n: i32) -> i32 {
        if n < 0 {
            return 0;
        }
        let name = self.resolve(name);
        self.engine_mut().inventory_remove(&name, item, n as u32) as i32
    }

    /// Transfer up to `n` units of an item (by name) between two named
    /// entities' inventories, returning the amount actually transferred.
    pub fn inventory_transfer(&mut self, from: &str, to: &str, item: &str, n: i32) -> i32 {
        if n < 0 {
            return 0;
        }
        let from = self.resolve(from);
        let to = self.resolve(to);
        self.engine_mut().inventory_transfer(&from, &to, item, n as u32) as i32
    }

    /// Serialize an item definition to JSON (empty string when unknown).
    pub fn item_def(&mut self, name: &str) -> String {
        self.engine().item_def(name).unwrap_or_default()
    }

    /// Show the container-inventory hover tooltip for a named entity, or hide
    /// it when `name` is empty.  Returns 1.
    pub fn inventory_ui_show(&mut self, name: &str) -> i32 {
        self.engine_mut().inventory_ui_show(name);
        1
    }

    /// Read the camera position (x, y) and uniform scale.
    pub fn get_camera(&mut self) -> (f64, f64, f64) {
        let (x, y, s) = self.engine().get_camera();
        (x as f64, y as f64, s as f64)
    }

    /// Set the camera position (x, y) and uniform scale.
    pub fn set_camera(&mut self, x: f64, y: f64, scale: f64) -> i32 {
        self.engine_mut().set_camera(x as f32, y as f32, scale as f32);
        1
    }

    /// Show or hide the tilemap editor grid overlay.
    pub fn set_grid(&mut self, show: i32) -> i32 {
        self.engine_mut().set_grid(show != 0);
        1
    }

    /// The name of the top gameplay entity under a screen point, optionally
    /// filtered to entities carrying `filter`'s component (empty filter = any).
    pub fn pick_at(&mut self, x: f64, y: f64, filter: &str) -> String {
        self.engine().pick_at(x as f32, y as f32, filter).unwrap_or_default()
    }

    /// Mark (or clear) a named entity's collider as a navigation obstacle.
    pub fn set_collider_blocks_nav(&mut self, name: &str, blocks: i32) -> i32 {
        self.engine_mut().set_collider_blocks_nav(name, blocks != 0) as i32
    }

    /// Whether a mouse button is held (0 = left, 1 = right, 2 = middle).
    pub fn mouse_down(&mut self, button: i32) -> i32 {
        if button < 0 {
            return 0;
        }
        self.engine().input.is_mouse_down(button as usize) as i32
    }

    /// Whether a mouse button was released this frame.
    pub fn mouse_released(&mut self, button: i32) -> i32 {
        if button < 0 {
            return 0;
        }
        self.engine().input.was_mouse_released(button as usize) as i32
    }

    /// The current mouse wheel value (decays to zero each frame).
    pub fn mouse_wheel(&mut self) -> f64 {
        self.engine().input.mouse_wheel as f64
    }

    /// Whether a key was released this frame (edge-triggered).
    pub fn key_up(&mut self, key: &str) -> i32 {
        self.engine().input.was_key_released(key) as i32
    }

    /// Read the light uniforms, as three `[f64; 3]` (ambient, direction, color).
    pub fn get_light(&mut self) -> ([f64; 3], [f64; 3], [f64; 3]) {
        let (a, d, c) = self.engine().get_light();
        (
            [a[0] as f64, a[1] as f64, a[2] as f64],
            [d[0] as f64, d[1] as f64, d[2] as f64],
            [c[0] as f64, c[1] as f64, c[2] as f64],
        )
    }

    /// Set the light uniforms (ambient, direction, color).
    #[allow(clippy::too_many_arguments)]
    pub fn set_light(
        &mut self,
        a0: f64,
        a1: f64,
        a2: f64,
        d0: f64,
        d1: f64,
        d2: f64,
        c0: f64,
        c1: f64,
        c2: f64,
    ) -> i32 {
        self.engine_mut().set_light(
            [a0 as f32, a1 as f32, a2 as f32],
            [d0 as f32, d1 as f32, d2 as f32],
            [c0 as f32, c1 as f32, c2 as f32],
        );
        1
    }

    /// Spawn a dynamic light in the host light pool.  `kind` is `0` = point,
    /// `1` = spot; `ttl <= 0` is persistent (otherwise the light auto-releases
    /// after `ttl` seconds).  Returns the light handle, or `-1` when the pool
    /// is full.
    #[allow(clippy::too_many_arguments)]
    pub fn light_spawn(
        &mut self,
        kind: i32,
        x: f64,
        y: f64,
        z: f64,
        r: f64,
        g: f64,
        b: f64,
        intensity: f64,
        radius: f64,
        ttl: f64,
    ) -> i32 {
        let light = Light {
            kind: if kind == 1 { LightKind::Spot } else { LightKind::Point },
            position: glam::Vec3::new(x as f32, y as f32, z as f32),
            color: [r as f32, g as f32, b as f32],
            intensity: intensity as f32,
            radius: radius as f32,
            dir: glam::Vec3::ZERO,
            cone_angle: 0.0,
            parent: None,
        };
        let ttl = if ttl <= 0.0 { None } else { Some(ttl as f32) };
        self.engine_mut().spawn_light(light, ttl).map(|h| h as i32).unwrap_or(-1)
    }

    /// Overwrite an active pooled light's parameters by handle.  Returns `1` on
    /// success, `0` for an unknown/inactive handle.
    #[allow(clippy::too_many_arguments)]
    pub fn light_set(
        &mut self,
        handle: i32,
        x: f64,
        y: f64,
        z: f64,
        r: f64,
        g: f64,
        b: f64,
        intensity: f64,
        radius: f64,
    ) -> i32 {
        // `light_set` only updates the transform/radiance fields; it must not
        // demote a spot light or detach a parented light.  A negative handle is
        // invalid (the host treats handles as unsigned slot indices, so `-1`
        // must not silently alias slot 0).
        if handle < 0 {
            return 0;
        }
        let handle = handle as u32;
        let mut light = self.engine().light_by_handle(handle).unwrap_or(Light {
            kind: LightKind::Point,
            position: glam::Vec3::ZERO,
            color: [1.0, 1.0, 1.0],
            intensity: 1.0,
            radius: 200.0,
            dir: glam::Vec3::ZERO,
            cone_angle: 0.0,
            parent: None,
        });
        light.position = glam::Vec3::new(x as f32, y as f32, z as f32);
        light.color = [r as f32, g as f32, b as f32];
        light.intensity = intensity as f32;
        light.radius = radius as f32;
        self.engine_mut().update_light(handle, light) as i32
    }

    /// Release a pooled light back to the free-list.  Returns `1` on success,
    /// `0` for an unknown/inactive handle.
    pub fn light_release(&mut self, handle: i32) -> i32 {
        if handle < 0 {
            return 0;
        }
        self.engine_mut().release_light(handle as u32) as i32
    }

    /// Spawn a named screen-space solid-color rectangle.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_rect(
        &mut self,
        name: &str,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        r: f64,
        g: f64,
        b: f64,
        a: f64,
    ) -> i32 {
        let name = self.qualify(name);
        self.engine_mut().spawn_rect(
            &name,
            x as f32,
            y as f32,
            w as f32,
            h as f32,
            [r as f32, g as f32, b as f32, a as f32],
        ) as i32
    }

    /// Spawn a named screen-space SDF text label.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_text(
        &mut self,
        name: &str,
        x: f64,
        y: f64,
        text: &str,
        scale: f64,
        r: f64,
        g: f64,
        b: f64,
        a: f64,
    ) -> i32 {
        let name = self.qualify(name);
        self.engine_mut().spawn_text(
            &name,
            x as f32,
            y as f32,
            text,
            scale as f32,
            [r as f32, g as f32, b as f32, a as f32],
        ) as i32
    }

    /// Update a named SDF text label's string.
    pub fn set_text(&mut self, name: &str, text: &str) -> i32 {
        let name = self.resolve(name);
        self.engine_mut().set_text(&name, text) as i32
    }

    // ---- UIManager registration (guest-managed responsive UI) -------------

    #[allow(clippy::too_many_arguments)]
    pub fn ui_container(
        &mut self,
        name: &str,
        w: f64,
        h: f64,
        r: f64,
        g: f64,
        b: f64,
        a: f64,
    ) -> i32 {
        let name = self.qualify(name);
        self.engine_mut().ui_container(
            &name,
            w as f32,
            h as f32,
            [r as f32, g as f32, b as f32, a as f32],
        ) as i32
    }

    #[allow(clippy::too_many_arguments)]
    pub fn ui_text(
        &mut self,
        name: &str,
        text: &str,
        scale: f64,
        max_width: f64,
        r: f64,
        g: f64,
        b: f64,
        a: f64,
        justify_idx: i32,
    ) -> i32 {
        let name = self.qualify(name);
        self.engine_mut().ui_text(
            &name,
            text,
            scale as f32,
            max_width as f32,
            [r as f32, g as f32, b as f32, a as f32],
            justify(justify_idx),
        ) as i32
    }

    #[allow(clippy::too_many_arguments)]
    pub fn ui_button(
        &mut self,
        name: &str,
        text: &str,
        w: f64,
        h: f64,
        r: f64,
        g: f64,
        b: f64,
        a: f64,
    ) -> i32 {
        let name = self.qualify(name);
        self.engine_mut().ui_button(
            &name,
            text,
            w as f32,
            h as f32,
            [r as f32, g as f32, b as f32, a as f32],
        ) as i32
    }

    #[allow(clippy::too_many_arguments)]
    pub fn ui_array(
        &mut self,
        name: &str,
        vertical: i32,
        align_idx: i32,
        spacing: f64,
        r: f64,
        g: f64,
        b: f64,
        a: f64,
    ) -> i32 {
        let name = self.qualify(name);
        self.engine_mut().ui_array(
            &name,
            vertical != 0,
            align(align_idx),
            spacing as f32,
            [r as f32, g as f32, b as f32, a as f32],
        ) as i32
    }

    #[allow(clippy::too_many_arguments)]
    pub fn ui_padding(
        &mut self,
        name: &str,
        top: f64,
        right: f64,
        bottom: f64,
        left: f64,
        r: f64,
        g: f64,
        b: f64,
        a: f64,
    ) -> i32 {
        let name = self.qualify(name);
        self.engine_mut().ui_padding(
            &name,
            top as f32,
            right as f32,
            bottom as f32,
            left as f32,
            [r as f32, g as f32, b as f32, a as f32],
        ) as i32
    }

    #[allow(clippy::too_many_arguments)]
    pub fn ui_sprite(
        &mut self,
        name: &str,
        texture: &str,
        w: f64,
        h: f64,
        frame: f64,
        tsx: f64,
        tsy: f64,
    ) -> i32 {
        let name = self.qualify(name);
        self.engine_mut().ui_sprite(
            &name,
            texture,
            w as f32,
            h as f32,
            frame as f32,
            [tsx as f32, tsy as f32],
        ) as i32
    }

    pub fn ui_add_child(
        &mut self,
        parent: &str,
        child: &str,
        self_anchor: i32,
        child_anchor: i32,
    ) -> i32 {
        let parent = self.resolve(parent);
        let child = self.resolve(child);
        self.engine_mut().ui_add_child(&parent, &child, anchor(self_anchor), anchor(child_anchor))
            as i32
    }

    pub fn ui_add_to_root(&mut self, name: &str, self_anchor: i32, child_anchor: i32) -> i32 {
        let name = self.resolve(name);
        self.engine_mut().ui_add_to_root(&name, anchor(self_anchor), anchor(child_anchor)) as i32
    }

    pub fn ui_set_size(&mut self, name: &str, w: f64, h: f64) -> i32 {
        let name = self.resolve(name);
        self.engine_mut().ui_set_size(&name, w as f32, h as f32) as i32
    }

    pub fn ui_set_anchor(&mut self, name: &str, anchor_idx: i32) -> i32 {
        let name = self.resolve(name);
        self.engine_mut().ui_set_anchor(&name, anchor(anchor_idx)) as i32
    }

    #[allow(clippy::too_many_arguments)]
    pub fn ui_set_color(&mut self, name: &str, r: f64, g: f64, b: f64, a: f64) -> i32 {
        let name = self.resolve(name);
        self.engine_mut().ui_set_color(&name, [r as f32, g as f32, b as f32, a as f32]) as i32
    }

    pub fn ui_set_fixed(&mut self, name: &str, fixed: i32) -> i32 {
        let name = self.resolve(name);
        self.engine_mut().ui_set_fixed(&name, fixed != 0) as i32
    }

    /// Subscribe a named entity to interaction events (click/enter/exit).
    pub fn subscribe(&mut self, name: &str) -> i32 {
        let name = self.resolve(name);
        self.engine_mut().subscribe(&name) as i32
    }

    /// Pop the next queued guest event, as `(kind, name)` (0=click, 1=enter, 2=exit).
    pub fn poll_event(&mut self) -> Option<(u32, String)> {
        self.engine_mut().poll_event().map(|e| (e.kind, e.name))
    }

    /// Attach an axis-aligned rectangle collider to a named entity (screen space).
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_collider(&mut self, name: &str, x: f64, y: f64, w: f64, h: f64) -> i32 {
        let name = self.resolve(name);
        self.engine_mut().spawn_collider(&name, x as f32, y as f32, w as f32, h as f32) as i32
    }

    /// Read a named entity's current animation name and frame.
    pub fn get_anim(&mut self, name: &str) -> Option<(String, f64)> {
        let name = self.resolve(name);
        self.engine().get_anim(&name).map(|(n, f)| (n, f as f64))
    }

    /// Whether a named resource exists (0 = texture, 1 = font, 2 = animation).
    pub fn has_resource(&mut self, kind: i32, name: &str) -> i32 {
        (match kind {
            0 => self.engine().has_texture(name),
            1 => self.engine().has_font(name),
            2 => self.engine().has_animation(name),
            _ => false,
        }) as i32
    }

    /// The pixel dimensions of a loaded texture, if any.
    pub fn texture_size(&mut self, name: &str) -> Option<(f64, f64)> {
        self.engine().texture_size(name).map(|(w, h)| (w as f64, h as f64))
    }

    // ---- Bulk noise fields (host generates, guest composes) ----------------

    #[allow(clippy::too_many_arguments)]
    pub fn fbm_field(
        &mut self,
        w: i32,
        h: i32,
        seed: &str,
        octaves: u32,
        freq: f64,
        lacunarity: f64,
        gain: f64,
    ) -> Vec<f32> {
        classic_core::terrain::noise_fields::fbm_field(w, h, seed, octaves, freq, lacunarity, gain)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn ridged_field(
        &mut self,
        w: i32,
        h: i32,
        seed: &str,
        octaves: u32,
        freq: f64,
        lacunarity: f64,
        gain: f64,
        warp_amp: f64,
    ) -> Vec<f32> {
        classic_core::terrain::noise_fields::ridged_field(
            w, h, seed, octaves, freq, lacunarity, gain, warp_amp,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn billow_field(
        &mut self,
        w: i32,
        h: i32,
        seed: &str,
        octaves: u32,
        freq: f64,
        lacunarity: f64,
        gain: f64,
    ) -> Vec<f32> {
        classic_core::terrain::noise_fields::billow_field(
            w, h, seed, octaves, freq, lacunarity, gain,
        )
    }

    pub fn tiling_field(
        &mut self,
        w: i32,
        h: i32,
        seed: &str,
        period: f64,
        octaves: u32,
        radius: f64,
    ) -> Vec<f32> {
        classic_core::terrain::noise_fields::tiling_field(w, h, seed, period, octaves, radius)
    }

    pub fn noise_field(
        &mut self,
        w: i32,
        h: i32,
        seed: &str,
        freq_x: f64,
        freq_y: f64,
    ) -> Vec<f32> {
        classic_core::terrain::noise_fields::noise_field(w, h, seed, freq_x, freq_y)
    }

    /// Single-point raw 2D simplex sample (for non-uniform noise).
    pub fn noise2d(&mut self, seed: &str, x: f64, y: f64) -> f64 {
        classic_core::terrain::noise_fields::noise2d(seed, x, y)
    }

    // ---- Bulk terrain upload (guest generates → host stores) ---------------

    pub fn set_tiles(&mut self, tiles: &[u32]) -> i32 {
        self.engine_mut().set_tiles_bulk(tiles) as i32
    }

    pub fn set_heights(&mut self, heights: &[f32]) -> i32 {
        self.engine_mut().set_heights_bulk(heights) as i32
    }

    pub fn set_nav(&mut self, nav: &[u32]) -> i32 {
        self.engine_mut().set_nav_bulk(nav) as i32
    }

    pub fn set_tileset(&mut self, rgba: &[u8], w: u32, h: u32) -> i32 {
        self.engine_mut().set_tileset_bulk(rgba, w, h) as i32
    }

    /// Commit a guest-generated terrain (install or rebuild mesh + nav overlay).
    pub fn commit_terrain(&mut self, height_scale: f64) -> i32 {
        self.engine_mut().commit_terrain(height_scale as f32) as i32
    }

    // ---- Field-buffer registry + grid kernels (host-owned scratch) --------

    /// Allocate a zero-filled `w`×`h` field (`dtype`: 0 = f32, 1 = u32).
    pub fn alloc_field(&mut self, name: &str, w: i32, h: i32, dtype: i32) -> i32 {
        self.engine_mut().fields.alloc(name, w, h, FieldDtype::from_i32(dtype)) as i32
    }

    /// Remove a named field.
    pub fn free_field(&mut self, name: &str) -> i32 {
        self.engine_mut().fields.free(name) as i32
    }

    /// Overwrite an `f32` field's data from a guest buffer.
    pub fn write_field(&mut self, name: &str, data: &[f32]) -> i32 {
        self.engine_mut().fields.write(name, data) as i32
    }

    /// Overwrite a `u32` field's data from a guest buffer.
    pub fn write_field_u32(&mut self, name: &str, data: &[u32]) -> i32 {
        self.engine_mut().fields.write_u32(name, data) as i32
    }

    /// Download an `f32` field (empty if the field does not exist / is not f32).
    pub fn read_field(&mut self, name: &str) -> Vec<f32> {
        self.engine().fields.f32(name).map(|(d, _, _)| d.to_vec()).unwrap_or_default()
    }

    /// In-place `dst = dst op src` (`op`: 0 add, 1 sub, 2 mul, 3 min, 4 max).
    pub fn map_field(&mut self, op: i32, dst: &str, src: &str) -> i32 {
        self.engine_mut().fields.map_field(field_op(op), dst, src) as i32
    }

    /// In-place `dst = dst op scalar`.
    pub fn map_scalar(&mut self, op: i32, dst: &str, scalar: f64) -> i32 {
        self.engine_mut().fields.map_scalar(field_op(op), dst, scalar as f32) as i32
    }

    /// In-place N×N box blur of an `f32` field.
    pub fn blur_box_field(&mut self, name: &str, radius: i32) -> i32 {
        self.engine_mut().fields.blur_box(name, radius) as i32
    }

    /// In-place slope relaxation; `pinned` is an optional `u32` field name
    /// (empty string = none).  Returns the worst remaining slope.
    pub fn relax_slopes_field(
        &mut self,
        name: &str,
        max_slope: f64,
        iterations: i32,
        tolerance: f64,
        pinned: &str,
    ) -> f64 {
        let pinned = if pinned.is_empty() { None } else { Some(pinned) };
        self.engine_mut()
            .fields
            .relax_slopes(
                name,
                max_slope as f32,
                iterations.max(0) as u32,
                tolerance as f32,
                pinned,
            )
            .map(|(_, worst)| worst as f64)
            .unwrap_or(-1.0)
    }

    /// Derive a per-tile `f32` gradient field under `dst` from a vertex height
    /// field.
    pub fn gradient_magnitude_field(&mut self, heights: &str, dst: &str) -> i32 {
        self.engine_mut().fields.gradient_magnitude(heights, dst) as i32
    }

    /// Threshold an `f32` field into a `u32` field (`1` where `<= t`).
    pub fn threshold_le_field(&mut self, src: &str, dst: &str, t: f64) -> i32 {
        self.engine_mut().fields.threshold_le(src, dst, t as f32) as i32
    }

    /// Prune every walkable cell not in the largest component of a `u32` field.
    pub fn prune_components_field(&mut self, name: &str) -> i32 {
        self.engine_mut().fields.prune_components(name) as i32
    }

    /// Reduce an `f32` field (`op`: 0 min, 1 max, 2 mean, 3 variance).
    pub fn reduce_field(&mut self, name: &str, op: i32) -> f64 {
        self.engine().fields.reduce(name, reduce_op(op)).unwrap_or(f32::NAN) as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use classic_engine::Engine;

    #[test]
    fn resolve_scopes_names_to_guest_namespace() {
        let mut e = Engine::new_for_test();
        // A global entity (empty namespace, e.g. a shared dependency ROM).
        e.load_state(r#"{"entities":{"globalEnt":{"components":[]}}}"#).unwrap();
        // A namespaced ROM's entities.
        e.namespace = "scene".into();
        e.load_state(r#"{"entities":{"rocket":{"components":[]}}}"#).unwrap();

        let mut host = GuestHost::new();
        host.set_engine(&mut e);
        host.set_namespace("scene");

        // Bare names resolve in the guest's namespace first.
        assert_eq!(host.resolve("rocket"), "scene::rocket");
        // ...then fall back to the global namespace.
        assert_eq!(host.resolve("globalEnt"), "globalEnt");
        // Qualified names pass through verbatim.
        assert_eq!(host.resolve("common::tilemap"), "common::tilemap");
        // Unknown names fall back to the qualified key.
        assert_eq!(host.resolve("missing"), "scene::missing");

        // Spawn qualification prefixes the guest namespace.
        assert_eq!(host.qualify("car"), "scene::car");
        assert_eq!(host.qualify("common::tilemap"), "common::tilemap");
    }
}
