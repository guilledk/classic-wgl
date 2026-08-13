//! The host-side SDK: bridges guest host-imports to the engine.
//!
//! [`GuestHost`] is a thin raw-pointer bridge over [`classic_engine::Engine`].
//! It also owns the wasmi [`StoreLimits`] used to cap guest linear memory.  The
//! heavy lifting lives in safe `Engine` methods; only the pointer deref is
//! `unsafe`.

use classic_core::instrument::Chan;
use classic_engine::Engine;

/// Host state shared with the wasmi store: a pointer to the engine plus the
/// guest's resource limits.
pub struct GuestHost {
    engine: *mut Engine,
    limits: wasmi::StoreLimits,
}

impl GuestHost {
    pub(crate) fn new(limits: wasmi::StoreLimits) -> Self {
        Self { engine: std::ptr::null_mut(), limits }
    }

    /// Re-point the host at the engine for the current frame.
    pub(crate) fn set_engine(&mut self, engine: &mut Engine) {
        self.engine = engine as *mut Engine;
    }

    /// The guest resource limiter (memory cap).
    pub(crate) fn resource_limiter(&mut self) -> &mut dyn wasmi::ResourceLimiter {
        &mut self.limits
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

    /// Log a message through the `guest` CLASSIC_LOG channel.
    pub fn log(&mut self, msg: &str) {
        classic_core::cl_info!(Chan::Guest, "{}", msg);
    }

    pub fn spawn(&mut self, name: &str) -> i32 {
        self.engine_mut().spawn_named(name) as i32
    }

    pub fn despawn(&mut self, name: &str) -> i32 {
        self.engine_mut().despawn_named(name) as i32
    }

    pub fn has(&mut self, name: &str) -> i32 {
        self.engine().has_name(name) as i32
    }

    /// The ordered list of entity names, as a JSON array.
    pub fn names(&mut self) -> String {
        serde_json::to_string(&self.engine().entity_names()).unwrap_or_default()
    }

    /// Dump a named entity's components to a JSON string.
    pub fn get(&mut self, name: &str) -> String {
        self.engine().dump_entity_json(name).unwrap_or_default()
    }

    /// Dump one component of a named entity to a JSON string.
    pub fn get_comp(&mut self, name: &str, comp: &str) -> String {
        self.engine().dump_component_json(name, comp).unwrap_or_default()
    }

    /// Set a named entity's components from a JSON `{"components": [...]}` string.
    pub fn set(&mut self, name: &str, json: &str) -> i32 {
        let value: serde_json::Value = match serde_json::from_str(json) {
            Ok(v) => v,
            Err(e) => {
                classic_core::cl_error!(Chan::Guest, "set('{name}'): bad JSON: {e}");
                return 0;
            }
        };
        let Some(components) = value.get("components").and_then(|v| v.as_array()) else {
            classic_core::cl_error!(Chan::Guest, "set('{name}'): missing components array");
            return 0;
        };
        let mut ok = true;
        for comp in components {
            let Some(comp_type) = comp.get("type").and_then(|v| v.as_str()) else {
                ok = false;
                continue;
            };
            if let Err(e) = self.engine_mut().set_component_json(name, comp_type, comp.clone()) {
                classic_core::cl_error!(Chan::Guest, "set('{name}', '{comp_type}'): {e}");
                ok = false;
            }
        }
        ok as i32
    }

    /// Set one component of a named entity from a JSON string.
    pub fn set_comp(&mut self, name: &str, comp: &str, json: &str) -> i32 {
        let value: serde_json::Value = match serde_json::from_str(json) {
            Ok(v) => v,
            Err(e) => {
                classic_core::cl_error!(Chan::Guest, "set_comp('{name}', '{comp}'): bad JSON: {e}");
                return 0;
            }
        };
        match self.engine_mut().set_component_json(name, comp, value) {
            Ok(()) => 1,
            Err(e) => {
                classic_core::cl_error!(Chan::Guest, "set_comp('{name}', '{comp}'): {e}");
                0
            }
        }
    }

    pub fn set_pos(&mut self, name: &str, x: f64, y: f64, z: f64) -> i32 {
        self.engine_mut().set_pos(name, x as f32, y as f32, z as f32) as i32
    }

    pub fn get_pos(&mut self, name: &str) -> Option<(f64, f64, f64)> {
        self.engine().get_pos(name).map(|(x, y, z)| (x as f64, y as f64, z as f64))
    }

    pub fn mouse(&mut self) -> (f64, f64) {
        let p = self.engine().input.mouse_pos;
        (p.x as f64, p.y as f64)
    }

    /// The iso tile coordinates under the mouse cursor.
    pub fn mouse_iso(&mut self) -> Option<(f64, f64)> {
        self.engine().mouse_iso().map(|(x, y)| (x as f64, y as f64))
    }

    /// Terrain height (world z) at an iso tile coordinate.
    pub fn height_at(&mut self, x: f64, y: f64) -> f64 {
        self.engine().height_at(x as f32, y as f32) as f64
    }

    /// Set a named entity's animator to play a looping animation.
    pub fn set_anim(&mut self, name: &str, anim: &str) -> i32 {
        self.engine_mut().set_anim(name, anim) as i32
    }

    /// Whether the editor's agent tool is active.
    pub fn agent_selected(&mut self) -> i32 {
        self.engine().agent_selected as i32
    }

    /// Whether a UI element consumed this frame's click.
    pub fn ui_consumed_click(&mut self) -> i32 {
        self.engine().ui_consumed_click as i32
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

    /// Generate a named terrain (see `classic_core::terrain::generate`) and
    /// install or regenerate it — the generic terrain prefab.
    pub fn generate_terrain(&mut self, kind: &str, seed: &str, height_scale: f64) -> i32 {
        self.engine_mut().generate_terrain(kind, seed, height_scale as f32) as i32
    }

    /// A* path over the nav mesh from `(sx, sy)` to `(ex, ey)` as integer tile
    /// coordinates (empty if no path exists).
    pub fn find_path(&mut self, sx: i32, sy: i32, ex: i32, ey: i32) -> Vec<(i32, i32)> {
        self.engine().find_path((sx, sy), (ex, ey)).unwrap_or_default()
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

    /// The name of the top gameplay entity under a screen point (empty if none).
    pub fn pick_at(&mut self, x: f64, y: f64) -> String {
        self.engine().pick_at(x as f32, y as f32).unwrap_or_default()
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
        self.engine_mut().spawn_rect(
            name,
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
        self.engine_mut().spawn_text(
            name,
            x as f32,
            y as f32,
            text,
            scale as f32,
            [r as f32, g as f32, b as f32, a as f32],
        ) as i32
    }

    /// Update a named SDF text label's string.
    pub fn set_text(&mut self, name: &str, text: &str) -> i32 {
        self.engine_mut().set_text(name, text) as i32
    }
}
