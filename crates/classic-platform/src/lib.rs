//! # Skill: `classic-platform`
//!
//! **Read `.agents/skills/classic-platform/SKILL.md` before working on this module.**
//!
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
// Boot timing
// ---------------------------------------------------------------------------

/// A platform-neutral monotonic boot timer.
///
/// `std::time::Instant` panics on `wasm32-unknown-unknown` ("time not
/// implemented on this platform"), so boot timing uses this: [`std::time::Instant`]
/// on native, `js_sys::Date::now()` (milliseconds since the Unix epoch) on web.
#[derive(Clone, Copy, Debug)]
pub struct BootTimer {
    #[cfg(not(target_arch = "wasm32"))]
    start: std::time::Instant,
    #[cfg(target_arch = "wasm32")]
    start: f64,
}

impl BootTimer {
    pub fn start() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        {
            Self { start: std::time::Instant::now() }
        }
        #[cfg(target_arch = "wasm32")]
        {
            Self { start: js_sys::Date::now() }
        }
    }

    pub fn elapsed_secs(&self) -> f32 {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.start.elapsed().as_secs_f32()
        }
        #[cfg(target_arch = "wasm32")]
        {
            ((js_sys::Date::now() - self.start) / 1000.0) as f32
        }
    }

    pub fn elapsed_ms(&self) -> u128 {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.start.elapsed().as_millis()
        }
        #[cfg(target_arch = "wasm32")]
        {
            (js_sys::Date::now() - self.start) as u128
        }
    }

    pub fn elapsed_duration(&self) -> std::time::Duration {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.start.elapsed()
        }
        #[cfg(target_arch = "wasm32")]
        {
            std::time::Duration::from_secs_f64((js_sys::Date::now() - self.start) / 1000.0)
        }
    }
}

// ---------------------------------------------------------------------------
// Display scale (logical points vs physical pixels)
// ---------------------------------------------------------------------------

/// The smallest display scale a backend accepts (a smaller or non-finite
/// value from a display, `CLASSIC_PIXEL_RATIO` or `?pixel_ratio=` is
/// clamped up to it / replaced by 1).
pub const MIN_PIXEL_RATIO: f32 = 0.5;

/// Sanitise a display scale: non-finite or non-positive -> 1, else at least
/// [`MIN_PIXEL_RATIO`].
pub fn clamp_pixel_ratio(ratio: f64) -> f32 {
    if ratio.is_finite() && ratio > 0.0 {
        (ratio as f32).max(MIN_PIXEL_RATIO)
    } else {
        1.0
    }
}

/// A frame's view size.  The engine works in **logical points**: `width` /
/// `height`, the mouse position and every layout / camera / pick
/// coordinate.  The drawable is `pixel_ratio` physical pixels per point
/// (`physical()`), so a 4K screen at scale 2 renders a 1920x1080-point view
/// at full resolution instead of a half-size one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameSize {
    pub width: f32,
    pub height: f32,
    pub pixel_ratio: f32,
}

impl FrameSize {
    /// The logical size of a `pw`x`ph` physical drawable at `pixel_ratio`
    /// (sanitised through [`clamp_pixel_ratio`]).
    pub fn from_physical(pw: u32, ph: u32, pixel_ratio: f32) -> Self {
        let r = clamp_pixel_ratio(pixel_ratio as f64);
        Self { width: pw as f32 / r, height: ph as f32 / r, pixel_ratio: r }
    }

    /// A logical size at scale 1 (headless tests, the old callers).
    pub fn logical(width: f32, height: f32) -> Self {
        Self { width, height, pixel_ratio: 1.0 }
    }

    /// The physical drawable size in pixels (logical × ratio, rounded).
    pub fn physical(&self) -> (u32, u32) {
        (to_physical(self.width, self.pixel_ratio), to_physical(self.height, self.pixel_ratio))
    }
}

/// A logical length as whole physical pixels (rounded; recovers the exact
/// drawable size [`FrameSize::from_physical`] divided out).
pub fn to_physical(logical: f32, pixel_ratio: f32) -> u32 {
    (logical * pixel_ratio).round().max(0.0) as u32
}

/// A physical-pixel position (a window event) as logical points.
pub fn to_logical(physical: Vec2, pixel_ratio: f32) -> Vec2 {
    physical / pixel_ratio
}

/// Set `input`'s mouse position from a logical-point position, clamped to the
/// `w`x`h` logical view, and derive `mouse_axis` (`[-1, 1]`).
pub fn set_mouse_pos(input: &mut InputState, pos: Vec2, w: f32, h: f32) {
    let (w, h) = (w.max(1.0), h.max(1.0));
    input.mouse_pos = Vec2::new(pos.x.clamp(0.0, w), pos.y.clamp(0.0, h));
    input.mouse_axis =
        Vec2::new((input.mouse_pos.x / w - 0.5) * 2.0, (input.mouse_pos.y / h - 0.5) * 2.0);
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
    /// The current view size in logical points (plus its pixel ratio).
    fn viewport(&self) -> FrameSize;

    /// Run the main event loop.
    /// The callback receives `(gl, input, size, delta, should_close)`, with
    /// `size` and `input`'s mouse in logical points (see [`FrameSize`]).
    fn run_loop<F>(self, on_frame: F)
    where
        F: FnMut(Rc<glow::Context>, &mut InputState, FrameSize, f32, &mut bool) + 'static;
}

#[cfg(test)]
mod scale_tests {
    use super::*;

    #[test]
    fn physical_to_logical_at_common_ratios() {
        for (r, lw, lh) in [(1.0, 2560.0, 1440.0), (1.5, 2560.0 / 1.5, 960.0), (2.0, 1280.0, 720.0)]
        {
            let s = FrameSize::from_physical(2560, 1440, r);
            assert_eq!(s.pixel_ratio, r);
            assert!((s.width - lw).abs() < 1e-3 && (s.height - lh).abs() < 1e-3, "{r}: {s:?}");
            assert_eq!(s.physical(), (2560, 1440), "round trip at {r}");
        }
        // 4K at 1.5 has a fractional logical width; the drawable is exact.
        let s = FrameSize::from_physical(3840, 2160, 1.5);
        assert_eq!((s.width, s.height), (2560.0, 1440.0));
        let s = FrameSize::from_physical(1921, 1081, 1.5);
        assert_eq!(s.physical(), (1921, 1081));
    }

    #[test]
    fn pixel_ratio_is_sanitised() {
        assert_eq!(clamp_pixel_ratio(2.0), 2.0);
        assert_eq!(clamp_pixel_ratio(0.25), MIN_PIXEL_RATIO);
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(clamp_pixel_ratio(bad), 1.0, "{bad}");
        }
    }

    #[test]
    fn mouse_converts_to_clamped_logical_points() {
        let mut input = InputState::new();
        for (r, phys, want) in [
            (1.0, Vec2::new(640.0, 360.0), Vec2::new(640.0, 360.0)),
            (1.5, Vec2::new(960.0, 540.0), Vec2::new(640.0, 360.0)),
            (2.0, Vec2::new(1280.0, 720.0), Vec2::new(640.0, 360.0)),
        ] {
            set_mouse_pos(&mut input, to_logical(phys, r), 1280.0, 720.0);
            assert_eq!(input.mouse_pos, want, "{r}");
            assert_eq!(input.mouse_axis, Vec2::ZERO, "{r}");
        }
        set_mouse_pos(&mut input, to_logical(Vec2::new(9000.0, -5.0), 2.0), 1280.0, 720.0);
        assert_eq!(input.mouse_pos, Vec2::new(1280.0, 0.0));
        assert_eq!(input.mouse_axis, Vec2::new(1.0, -1.0));
    }
}

// ---------------------------------------------------------------------------
// Asset source (abstracts filesystem vs. fetch)
// ---------------------------------------------------------------------------

// The `AssetLoader` abstraction lives in `classic-rom` (the resource
// foundation crate) so `ResourceSet` can build from either a `RomArchive` or
// an `AssetLoader` without pulling in a platform/GL dependency.  Re-export
// the surface here for the existing `classic_platform::AssetLoader` callers.
#[cfg(not(target_arch = "wasm32"))]
pub use classic_rom::FsAssetLoader;
pub use classic_rom::{AssetBytes, AssetLoader, EmbeddedAssetLoader};

/// Longest real frame delta (seconds) a backend hands the frame callback.
/// A slower frame (a stall, a backgrounded tab resuming, a software
/// rasteriser) slows the game down instead of taking one huge step: the
/// engine's explicit integrators (vehicle springs, animation, particles)
/// are tuned for ~60 Hz steps.  Every backend clamps through
/// [`frame_delta`], so native, headless and web agree.
pub const MAX_FRAME_DT: f32 = 0.1;

/// Clamp a measured frame delta to `[0, MAX_FRAME_DT]` (a non-finite delta
/// counts as zero).
pub fn frame_delta(seconds: f32) -> f32 {
    if seconds.is_finite() {
        seconds.clamp(0.0, MAX_FRAME_DT)
    } else {
        0.0
    }
}

// ---------------------------------------------------------------------------
// ROM source resolution (named registry / file path / URL)
// ---------------------------------------------------------------------------

pub mod rom;
pub use rom::LogBootSink;
#[cfg(not(target_arch = "wasm32"))]
pub use rom::{load_rom_bytes, resolve_rom, resolve_roms};
#[cfg(target_arch = "wasm32")]
pub use rom::{load_rom_bytes_async, resolve_rom_async, resolve_roms_async};
pub use rom::{parse_rom_selector, resolve_rom_source};

// ---------------------------------------------------------------------------
// Boot resource sampling (native sysinfo CPU+RSS, web JS-heap memory)
// ---------------------------------------------------------------------------

pub mod metrics;
pub use metrics::ResourceUsageSampler;

// ---------------------------------------------------------------------------
// Native backend  (not wasm32)
// ---------------------------------------------------------------------------

#[cfg(feature = "native")]
pub mod native;

#[cfg(feature = "native")]
pub use native::NativePlatform;

// Headless EGL needs no window system (libEGL is `dlopen`ed at runtime), so
// it is built on every native target, `native` feature or not: engine tests
// use it for a real GL context and skip when libEGL is absent.
#[cfg(not(target_arch = "wasm32"))]
pub mod headless;

#[cfg(not(target_arch = "wasm32"))]
pub use headless::HeadlessPlatform;

// ---------------------------------------------------------------------------
// Web backend  (wasm32)
// ---------------------------------------------------------------------------

#[cfg(target_arch = "wasm32")]
pub mod web;

#[cfg(target_arch = "wasm32")]
pub use web::WebPlatform;
