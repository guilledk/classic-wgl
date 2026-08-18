---
name: classic-platform
description: >
    Platform backends and input handling for classic-wgl's Rust port.
    Covers native (winit+glutin), web (web-sys+trunk), and headless
    (EGL) backends, the Platform trait, InputState, mouse/keyboard
    events, frame timing, and build configuration.
    Trigger phrases: "Platform trait", "native", "winit", "web-sys",
    "headless", "EGL", "InputState", "mouse_wheel", "wheel sign",
    "CLASSIC_FRAMES", "CLASSIC_HEADLESS", "pointer lock", "build.rs".
---

# Classic Platform — Backends and Input Handling

## 1. Platform Trait

The `Platform` trait abstracts window creation, GL context, and the event loop:

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

The callback receives `(gl, input, vw, vh, delta, should_close)`. The platform
takes ownership and drives the event loop — `run_loop` is the terminal call
that blocks until the window closes. Three backends implement this trait:
`NativePlatform`, `WebPlatform`, `HeadlessPlatform`.

The trait also exposes `window()` and `gl_context()` accessors, but only
`WebPlatform` implements them for real. `NativePlatform` and `HeadlessPlatform`
return `unimplemented!()`. In practice, the engine only interacts through
`run_loop`. These accessors exist for potential headless FBO capture or toolkit
integration but are dead code in the current main loop path.

## 2. Native Backend (NativePlatform)

File: `crates/classic-platform/src/native.rs`

Gated behind `#[cfg(feature = "native")]` and `#[cfg(not(target_arch =
"wasm32"))]`. Uses `winit` for windowing and `glutin` for GL context creation.

### GL window creation (`create_gl_window`)
1. Extracts `RawWindowHandle` from the winit `Window`
2. On Linux, calls `XOpenDisplay(nullptr)` via raw FFI to obtain an X11 display
   pointer (needed because glutin's `DisplayApiPreference::Egl` requires the
   native display handle)
3. Creates a `glutin::Display` with EGL preference
4. Selects a framebuffer config, creates a `WindowSurface`, creates a GL context
   with no version constraints (the system default), and calls `make_current`
5. Wraps the context in `glow::Context` using `get_proc_address` for extension
   loading

### Event loop
The native backend uses `winit::event_loop::EventLoop::run_app()` with an
`ApplicationHandler` impl. Key event handling:

- **RedrawRequested**: Computes `real_delta = (now - prev_time).as_secs_f32()`
  clamped to 0.1s max. Calls `on_frame`. Then `input.end_frame()`, `swap_buffers()`,
  `request_redraw()` (continuous rendering).

- **CursorMoved**: Clamps `mouse_pos` to viewport bounds, computes
  `mouse_axis` as normalized device coordinates `[-1, 1]`.

- **MouseInput**: Left=idx 0, Right=idx 1, Middle=idx 2. Press sets both
  `mouse_down` and `mouse_pressed`. Left press also sets `frame_had_click = true`.

- **MouseWheel**: `LineDelta(_, y)` is multiplied by 50.0 (winit's arbitrary
  LineDelta unit → pixel equivalent). `PixelDelta(p)` uses `p.y` directly.
  Both are accumulated into `mouse_wheel += dy * 2.0 / viewport.height`.

- **KeyboardInput**: Key events use `PhysicalKey::Code` (not `NamedKey`).
  The key string is `format!("{code:?}")`. Both press and release set state.

- **Resized**: Calls `surface.resize()` with the new dimensions.

- **Focused**: Updates `input.focused`.

### X11 linking
The `build.rs` script in `classic-platform` uses `pkg-config --libs x11` to
discover library paths. If `pkg-config` fails, it falls back to
`cargo:rustc-link-lib=X11` without a search path. On NixOS, `pkg-config` must
be able to find x11 (typically provided via `buildInputs` in the dev shell or
`nativeBuildInputs` with `pkg-config`).

## 3. Web Backend (WebPlatform)

File: `crates/classic-platform/src/web.rs`

Gated behind `#[cfg(target_arch = "wasm32")]`. Uses `web-sys` for DOM access
and `trunk` for the build pipeline.

### Initialization (`WebPlatform::new(canvas_id)`)
1. Gets the `<canvas>` element by ID
2. Obtains a `WebGL2RenderingContext`
3. Wraps it in `glow::Context::from_webgl2_context()`
4. Creates `Rc<RefCell<InputState>>` shared across all event closures
5. Sets canvas pixel dimensions to `window.innerWidth/innerHeight`
6. Registers a `resize` listener that updates canvas dimensions

### Input event wiring
All event closures use `Rc<RefCell<InputState>>` for shared mutable state. Each
closure is created with `Closure::wrap`, registered with
`add_event_listener_with_callback`, and `.forget()`-ted to prevent GC.

- **mousedown**: Sets `mouse_down` and `mouse_pressed`. On left-click while not
  focused, calls `request_pointer_lock()` and sets `frame_had_click = true`.

- **mouseup**: Sets `mouse_down = false`, `mouse_released = true`.

- **mousemove**: Updates `mouse_pos` clamped to canvas dimensions, computes
  `mouse_axis` in `[-1, 1]`. NOTE: The `cw`/`ch` values are captured from
  `canvas.width/height` at the time the closure is created. The resize handler
  updates the canvas element but does NOT update the closures' captured
  variables. After a resize, mouse position clamping may use stale dimensions.

- **wheel**: Accumulates `mouse_wheel -= e.delta_y() * 2.0 / canvas_height`.
  Note the sign: web wheel delta is inverted relative to native (scroll-up
  produces positive delta_y on native but negative on web before the
  subtraction flips it).

- **keydown/keyup**: Sets `keys_down` / `keys_pressed` / `keys_released` using
  `e.code()`.

- **pointerlockchange**: Syncs `input.focused` by checking if the canvas is the
  `document.pointerLockElement`.

### Animation frame loop
The `run_loop` implementation uses `request_animation_frame` recursively. It
stores `prev_time` to compute `real_delta` in seconds. On the first frame
(`prev_time = 0`), delta defaults to 0.016 (16ms, ~60fps). The web backend
does NOT respect `CLASSIC_FRAMES`.

## 4. Headless Backend (HeadlessPlatform)

File: `crates/classic-platform/src/headless.rs`

Gated behind `#[cfg(feature = "native")]` (not `wasm32`). Activated by
`CLASSIC_HEADLESS=1`. Creates a surfaceless EGL context via a pbuffer surface.

### Initialization
1. Dynamically loads `libEGL.so.1` via `libloading::Library`
2. Resolves EGL function pointers through type transmutation (all `unsafe`)
3. Creates a display with `EGL_DEFAULT_DISPLAY`, initializes EGL
4. Selects a config with `EGL_PBUFFER_BIT`, 8-bit RGBA, 16-bit depth
5. Creates a GL 3.x context
6. Creates a pbuffer surface at the requested width/height
7. Makes the context current, wraps in `glow::Context`

### Event loop
Does NOT use window events. Runs a `while !should_close` loop with
`instant.now()` timing. The loop:

1. Computes `delta = (now - prev_time).as_secs_f32().min(0.1)`
2. Calls `on_frame()` with a shared `InputState` (no real input)
3. Calls `input.end_frame()`

### CLASSIC_FRAMES
When the `CLASSIC_FRAMES` env var is set to a number, the loop exits after that
many frames. Critically, `should_close` is forced `false` until the frame limit
is reached — this overrides any test/golden completion signal, ensuring the
golden capture runs after the test scenario completes. On reaching the limit,
it logs the exit via `cl_info!(Chan::Platform, ...)`.

`CLASSIC_OFFSCREEN` is implied in headless mode — the engine will render to an
FBO.

## 5. InputState

```rust
pub struct InputState {
    pub mouse_pos: Vec2,        // screen-space, clamped to viewport
    pub mouse_axis: Vec2,       // normalized [-1, 1]
    pub mouse_wheel: f32,       // accumulated, decays per frame
    pub mouse_down: [bool; 3],  // currently held
    pub mouse_pressed: [bool; 3],  // just pressed this frame
    pub mouse_released: [bool; 3], // just released this frame
    pub keys_down: HashMap<String, bool>,
    pub keys_pressed: HashMap<String, bool>,
    pub keys_released: HashMap<String, bool>,
    pub mouse_sensitivity: f32, // always 1.0
    pub focused: bool,          // window/pointer-lock focused
    pub frame_had_click: bool,  // true if any click occurred this frame
}
```

Key method: `end_frame()` clears `mouse_pressed`, `mouse_released`,
`keys_pressed`, and `keys_released` arrays/maps. This is called at the end of
each frame by the platform's run_loop closure. NOTE: `frame_had_click` is never
reset in `end_frame()` — it persists once set until some other code clears it
(externally).

## 6. Mouse Wheel Handling

Both native and web backends accumulate wheel movements into
`input.mouse_wheel`. The native backend adds (positive = scroll up), while the
web backend subtracts (which flips the web convention to match native).

After each frame's callbacks run, the engine applies wheel decay:
```rust
mw = (mw.abs() - 1.4 * delta).max(0.0) * mw.signum();
mw = mw.clamp(-1.0, 1.0);
```

This produces a smooth decay to zero after wheel input stops. The value is
written back to the platform's `InputState` after decay, so it persists across
frames.

The engine's mouse wheel routing precedes `on_update` closures. If the mouse is
over the text demo panel and `editor_target == "textDemo"`, the wheel scrolls
the text content and zeros `mouse_wheel` so the camera zoom closure doesn't
also fire.

`LineDelta` values are multiplied by 50.0 before accumulation. This scale
factor was chosen to match platform wheel sensitivity differences. Different
OSes/window systems produce different LineDelta magnitudes, but the decay+clamp
normalizes behaviour across platforms.

## 7. Frame Timing

### Real delta

- **Native**: `(instant.now() - prev_time).as_secs_f32().min(0.1)`. The 0.1 cap
  prevents huge delta spikes on window focus changes.

- **Web**: `(timestamp - prev_time) / 1000.0`. On first frame (prev_time=0),
  defaults to 0.016 (16ms). Subsequent frames use real rAF timestamps.

- **Headless**: Same as native — monotonic clock, capped at 0.1.

### CLASSIC_FIXED_DT

When `CLASSIC_FIXED_DT` is set, the engine overrides the real delta with the
value from the env var. When `CLASSIC_TEST` is active, fixed delta auto-defaults
to `1.0/60.0` if not explicitly set. This ensures deterministic test behaviour.

### Headless timing

Headless runs without a display refresh sync, so frames execute as fast as
possible. `CLASSIC_FRAMES` provides the only exit mechanism. The tight loop
means real delta is often sub-millisecond; `CLASSIC_FIXED_DT` is essential
for deterministic headless golden runs.

## 8. Build Configuration

### Feature gating
The `classic-platform` crate uses `#[cfg(feature = "native")]` to gate the
`native` and `headless` modules. The web module is gated by
`#[cfg(target_arch = "wasm32")]`. In practice:
- `classic-desktop` depends on `classic-platform` with `features = ["native"]`
- `classic-web` depends on `classic-platform` without `native` (wasm32 target)

### build.rs
The `classic-platform/build.rs` script only runs when NOT on wasm32 AND the
`native` feature is enabled. It queries `pkg-config --libs x11` to find
X11 library paths and outputs `cargo:rustc-link-search` and
`cargo:rustc-link-lib` directives. Falls back to bare `-lX11` if pkg-config
fails.

### Asset loading
The `AssetLoader` trait abstracts filesystem vs embedded loading. On native,
assets are loaded from the filesystem (or `include_bytes!` at the desktop app
level). On web, assets are pre-loaded and stored in the wasm binary via
`include_bytes!` in the web crate. The trait provides `load_bytes(path) →
AssetBytes` and `load_string(path) → String`. `AssetBytes` is an enum that can
hold either owned `Vec<u8>` or borrowed `&'static [u8]`.

## 9. Known-divergent / non-functional

- **`NativePlatform::window()` and `gl_context()`** — Both return
  `unimplemented!()`. These `Platform` trait methods are never called because
  `run_loop` takes ownership of the platform and captures everything needed
  before the frame callback. They exist to satisfy the trait contract but
  would panic if invoked.

- **Web resize and mouse position** — The `mousemove` closure captures
  `cw`/`ch` from the canvas dimensions at construction time. The `resize`
  listener updates the canvas element's pixel size but does not update the
  closures' captured variables. After a browser resize, mouse position
  clamping uses stale canvas dimensions until page reload.

- **`frame_had_click` persistence** — Set on `mousedown` but never cleared by
  `end_frame()`. External code (the engine's click dispatch logic) must
  explicitly reset it. If an engine path forgets to clear it, a single click
  can be interpreted as a click on subsequent frames.

- **`mouse_sensitivity`** — Initialized to 1.0 and never changed. The field
  exists in the struct but no code path sets it to a different value. This
  was intended for mouse speed configuration but is dead code.

- **`HeadlessPlatform`'s `Window` type** — Associated type is `()`, and
  `window()` returns `unimplemented!()`. The pbuffer surface is stored as a
  raw `EGLSurface` pointer in a private field — there is no winit `Window` to
  return.

- **Wheel sign on native vs web** — The native backend adds to `mouse_wheel`
  (positive = scroll up). The web backend subtracts from `mouse_wheel`
  (flipping web's convention). Both approaches intend to produce "positive =
  scroll up" semantics, but the web delta sign convention varies across
  browsers. The `* 2.0 / vh` scaling also differs (native uses viewport height
  from the active window; web uses the initial canvas height as a constant).

- **Keyboard key format** — Native uses `format!("{key_code:?}")` which
  produces strings like `"KeyW"`, `"Space"`, `"Escape"`. Web uses
  `e.code()` which produces strings like `"KeyW"`, `"Space"`, `"Escape"`.
  These SHOULD match between backends, but winit's `PhysicalKey::Code`
  debugging format and web's `KeyboardEvent.code` may differ for some keys
  (e.g. numpad, media keys).
