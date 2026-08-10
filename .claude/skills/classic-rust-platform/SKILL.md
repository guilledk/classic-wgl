---
name: classic-rust-platform
description: >
    Native (winit+glutin) and web (web-sys+trunk) platform backends for
    classic-wgl's Rust port.  Covers GL context creation, input handling,
    X11 linking on NixOS, cargo feature gating for native/wasm, frame
    delta timing, wheel handling sign inversion, mouse tracking differences,
    and the `Platform` trait contract.  Use when debugging platform-specific
    rendering differences, input weirdness, linker errors, or build failures
    that only occur on one target.
    Trigger phrases: "real delta", "wheel sign", "wheel inverted",
    "LineDelta", "X11 linking", "NixOS X11", "winit", "glutin",
    "pkg-config x11", "RUSTFLAGS", "native feature", "feature gate",
    "trunk build fails -lX11", "request_animation_frame", "cursor jitter",
    "device_event", "pointer lock", "LD_LIBRARY_PATH NixOS",
    "libxkbcommon-x11.so", "swap_buffers", "make_current".
compatibility: winit 0.30, glutin 0.32, glow 0.16, web-sys 0.3, raw-window-handle 0.6.
metadata:
    author: classic-wgl
    version: '0.1'
allowed-tools: Read, Grep, Glob, Bash(git *), Edit
---

## Scope

This skill covers the two platform backends in `crates/classic-platform/`:
- **native.rs** — winit + glutin + glow (desktop GL)
- **web.rs** — web-sys canvas + WebGL2 + trunk (browser)

Plus the `Platform` trait, `InputState`, and cargo feature gating that
prevents native libraries from leaking into wasm builds.

---

## 1. Cargo Feature Gating

The `classic-platform` crate has a `native` feature that gates
native-only dependencies.  Without this, X11/glutin build scripts
emit `cargo:rustc-link-lib=X11` directives that cargo unifies across
the workspace, causing wasm32 link failures.

```toml
# crates/classic-platform/Cargo.toml
[features]
default = []
native = ["glutin", "glutin-winit", "raw-window-handle", "x11"]

# Desktop opts in
# apps/desktop/Cargo.toml: classic-platform = { features = ["native"] }

# Web + engine opt out
# apps/web/Cargo.toml: classic-platform = { default-features = false }
# classic-engine/Cargo.toml: classic-platform = { default-features = false }
```

**Critical:** Feature unification means if ANY workspace member enables
`native`, ALL copies of `classic-platform` in the dep graph get it.
This is why `apps/desktop` must be excluded from wasm32 builds via
`[target.'cfg(not(target_arch = "wasm32"))'.dependencies]`.

The wasm-only deps (`wasm-bindgen`, `web-sys`, etc.) stay in
`[target.'cfg(target_arch = "wasm32")'.dependencies]` — these don't
emit native link flags, so unification isn't a problem.

---

## 2. X11 Linking on NixOS

On NixOS, `libX11.so` is not in standard linker paths.  The `x11`
crate emits `cargo:rustc-link-lib=X11` via its build script, but
`pkg-config` (which the build script calls) cannot find the library
without `PKG_CONFIG_PATH`.

**Solution:** A `build.rs` in `classic-platform` that calls `pkg-config`
directly (the Nix-wrapped binary, which knows about `buildInputs`)
and emits `cargo:rustc-link-search=native=...` + `cargo:rustc-link-lib=X11`.

```rust
// crates/classic-platform/build.rs
fn main() {
    let target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    if target_arch == "wasm32" { return; }  // Never emit -lX11 for wasm

    let feature_native = std::env::var("CARGO_FEATURE_NATIVE").is_ok();
    if !feature_native { return; }  // Only link X11 when native feature is enabled

    if let Ok(output) = Command::new("pkg-config").args(["--libs", "x11"]).output() {
        if output.status.success() {
            // Parse -L and -l flags from pkg-config output
            // and emit cargo:rustc-link-search + cargo:rustc-link-lib
        }
    }
}
```

**IMPORTANT**: The feature gate prevents `-lX11` from being emitted when
building `classic-platform` without the `native` feature (e.g., `cargo test
-p classic-core -p classic-engine`).  Without this gate, `cargo test` fails
on systems without X11 installed.
Use `CARGO_CFG_TARGET_ARCH`, not `#[cfg(target_arch)]`.

At runtime, `libxkbcommon-x11.so` and `libEGL.so` must be findable.
Set `LD_LIBRARY_PATH` in the nix shell:

```nix
export LD_LIBRARY_PATH="${pkgs.libxkbcommon}/lib:${pkgs.libx11}/lib:${pkgs.mesa}/lib:...
```

---

## 3. Real Frame Delta

The TS engine uses `requestAnimationFrame`'s timestamp for `deltaTime`.
We must compute real elapsed time on both platforms — hardcoded `0.016`
causes frame-rate-dependent behavior (wheel decay, zoom speed, WASD
movement all run at wrong rates on high-refresh monitors).

**Native:** `std::time::Instant::now()` in `RedrawRequested`.
```rust
let now = Instant::now();
let real_delta = (now - self.prev_time).as_secs_f32().min(0.1); // cap at 100ms
self.prev_time = now;
```

**Web:** rAF passes a `DOMHighResTimeStamp` (f64 milliseconds).
```rust
let dom_timestamp: f64 = ...;  // from rAF callback
let real_delta = if prev > 0.0 {
    ((dom_timestamp - prev) / 1000.0) as f32
} else {
    0.016  // first frame fallback
};
prev = dom_timestamp;
```

---

## 4. Wheel Handling — Sign Inversion

The native and web platforms have INVERTED scroll wheel signs by default.
This was a live bug for ~2 hours of development.

**Web** (matches TS): `e.deltaY` is positive for scroll DOWN.
```
wheel -= deltaY * 2 / viewport_h → scroll UP = positive wheel = zoom IN
```

**Native** (winit on X11): `LineDelta y` is +1 for scroll UP.
```
wheel -= dy * 2 / viewport_h   → WRONG: scroll UP = negative wheel = zoom OUT
wheel += dy * 2 / viewport_h   → CORRECT: same sign as web
```

**Rule:** Use `+=` on native, `-=` on web.  Both should result in:
`scroll away from user (up) = zoom IN, scroll toward user (down) = zoom OUT`.

---

## 5. Mouse Tracking

**Native** uses absolute `CursorMoved { position }` with
`sensitivity = 1.0`.  The position is clamped to `[0, viewport_size]`.

**Web** uses `mousemove` with `client_x()`/`client_y()` and sensitivity too.

**Key gotcha:** The web platform's mousemove handler captures `canvas.width()`
and `canvas.height()` at CREATION TIME (inside a `move` closure).  If the
resize handler runs AFTER the mousemove handler is created, the captured
dimensions are the HTML default (300×150).  **Always run the resize handler
BEFORE creating input handlers**, including the initial `resize()` call.

**Pointer lock:** Now implemented (see §9). Native: `set_cursor_visible(false)`
at window creation. Web: `canvas.requestPointerLock()` on first mousedown,
`pointerlockchange` listener syncs `InputState.focused`.
After lock, both platforms continue using absolute `clientX`/`clientY`
(position) since most browsers provide these in locked mode.

---

## 6. Platform Trait

```rust
pub trait Platform {
    type Window;
    fn window(&self) -> &Self::Window;
    fn gl_context(&self) -> &glow::Context;
    fn viewport(&self) -> (f32, f32);
    fn run_loop<F>(self, on_frame: F)
    where
        F: FnMut(Rc<glow::Context>, &mut InputState, f32, f32, f32, &mut bool) + 'static;
}
```

The callback receives `(gl, input, vw, vh, delta, should_close)`.
- `gl: Rc<glow::Context>` — shared ownership (native context isn't `Clone`)
- `input: &mut InputState` — engine writes BACK to the platform's copy
  (critical for wheel decay to persist across frames)
- `vw, vh` — actual viewport dimensions from the window/canvas
- `delta` — real frame delta (seconds)
- `should_close` — engine sets to `true` to request shutdown

---

## 7. InputState — Wheel Decay Write-Back

The engine's `frame()` method clones the input, decays the wheel on its
copy, and MUST write the decayed value back:

```rust
pub fn frame(&mut self, input: &mut InputState, ...) {
    self.input = input.clone();
    // ... update callbacks ...
    // Decay wheel
    *self.input.mouse_wheel -= 1.4 * delta * signum;
    self.input.mouse_wheel = clamp(self.input.mouse_wheel, -1, 1);
    // WRITE BACK — without this, the platform keeps the raw undecayed value
    input.mouse_wheel = self.input.mouse_wheel;
}
```

Without the write-back, a single scroll notch produces infinite zoom
(because the platform's `mouse_wheel` never decays between frames).

---

## 8. Glutin Display Creation (X11/EGL)

Creating a glutin `Display` from a winit `Window` requires extracting
the `RawDisplayHandle`.  On Linux X11, this requires opening the X11
display via `XOpenDisplay(NULL)` and constructing the handle manually.

```rust
fn raw_display_from_window(raw: RawWindowHandle) -> RawDisplayHandle {
    let dpy_ptr = unsafe { x11::xlib::XOpenDisplay(std::ptr::null()) };
    let dpy = NonNull::new(dpy_ptr).unwrap();
    match raw {
        RawWindowHandle::Xlib(_) => RawDisplayHandle::Xlib(
            XlibDisplayHandle::new(Some(dpy), 0),
        ),
        // ... similar for Xcb, Wayland
    }
}
```

This is needed because winit's `WindowHandle` wraps `raw_window_handle::WindowHandle`
but doesn't expose `HasDisplayHandle` — you cannot get the display handle through
winit's wrapper type directly.

---

## 7. CLASSIC_FRAMES Headless Mode

The desktop binary supports `CLASSIC_FRAMES=N` env var for automated
frame-limited runs (used in CI/testing):

```rust
let max_frames: Option<u64> = std::env::var("CLASSIC_FRAMES").ok()
    .and_then(|v| v.parse().ok());
let mut frame_count: u64 = 0;

platform.run_loop(move |gl, input, vw, vh, _delta, should_close| {
    if let Some(limit) = max_frames {
        if frame_count >= limit {
            *should_close = true;
            return;
        }
        frame_count += 1;
    }
});
```

**Critical:** Use `*should_close = true` — sets a `&mut bool` that
triggers `event_loop.exit()`.  Using `return` only exits the closure;
the event loop keeps running indefinitely.  The `should_close`
parameter must NOT be underscore-prefixed when used.

## 8. Web Platform Parity Notes

- Web uses `requestAnimationFrame` for frame timing; delta is
  rAF timestamp difference (same approach as native `Instant::now()`)
- GL state handling is identical (GLES 3.0 / WebGL2)
- Texture loading uses `include_bytes!` at compile time on both
  targets (no runtime fetch)
- `native` cargo feature gate prevents X11/glutin link flags from
  leaking into wasm32 builds
- Shader compilation and draw calls are identical (glow abstracts
  both backends)

## 9. Cursor Visibility & Pointer Lock

**Desktop (native):** After window creation in `resumed()`:
```rust
window.set_cursor_visible(false);  // hide OS cursor
self.input.focused = true;         // desktop is always focused
```
The engine renders a custom cursor sprite that follows `input.mouse_pos`.
The OS cursor is invisible; the custom sprite serves as the only visual
cursor.

**Web:** On first mousedown (left click), request pointer lock:
```rust
if btn == 0 && !i.focused {
    drop(i);  // release InputState borrow before calling canvas method
    canvas.request_pointer_lock();
}
```
Plus a `pointerlockchange` listener on `document`:
```rust
document.add_event_listener_with_callback("pointerlockchange", Closure::wrap(
    move || {
        let mut i = inp.borrow_mut();
        i.focused = document.pointer_lock_element()
            .map(|el| el.as_ref() == canvas_val)
            .unwrap_or(false);
    }
)).ok();
```
After pointer lock, the browser hides its cursor; our custom sprite appears.
`clientX`/`clientY` still work in locked mode on most browsers (Chrome, Firefox).

---

## 10. `InputState.focused`

```rust
pub struct InputState {
    // ... other fields ...
    pub focused: bool,
}
```

`focused` indicates whether the window/canvas has captured the mouse.
- Native: set `true` at window creation (always focused)
- Web: toggled by `pointerlockchange` listener

The engine checks `focused` before processing mouse wheel and other input.
Pointer lock is requested on the first mousedown when `!focused`.
