# Contributing to classic-wgl

Thanks for taking a look at `classic-wgl`. This is a small, hobby-scale
isometric game engine — the process is intentionally lightweight.

## Getting set up

```bash
git clone <repo-url>
cd classic-wgl
git submodule update --init   # private assets repo (needs a repo-scoped token)
nix develop                   # Rust toolchain + wasm target + GL/EGL deps
cargo xtask all               # build the scene ROMs into roms/out/
```

See `AGENTS.md` for a full architecture/conventions overview before making
non-trivial changes — it covers the ECS, the component registry, the guest
runtime, the ROM format, and the code style used throughout.

## Before opening a PR

Run the same checks CI runs (`.github/workflows/ci.yml`):

```bash
cargo fmt --all -- --check
cargo clippy -p classic-core -p classic-gfx -p classic-engine -p classic-platform \
  -p classic-rom -p classic-guest -p classic-demo --all-targets -- -D warnings
cargo test
```

Rust is formatted with `cargo fmt` (default style, width 100 via
`rustfmt.toml`).  Clippy runs strict (`-D warnings`).  Match the existing
conventions: crate-prefixed imports (`classic_core::`, …), `snake_case`
fields, `PascalCase` types, prefab initializers named `init_*()`.

## Branching / commits

- Base all work on `master` (the default branch); CI runs on pushes to
  `master` and on every PR.
- Keep commit messages short, lowercase, and imperative (e.g. `fix nav
  walkability transpose`) — this repo does not use Conventional Commits
  prefixes.
- Prefer small, focused commits over one large commit per feature.

## Adding a new component

Components are plain structs in `crates/classic-core/src/components/mod.rs`
that derive `Serialize + Deserialize`.  To make one serializable into a ROM:

1. Register it in `register_all_components()` (`crates/classic-core/src/lib.rs`)
   with a `ComponentReg` entry (name, `spawn` from JSON, optional `dump`, and
   `subsumes` fan-out de-duplication).
2. Add a `#[cfg(test)]` round-trip case under `crates/classic-core/tests/`
   if it has non-trivial state.

## Reporting issues

Open a GitHub issue with steps to reproduce, expected vs. actual behavior,
and, if relevant, browser/OS info (this is a WebGL2 app so GPU driver quirks
matter).
