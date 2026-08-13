//! The host-side SDK: bridges guest host-imports to the engine.
//!
//! [`GuestHost`] is a thin raw-pointer bridge over [`classic_engine::Engine`]
//! (the same shape as the retired rhai `GameCtx`).  It also owns the wasmi
//! [`StoreLimits`] used to cap guest linear memory.  The heavy lifting lives in
//! safe `Engine` methods; only the pointer deref is `unsafe`.

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

    pub fn set_pos(&mut self, name: &str, x: f64, y: f64) -> i32 {
        self.engine_mut().set_pos(name, x as f32, y as f32) as i32
    }

    pub fn get_pos(&mut self, name: &str) -> Option<(f64, f64)> {
        self.engine().get_pos(name).map(|(x, y)| (x as f64, y as f64))
    }

    pub fn mouse(&mut self) -> (f64, f64) {
        let p = self.engine().input.mouse_pos;
        (p.x as f64, p.y as f64)
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
}
