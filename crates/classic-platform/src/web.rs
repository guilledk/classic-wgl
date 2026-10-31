//! # Skill: `classic-platform`
//!
//! **Read `.agents/skills/classic-platform/SKILL.md` before working on this module.**
//!
//! web-sys WebGL2 backend for wasm32.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{HtmlCanvasElement, WebGl2RenderingContext};

use crate::{FrameSize, InputState, Platform, WebScale};

pub struct WebPlatform {
    canvas: HtmlCanvasElement,
    gl: Rc<glow::Context>,
    /// The raw context behind `gl` (for APIs glow does not cover).
    raw: WebGl2RenderingContext,
    input: Rc<RefCell<InputState>>,
    /// `window.devicePixelRatio` (the backing-store resolution) and the UI
    /// scale (the pinned ratio, else the dpr), re-read on every `resize`.
    scale: Rc<Cell<WebScale>>,
}

/// The canvas's current view: its backing store is `CSS size × dpr`
/// physical pixels, its logical size the backing store / UI scale.
fn canvas_size(canvas: &HtmlCanvasElement, scale: WebScale) -> FrameSize {
    scale.frame_size(canvas.width(), canvas.height())
}

impl WebPlatform {
    /// Bind the canvas `canvas_id`.  `pixel_ratio` pins the UI scale
    /// (`?pixel_ratio=`, physical pixels per logical point, like native
    /// `CLASSIC_PIXEL_RATIO`); `None` follows `window.devicePixelRatio`.
    /// The backing store always follows `devicePixelRatio` (see
    /// [`WebScale`]).
    pub fn new(canvas_id: &str, pixel_ratio: Option<f32>) -> Result<Self, JsValue> {
        let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
        let document = window.document().ok_or_else(|| JsValue::from_str("no document"))?;
        let canvas = document
            .get_element_by_id(canvas_id)
            .ok_or_else(|| JsValue::from_str("no canvas element"))?;
        let canvas: HtmlCanvasElement =
            canvas.dyn_into().map_err(|_| JsValue::from_str("element is not a canvas"))?;

        // `stencil` for the sprite ghost groups.  `antialias: false` keeps
        // the default framebuffer single-sampled like native (whose GL
        // config requests no MSAA): the browser default (`true`) makes it
        // multisampled, which the half-res particle pass cannot blit depth
        // from, so particles fell back to full resolution on the web only.
        // `depth` stays at its default (`true`); with `stencil` that is a
        // `DEPTH24_STENCIL8` buffer, the format the particle pass blits into.
        let gl_opts = js_sys::Object::new();
        for (key, on) in [("stencil", true), ("antialias", false)] {
            js_sys::Reflect::set(&gl_opts, &JsValue::from_str(key), &JsValue::from_bool(on))
                .map_err(|_| JsValue::from_str("failed to set a context option"))?;
        }
        let gl_ctx = canvas
            .get_context_with_context_options("webgl2", &gl_opts)
            .map_err(|_| JsValue::from_str("getContext failed"))?
            .ok_or_else(|| JsValue::from_str("webgl2 not available"))?;
        let gl_ctx: WebGl2RenderingContext =
            gl_ctx.dyn_into().map_err(|_| JsValue::from_str("not a WebGL2 context"))?;

        let raw = gl_ctx.clone();
        let gl = Rc::new(glow::Context::from_webgl2_context(gl_ctx));

        let input = Rc::new(RefCell::new(InputState::new()));

        // Size the canvas backing store to `CSS size × devicePixelRatio`
        // (the CSS size stays the window's inner size, pinned on the
        // element's style so a larger backing store never grows the
        // layout); the UI scale only divides the logical view out of it.
        // Re-run on `resize`, which also fires when the browser zoom or the
        // display's devicePixelRatio changes.
        let pinned = pixel_ratio.map(|r| crate::clamp_pixel_ratio(r as f64));
        let scale = Rc::new(Cell::new(WebScale::new(1.0, pinned)));
        {
            let canvas_for_resize = canvas.clone();
            let scale_for_resize = Rc::clone(&scale);
            let resize = move || {
                let Some(w) = web_sys::window() else { return };
                let css_w = w.inner_width().ok().and_then(|v| v.as_f64()).unwrap_or(1024.0);
                let css_h = w.inner_height().ok().and_then(|v| v.as_f64()).unwrap_or(768.0);
                let s = WebScale::new(w.device_pixel_ratio(), pinned);
                scale_for_resize.set(s);
                let style = canvas_for_resize.style();
                let _ = style.set_property("width", &format!("{css_w}px"));
                let _ = style.set_property("height", &format!("{css_h}px"));
                let (bw, bh) = s.backing_size(css_w as f32, css_h as f32);
                canvas_for_resize.set_width(bw);
                canvas_for_resize.set_height(bh);
            };
            resize();
            {
                let s = scale.get();
                let view = canvas_size(&canvas, s);
                classic_core::cl_info!(
                    classic_core::instrument::Chan::Platform,
                    "web: devicePixelRatio {}, ui scale {}{}: canvas {}x{} px, view {:.1}x{:.1} pt",
                    s.dpr,
                    s.ui_scale,
                    if pinned.is_some() { " (pinned)" } else { "" },
                    canvas.width(),
                    canvas.height(),
                    view.width,
                    view.height
                );
            }
            let c = Closure::wrap(Box::new(resize) as Box<dyn FnMut()>);
            window.add_event_listener_with_callback("resize", c.as_ref().unchecked_ref()).ok();
            c.forget();
        }

        // Wire up input events.
        {
            let canvas = canvas.clone();
            let canvas4lock = canvas.clone();
            let inp = Rc::clone(&input);
            let c = Closure::wrap(Box::new(move |e: web_sys::MouseEvent| {
                let mut i = inp.borrow_mut();
                let btn = e.button() as usize;
                i.mouse_down[btn.min(2)] = true;
                i.mouse_pressed[btn.min(2)] = true;
                if btn == 0 {
                    i.frame_had_click = true;
                    if !i.focused {
                        drop(i);
                        canvas4lock.request_pointer_lock();
                    }
                }
                e.prevent_default();
            }) as Box<dyn FnMut(_)>);
            canvas.add_event_listener_with_callback("mousedown", c.as_ref().unchecked_ref()).ok();
            c.forget();
        }
        {
            let canvas = canvas.clone();
            let inp = Rc::clone(&input);
            let c = Closure::wrap(Box::new(move |e: web_sys::MouseEvent| {
                let mut i = inp.borrow_mut();
                let btn = e.button() as usize;
                i.mouse_down[btn.min(2)] = false;
                i.mouse_released[btn.min(2)] = true;
            }) as Box<dyn FnMut(_)>);
            canvas.add_event_listener_with_callback("mouseup", c.as_ref().unchecked_ref()).ok();
            c.forget();
        }
        {
            // `clientX/Y` and the pointer-lock `movementX/Y` are CSS pixels:
            // × dpr / UI scale makes them logical points (a no-op unpinned).
            // The bounds are read live so a resize never leaves a stale clamp.
            let canvas_for_move = canvas.clone();
            let scale = Rc::clone(&scale);
            let inp = Rc::clone(&input);
            let c = Closure::wrap(Box::new(move |e: web_sys::MouseEvent| {
                let mut i = inp.borrow_mut();
                let s = i.mouse_sensitivity;
                let ws = scale.get();
                let view = canvas_size(&canvas_for_move, ws);
                let pos = if i.focused {
                    // Pointer locked: clientX/Y are frozen; accumulate
                    // movement deltas to track a virtual cursor position.
                    let d = glam::Vec2::new(e.movement_x() as f32, e.movement_y() as f32);
                    i.mouse_pos + ws.css_to_logical(d) * s
                } else {
                    ws.css_to_logical(glam::Vec2::new(e.client_x() as f32, e.client_y() as f32)) * s
                };
                crate::set_mouse_pos(&mut i, pos, view.width, view.height);
            }) as Box<dyn FnMut(_)>);
            canvas.add_event_listener_with_callback("mousemove", c.as_ref().unchecked_ref()).ok();
            c.forget();
        }
        {
            // `deltaY` is CSS pixels: made logical like the cursor, so a
            // notch moves the same fraction of the view at any UI scale
            // (native divides `PixelDelta` by its ratio the same way).
            let canvas_for_wheel = canvas.clone();
            let scale = Rc::clone(&scale);
            let inp = Rc::clone(&input);
            let c = Closure::wrap(Box::new(move |e: web_sys::WheelEvent| {
                let mut i = inp.borrow_mut();
                let ws = scale.get();
                let ch = canvas_size(&canvas_for_wheel, ws).height.max(1.0);
                let dy = ws.css_to_logical(glam::Vec2::new(0.0, e.delta_y() as f32)).y;
                i.mouse_wheel -= (dy * 2.0) / ch;
                e.prevent_default();
            }) as Box<dyn FnMut(_)>);
            canvas.add_event_listener_with_callback("wheel", c.as_ref().unchecked_ref()).ok();
            c.forget();
        }
        {
            let inp = Rc::clone(&input);
            let c = Closure::wrap(Box::new(move |e: web_sys::KeyboardEvent| {
                let mut i = inp.borrow_mut();
                let code = e.code();
                i.keys_down.insert(code.clone(), true);
                i.keys_pressed.insert(code, true);
            }) as Box<dyn FnMut(_)>);
            window.add_event_listener_with_callback("keydown", c.as_ref().unchecked_ref()).ok();
            c.forget();
        }
        {
            let inp = Rc::clone(&input);
            let c = Closure::wrap(Box::new(move |e: web_sys::KeyboardEvent| {
                let mut i = inp.borrow_mut();
                let code = e.code();
                i.keys_down.insert(code.clone(), false);
                i.keys_released.insert(code, true);
            }) as Box<dyn FnMut(_)>);
            window.add_event_listener_with_callback("keyup", c.as_ref().unchecked_ref()).ok();
            c.forget();
        }
        // Pointer lock change listener — syncs focused state with browser lock.
        {
            let inp = Rc::clone(&input);
            let canvas_for_lock = canvas.clone();
            let c = Closure::wrap(Box::new(move || {
                let mut i = inp.borrow_mut();
                i.focused = web_sys::window()
                    .and_then(|w| w.document())
                    .and_then(|doc| doc.pointer_lock_element())
                    .map(|el| {
                        let el_val: &wasm_bindgen::JsValue = el.as_ref();
                        let canvas_val: &wasm_bindgen::JsValue = canvas_for_lock.as_ref();
                        el_val == canvas_val
                    })
                    .unwrap_or(false);
            }) as Box<dyn FnMut()>);
            document
                .add_event_listener_with_callback("pointerlockchange", c.as_ref().unchecked_ref())
                .ok();
            c.forget();
        }

        Ok(Self { canvas, gl, raw, input, scale })
    }

    /// The WebGL2 context, cloned for async engine bootstrap before `run_loop`.
    pub fn gl(&self) -> Rc<glow::Context> {
        self.gl.clone()
    }

    /// The raw `WebGl2RenderingContext` behind [`Self::gl`] (the frame
    /// profiler reads its timer queries through it).
    pub fn webgl2(&self) -> &WebGl2RenderingContext {
        &self.raw
    }

    /// The shared input state (keydown/keyup listeners installed in `new`),
    /// cloned so the async boot loop can poll for an Esc abort before
    /// `run_loop` starts.
    pub fn input(&self) -> Rc<RefCell<InputState>> {
        self.input.clone()
    }
}

impl Platform for WebPlatform {
    type Window = HtmlCanvasElement;

    fn window(&self) -> &HtmlCanvasElement {
        &self.canvas
    }
    fn gl_context(&self) -> &glow::Context {
        &self.gl
    }
    fn viewport(&self) -> FrameSize {
        canvas_size(&self.canvas, self.scale.get())
    }

    fn run_loop<F>(self, on_frame: F)
    where
        F: FnMut(Rc<glow::Context>, &mut InputState, FrameSize, f32, &mut bool) + 'static,
    {
        let platform = Rc::new(RefCell::new(self));

        let p = Rc::clone(&platform);
        let f = Rc::new(RefCell::new(on_frame));
        let cb: Rc<RefCell<Option<Closure<dyn FnMut(f64)>>>> = Rc::new(RefCell::new(None));

        let p2 = Rc::clone(&p);
        let f2 = Rc::clone(&f);
        let cb2 = Rc::clone(&cb);
        let prev_time: Rc<RefCell<f64>> = Rc::new(RefCell::new(0.0));
        let pt = Rc::clone(&prev_time);

        let closure: Closure<dyn FnMut(f64)> = Closure::new(Box::new(move |timestamp: f64| {
            let platform = p2.borrow();
            let gl = platform.gl.clone();
            let mut should_close = false;
            {
                let mut inp = platform.input.borrow_mut();
                let view = platform.viewport();
                let mut prev = pt.borrow_mut();
                // Clamped like native/headless: an unclamped rAF gap (a
                // slow software-GL frame, a tab resuming) was one huge step.
                let real_delta = if *prev > 0.0 {
                    crate::frame_delta(((timestamp - *prev) / 1000.0) as f32)
                } else {
                    0.016
                };
                *prev = timestamp;
                (f2.borrow_mut())(gl, &mut inp, view, real_delta, &mut should_close);
                inp.end_frame();
            }
            if should_close {
                return;
            }

            if let Some(c) = cb2.borrow().as_ref() {
                web_sys::window()
                    .and_then(|w| w.request_animation_frame(c.as_ref().unchecked_ref()).ok())
                    .unwrap();
            }
        }));

        *cb.borrow_mut() = Some(closure);

        // Kick off
        if let Some(c) = cb.borrow().as_ref() {
            web_sys::window()
                .and_then(|w| w.request_animation_frame(c.as_ref().unchecked_ref()).ok())
                .unwrap();
        }

        std::mem::forget(platform);
        std::mem::forget(f);
        std::mem::forget(cb);
    }
}
