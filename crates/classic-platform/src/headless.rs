//! Surfaceless EGL renderer — no window, no compositor.
//!
//! Used for CI golden-test runs and headless debugging with
//! CLASSIC_HEADLESS=1.  Dynamically loads libEGL at startup,
//! creates a surfaceless context, and renders every frame into
//! an offscreen FBO (CLASSIC_OFFSCREEN is implied).

use std::ffi::c_void;
use std::ffi::CString;
use std::rc::Rc;
use std::time::Instant;

use crate::{InputState, Platform};
use glow::HasContext;

type EGLDisplay = *mut c_void;
type EGLConfig = *mut c_void;
type EGLContext = *mut c_void;
type EGLSurface = *mut c_void;

const EGL_DEFAULT_DISPLAY: EGLDisplay = 0 as EGLDisplay;
#[allow(dead_code)]
const EGL_NO_SURFACE: EGLSurface = 0 as EGLSurface;
const EGL_NO_CONTEXT: EGLContext = 0 as EGLContext;

const EGL_RENDERABLE_TYPE: i32 = 0x3040;
const EGL_OPENGL_BIT: i32 = 0x0001;
const EGL_RED_SIZE: i32 = 0x3024;
const EGL_GREEN_SIZE: i32 = 0x3023;
const EGL_BLUE_SIZE: i32 = 0x3022;
const EGL_ALPHA_SIZE: i32 = 0x3021;
const EGL_DEPTH_SIZE: i32 = 0x3025;
const EGL_NONE: i32 = 0x3038;
const EGL_CONTEXT_MAJOR_VERSION: i32 = 0x3098;
const EGL_SURFACE_TYPE: i32 = 0x3033;
const EGL_PBUFFER_BIT: i32 = 0x0001;
const EGL_WIDTH: i32 = 0x3057;
const EGL_HEIGHT: i32 = 0x3056;

// These cannot be safe Rust types, so we transmute from
// `unsafe extern "C" fn() -> *mut c_void` to the real signature.
type VoidFn = unsafe extern "C" fn() -> *mut c_void;

macro_rules! load_fn {
    ($lib:expr, $name:expr, $ret:ty, $($arg:ty),*) => {{
        let sym = $lib
            .get::<VoidFn>($name)
            .map_err(|e| format!("{}: {e}", std::str::from_utf8($name).unwrap_or("?")))?;
        std::mem::transmute::<VoidFn, unsafe extern "C" fn($($arg),*) -> $ret>(*sym)
    }};
}

pub struct HeadlessPlatform {
    gl: Rc<glow::Context>,
    width: u32,
    height: u32,
    _egl_lib: libloading::Library,
    _display: EGLDisplay,
    _context: EGLContext,
    _surface: EGLSurface,
}

impl HeadlessPlatform {
    pub fn new(width: u32, height: u32) -> Result<Self, String> {
        unsafe {
            let lib =
                libloading::Library::new("libEGL.so.1").map_err(|e| format!("load libEGL: {e}"))?;

            let egl_get_display = load_fn!(lib, b"eglGetDisplay\x00", EGLDisplay, EGLDisplay);
            let egl_initialize =
                load_fn!(lib, b"eglInitialize\x00", i32, EGLDisplay, *mut i32, *mut i32);
            let egl_get_error = load_fn!(lib, b"eglGetError\x00", u32,);
            let egl_choose_config = load_fn!(
                lib,
                b"eglChooseConfig\x00",
                i32,
                EGLDisplay,
                *const i32,
                *mut EGLConfig,
                i32,
                *mut i32
            );
            let egl_create_context = load_fn!(
                lib,
                b"eglCreateContext\x00",
                EGLContext,
                EGLDisplay,
                EGLConfig,
                EGLContext,
                *const i32
            );
            let egl_create_pbuffer_surface = load_fn!(
                lib,
                b"eglCreatePbufferSurface\x00",
                EGLSurface,
                EGLDisplay,
                EGLConfig,
                *const i32
            );
            let egl_make_current = load_fn!(
                lib,
                b"eglMakeCurrent\x00",
                i32,
                EGLDisplay,
                EGLSurface,
                EGLSurface,
                EGLContext
            );
            let egl_get_proc_address =
                load_fn!(lib, b"eglGetProcAddress\x00", *mut c_void, *const i8);

            let error_str = |code: u32| match code {
                0x3000 => "EGL_SUCCESS",
                0x3001 => "EGL_NOT_INITIALIZED",
                0x3002 => "EGL_BAD_ACCESS",
                0x3003 => "EGL_BAD_ALLOC",
                0x3004 => "EGL_BAD_ATTRIBUTE",
                0x3005 => "EGL_BAD_CONFIG",
                0x3006 => "EGL_BAD_CONTEXT",
                0x3008 => "EGL_BAD_DISPLAY",
                0x300D => "EGL_BAD_MATCH",
                0x300E => "EGL_BAD_NATIVE_PIXMAP",
                0x300F => "EGL_BAD_NATIVE_WINDOW",
                0x3011 => "EGL_BAD_SURFACE",
                _ => "<unknown>",
            };

            macro_rules! egl_check {
                ($call:expr, $label:expr) => {
                    if $call == 0 {
                        let code = egl_get_error();
                        return Err(format!(
                            "{} failed: {} (0x{:X})",
                            $label,
                            error_str(code),
                            code
                        ));
                    }
                };
            }

            let display = egl_get_display(EGL_DEFAULT_DISPLAY);
            if display.is_null() {
                return Err("eglGetDisplay returned NULL".into());
            }
            let mut major = 0i32;
            let mut minor = 0i32;
            egl_check!(egl_initialize(display, &mut major, &mut minor), "eglInitialize");
            classic_core::cl_info!(
                classic_core::instrument::Chan::Platform,
                "headless: EGL {major}.{minor}"
            );

            let config_attribs = [
                EGL_SURFACE_TYPE,
                EGL_PBUFFER_BIT,
                EGL_RENDERABLE_TYPE,
                EGL_OPENGL_BIT,
                EGL_RED_SIZE,
                8,
                EGL_GREEN_SIZE,
                8,
                EGL_BLUE_SIZE,
                8,
                EGL_ALPHA_SIZE,
                8,
                EGL_DEPTH_SIZE,
                16,
                EGL_NONE,
            ];
            let mut config: EGLConfig = std::ptr::null_mut();
            let mut num_configs = 0i32;
            egl_check!(
                egl_choose_config(
                    display,
                    config_attribs.as_ptr(),
                    &mut config,
                    1,
                    &mut num_configs
                ),
                "eglChooseConfig"
            );
            if num_configs == 0 {
                return Err("no EGL config found".into());
            }

            let ctx_attribs = [EGL_CONTEXT_MAJOR_VERSION, 3, EGL_NONE];
            let context = egl_create_context(display, config, EGL_NO_CONTEXT, ctx_attribs.as_ptr());
            if context.is_null() {
                return Err("eglCreateContext failed".into());
            }

            let pbuffer_attribs = [EGL_WIDTH, width as i32, EGL_HEIGHT, height as i32, EGL_NONE];
            let surface = egl_create_pbuffer_surface(display, config, pbuffer_attribs.as_ptr());
            if surface.is_null() {
                return Err("eglCreatePbufferSurface failed".into());
            }
            egl_check!(egl_make_current(display, surface, surface, context), "eglMakeCurrent");

            let gl = {
                let proc_addr = move |name: &str| -> *const c_void {
                    let cname = CString::new(name).unwrap();
                    egl_get_proc_address(cname.as_ptr())
                };
                glow::Context::from_loader_function(proc_addr)
            };

            let ver = gl.version();
            classic_core::cl_info!(
                classic_core::instrument::Chan::Platform,
                "headless: GL vendor={} version={}.{}",
                ver.vendor_info,
                ver.major,
                ver.minor,
            );

            Ok(Self {
                gl: Rc::new(gl),
                width,
                height,
                _egl_lib: lib,
                _display: display,
                _context: context,
                _surface: surface,
            })
        }
    }
}

impl Platform for HeadlessPlatform {
    type Window = ();

    fn window(&self) -> &() {
        unimplemented!()
    }

    fn gl_context(&self) -> &glow::Context {
        &self.gl
    }

    fn viewport(&self) -> (f32, f32) {
        (self.width as f32, self.height as f32)
    }

    fn run_loop<F>(self, mut on_frame: F)
    where
        F: FnMut(Rc<glow::Context>, &mut InputState, f32, f32, f32, &mut bool) + 'static,
    {
        let max_frames: Option<u64> =
            std::env::var("CLASSIC_FRAMES").ok().and_then(|v| v.parse().ok());
        let mut frame_count: u64 = 0;
        let mut input = InputState::new();
        let mut should_close = false;
        let mut prev_time = Instant::now();
        let w = self.width as f32;
        let h = self.height as f32;
        let gl = self.gl.clone();

        while !should_close {
            let now = Instant::now();
            let delta = (now - prev_time).as_secs_f32().min(0.1);
            prev_time = now;

            on_frame(gl.clone(), &mut input, w, h, delta, &mut should_close);
            input.end_frame();

            if let Some(limit) = max_frames {
                if frame_count >= limit {
                    classic_core::cl_info!(
                        classic_core::instrument::Chan::Platform,
                        "headless: CLASSIC_FRAMES={limit} reached"
                    );
                    should_close = true;
                } else {
                    // Override any test/golden prompt to close — CLASSIC_FRAMES
                    // controls exit timing so golden capture runs after test completion.
                    should_close = false;
                }
                frame_count += 1;
            }
        }
    }
}
