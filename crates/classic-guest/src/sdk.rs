//! The host-side SDK: bridges guest host-imports to the engine.
//!
//! [`GuestHost`] is a thin raw-pointer bridge over [`classic_engine::Engine`],
//! shared by every runtime backend (wasmi, wasmtime).  Each runtime wraps it in
//! its own store data and owns its own resource limiter (memory cap).  The
//! heavy lifting lives in safe `Engine` methods; only the pointer deref is
//! `unsafe`.

use classic_core::components::{TextJustify, UiAlign, UiAnchor};
use classic_core::instrument::Chan;
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

/// Host state shared with every guest runtime store: a raw pointer to the
/// engine, re-pointed for each guest entry point.
pub struct GuestHost {
    engine: *mut Engine,
}

impl GuestHost {
    pub(crate) fn new() -> Self {
        Self { engine: std::ptr::null_mut() }
    }

    /// Re-point the host at the engine for the current call.
    pub(crate) fn set_engine(&mut self, engine: &mut Engine) {
        self.engine = engine as *mut Engine;
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
        self.engine_mut().set_anim(name, anim) as i32
    }

    /// Restart a named entity's animator from frame zero (optionally one-shot).
    pub fn start_anim(&mut self, name: &str, anim: &str, repeat: i32) -> i32 {
        self.engine_mut().start_anim(name, anim, repeat != 0) as i32
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

    /// Show or hide the tilemap editor grid overlay.
    pub fn set_grid(&mut self, show: i32) -> i32 {
        self.engine_mut().set_grid(show != 0);
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
        self.engine_mut().ui_container(
            name,
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
        self.engine_mut().ui_text(
            name,
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
        self.engine_mut().ui_button(
            name,
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
        self.engine_mut().ui_array(
            name,
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
        self.engine_mut().ui_padding(
            name,
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
        self.engine_mut().ui_sprite(
            name,
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
        self.engine_mut().ui_add_child(parent, child, anchor(self_anchor), anchor(child_anchor))
            as i32
    }

    pub fn ui_add_to_root(&mut self, name: &str, self_anchor: i32, child_anchor: i32) -> i32 {
        self.engine_mut().ui_add_to_root(name, anchor(self_anchor), anchor(child_anchor)) as i32
    }

    pub fn ui_set_size(&mut self, name: &str, w: f64, h: f64) -> i32 {
        self.engine_mut().ui_set_size(name, w as f32, h as f32) as i32
    }

    pub fn ui_set_anchor(&mut self, name: &str, anchor_idx: i32) -> i32 {
        self.engine_mut().ui_set_anchor(name, anchor(anchor_idx)) as i32
    }

    #[allow(clippy::too_many_arguments)]
    pub fn ui_set_color(&mut self, name: &str, r: f64, g: f64, b: f64, a: f64) -> i32 {
        self.engine_mut().ui_set_color(name, [r as f32, g as f32, b as f32, a as f32]) as i32
    }

    pub fn ui_set_fixed(&mut self, name: &str, fixed: i32) -> i32 {
        self.engine_mut().ui_set_fixed(name, fixed != 0) as i32
    }

    /// Subscribe a named entity to interaction events (click/enter/exit).
    pub fn subscribe(&mut self, name: &str) -> i32 {
        self.engine_mut().subscribe(name) as i32
    }

    /// Pop the next queued guest event, as `(kind, name)` (0=click, 1=enter, 2=exit).
    pub fn poll_event(&mut self) -> Option<(u32, String)> {
        self.engine_mut().poll_event().map(|e| (e.kind, e.name))
    }

    /// Attach an axis-aligned rectangle collider to a named entity (screen space).
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_collider(&mut self, name: &str, x: f64, y: f64, w: f64, h: f64) -> i32 {
        self.engine_mut().spawn_collider(name, x as f32, y as f32, w as f32, h as f32) as i32
    }

    /// Read a named entity's current animation name and frame.
    pub fn get_anim(&mut self, name: &str) -> Option<(String, f64)> {
        self.engine().get_anim(name).map(|(n, f)| (n, f as f64))
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
}
