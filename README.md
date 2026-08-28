# classic-wgl

A small isometric game engine with a retained-mode UI/layout layer, written in
Rust.  Two targets: **native** (winit + glutin, desktop GL) and **web**
(web-sys + trunk, WebGL 2).  There is no framework — the whole app is a single
`<canvas>` / winit window.

Game content ships as self-contained **ROMs**: a zip archive bundling a
per-scene manifest, entity state, resources (textures, fonts, animations, and
binary tile/nav/height grids), and a compiled WASM guest module.  See
`AGENTS.md` for the full architecture.

## Develop (nix)

```bash
nix develop                 # dev shell (Rust toolchain, wasm target, GL/EGL deps)

# fetch the published scene ROMs (demo/lunar) into roms/out/
cargo xtask fetch-roms

cargo run -p classic-desktop          # native, interactive
trunk serve apps/web/index.html       # web dev server
trunk build apps/web/index.html --release  # web release
```

## Testing

```bash
cargo test                    # all unit/integration tests
cargo fmt --all -- --check    # formatting
cargo clippy --all-targets -- -D warnings
```

The headless e2e + golden harness runs under `LIBGL_ALWAYS_SOFTWARE=1` and
`EGL_PLATFORM=surfaceless` — see `AGENTS.md` for the exact invocations and the
`CLASSIC_*` environment-variable reference.

## GLSL resources

https://learnwebgl.brown37.net/12_shader_language/documents/webgl-reference-card-1_0.pdf
https://learnwebgl.brown37.net/12_shader_language/glsl_mathematical_operations.html
https://gist.github.com/patriciogonzalezvivo/670c22f3966e662d2f83
