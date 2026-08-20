//! # Skill: `classic-platform`
//!
//! **Read `.claude/skills/classic-platform/SKILL.md` before working on this module.**
//!
//! web-sys WebGL2 backend for wasm32.

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{HtmlCanvasElement, WebGl2RenderingContext};

use crate::{InputState, Platform};

pub struct WebPlatform {
    canvas: HtmlCanvasElement,
    gl: Rc<glow::Context>,
    input: Rc<RefCell<InputState>>,
}

impl WebPlatform {
    pub fn new(canvas_id: &str) -> Result<Self, JsValue> {
        let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
        let document = window.document().ok_or_else(|| JsValue::from_str("no document"))?;
        let canvas = document
            .get_element_by_id(canvas_id)
            .ok_or_else(|| JsValue::from_str("no canvas element"))?;
        let canvas: HtmlCanvasElement =
            canvas.dyn_into().map_err(|_| JsValue::from_str("element is not a canvas"))?;

        let gl_opts = js_sys::Object::new();
        js_sys::Reflect::set(&gl_opts, &JsValue::from_str("stencil"), &JsValue::from_bool(true))
            .map_err(|_| JsValue::from_str("failed to set stencil context option"))?;
        let gl_ctx = canvas
            .get_context_with_context_options("webgl2", &gl_opts)
            .map_err(|_| JsValue::from_str("getContext failed"))?
            .ok_or_else(|| JsValue::from_str("webgl2 not available"))?;
        let gl_ctx: WebGl2RenderingContext =
            gl_ctx.dyn_into().map_err(|_| JsValue::from_str("not a WebGL2 context"))?;

        let gl = Rc::new(glow::Context::from_webgl2_context(gl_ctx));

        let input = Rc::new(RefCell::new(InputState::new()));

        // Sync canvas pixel dimensions to the CSS layout size
        // (must run before input handlers so mousemove captures the correct
        // canvas dimensions).
        {
            let canvas_for_resize = canvas.clone();
            let resize = || {
                let w = web_sys::window()
                    .and_then(|w| w.inner_width().ok())
                    .and_then(|v| v.as_f64())
                    .unwrap_or(1024.0) as u32;
                let h = web_sys::window()
                    .and_then(|w| w.inner_height().ok())
                    .and_then(|v| v.as_f64())
                    .unwrap_or(768.0) as u32;
                canvas_for_resize.set_width(w);
                canvas_for_resize.set_height(h);
            };
            resize();
            let c = Closure::wrap(Box::new(move || {
                let w = web_sys::window()
                    .and_then(|w| w.inner_width().ok())
                    .and_then(|v| v.as_f64())
                    .unwrap_or(1024.0) as u32;
                let h = web_sys::window()
                    .and_then(|w| w.inner_height().ok())
                    .and_then(|v| v.as_f64())
                    .unwrap_or(768.0) as u32;
                canvas_for_resize.set_width(w);
                canvas_for_resize.set_height(h);
            }) as Box<dyn FnMut()>);
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
            let cw = canvas.width() as f32;
            let ch = canvas.height() as f32;
            let inp = Rc::clone(&input);
            let c = Closure::wrap(Box::new(move |e: web_sys::MouseEvent| {
                let mut i = inp.borrow_mut();
                let s = i.mouse_sensitivity;
                if i.focused {
                    // Pointer locked: clientX/Y are frozen; accumulate
                    // movement deltas to track a virtual cursor position.
                    i.mouse_pos.x = (i.mouse_pos.x + e.movement_x() as f32 * s).clamp(0.0, cw);
                    i.mouse_pos.y = (i.mouse_pos.y + e.movement_y() as f32 * s).clamp(0.0, ch);
                } else {
                    i.mouse_pos.x = (e.client_x() as f32 * s).clamp(0.0, cw);
                    i.mouse_pos.y = (e.client_y() as f32 * s).clamp(0.0, ch);
                }
                i.mouse_axis.x = (i.mouse_pos.x / cw - 0.5) * 2.0;
                i.mouse_axis.y = (i.mouse_pos.y / ch - 0.5) * 2.0;
            }) as Box<dyn FnMut(_)>);
            canvas.add_event_listener_with_callback("mousemove", c.as_ref().unchecked_ref()).ok();
            c.forget();
        }
        {
            let ch = canvas.height() as f32;
            let inp = Rc::clone(&input);
            let c = Closure::wrap(Box::new(move |e: web_sys::WheelEvent| {
                let mut i = inp.borrow_mut();
                i.mouse_wheel -= (e.delta_y() as f32 * 2.0) / ch;
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

        Ok(Self { canvas, gl, input })
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
    fn viewport(&self) -> (f32, f32) {
        (self.canvas.width() as f32, self.canvas.height() as f32)
    }

    fn run_loop<F>(self, on_frame: F)
    where
        F: FnMut(Rc<glow::Context>, &mut InputState, f32, f32, f32, &mut bool) + 'static,
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
                let vw = platform.canvas.width() as f32;
                let vh = platform.canvas.height() as f32;
                let mut prev = pt.borrow_mut();
                let real_delta =
                    if *prev > 0.0 { ((timestamp - *prev) / 1000.0) as f32 } else { 0.016 };
                *prev = timestamp;
                (f2.borrow_mut())(gl, &mut inp, vw, vh, real_delta, &mut should_close);
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
