//! classic-platform: Windowing, GL context, and asset I/O.
//!
//! Two backends:
//! - Native:  winit + glutin + glow (desktop GL)
//! - Web:     winit-canvas + web-sys + glow (WebGL2)

use glam::Vec2;
use std::rc::Rc;

// ---------------------------------------------------------------------------
// Input state
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct InputState {
    pub mouse_pos: Vec2,
    pub mouse_axis: Vec2,
    pub mouse_wheel: f32,
    pub mouse_down: [bool; 3],
    pub mouse_pressed: [bool; 3],
    pub mouse_released: [bool; 3],
    pub keys_down: std::collections::HashMap<String, bool>,
    pub keys_pressed: std::collections::HashMap<String, bool>,
    pub keys_released: std::collections::HashMap<String, bool>,
    pub mouse_sensitivity: f32,
    pub focused: bool,
    pub frame_had_click: bool,
}

impl InputState {
    pub fn new() -> Self {
        Self { mouse_sensitivity: 1.0, ..Default::default() }
    }

    pub fn is_mouse_down(&self, button: usize) -> bool {
        self.mouse_down.get(button).copied().unwrap_or(false)
    }

    pub fn was_mouse_pressed(&self, button: usize) -> bool {
        self.mouse_pressed.get(button).copied().unwrap_or(false)
    }

    pub fn was_mouse_released(&self, button: usize) -> bool {
        self.mouse_released.get(button).copied().unwrap_or(false)
    }

    pub fn is_key_down(&self, code: &str) -> bool {
        self.keys_down.get(code).copied().unwrap_or(false)
    }

    pub fn was_key_pressed(&self, code: &str) -> bool {
        self.keys_pressed.get(code).copied().unwrap_or(false)
    }

    pub fn was_key_released(&self, code: &str) -> bool {
        self.keys_released.get(code).copied().unwrap_or(false)
    }

    /// Reset per-frame press/release flags (called at end of frame).
    pub fn end_frame(&mut self) {
        self.mouse_pressed = [false; 3];
        self.mouse_released = [false; 3];
        self.keys_pressed.clear();
        self.keys_released.clear();
    }
}

// ---------------------------------------------------------------------------
// Platform trait
// ---------------------------------------------------------------------------

/// The platform needs to be constructable, then yields a window + GL context.
///
/// The `run` method takes control of the event loop and calls the provided
/// closure each frame with the current GL context and input state.
pub trait Platform {
    type Window;

    fn window(&self) -> &Self::Window;
    fn gl_context(&self) -> &glow::Context;
    fn viewport(&self) -> (f32, f32);

    /// Run the main event loop.
    /// The callback receives `(gl, input, vw, vh, delta, should_close)`.
    fn run_loop<F>(self, on_frame: F)
    where
        F: FnMut(Rc<glow::Context>, &mut InputState, f32, f32, f32, &mut bool) + 'static;
}

// ---------------------------------------------------------------------------
// Asset source (abstracts filesystem vs. fetch)
// ---------------------------------------------------------------------------

pub enum AssetBytes {
    Owned(Vec<u8>),
    Borrowed(&'static [u8]),
}

impl std::ops::Deref for AssetBytes {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            AssetBytes::Owned(v) => v,
            AssetBytes::Borrowed(b) => b,
        }
    }
}

pub trait AssetLoader {
    /// Load a raw byte blob from a path (e.g. "/res/sprite.png" or
    /// "/manifest.json").  On native this is `std::fs::read`.
    fn load_bytes(&self, path: &str) -> anyhow::Result<AssetBytes>;
    /// Load a UTF-8 string from a path.
    fn load_string(&self, path: &str) -> anyhow::Result<String> {
        let b = self.load_bytes(path)?;
        Ok(String::from_utf8(b.to_vec())?)
    }
}

// ---------------------------------------------------------------------------
// Native backend  (not wasm32)
// ---------------------------------------------------------------------------

#[cfg(feature = "native")]
pub mod native;

#[cfg(feature = "native")]
pub use native::NativePlatform;

#[cfg(feature = "native")]
pub mod headless;

#[cfg(feature = "native")]
pub use headless::HeadlessPlatform;

// ---------------------------------------------------------------------------
// Web backend  (wasm32)
// ---------------------------------------------------------------------------

#[cfg(target_arch = "wasm32")]
pub mod web;

#[cfg(target_arch = "wasm32")]
pub use web::WebPlatform;
