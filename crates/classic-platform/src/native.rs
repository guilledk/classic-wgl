//! # Skill: `classic-platform`
//!
//! **Read `.agents/skills/classic-platform/SKILL.md` before working on this module.**
//!
//! winit + glutin native backend.

use std::ffi::CString;
use std::num::NonZeroU32;
use std::ptr::NonNull;
use std::rc::Rc;
use std::time::Instant;

use glutin::context::PossiblyCurrentContext;
use glutin::display::Display;
use glutin::display::DisplayApiPreference;
use glutin::prelude::*;
use glutin::surface::{Surface, SurfaceAttributesBuilder, WindowSurface};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::PhysicalKey;
use winit::raw_window_handle::{HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use winit::window::{Window, WindowAttributes};

use crate::{FrameSize, InputState, Platform};

struct GlWindow {
    #[allow(dead_code)]
    display: Display,
    surface: Surface<WindowSurface>,
    context: PossiblyCurrentContext,
    gl: Rc<glow::Context>,
}

/// The winit window backend.  Its frames are in logical points: the window
/// opens at 1280x720 points, and each frame's [`FrameSize`] and mouse
/// position divide the physical surface by the display scale
/// (`Window::scale_factor`, tracked through `ScaleFactorChanged`) unless
/// [`NativePlatform::with_pixel_ratio`] pins one.
#[derive(Default)]
pub struct NativePlatform {
    pixel_ratio: Option<f32>,
}

impl NativePlatform {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pin the pixel ratio (`CLASSIC_PIXEL_RATIO`, or 1 for a deterministic
    /// test run) instead of following the display's scale factor; `None`
    /// follows the display.
    pub fn with_pixel_ratio(mut self, pixel_ratio: Option<f32>) -> Self {
        self.pixel_ratio = pixel_ratio.map(|r| crate::clamp_pixel_ratio(r as f64));
        self
    }
}

impl Platform for NativePlatform {
    type Window = Window;

    fn window(&self) -> &Window {
        unimplemented!()
    }

    fn gl_context(&self) -> &glow::Context {
        unimplemented!()
    }

    fn viewport(&self) -> FrameSize {
        FrameSize::logical(1280.0, 720.0)
    }

    fn run_loop<F>(self, on_frame: F)
    where
        F: FnMut(Rc<glow::Context>, &mut InputState, FrameSize, f32, &mut bool) + 'static,
    {
        let el = winit::event_loop::EventLoop::new().expect("event loop");
        let mut app = NativeApp {
            state: None,
            input: InputState::new(),
            on_frame,
            init: false,
            prev_time: Instant::now(),
            pinned_ratio: self.pixel_ratio,
        };
        let _ = el.run_app(&mut app);
    }
}

struct NativeWindowState {
    window: Window,
    glw: GlWindow,
    /// Logical points -> physical pixels: the pinned ratio, else the
    /// window's scale factor (updated on `ScaleFactorChanged`).
    pixel_ratio: f32,
}

struct NativeApp<F> {
    state: Option<NativeWindowState>,
    input: InputState,
    on_frame: F,
    init: bool,
    prev_time: Instant,
    pinned_ratio: Option<f32>,
}

impl<F> ApplicationHandler for NativeApp<F>
where
    F: FnMut(Rc<glow::Context>, &mut InputState, FrameSize, f32, &mut bool) + 'static,
{
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if !self.init {
            self.init = true;
            let window = event_loop
                .create_window(
                    WindowAttributes::default()
                        .with_title("classic-wgl")
                        .with_inner_size(winit::dpi::LogicalSize::new(1280, 720)),
                )
                .expect("create window");
            window.set_cursor_visible(false);
            self.input.focused = true;
            let glw = create_gl_window(&window);
            let pixel_ratio = self
                .pinned_ratio
                .unwrap_or_else(|| crate::clamp_pixel_ratio(window.scale_factor()));
            classic_core::cl_info!(
                classic_core::instrument::Chan::Platform,
                "native: scale factor {} -> pixel ratio {pixel_ratio}{}",
                window.scale_factor(),
                if self.pinned_ratio.is_some() { " (pinned)" } else { "" }
            );
            self.state = Some(NativeWindowState { window, glw, pixel_ratio });
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        let pinned = self.pinned_ratio.is_some();
        let Some(state) = self.state.as_mut() else { return };
        let glw = &state.glw;
        let input = &mut self.input;

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                // winit resizes the window to the new scale; `Resized`
                // follows with the new physical size.
                if !pinned {
                    state.pixel_ratio = crate::clamp_pixel_ratio(scale_factor);
                }
            }
            WindowEvent::Resized(size) => {
                let w = NonZeroU32::new(size.width.max(1)).unwrap();
                let h = NonZeroU32::new(size.height.max(1)).unwrap();
                glw.surface.resize(&glw.context, w, h);
            }
            WindowEvent::RedrawRequested => {
                let mut should_close = false;
                let view = frame_size(state);
                let now = Instant::now();
                let real_delta = crate::frame_delta((now - self.prev_time).as_secs_f32());
                self.prev_time = now;
                (self.on_frame)(glw.gl.clone(), input, view, real_delta, &mut should_close);
                if should_close {
                    event_loop.exit();
                }
                input.end_frame();
                state.window.request_redraw();
                glw.surface.swap_buffers(&glw.context).expect("swap buffers");
            }
            WindowEvent::CursorMoved { position, .. } => {
                // Physical window pixels -> logical points.
                let s = input.mouse_sensitivity;
                let view = frame_size(state);
                let phys = glam::Vec2::new(position.x as f32, position.y as f32);
                let pos = crate::to_logical(phys, view.pixel_ratio) * s;
                crate::set_mouse_pos(input, pos, view.width, view.height);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let idx = match button {
                    winit::event::MouseButton::Left => 0,
                    winit::event::MouseButton::Right => 1,
                    winit::event::MouseButton::Middle => 2,
                    _ => return,
                };
                match state {
                    ElementState::Pressed => {
                        input.mouse_down[idx] = true;
                        input.mouse_pressed[idx] = true;
                        if idx == 0 {
                            input.frame_had_click = true;
                        }
                    }
                    ElementState::Released => {
                        input.mouse_down[idx] = false;
                        input.mouse_released[idx] = true;
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let dy = match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => y * 50.0,
                    winit::event::MouseScrollDelta::PixelDelta(p) => p.y as f32 / state.pixel_ratio,
                };
                input.mouse_wheel += dy * 2.0 / frame_size(state).height.max(1.0);
            }
            WindowEvent::KeyboardInput {
                event: KeyEvent { physical_key: PhysicalKey::Code(code), state, .. },
                ..
            } => {
                let key_str = format!("{code:?}");
                match state {
                    ElementState::Pressed => {
                        input.keys_down.insert(key_str.clone(), true);
                        input.keys_pressed.insert(key_str, true);
                    }
                    ElementState::Released => {
                        input.keys_down.insert(key_str.clone(), false);
                        input.keys_released.insert(key_str, true);
                    }
                }
            }
            WindowEvent::Focused(focused) => input.focused = focused,
            _ => {}
        }
    }
}

/// The window's current size in logical points.
fn frame_size(state: &NativeWindowState) -> FrameSize {
    let s = state.window.inner_size();
    FrameSize::from_physical(s.width, s.height, state.pixel_ratio)
}

/// Extract the display handle from a raw window handle.
fn raw_display_from_window(raw: RawWindowHandle) -> RawDisplayHandle {
    #[cfg(target_os = "linux")]
    {
        let dpy_ptr = unsafe { x11::xlib::XOpenDisplay(std::ptr::null()) } as *mut std::ffi::c_void;
        if dpy_ptr.is_null() {
            panic!("XOpenDisplay returned NULL");
        }
        let dpy = NonNull::new(dpy_ptr).unwrap();

        match raw {
            RawWindowHandle::Xlib(_) => RawDisplayHandle::Xlib(
                winit::raw_window_handle::XlibDisplayHandle::new(Some(dpy), 0),
            ),
            RawWindowHandle::Xcb(_) => {
                RawDisplayHandle::Xcb(winit::raw_window_handle::XcbDisplayHandle::new(Some(dpy), 0))
            }
            RawWindowHandle::Wayland(_) => {
                RawDisplayHandle::Wayland(winit::raw_window_handle::WaylandDisplayHandle::new(dpy))
            }
            _ => panic!("unsupported window handle for linux: {raw:?}"),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = raw;
        panic!("native GL platform not yet implemented for this OS")
    }
}

fn create_gl_window(window: &Window) -> GlWindow {
    let raw_handle = window.window_handle().expect("window handle").as_raw();
    let raw_display = raw_display_from_window(raw_handle);
    let display =
        unsafe { Display::new(raw_display, DisplayApiPreference::Egl) }.expect("create display");

    let template = glutin::config::ConfigTemplateBuilder::new().with_stencil_size(8).build();
    let config = unsafe { display.find_configs(template) }
        .expect("find configs")
        .next()
        .expect("no GL config");

    let inner = window.inner_size();
    let attrs = SurfaceAttributesBuilder::<WindowSurface>::new().build(
        raw_handle,
        NonZeroU32::new(inner.width.max(1)).unwrap(),
        NonZeroU32::new(inner.height.max(1)).unwrap(),
    );
    let surface =
        unsafe { display.create_window_surface(&config, &attrs) }.expect("create surface");

    let context = unsafe {
        display
            .create_context(&config, &glutin::context::ContextAttributesBuilder::new().build(None))
            .expect("create context")
            .make_current(&surface)
            .expect("make current")
    };

    let gl = unsafe {
        glow::Context::from_loader_function(|name| {
            display.get_proc_address(&CString::new(name).unwrap()).cast()
        })
    };

    GlWindow { display, surface, context, gl: Rc::new(gl) }
}
